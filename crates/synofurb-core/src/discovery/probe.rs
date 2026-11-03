//! Port/Web API probing: proves that a host runs DSM, and sweeps local subnets when multicast is blocked.

use super::Found;
use super::scan::CancelToken;
use anyhow::Result;
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// DSM's documented, unauthenticated API discovery call.
pub const API_INFO_PATH: &str = "/webapi/query.cgi?api=SYNO.API.Info&version=1&method=query&query=SYNO.API.Auth";

/// True if an `SYNO.API.Info` response looks like DSM.
pub fn looks_like_dsm(body: &str) -> bool {
    body.contains("SYNO.API.Auth") && body.contains("\"success\"")
}

pub fn probe_dsm(ip: IpAddr) -> Option<Found> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(1000)).build();
    let body = agent.get(&format!("http://{ip}:5000{API_INFO_PATH}")).call().ok()?.into_string().ok()?;
    if !looks_like_dsm(&body) {
        return None;
    }
    let https = TcpStream::connect_timeout(&SocketAddr::new(ip, 5001), Duration::from_millis(300)).is_ok();
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

/// Is a DSM answering on `ip`? Port 5000 must accept within 350 ms, then the Web API must identify DSM.
pub fn probe_host(ip: Ipv4Addr) -> Option<Found> {
    let ip = IpAddr::V4(ip);
    TcpStream::connect_timeout(&SocketAddr::new(ip, 5000), Duration::from_millis(350)).ok()?;
    probe_dsm(ip)
}

/// Probe every address of every list with a pool of workers. `lists[i]` is the address list of network `i`.
/// `on_found(i, device)` runs for each DSM; `on_progress(i, done, total)` runs about five times a second while `done` changes
/// and once more at the end. Stops taking new addresses as soon as `cancel` is set (a probe already in flight finishes: at
/// most about a second). Returns the number of DSMs found per network.
pub fn sweep_lists(
    lists: &[Vec<Ipv4Addr>],
    cancel: &CancelToken,
    probe: &(dyn Fn(Ipv4Addr) -> Option<Found> + Sync),
    on_found: &(dyn Fn(usize, Found) + Sync),
    on_progress: &(dyn Fn(usize, u32, u32) + Sync),
) -> Vec<usize> {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
    let queue: Mutex<VecDeque<(usize, Ipv4Addr)>> =
        Mutex::new(lists.iter().enumerate().flat_map(|(i, l)| l.iter().map(move |h| (i, *h))).collect());
    let total: Vec<u32> = lists.iter().map(|l| l.len() as u32).collect();
    let done: Vec<AtomicU32> = lists.iter().map(|_| AtomicU32::new(0)).collect();
    let found: Vec<AtomicUsize> = lists.iter().map(|_| AtomicUsize::new(0)).collect();
    let n_workers = queue.lock().map(|q| q.len()).unwrap_or(0).clamp(1, 128);
    let live = AtomicUsize::new(n_workers);
    let report = |last: &mut Vec<u32>| {
        for i in 0..lists.len() {
            let d = done[i].load(Relaxed);
            if d != last[i] {
                last[i] = d;
                on_progress(i, d, total[i]);
            }
        }
    };
    std::thread::scope(|s| {
        for _ in 0..n_workers {
            s.spawn(|| {
                while !cancel.is_cancelled() {
                    let Some((i, h)) = queue.lock().ok().and_then(|mut q| q.pop_front()) else { break };
                    if let Some(f) = probe(h) {
                        found[i].fetch_add(1, Relaxed);
                        on_found(i, f);
                    }
                    done[i].fetch_add(1, Relaxed);
                }
                live.fetch_sub(1, Relaxed);
            });
        }
        let mut last = vec![u32::MAX; lists.len()];
        while live.load(Relaxed) > 0 {
            std::thread::sleep(Duration::from_millis(100));
            report(&mut last);
        }
        report(&mut last);
    });
    found.iter().map(|f| f.load(Relaxed)).collect()
}

/// Sweep the recommended networks (real LAN only), never our own address. Prefer [`crate::discovery::scan_with`].
pub fn sweep(tx: &Sender<Found>) -> Result<usize> {
    let nets = super::recommended_resolved();
    let own: Vec<Ipv4Addr> = super::networks::local_addrs().into_iter().map(|a| a.ip).collect();
    let mut seen = std::collections::HashSet::new();
    let lists: Vec<Vec<Ipv4Addr>> =
        nets.iter().map(|n| super::networks::sweep_hosts(&n.sweep, &own).into_iter().filter(|h| seen.insert(*h)).collect()).collect();
    let found = sweep_lists(&lists, &CancelToken::new(), &probe_host, &|_, f| drop(tx.send(f)), &|_, _, _| {});
    Ok(found.iter().sum())
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
