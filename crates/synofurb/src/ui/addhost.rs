//! "Add by IP": what a person typed becomes a checked address.
//!
//! * [`parse_target`] turns the text into a host (IPv4, IPv6 or host name) and an optional port, or says what is wrong with it.
//!   Pure, so every rule is a unit test.
//! * [`probe`] looks at the address from several sides at once (name lookup, SSH, DSM web ports, Synology's UDP query) and reports
//!   each check as it finishes. It never throws; every failure is a line in the report.
//! * [`outcome`] reads the report: a Synology unit, an unknown device that answered, or nothing at all.
//! * [`AddedHosts`] is what is kept on disk (`added-hosts.json`): one entry per address, replaced when it is checked again.
//!
//! No GPUI types here.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::time::Duration;
use synofurb_core::discovery::{Found, Inventory, probe as dsm_probe, syno_udp};

// ------------------------------------------------------------------------------------------------ parsing

/// An address a person typed, normalised.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// IPv4 or IPv6 text without brackets, or a lower-case host name.
    pub host: String,
    /// The SSH port when one was typed (`nas:2222`, `[fe80::1]:2222`).
    pub port: Option<u16>,
}

impl Target {
    /// What a row and the probe list call it: `192.168.1.20`, `nas.local:2222`, `[2001:db8::1]:22`.
    pub fn display(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        match self.port {
            Some(p) => format!("{h}:{p}"),
            None => h,
        }
    }
    pub fn ip(&self) -> Option<IpAddr> {
        self.host.parse().ok()
    }
    pub fn ssh_port(&self) -> u16 {
        self.port.unwrap_or(22)
    }
}

/// Why an address cannot be used. Each has one plain sentence in `copy.rs` (`copy::add_error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    Empty,
    /// Spaces or other characters that no address has.
    BadChars,
    /// Looks like an IPv4 address but is not one (`192.168.1`, `192.168.1.256`).
    BadIp,
    BadPort,
    BadName,
    /// `0.0.0.0`, `255.255.255.255`, a multicast address: nothing can be there.
    NotHost,
    /// `127.0.0.1`, `::1`: this computer, not a NAS.
    ThisComputer,
}

/// Parse and normalise what was typed: trims, accepts a `ssh://`, `http://` or `https://` prefix and a trailing path, one
/// address with an optional `:port`, bracketed or bare IPv6, and host names.
pub fn parse_target(input: &str) -> Result<Target, AddError> {
    let mut s = input.trim();
    if s.is_empty() {
        return Err(AddError::Empty);
    }
    for scheme in ["ssh://", "http://", "https://"] {
        if s.len() >= scheme.len() && s[..scheme.len()].eq_ignore_ascii_case(scheme) {
            s = &s[scheme.len()..];
        }
    }
    if let Some(i) = s.find(['/', '?', '#']) {
        s = &s[..i];
    }
    if s.is_empty() {
        return Err(AddError::Empty);
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | ';' | '@' | '\\' | '"' | '\'')) {
        return Err(AddError::BadChars);
    }
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, tail) = rest.split_once(']').ok_or(AddError::BadChars)?;
        let port = match tail {
            "" => None,
            t => Some(t.strip_prefix(':').ok_or(AddError::BadChars)?),
        };
        (h, port)
    } else {
        match s.matches(':').count() {
            0 => (s, None),
            1 => {
                let (h, p) = s.split_once(':').unwrap_or((s, ""));
                (h, Some(p))
            }
            _ => (s, None), // a bare IPv6 address has no port
        }
    };
    let port = match port {
        None => None,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 => Some(n),
            _ => return Err(AddError::BadPort),
        },
    };
    if host.is_empty() {
        return Err(AddError::Empty);
    }
    if host.contains(':') {
        let ip: std::net::Ipv6Addr = host.parse().map_err(|_| AddError::BadIp)?;
        if ip.is_loopback() {
            return Err(AddError::ThisComputer);
        }
        if ip.is_unspecified() || ip.is_multicast() {
            return Err(AddError::NotHost);
        }
        return Ok(Target { host: ip.to_string(), port });
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        if ip.is_loopback() {
            return Err(AddError::ThisComputer);
        }
        if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
            return Err(AddError::NotHost);
        }
        return Ok(Target { host: ip.to_string(), port });
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    // digits and dots only: someone meant an IPv4 address
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(AddError::BadIp);
    }
    if name.is_empty() || name.len() > 253 {
        return Err(AddError::BadName);
    }
    for label in name.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !ok {
            return Err(if name.is_ascii() { AddError::BadName } else { AddError::BadChars });
        }
    }
    if name == "localhost" {
        return Err(AddError::ThisComputer);
    }
    Ok(Target { host: name, port })
}

