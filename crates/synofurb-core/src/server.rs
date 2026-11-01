//! The small server the NAS talks back to during the install:
//! * HTTP: `GET /<token>/{stage2.sh,payload.tar.gz,preseed.cfg}` (token = 128 random bits)
//! * offline mode: read-only `GET|HEAD /<token>/mirror/<path>` serving an offline bundle's `mirror/` directory
//! * TCP log stream (stage-2 pipes its log here) and UDP syslog (Debian installer `log_host`)

use crate::logs::{Logger, Source};
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// URL segment below `/<token>/` under which an offline bundle's mirror is published.
pub const OFFLINE_MIRROR_ROUTE: &str = "mirror";

pub struct Served {
    /// Large files (installer kernel/initrd) served at `/<token>/files/<name>`, streamed from disk.
    pub files: std::collections::HashMap<String, std::path::PathBuf>,
    pub token: String,
    pub stage2: String,
    pub payload: Vec<u8>,
    pub preseed: String,
    /// Offline mode: the bundle's `mirror/` directory, served read-only at `/<token>/mirror/<path>`.
    pub mirror: Option<std::path::PathBuf>,
}

pub struct Server {
    pub http_port: u16,
    pub tcp_log_port: u16,
    pub udp_log_port: u16,
    stop: Arc<AtomicBool>,
    http: Arc<tiny_http::Server>,
    threads: Vec<JoinHandle<()>>,
}

/// Which of our addresses does `peer` see? (UDP connect picks the outgoing interface, sends nothing.)
pub fn local_ip_towards(peer: IpAddr) -> Result<IpAddr> {
    let bind = if peer.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let s = UdpSocket::bind(bind)?;
    s.connect(SocketAddr::new(peer, 9))?;
    Ok(s.local_addr()?.ip())
}

impl Server {
    /// Binds to `bind_ip` on ephemeral ports (or the given ones when non-zero).
    pub fn start(bind_ip: IpAddr, served: Served, log: Logger, ports: (u16, u16, u16)) -> Result<Server> {
        let http = Arc::new(
            tiny_http::Server::http(SocketAddr::new(bind_ip, ports.0))
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("binding HTTP server")?,
        );
        let http_port = http.server_addr().to_ip().map(|a| a.port()).context("no http port")?;
        let tcp = TcpListener::bind(SocketAddr::new(bind_ip, ports.1)).context("binding TCP log port")?;
        let tcp_log_port = tcp.local_addr()?.port();
        let udp = UdpSocket::bind(SocketAddr::new(bind_ip, ports.2)).context("binding UDP syslog port")?;
        let udp_log_port = udp.local_addr()?.port();
        udp.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
        tcp.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = vec![];

        // HTTP
        {
            let (http, stop, log) = (http.clone(), stop.clone(), log.clone());
            let served = Arc::new(served);
            let mirror = served.mirror.clone().map(|root| Arc::new(MirrorState::new(root)));
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok(Some(req)) = http.recv_timeout(std::time::Duration::from_millis(500)) else { continue };
                    let url = req.url().to_string();
                    let peer = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
                    let prefix = format!("/{}/", served.token);
                    if let Some(rest) = url.strip_prefix(&format!("{prefix}{OFFLINE_MIRROR_ROUTE}/")) {
                        match &mirror {
                            // package downloads run on their own thread so a slow NAS never blocks stage 2 / log traffic
                            Some(m) if m.try_acquire() => {
                                let (m, log, rest) = (m.clone(), log.clone(), rest.to_string());
                                std::thread::spawn(move || {
                                    m.serve(req, &rest, &peer, &log);
                                    m.release();
                                });
                            }
                            Some(_) => {
                                let _ = req.respond(tiny_http::Response::from_string("busy").with_status_code(503));
                            }
                            None => {
                                let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                            }
                        }
                        continue;
                    }
                    if let Some(name) = url.strip_prefix(&format!("{prefix}files/")) {
                        match served.files.get(name).and_then(|p| std::fs::File::open(p).ok()) {
                            Some(f) => {
                                log.info(format!("HTTP {peer} GET files/{name}"));
                                let _ = req.respond(tiny_http::Response::from_file(f));
                            }
                            None => {
                                log.warn(format!("HTTP {peer} rejected request for unknown file '{name}'"));
                                let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                            }
                        }
                        continue;
                    }
                    let resp = match url.strip_prefix(&prefix) {
                        Some("stage2.sh") => Some((served.stage2.clone().into_bytes(), "text/x-shellscript")),
                        Some("preseed.cfg") => Some((served.preseed.clone().into_bytes(), "text/plain")),
                        Some("payload.tar.gz") => Some((served.payload.clone(), "application/gzip")),
                        _ => None,
                    };
                    match resp {
                        Some((body, ct)) => {
                            log.info(format!("HTTP {peer} GET {} ({} bytes)", url.strip_prefix(&prefix).unwrap_or(""), body.len()));
                            let _ = req.respond(
                                tiny_http::Response::from_data(body)
                                    .with_header(tiny_http::Header::from_bytes("Content-Type", ct).unwrap()),
                            );
                        }
                        None => {
                            log.warn(format!("HTTP {peer} rejected request for an unknown path"));
                            let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                        }
                    }
                }
            }));
        }
        // TCP log stream: one thread per connection
        {
            let (stop, log) = (stop.clone(), log.clone());
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match tcp.accept() {
                        Ok((conn, addr)) => {
                            log.info(format!("installer log stream connected from {addr}"));
                            let log = log.clone();
                            std::thread::spawn(move || {
                                let _ = conn.set_nonblocking(false);
                                for line in BufReader::new(conn).split(b'\n').map_while(Result::ok) {
                                    log.installer_line(Source::Installer, &String::from_utf8_lossy(&line));
                                }
                                log.info("installer log stream closed");
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(200)),
                        Err(_) => break,
                    }
                }
            }));
        }
        // UDP syslog
        {
            let (stop, log) = (stop.clone(), log.clone());
            threads.push(std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((n, _)) = udp.recv_from(&mut buf) {
                        let msg = String::from_utf8_lossy(&buf[..n]).to_string();
                        log.installer_line(Source::Syslog, strip_syslog_pri(&msg));
                    }
                }
            }));
        }
        log.info(format!("serving on {bind_ip}: http {http_port}, log tcp {tcp_log_port}, syslog udp {udp_log_port}"));
        Ok(Server { http_port, tcp_log_port, udp_log_port, stop, http, threads })
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.http.unblock();
        for t in self.threads {
            let _ = t.join();
        }
    }
}

