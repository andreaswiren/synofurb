//! Bonjour/mDNS: DSM advertises `_diskstation._tcp`; a NAS installed by synofurb advertises `_synofurb._tcp`.

use super::Found;
use super::scan::CancelToken;
use anyhow::Result;
use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub const DSM_SERVICE: &str = "_diskstation._tcp.local.";
pub const SYNOFURB_SERVICE: &str = "_synofurb._tcp.local.";

/// Browse on the adapter that has `local` only, for `timeout`, or until `cancel`. `progress` gets `(elapsed_ms, timeout_ms)`.
pub fn scan_network(
    local: Ipv4Addr,
    timeout: Duration,
    emit: &dyn Fn(Found),
    cancel: &CancelToken,
    progress: &dyn Fn(u32, u32),
) -> Result<usize> {
    let d = ServiceDaemon::new()?;
    // only the adapter of the selected network (later selections win): the other adapters send and hear nothing
    d.disable_interface(IfKind::All)?;
    d.enable_interface(IfKind::Addr(IpAddr::V4(local)))?;
    let r1 = d.browse(DSM_SERVICE)?;
    let r2 = d.browse(SYNOFURB_SERVICE)?;
    let (start, mut n, mut last) = (Instant::now(), 0, Duration::ZERO);
    let total_ms = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
    while start.elapsed() < timeout && !cancel.is_cancelled() {
        for (rx, ours) in [(&r1, false), (&r2, true)] {
            while let Ok(ev) = rx.try_recv() {
                if let ServiceEvent::ServiceResolved(info) = ev {
                    let ips: Vec<IpAddr> = info.get_addresses_v4().into_iter().map(IpAddr::V4).collect();
                    if ips.is_empty() {
                        continue;
                    }
                    let host = info.get_hostname().trim_end_matches('.').trim_end_matches(".local").to_string();
                    let txt = |k: &str| info.get_property_val_str(k).unwrap_or("").to_string();
                    let f = Found {
                        ips,
                        hostname: host,
                        model: txt("model"),
                        serial: txt("serial"),
                        mac: info.get_property_val_str("mac_address").and_then(|m| m.split([';', ',']).next().map(String::from)),
                        http_port: if ours { None } else { Some(info.get_port()) },
                        synology: !ours,
                        synofurb_debian: ours,
                        sources: vec![if ours { "mdns:synofurb".into() } else { "mdns:diskstation".into() }],
                        ..Default::default()
                    };
                    n += 1;
                    emit(f);
                }
            }
        }
        if start.elapsed() >= last + Duration::from_millis(250) {
            last = start.elapsed();
            progress(last.as_millis() as u32, total_ms);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = d.shutdown();
    progress(start.elapsed().as_millis().min(u128::from(total_ms)) as u32, total_ms);
    Ok(n)
}

/// All recommended networks (real LAN only). Prefer [`crate::discovery::scan_with`].
pub fn scan(timeout: Duration, tx: &Sender<Found>) -> Result<usize> {
    let cancel = CancelToken::new();
    let nets = super::recommended_resolved();
    super::per_network(&nets, |n| match n.local_ip {
        Some(ip) => scan_network(ip, timeout, &|f| drop(tx.send(f)), &cancel, &|_, _| {}),
        None => Ok(0),
    })
}
