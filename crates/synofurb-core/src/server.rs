//! The small server the NAS talks back to during the install:
//! * HTTP: `GET /<token>/{stage2.sh,payload.tar.gz,preseed.cfg}` (token = 128 random bits)
//! * TCP log stream (stage-2 pipes its log here) and UDP syslog (Debian installer `log_host`)

use crate::logs::{Logger, Source};
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

pub struct Served {
    pub token: String,
    pub stage2: String,
    pub payload: Vec<u8>,
    pub preseed: String,
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
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok(Some(req)) = http.recv_timeout(std::time::Duration::from_millis(500)) else { continue };
                    let url = req.url().to_string();
                    let peer = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
                    let prefix = format!("/{}/", served.token);
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
        let served = Served { token: "abc123".into(), stage2: "echo stage2".into(), payload: vec![1, 2, 3], preseed: "d-i x".into() };
        let srv = Server::start("127.0.0.1".parse().unwrap(), served, log, (0, 0, 0)).unwrap();
        assert_eq!(get(srv.http_port, "/abc123/stage2.sh"), (200, b"echo stage2".to_vec()));
        assert_eq!(get(srv.http_port, "/abc123/payload.tar.gz").1, vec![1, 2, 3]);
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