// --------------------------------------------------------------------------------------------
// offline mirror route
// --------------------------------------------------------------------------------------------

/// Decode a request path below `mirror/` into safe path segments, or `None` when it must be refused.
///
/// Refused: empty segments (`//`), `.` and `..` (also percent-encoded), NUL, `/` or `\` produced by decoding,
/// control characters, a trailing slash (no directory listings) and anything not valid UTF-8. A query string is
/// ignored. apt percent-encodes `+` and `~`, so decoding is required.
pub fn mirror_segments(rest: &str) -> Option<Vec<String>> {
    let path = rest.split(['?', '#']).next().unwrap_or("");
    if path.is_empty() || path.ends_with('/') {
        return None;
    }
    let mut out = vec![];
    for raw in path.split('/') {
        let seg = percent_decode(raw)?;
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(['/', '\\', '\0']) || seg.chars().any(|c| c.is_control()) {
            return None;
        }
        out.push(seg);
    }
    Some(out)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Map safe segments to a regular file inside `root` (symlinks may not lead out of it).
pub fn resolve_mirror_file(root: &std::path::Path, segs: &[String]) -> Option<std::path::PathBuf> {
    let mut p = root.to_path_buf();
    for s in segs {
        p.push(s);
    }
    let real = std::fs::canonicalize(&p).ok()?;
    let real_root = std::fs::canonicalize(root).ok()?;
    (real.starts_with(&real_root) && real.is_file()).then_some(real)
}

/// What a `Range` header asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum RangeSpec {
    Full,
    Partial(u64, u64),
    Unsatisfiable,
}