// ------------------------------------------------------------------------------------------------ the report

/// How one check ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckState {
    /// Something answered the way the check hoped.
    Answered,
    /// The host answered and said no (a closed port, or a reply that is not what the check looks for). The host is there.
    Closed,
    /// Nothing came back in time.
    NoReply,
    /// The check could not run (name not found, no route to the network).
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// What was asked: `SSH, port 22`, `DSM web, port 5000`.
    pub check: String,
    pub state: CheckState,
    /// What came back, or why not: `SSH-2.0-OpenSSH_9.2`, `connection refused`, `no reply in 1.5 s`.
    pub detail: String,
}

impl CheckResult {
    /// Did this check prove that something is at the address?
    pub fn answered(&self) -> bool {
        matches!(self.state, CheckState::Answered | CheckState::Closed)
    }
}

/// Everything the checks learned.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbeReport {
    pub checks: Vec<CheckResult>,
    /// Set when a check identified a Synology unit (merged from the DSM web API and Synology's UDP query).
    pub found: Option<Found>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Identified as a Synology unit.
    Synology,
    /// Something answered, but nothing says it is a Synology unit.
    Unknown,
    /// Nothing answered at all.
    Unreachable,
}

pub fn outcome(r: &ProbeReport) -> Outcome {
    if r.found.as_ref().is_some_and(|f| f.synology) {
        Outcome::Synology
    } else if r.checks.iter().any(CheckResult::answered) {
        Outcome::Unknown
    } else {
        Outcome::Unreachable
    }
}

/// The lines for the info popover: what answered (Unknown) or what failed (Unreachable), one per check.
pub fn detail_lines(r: &ProbeReport) -> Vec<String> {
    r.checks.iter().map(|c| format!("{}: {}", c.check, c.detail)).collect()
}

/// The checks that run for `t`, in the order shown: a name lookup only for a host name.
pub fn check_names(t: &Target) -> Vec<String> {
    let mut v = vec![];
    if t.ip().is_none() {
        v.push("Name lookup".to_string());
    }
    v.push(format!("SSH, port {}", t.ssh_port()));
    v.push("DSM web, port 5000".to_string());
    v.push("DSM secure web, port 5001".to_string());
    v.push("Synology query, UDP 9999".to_string());
    v
}

// ------------------------------------------------------------------------------------------------ the probe

const TCP_TIMEOUT: Duration = Duration::from_millis(1500);
const UDP_TIMEOUT: Duration = Duration::from_millis(1500);

fn secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f32())
}

/// Plain words for a failed `connect`.
fn connect_error(e: &std::io::Error) -> (CheckState, String) {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => (CheckState::Closed, "connection refused (the host is there, the port is closed)".into()),
        TimedOut | WouldBlock => (CheckState::NoReply, format!("no reply in {}", secs(TCP_TIMEOUT))),
        HostUnreachable | NetworkUnreachable => (CheckState::Failed, "no route to this address".into()),
        _ => (CheckState::Failed, e.to_string()),
    }
}

fn tcp(addr: SocketAddr) -> Result<TcpStream, (CheckState, String)> {
    TcpStream::connect_timeout(&addr, TCP_TIMEOUT).map_err(|e| connect_error(&e))
}

