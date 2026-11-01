//! Bonjour/mDNS: DSM advertises `_diskstation._tcp`; a NAS installed by synofurb advertises `_synofurb._tcp`.

use super::Found;
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::net::IpAddr;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub const DSM_SERVICE: &str = "_diskstation._tcp.local.";
pub const SYNOFURB_SERVICE: &str = "_synofurb._tcp.local.";

pub fn scan(timeout: Duration, tx: &Sender<Found>) -> Result<usize> {
    let d = ServiceDaemon::new()?;
    let r1 = d.browse(DSM_SERVICE)?;
    let r2 = d.browse(SYNOFURB_SERVICE)?;
    let (start, mut n) = (Instant::now(), 0);
    while start.elapsed() < timeout {
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
                    let _ = tx.send(f);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = d.shutdown();
    Ok(n)
}