/// Single-range `bytes=a-b`, `bytes=a-`, `bytes=-n`. Anything else (multi-range, other units) is served in full.
pub fn parse_range(header: Option<&str>, len: u64) -> RangeSpec {
    let Some(h) = header else { return RangeSpec::Full };
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return RangeSpec::Full };
    if spec.contains(',') {
        return RangeSpec::Full;
    }
    let Some((a, b)) = spec.split_once('-') else { return RangeSpec::Full };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let Ok(n) = b.parse::<u64>() else { return RangeSpec::Full };
        return if n == 0 || len == 0 { RangeSpec::Unsatisfiable } else { RangeSpec::Partial(len.saturating_sub(n), len - 1) };
    }
    let Ok(start) = a.parse::<u64>() else { return RangeSpec::Full };
    if start >= len {
        return RangeSpec::Unsatisfiable;
    }
    let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().map(|e| e.min(len - 1)).unwrap_or(len - 1) };
    if end < start { RangeSpec::Full } else { RangeSpec::Partial(start, end) }
}

/// Aliases for apt's `by-hash/SHA256/<hash>` URLs: the bundle keeps the verbatim `dists/` tree only, so the server
/// answers by-hash requests from the file the (verbatim) Release file lists under that hash.
fn by_hash_aliases(root: &std::path::Path) -> std::collections::HashMap<String, std::path::PathBuf> {
    let mut map = std::collections::HashMap::new();
    let Ok(mirrors) = std::fs::read_dir(root) else { return map };
    for m in mirrors.flatten() {
        let dists = m.path().join("dists");
        let Ok(suites) = std::fs::read_dir(&dists) else { continue };
        for s in suites.flatten() {
            let Ok(text) = std::fs::read_to_string(s.path().join("InRelease")) else { continue };
            let rel = crate::offline::parse_release(&text);
            for e in &rel.entries {
                let file = e.path.split('/').fold(s.path(), |p, seg| p.join(seg));
                if !file.is_file() {
                    continue;
                }
                let dir = e.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                let key = format!(
                    "{}/dists/{}/{}{}by-hash/SHA256/{}",
                    m.file_name().to_string_lossy(),
                    s.file_name().to_string_lossy(),
                    dir,
                    if dir.is_empty() { "" } else { "/" },
                    e.sha256
                );
                map.insert(key, file);
            }
        }
    }
    map
}

struct MirrorState {
    root: std::path::PathBuf,
    aliases: std::collections::HashMap<String, std::path::PathBuf>,
    in_flight: std::sync::atomic::AtomicUsize,
    files: std::sync::atomic::AtomicU64,
    bytes: std::sync::atomic::AtomicU64,
}

const MAX_MIRROR_CONNECTIONS: usize = 32;

impl MirrorState {
    fn new(root: std::path::PathBuf) -> MirrorState {
        let aliases = by_hash_aliases(&root);
        MirrorState { root, aliases, in_flight: Default::default(), files: Default::default(), bytes: Default::default() }
    }