fn check_ssh(addr: SocketAddr, name: String) -> CheckResult {
    match tcp(addr) {
        Err((state, detail)) => CheckResult { check: name, state, detail },
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(TCP_TIMEOUT));
            let mut buf = [0u8; 128];
            let n = s.read(&mut buf).unwrap_or(0);
            let text = String::from_utf8_lossy(&buf[..n]);
            let line = text.lines().next().unwrap_or("").trim().to_string();
            if line.starts_with("SSH-") {
                CheckResult { check: name, state: CheckState::Answered, detail: line }
            } else {
                CheckResult { check: name, state: CheckState::Closed, detail: "open, but it does not speak SSH".into() }
            }
        }
    }
}

/// Port 5000: DSM's unauthenticated Web API fingerprint. Another web server on the port is reported by its status line.
fn check_dsm_web(ip: IpAddr, name: String) -> (CheckResult, Option<Found>) {
    let addr = SocketAddr::new(ip, 5000);
    let mut s = match tcp(addr) {
        Err((state, detail)) => return (CheckResult { check: name, state, detail }, None),
        Ok(s) => s,
    };
    if let Some(f) = dsm_probe::probe_dsm(ip) {
        return (CheckResult { check: name, state: CheckState::Answered, detail: "DSM answered its Web API".into() }, Some(f));
    }
    let _ = s.set_read_timeout(Some(TCP_TIMEOUT));
    let _ = s.set_write_timeout(Some(TCP_TIMEOUT));
    let _ = s.write_all(b"GET / HTTP/1.0\r\nHost: nas\r\nConnection: close\r\n\r\n");
    let mut buf = [0u8; 160];
    let n = s.read(&mut buf).unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..n]);
    let first = text.lines().next().unwrap_or("").trim();
    let detail = if first.starts_with("HTTP/") {
        format!("a web server answered ({first}), but not DSM")
    } else {
        "open, but it is not DSM's web server".to_string()
    };
    (CheckResult { check: name, state: CheckState::Closed, detail }, None)
}

fn check_port_open(ip: IpAddr, port: u16, name: String, what: &str) -> CheckResult {
    match tcp(SocketAddr::new(ip, port)) {
        Ok(_) => CheckResult { check: name, state: CheckState::Answered, detail: format!("open ({what})") },
        Err((state, detail)) => CheckResult { check: name, state, detail },
    }
}

/// Synology's own discovery query, sent straight to the address. Answers carry model, serial and DSM version.
fn check_udp(ip: IpAddr, name: String) -> (CheckResult, Option<Found>) {
    let IpAddr::V4(v4) = ip else {
        return (CheckResult { check: name, state: CheckState::Failed, detail: "not used for IPv6".into() }, None);
    };
    let run = || -> std::io::Result<Option<Found>> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_read_timeout(Some(Duration::from_millis(300)))?;
        let query = syno_udp::build_query();
        sock.send_to(&query, (v4, 9999))?;
        let end = std::time::Instant::now() + UDP_TIMEOUT;
        let mut buf = [0u8; 2048];
        while std::time::Instant::now() < end {
            if let Ok((n, from)) = sock.recv_from(&mut buf)
                && from.ip() == ip
                && let Some(f) = syno_udp::parse_response(&buf[..n], ip)
            {
                return Ok(Some(f));
            }
        }
        Ok(None)
    };
    match run() {
        Ok(Some(f)) => {
            let what = if f.model.is_empty() { "a Synology unit".to_string() } else { f.model.clone() };
            (CheckResult { check: name, state: CheckState::Answered, detail: format!("{what} answered") }, Some(f))
        }
        Ok(None) => (CheckResult { check: name, state: CheckState::NoReply, detail: format!("no reply in {}", secs(UDP_TIMEOUT)) }, None),
        Err(e) => (CheckResult { check: name, state: CheckState::Failed, detail: e.to_string() }, None),
    }
}

/// Resolve a host name off the calling thread so a slow resolver cannot hold the check past `limit`.
fn resolve(host: &str, port: u16, limit: Duration) -> Result<IpAddr, String> {
    let (tx, rx) = channel();
    let h = host.to_string();
    std::thread::spawn(move || {
        let r = (h.as_str(), port).to_socket_addrs().map(|it| it.map(|a| a.ip()).collect::<Vec<_>>());
        let _ = tx.send(r);
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(ips)) => ips.iter().find(|i| i.is_ipv4()).or(ips.first()).copied().ok_or_else(|| "the name has no address".to_string()),
        Ok(Err(_)) => Err("the name was not found".to_string()),
        Err(_) => Err(format!("no answer from the name server in {}", secs(limit))),
    }
}

