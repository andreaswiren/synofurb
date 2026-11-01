//! Port/Web API probing: proves that a host runs DSM, and sweeps local subnets when multicast is blocked.

use super::Found;
use anyhow::Result;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// DSM's documented, unauthenticated API discovery call.
pub const API_INFO_PATH: &str = "/webapi/query.cgi?api=SYNO.API.Info&version=1&method=query&query=SYNO.API.Auth";

/// True if an `SYNO.API.Info` response looks like DSM.
pub fn looks_like_dsm(body: &str) -> bool {
    body.contains("SYNO.API.Auth") && body.contains("\"success\"")
}

pub fn probe_dsm(ip: IpAddr) -> Option<Found> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(1500)).build();
    let body = agent.get(&format!("http://{ip}:5000{API_INFO_PATH}")).call().ok()?.into_string().ok()?;
    if !looks_like_dsm(&body) {
        return None;
    }
    let https = TcpStream::connect_timeout(&SocketAddr::new(ip, 5001), Duration::from_millis(500)).is_ok();
    Some(Found {
        ips: vec![ip],
        http_port: Some(5000),
        https_port: https.then_some(5001),
        synology: true,
        sources: vec!["dsm-api".into()],
        ..Default::default()
    })
}

/// Hosts of the /24 containing `ip` (limits sweeps of larger networks to the local /24).
pub fn local_24(ip: Ipv4Addr) -> Vec<Ipv4Addr> {
    let o = ip.octets();
    (1..=254u8).map(|h| Ipv4Addr::new(o[0], o[1], o[2], h)).filter(|a| *a != ip).collect()
}

pub fn sweep(tx: &Sender<Found>) -> Result<usize> {
    let mut hosts = vec![];
    for i in if_addrs::get_if_addrs()? {
        if let if_addrs::IfAddr::V4(a) = i.addr
            && !a.ip.is_loopback()
            && !a.ip.is_link_local()
        {
            hosts.extend(local_24(a.ip));
        }
    }
    hosts.sort();
    hosts.dedup();
    let hosts = std::sync::Arc::new(std::sync::Mutex::new(hosts));
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut workers = vec![];
    for _ in 0..64 {
        let (hosts, tx, n) = (hosts.clone(), tx.clone(), n.clone());
        workers.push(std::thread::spawn(move || {
            loop {
                let Some(h) = hosts.lock().unwrap().pop() else { break };
                let ip = IpAddr::V4(h);
                if TcpStream::connect_timeout(&SocketAddr::new(ip, 5000), Duration::from_millis(350)).is_ok()
                    && let Some(f) = probe_dsm(ip)
                {
                    n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = tx.send(f);
                }
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(n.load(std::sync::atomic::Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsm_fingerprint() {
        assert!(looks_like_dsm(r#"{"data":{"SYNO.API.Auth":{"maxVersion":7,"minVersion":1,"path":"entry.cgi"}},"success":true}"#));
        assert!(!looks_like_dsm("<html>router</html>"));
    }

    #[test]
    fn sweep_hosts() {
        let h = local_24("192.168.1.7".parse().unwrap());
        assert_eq!(h.len(), 253);
        assert!(!h.contains(&"192.168.1.7".parse().unwrap()));
    }
}