    fn try_acquire(&self) -> bool {
        if self.in_flight.fetch_add(1, Ordering::AcqRel) >= MAX_MIRROR_CONNECTIONS {
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    fn release(&self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }

    fn serve(&self, req: tiny_http::Request, rest: &str, peer: &str, log: &Logger) {
        use std::io::{Read, Seek, SeekFrom};
        use tiny_http::{Header, Method, Response, StatusCode};
        let head = match req.method() {
            Method::Get => false,
            Method::Head => true,
            _ => {
                let _ = req.respond(
                    Response::from_string("method not allowed")
                        .with_status_code(405)
                        .with_header(Header::from_bytes("Allow", "GET, HEAD").unwrap()),
                );
                return;
            }
        };
        let not_found = |req: tiny_http::Request, why: &str| {
            log.warn(format!("HTTP {peer} mirror request refused or not in the bundle: {why}"));
            let _ = req.respond(Response::from_string("not found").with_status_code(404));
        };
        let Some(segs) = mirror_segments(rest) else { return not_found(req, "invalid path") };
        let file = resolve_mirror_file(&self.root, &segs).or_else(|| self.aliases.get(&segs.join("/")).cloned());
        let Some(path) = file else { return not_found(req, &segs.join("/")) };
        let Ok(mut f) = std::fs::File::open(&path) else { return not_found(req, &segs.join("/")) };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let range_header = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string());
        let (status, start, end) = match parse_range(range_header.as_deref(), len) {
            RangeSpec::Full => (200u16, 0, len.saturating_sub(1)),
            RangeSpec::Partial(a, b) => (206, a, b),
            RangeSpec::Unsatisfiable => {
                let _ = req.respond(
                    Response::from_string("range not satisfiable")
                        .with_status_code(416)
                        .with_header(Header::from_bytes("Content-Range", format!("bytes */{len}")).unwrap()),
                );
                return;
            }
        };
        let n = if len == 0 { 0 } else { end - start + 1 };
        if start > 0 && f.seek(SeekFrom::Start(start)).is_err() {
            return not_found(req, "seek");
        }
        let mut headers = vec![
            Header::from_bytes("Content-Type", "application/octet-stream").unwrap(),
            Header::from_bytes("Accept-Ranges", "bytes").unwrap(),
            Header::from_bytes("Cache-Control", "no-store").unwrap(),
        ];
        if status == 206 {
            headers.push(Header::from_bytes("Content-Range", format!("bytes {start}-{end}/{len}")).unwrap());
        }
        let reader: Box<dyn Read + Send> = if head { Box::new(std::io::empty()) } else { Box::new(f.take(n)) };
        let resp = Response::new(StatusCode(status), headers, reader, Some(n as usize), None);
        if req.respond(resp).is_ok() && !head {
            let files = self.files.fetch_add(1, Ordering::Relaxed) + 1;
            let bytes = self.bytes.fetch_add(n, Ordering::Relaxed) + n;
            log.debug(format!("HTTP {peer} GET mirror/{} ({n} bytes{})", segs.join("/"), if status == 206 { ", partial" } else { "" }));
            if files.is_multiple_of(100) {
                log.info(format!("offline mirror: {files} files served so far ({} MiB)", bytes / (1024 * 1024)));
            }
        }
    }
}

/// `<30>Nov  1 12:00:00 tag: msg` -> `Nov  1 12:00:00 tag: msg`
pub fn strip_syslog_pri(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix('<')
        && let Some((pri, tail)) = rest.split_once('>')
        && pri.chars().all(|c| c.is_ascii_digit())
        && !pri.is_empty()
    {
        return tail;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::Event;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::mpsc::channel;

    fn get(port: u16, path: &str) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.0\r\nHost: x\r\n\r\n").unwrap();
        let mut raw = vec![];
        s.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).to_string();
        let code: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|i| raw[i + 4..].to_vec()).unwrap_or_default();
        (code, body)
    }

    #[test]
    fn serves_only_under_the_token_and_receives_logs() {
        let (tx, rx) = channel();
        let log = Logger::memory(Some(tx));
        let big = std::env::temp_dir().join(format!("sf-served-{}.bin", std::process::id()));
        std::fs::write(&big, vec![7u8; 300_000]).unwrap();
        let mut files = std::collections::HashMap::new();
        files.insert("installer-initrd".to_string(), big.clone());
        let served = Served {
            files,
            token: "abc123".into(),
            stage2: "echo stage2".into(),
            payload: vec![1, 2, 3],
            preseed: "d-i x".into(),
            mirror: None,
        };
        let srv = Server::start("127.0.0.1".parse().unwrap(), served, log, (0, 0, 0)).unwrap();
        assert_eq!(get(srv.http_port, "/abc123/stage2.sh"), (200, b"echo stage2".to_vec()));
        assert_eq!(get(srv.http_port, "/abc123/payload.tar.gz").1, vec![1, 2, 3]);
        assert_eq!(get(srv.http_port, "/abc123/files/installer-initrd").1.len(), 300_000);
        assert_eq!(get(srv.http_port, "/abc123/files/../stage2.sh").0, 404, "only registered names are served");
        assert_eq!(get(srv.http_port, "/abc123/files/nope").0, 404);
        assert_eq!(get(srv.http_port, "/wrong/stage2.sh").0, 404);
        assert_eq!(get(srv.http_port, "/abc123/../etc/passwd").0, 404);
        assert_eq!(get(srv.http_port, "/stage2.sh").0, 404);

        let mut c = TcpStream::connect(("127.0.0.1", srv.tcp_log_port)).unwrap();
        writeln!(c, "12:00:00 #1 @@STEP 2/12 Preflight").unwrap();
        drop(c);
        let u = UdpSocket::bind("127.0.0.1:0").unwrap();
        u.send_to(b"<30>Nov  1 12:00:00 synofurb: @@POINT_OF_NO_RETURN", ("127.0.0.1", srv.udp_log_port)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let evs: Vec<Event> = rx.try_iter().collect();
        assert!(evs.iter().any(|e| matches!(e, Event::Step { n: 2, .. })), "tcp step marker");
        assert!(evs.iter().any(|e| matches!(e, Event::PointOfNoReturn)), "udp marker");
        srv.stop();
    }

    // ---- offline mirror route ----------------------------------------------------------------------------

    /// Raw HTTP request; returns (status, lowercase header text, body).
    fn http(port: u16, method: &str, path: &str, extra: &str) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "{method} {path} HTTP/1.0\r\nHost: x\r\n{extra}\r\n").unwrap();
        let mut raw = vec![];
        s.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
        let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
        let code: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (code, head, raw[split..].to_vec())
    }

    fn mirror_fixture(name: &str) -> (std::path::PathBuf, Server) {
        let dir = std::env::temp_dir().join(format!("sf-mirror-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = dir.join("mirror/debian/pool/main/libs/libstdc++6");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("libstdc++6_1-1_amd64.deb"), b"0123456789").unwrap();
        std::fs::create_dir_all(dir.join("mirror/debian/dists/trixie/main/binary-amd64")).unwrap();
        std::fs::write(dir.join("mirror/debian/dists/trixie/main/binary-amd64/Packages.xz"), b"INDEXDATA").unwrap();
        let hash = crate::debian::sha256_hex(b"INDEXDATA");
        std::fs::write(
            dir.join("mirror/debian/dists/trixie/InRelease"),
            format!("Origin: Debian\nAcquire-By-Hash: yes\nSHA256:\n {hash}  9 main/binary-amd64/Packages.xz\n"),
        )
        .unwrap();
        std::fs::write(dir.join("secret.txt"), b"TOP SECRET").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("secret.txt"), dir.join("mirror/debian/escape")).unwrap();
        let served = Served {
            files: Default::default(),
            token: "tok".into(),
            stage2: "s".into(),
            payload: vec![],
            preseed: "p".into(),
            mirror: Some(dir.join("mirror")),
        };
        let srv = Server::start("127.0.0.1".parse().unwrap(), served, Logger::memory(None), (0, 0, 0)).unwrap();
        (dir, srv)
    }

    #[test]
    fn mirror_route_serves_get_head_range_and_percent_encoded_names() {
        let (dir, srv) = mirror_fixture("serve");
        let p = srv.http_port;
        let deb = "/tok/mirror/debian/pool/main/libs/libstdc++6/libstdc++6_1-1_amd64.deb";
        let (c, h, b) = http(p, "GET", deb, "");
        assert_eq!((c, b.as_slice()), (200, &b"0123456789"[..]));
        assert!(h.contains("content-length: 10") && h.contains("accept-ranges: bytes"), "{h}");
        // apt percent-encodes '+'
        let (c, _, b) = http(p, "GET", "/tok/mirror/debian/pool/main/libs/libstdc%2b%2b6/libstdc%2B%2B6_1-1_amd64.deb", "");
        assert_eq!((c, b.len()), (200, 10));
        // HEAD: headers with the length, no body
        let (c, h, b) = http(p, "HEAD", deb, "");
        assert_eq!(c, 200);
        assert!(h.contains("content-length: 10"), "{h}");
        assert!(b.is_empty());
        // ranges
        let (c, h, b) = http(p, "GET", deb, "Range: bytes=2-5\r\n");
        assert_eq!((c, b.as_slice()), (206, &b"2345"[..]));
        assert!(h.contains("content-range: bytes 2-5/10") && h.contains("content-length: 4"), "{h}");
        assert_eq!(http(p, "GET", deb, "Range: bytes=7-\r\n").2, b"789");
        assert_eq!(http(p, "GET", deb, "Range: bytes=-3\r\n").2, b"789");
        let (c, h, _) = http(p, "GET", deb, "Range: bytes=10-\r\n");
        assert_eq!(c, 416);
        assert!(h.contains("content-range: bytes */10"), "{h}");
        // by-hash alias of the verbatim Release file
        let hash = crate::debian::sha256_hex(b"INDEXDATA");
        let (c, _, b) = http(p, "GET", &format!("/tok/mirror/debian/dists/trixie/main/binary-amd64/by-hash/SHA256/{hash}"), "");
        assert_eq!((c, b.as_slice()), (200, &b"INDEXDATA"[..]));
        assert_eq!(http(p, "GET", "/tok/mirror/debian/dists/trixie/main/binary-amd64/by-hash/SHA256/deadbeef", "").0, 404);
        srv.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mirror_route_refuses_traversal_listing_wrong_token_and_writes() {
        let (dir, srv) = mirror_fixture("deny");
        let p = srv.http_port;
        for bad in [
            "/tok/mirror/../secret.txt",
            "/tok/mirror/debian/../../secret.txt",
            "/tok/mirror/debian/%2e%2e/%2e%2e/secret.txt",
            "/tok/mirror/debian/%2E%2E%2Fsecret.txt",
            "/tok/mirror/debian/..%2f..%2fsecret.txt",
            "/tok/mirror/debian/pool/%2e/main",
            "/tok/mirror//debian/pool/main/libs/libstdc++6/libstdc++6_1-1_amd64.deb",
            "/tok/mirror/debian/pool/main/libs/libstdc++6/",
            "/tok/mirror/debian/pool/main",
            "/tok/mirror/debian/",
            "/tok/mirror/",
            "/tok/mirror/debian/pool/main/libs/libstdc++6/libstdc++6_1-1_amd64.deb%00.txt",
            "/tok/mirror/debian/%5c..%5csecret.txt",
            "/tok/mirror/debian/escape", // symlink out of the mirror
            "/wrong/mirror/debian/dists/trixie/InRelease",
            "/mirror/debian/dists/trixie/InRelease",
        ] {
            let (c, _, b) = http(p, "GET", bad, "");
            assert_eq!(c, 404, "{bad} must not be served");
            assert!(!String::from_utf8_lossy(&b).contains("SECRET"), "{bad} leaked");
            assert_eq!(http(p, "HEAD", bad, "").0, 404, "{bad} (HEAD)");
        }
        assert_eq!(http(p, "POST", "/tok/mirror/debian/dists/trixie/InRelease", "Content-Length: 0\r\n").0, 405);
        assert_eq!(http(p, "PUT", "/tok/mirror/debian/dists/trixie/InRelease", "Content-Length: 0\r\n").0, 405);
        assert_eq!(http(p, "GET", "/tok/mirror/debian/dists/trixie/InRelease", "").0, 200);
        srv.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mirror_without_a_bundle_is_404_and_segments_are_validated() {
        let served =
            Served { files: Default::default(), token: "t".into(), stage2: "s".into(), payload: vec![], preseed: "p".into(), mirror: None };
        let srv = Server::start("127.0.0.1".parse().unwrap(), served, Logger::memory(None), (0, 0, 0)).unwrap();
        assert_eq!(http(srv.http_port, "GET", "/t/mirror/debian/dists/trixie/InRelease", "").0, 404);
        srv.stop();
        assert_eq!(mirror_segments("debian/pool/a%2Bb.deb"), Some(vec!["debian".into(), "pool".into(), "a+b.deb".into()]));
        assert_eq!(mirror_segments("debian/x?query=1"), Some(vec!["debian".into(), "x".into()]));
        for bad in ["", "/", "a//b", "a/../b", "a/%2e%2e/b", "a/b/", "a/%zz", "a/%ff", "a/b%2Fc", "a\\\\b", "a/%00"] {
            assert_eq!(mirror_segments(bad), None, "{bad:?}");
        }
        assert_eq!(parse_range(None, 10), RangeSpec::Full);
        assert_eq!(parse_range(Some("bytes=0-0"), 10), RangeSpec::Partial(0, 0));
        assert_eq!(parse_range(Some("bytes=5-100"), 10), RangeSpec::Partial(5, 9));
        assert_eq!(parse_range(Some("bytes=-100"), 10), RangeSpec::Partial(0, 9));
        assert_eq!(parse_range(Some("bytes=0-1,3-4"), 10), RangeSpec::Full);
        assert_eq!(parse_range(Some("items=0-1"), 10), RangeSpec::Full);
        assert_eq!(parse_range(Some("bytes=5-"), 5), RangeSpec::Unsatisfiable);
    }

    #[test]
    fn syslog_priority_is_stripped() {
        assert_eq!(strip_syslog_pri("<30>hello"), "hello");
        assert_eq!(strip_syslog_pri("<x>hello"), "<x>hello");
        assert_eq!(strip_syslog_pri("plain"), "plain");
    }

    #[test]
    fn local_ip_is_found_for_loopback() {
        assert!(local_ip_towards("127.0.0.1".parse().unwrap()).unwrap().is_loopback());
    }
}