/// One finished check: index into [`check_names`], and its result.
pub type StepDone = (usize, CheckResult);

/// Run every check for `t`, in parallel, calling `on_step` as each finishes. `cancel` stops reporting early (checks that are
/// already running end on their own within about two seconds). Returns the whole report.
pub fn probe(t: &Target, cancel: &Arc<AtomicBool>, on_step: &mut dyn FnMut(StepDone)) -> ProbeReport {
    let names = check_names(t);
    let mut report = ProbeReport::default();
    let mut results: Vec<Option<CheckResult>> = vec![None; names.len()];
    let mut base = 0;
    let ip = match t.ip() {
        Some(ip) => ip,
        None => {
            let r = resolve(&t.host, t.ssh_port(), Duration::from_secs(4));
            match r {
                Ok(ip) => {
                    let res = CheckResult { check: names[0].clone(), state: CheckState::Answered, detail: format!("{} is {ip}", t.host) };
                    on_step((0, res.clone()));
                    results[0] = Some(res);
                    base = 1;
                    ip
                }
                Err(why) => {
                    let res = CheckResult { check: names[0].clone(), state: CheckState::Failed, detail: why };
                    on_step((0, res.clone()));
                    results[0] = Some(res);
                    // nothing to ask: the other checks are skipped with the reason
                    for (i, n) in names.iter().enumerate().skip(1) {
                        let skip =
                            CheckResult { check: n.clone(), state: CheckState::Failed, detail: "skipped: the name has no address".into() };
                        on_step((i, skip.clone()));
                        results[i] = Some(skip);
                    }
                    report.checks = results.into_iter().flatten().collect();
                    return report;
                }
            }
        }
    };
    let (tx, rx) = channel::<(usize, CheckResult, Option<Found>)>();
    let ssh_addr = SocketAddr::new(ip, t.ssh_port());
    std::thread::scope(|s| {
        let tx1 = tx.clone();
        let n = names.clone();
        s.spawn(move || {
            let _ = tx1.send((base, check_ssh(ssh_addr, n[base].clone()), None));
        });
        let (tx2, n) = (tx.clone(), names.clone());
        s.spawn(move || {
            let (r, f) = check_dsm_web(ip, n[base + 1].clone());
            let _ = tx2.send((base + 1, r, f));
        });
        let (tx3, n) = (tx.clone(), names.clone());
        s.spawn(move || {
            let _ = tx3.send((base + 2, check_port_open(ip, 5001, n[base + 2].clone(), "DSM's secure web interface"), None));
        });
        let (tx4, n) = (tx.clone(), names.clone());
        s.spawn(move || {
            let (r, f) = check_udp(ip, n[base + 3].clone());
            let _ = tx4.send((base + 3, r, f));
        });
        drop(tx);
        let mut found: Vec<Found> = vec![];
        for (i, r, f) in rx {
            if !cancel.load(Ordering::Relaxed) {
                on_step((i, r.clone()));
            }
            results[i] = Some(r);
            found.extend(f);
        }
        report.found = merge_found(t, ip, found);
    });
    report.checks = results.into_iter().flatten().collect();
    report
}

/// Merge what the DSM Web API and the UDP query said about one unit; the address and host name the person typed are kept.
fn merge_found(t: &Target, ip: IpAddr, found: Vec<Found>) -> Option<Found> {
    if found.is_empty() {
        return None;
    }
    let mut inv = Inventory::default();
    for mut f in found {
        if f.ips.is_empty() {
            f.ips.push(ip);
        }
        inv.merge(f);
    }
    let mut f = inv.devices.into_iter().next()?;
    f.synology = true;
    if !f.ips.contains(&ip) {
        f.ips.push(ip);
    }
    if t.ip().is_none() && f.hostname.is_empty() {
        f.hostname = t.host.clone();
    }
    if !f.sources.iter().any(|s| s == "added") {
        f.sources.push("added".into());
    }
    Some(f)
}

// ------------------------------------------------------------------------------------------------ what is kept

/// The SSH port the Connect form shows after a row was selected: the one just typed (`explicit`), else the one stored with the added
/// address, else 22 when another unit was selected, else nothing (the same unit again keeps what is typed).
pub fn port_to_show(explicit: Option<u16>, stored: Option<u16>, unit_changed: bool) -> Option<u16> {
    explicit.or(stored).or(unit_changed.then_some(22))
}

/// One address added by the user, with what the last check found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddedHost {
    pub target: Target,
    pub report: ProbeReport,
    /// `YYYY-MM-DD HH:MM` of the last check.
    pub checked: String,
}

impl AddedHost {
    pub fn outcome(&self) -> Outcome {
        outcome(&self.report)
    }
    /// The key rows and the selection use: the host without the port (one entry per address).
    pub fn key(&self) -> &str {
        &self.target.host
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AddedHosts {
    pub hosts: Vec<AddedHost>,
}

impl AddedHosts {
    pub fn load(path: &Path) -> AddedHosts {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
    /// Add, or replace the entry of the same address (checking again does not make a second row).
    pub fn upsert(&mut self, h: AddedHost) {
        match self.hosts.iter_mut().find(|x| x.key() == h.key()) {
            Some(x) => *x = h,
            None => self.hosts.push(h),
        }
    }
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.hosts.len();
        self.hosts.retain(|h| h.key() != key);
        before != self.hosts.len()
    }
    pub fn get(&self, key: &str) -> Option<&AddedHost> {
        self.hosts.iter().find(|h| h.key() == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Target {
        parse_target(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
    }

    #[test]
    fn plain_addresses_and_names_are_accepted_and_normalised() {
        assert_eq!(t("192.168.1.20"), Target { host: "192.168.1.20".into(), port: None });
        assert_eq!(t("  192.168.1.20  \n"), Target { host: "192.168.1.20".into(), port: None }, "trimmed");
        assert_eq!(t("192.168.1.20:2222").port, Some(2222));
        assert_eq!(t("NAS.Local").host, "nas.local", "host names are lower-cased");
        assert_eq!(t("nas.local.").host, "nas.local", "a trailing dot is dropped");
        assert_eq!(t("nas").host, "nas");
        assert_eq!(t("my-nas_2.lan:22"), Target { host: "my-nas_2.lan".into(), port: Some(22) });
        assert_eq!(t("ssh://admin.lan:2200/").port, Some(2200), "a scheme and a trailing slash are tolerated");
        assert_eq!(t("http://192.168.1.20:5000/webman/index.cgi").host, "192.168.1.20");
    }

    #[test]
    fn ipv6_with_and_without_brackets() {
        assert_eq!(t("2001:db8::1"), Target { host: "2001:db8::1".into(), port: None }, "bare IPv6 has no port");
        assert_eq!(t("[2001:DB8::1]:2222"), Target { host: "2001:db8::1".into(), port: Some(2222) });
        assert_eq!(t("[fe80::1]").host, "fe80::1");
        assert_eq!(t("[2001:db8::1]:2222").display(), "[2001:db8::1]:2222");
        assert_eq!(parse_target("2001:db8::zz"), Err(AddError::BadIp));
        assert_eq!(parse_target("[2001:db8::1"), Err(AddError::BadChars));
        assert_eq!(parse_target("[2001:db8::1]x"), Err(AddError::BadChars));
    }

    #[test]
    fn every_kind_of_bad_input_has_its_own_reason() {
        assert_eq!(parse_target(""), Err(AddError::Empty));
        assert_eq!(parse_target("   "), Err(AddError::Empty));
        assert_eq!(parse_target("192.168.1"), Err(AddError::BadIp));
        assert_eq!(parse_target("192.168.1.256"), Err(AddError::BadIp));
        assert_eq!(parse_target("192.168.1.20:0"), Err(AddError::BadPort));
        assert_eq!(parse_target("192.168.1.20:70000"), Err(AddError::BadPort));
        assert_eq!(parse_target("192.168.1.20:ssh"), Err(AddError::BadPort));
        assert_eq!(parse_target("192.168.1.20:"), Err(AddError::BadPort));
        assert_eq!(parse_target("192.168.1.20 192.168.1.21"), Err(AddError::BadChars));
        assert_eq!(parse_target("admin@192.168.1.20"), Err(AddError::BadChars));
        assert_eq!(parse_target("-bad.lan"), Err(AddError::BadName));
        assert_eq!(parse_target("a..b"), Err(AddError::BadName));
        assert_eq!(parse_target("exa$mple.com"), Err(AddError::BadName));
        assert_eq!(parse_target("0.0.0.0"), Err(AddError::NotHost));
        assert_eq!(parse_target("255.255.255.255"), Err(AddError::NotHost));
        assert_eq!(parse_target("224.0.0.1"), Err(AddError::NotHost));
        assert_eq!(parse_target("127.0.0.1"), Err(AddError::ThisComputer));
        assert_eq!(parse_target("localhost"), Err(AddError::ThisComputer));
        assert_eq!(parse_target("::1"), Err(AddError::ThisComputer));
    }

    #[test]
    fn the_dialogs_placeholder_is_not_something_that_looks_like_an_address() {
        // The first version showed 192.168.1.20 as the placeholder: it looked like typed text, so a click on Add with an empty
        // field seemed to do nothing. An empty field is an error with a reason, and the hint must not be a valid address.
        assert_eq!(parse_target(""), Err(AddError::Empty));
        assert!(parse_target(crate::ui::copy::INV_ADD_HINT.short).is_err());
        for e in [
            AddError::Empty,
            AddError::BadChars,
            AddError::BadIp,
            AddError::BadPort,
            AddError::BadName,
            AddError::NotHost,
            AddError::ThisComputer,
        ] {
            assert!(!crate::ui::copy::add_error(e).is_empty());
        }
    }

    fn c(check: &str, state: CheckState, detail: &str) -> CheckResult {
        CheckResult { check: check.into(), state, detail: detail.into() }
    }

    #[test]
    fn the_outcome_is_read_from_the_checks() {
        let nothing = ProbeReport {
            checks: vec![c("SSH, port 22", CheckState::NoReply, "no reply"), c("DSM web, port 5000", CheckState::NoReply, "no reply")],
            found: None,
        };
        assert_eq!(outcome(&nothing), Outcome::Unreachable);
        let router = ProbeReport {
            checks: vec![
                c("SSH, port 22", CheckState::NoReply, "no reply"),
                c("DSM web, port 5000", CheckState::Closed, "a web server answered"),
            ],
            found: None,
        };
        assert_eq!(outcome(&router), Outcome::Unknown, "something answered, but it is not recognised");
        let refused = ProbeReport { checks: vec![c("SSH, port 22", CheckState::Closed, "connection refused")], found: None };
        assert_eq!(outcome(&refused), Outcome::Unknown, "a refusal still proves a host is there");
        let nas = ProbeReport {
            checks: vec![c("SSH, port 22", CheckState::NoReply, "no reply")],
            found: Some(Found { synology: true, model: "DS918+".into(), ..Default::default() }),
        };
        assert_eq!(outcome(&nas), Outcome::Synology, "a recognised unit wins even when SSH is off");
        let lookup_failed = ProbeReport { checks: vec![c("Name lookup", CheckState::Failed, "the name was not found")], found: None };
        assert_eq!(outcome(&lookup_failed), Outcome::Unreachable);
        assert_eq!(outcome(&ProbeReport::default()), Outcome::Unreachable);
    }

    #[test]
    fn the_checks_for_a_name_start_with_a_lookup() {
        assert_eq!(check_names(&t("192.168.1.20")).len(), 4);
        assert_eq!(check_names(&t("192.168.1.20:2222"))[0], "SSH, port 2222");
        let named = check_names(&t("nas.local"));
        assert_eq!((named.len(), named[0].as_str()), (5, "Name lookup"));
    }

    #[test]
    fn added_hosts_are_kept_once_per_address_and_survive_a_restart() {
        let host = |ip: &str, model: &str| AddedHost {
            target: t(ip),
            report: ProbeReport { checks: vec![], found: Some(Found { synology: true, model: model.into(), ..Default::default() }) },
            checked: "2026-11-03 10:00".into(),
        };
        let mut a = AddedHosts::default();
        a.upsert(host("192.168.1.99", "DS220+"));
        a.upsert(host("192.168.1.99:2222", "DS220+ (again)"));
        assert_eq!(a.hosts.len(), 1, "checking again replaces the row");
        assert_eq!(a.get("192.168.1.99").unwrap().target.port, Some(2222));
        a.upsert(host("nas.lan", "DS1621+"));
        let dir = std::env::temp_dir().join(format!("sf-added-{}", std::process::id()));
        let path = dir.join("added-hosts.json");
        a.save(&path).unwrap();
        assert_eq!(AddedHosts::load(&path), a);
        std::fs::write(&path, "{ nope").unwrap();
        assert_eq!(AddedHosts::load(&path), AddedHosts::default(), "a corrupt file never stops the app");
        assert!(a.remove("nas.lan") && !a.remove("nas.lan"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_stored_ssh_port_comes_back_when_the_row_is_selected_and_never_leaks_to_another_unit() {
        // just added as nas:2222: the typed port is used
        assert_eq!(port_to_show(Some(2222), Some(2222), true), Some(2222));
        // selected again later (Connect, a click on the row): the port stored with the added address
        assert_eq!(port_to_show(None, Some(2222), true), Some(2222));
        assert_eq!(port_to_show(None, Some(2222), false), Some(2222));
        // another unit, no stored port: back to 22 instead of keeping 2222 from the last one
        assert_eq!(port_to_show(None, None, true), Some(22));
        // the same unit again: whatever is typed on the Connect form stays
        assert_eq!(port_to_show(None, None, false), None);
        // and the row says it when the port is not 22 (see UnitRow::address_text)
        let mut a = AddedHosts::default();
        a.upsert(AddedHost {
            target: t("nas.lan:2222"),
            report: ProbeReport { checks: vec![], found: None },
            checked: String::new(),
        });
        assert_eq!(a.get("nas.lan").and_then(|h| h.target.port), Some(2222));
    }

    #[test]
    fn probing_an_address_nothing_listens_on_ends_with_a_report_not_a_hang() {
        // 192.0.2.0/24 is reserved for documentation: it never answers (timeouts, or "no route" in a sandbox).
        let cancel = Arc::new(AtomicBool::new(false));
        let mut seen = 0;
        let r = probe(&t("192.0.2.77"), &cancel, &mut |_| seen += 1);
        assert_eq!(r.checks.len(), 4);
        assert_eq!(seen, 4, "every check reports as it ends");
        assert_ne!(outcome(&r), Outcome::Synology);
    }

    #[test]
    fn an_unknown_name_is_unreachable_and_says_why() {
        let cancel = Arc::new(AtomicBool::new(false));
        let r = probe(&t("no-such-host.invalid"), &cancel, &mut |_| {});
        assert_eq!(outcome(&r), Outcome::Unreachable);
        assert_eq!(r.checks[0].check, "Name lookup");
        assert_eq!(r.checks[0].state, CheckState::Failed);
    }

    #[test]
    fn a_listening_port_that_is_not_dsm_is_an_unknown_device() {
        // a local listener that speaks HTTP and SSH-like banners stands in for "something answered"
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten().take(1) {
                let mut s = s;
                let _ = s.write_all(b"SSH-2.0-OpenSSH_9.2\r\n");
            }
        });
        let r = check_ssh(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port), "SSH".into());
        assert_eq!((r.state, r.detail.as_str()), (CheckState::Answered, "SSH-2.0-OpenSSH_9.2"));
        let closed = check_ssh(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 1), "SSH".into());
        assert_eq!(closed.state, CheckState::Closed, "refused: the host is there, the port is not");
    }
}
