//! Finding Synology NAS units on the LAN.
//!
//! Synology's own tools find units with several independent mechanisms; we use the same ones and
//! merge the results:
//!
//! * **Synology Assistant broadcast**: UDP 9999 query, TLV replies with model/serial/MAC/DSM version
//!   (format documented in third-party reimplementations; best effort, see [`syno_udp`]).
//! * **Bonjour/mDNS**: DSM advertises `_diskstation._tcp`; we also look for `_synofurb._tcp`, which a
//!   NAS installed by this tool advertises.
//! * **SSDP/UPnP**: DSM answers M-SEARCH and its device description names the manufacturer and model.
//! * **DSM Web API fingerprint**: an unauthenticated `SYNO.API.Info` query on port 5000 proves a DSM.
//! * **MAC address (OUI)**: Synology's registered prefixes, read from the ARP cache where available.
//! * **Subnet sweep**: connect to the DSM ports across the local /24(s) when multicast is blocked.

pub mod mdns;
pub mod probe;
pub mod ssdp;
pub mod syno_udp;

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// IEEE OUI prefixes registered to Synology Inc. (lowercase, colon separated).
pub const SYNOLOGY_OUIS: &[&str] = &["00:11:32", "90:09:d0"];

pub fn normalize_mac(mac: &str) -> Option<String> {
    let hex: String = mac.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return None;
    }
    let hex = hex.to_lowercase();
    Some((0..6).map(|i| &hex[i * 2..i * 2 + 2]).collect::<Vec<_>>().join(":"))
}

pub fn is_synology_mac(mac: &str) -> bool {
    normalize_mac(mac).is_some_and(|m| SYNOLOGY_OUIS.iter().any(|o| m.starts_with(o)))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub ips: Vec<IpAddr>,
    pub mac: Option<String>,
    pub hostname: String,
    pub model: String,
    pub serial: String,
    pub dsm_version: String,
    pub platform: String,
    pub http_port: Option<u16>,
    pub https_port: Option<u16>,
    /// Positively identified as Synology hardware/DSM (not just an unknown host).
    pub synology: bool,
    /// Already running Debian installed by synofurb.
    pub synofurb_debian: bool,
    pub sources: Vec<String>,
    pub last_seen: String,
}

impl Found {
    pub fn primary_ip(&self) -> Option<IpAddr> {
        self.ips.iter().find(|i| i.is_ipv4()).or(self.ips.first()).copied()
    }
    pub fn title(&self) -> String {
        let name = if !self.model.is_empty() {
            self.model.clone()
        } else if !self.hostname.is_empty() {
            self.hostname.clone()
        } else {
            "Synology".into()
        };
        match self.primary_ip() {
            Some(ip) => format!("{name} ({ip})"),
            None => name,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub devices: Vec<Found>,
}

fn fill(dst: &mut String, src: &str) {
    if dst.is_empty() && !src.is_empty() {
        *dst = src.to_string();
    }
}

impl Inventory {
    /// Merge a sighting into the inventory (same MAC, serial, or any shared IP = same device).
    pub fn merge(&mut self, mut f: Found) {
        f.mac = f.mac.as_deref().and_then(normalize_mac);
        if f.mac.as_deref().is_some_and(is_synology_mac) {
            f.synology = true;
        }
        let idx = self.devices.iter().position(|d| {
            (f.mac.is_some() && d.mac == f.mac) || (!f.serial.is_empty() && d.serial == f.serial) || d.ips.iter().any(|i| f.ips.contains(i))
        });
        match idx {
            None => self.devices.push(f),
            Some(i) => {
                let d = &mut self.devices[i];
                for ip in f.ips {
                    if !d.ips.contains(&ip) {
                        d.ips.push(ip);
                    }
                }
                if d.mac.is_none() {
                    d.mac = f.mac;
                }
                fill(&mut d.hostname, &f.hostname);
                fill(&mut d.model, &f.model);
                fill(&mut d.serial, &f.serial);
                fill(&mut d.dsm_version, &f.dsm_version);
                fill(&mut d.platform, &f.platform);
                d.http_port = d.http_port.or(f.http_port);
                d.https_port = d.https_port.or(f.https_port);
                d.synology |= f.synology;
                d.synofurb_debian |= f.synofurb_debian;
                for s in f.sources {
                    if !d.sources.contains(&s) {
                        d.sources.push(s);
                    }
                }
                if !f.last_seen.is_empty() {
                    d.last_seen = f.last_seen;
                }
            }
        }
    }

    pub fn synology_devices(&self) -> impl Iterator<Item = &Found> {
        self.devices.iter().filter(|d| d.synology || d.synofurb_debian)
    }

    pub fn load(path: &std::path::Path) -> Inventory {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// MAC for an IPv4 address from the Linux ARP cache (empty on other platforms).
pub fn arp_lookup(ip: IpAddr) -> Option<String> {
    let text = std::fs::read_to_string("/proc/net/arp").ok()?;
    parse_arp(&text, ip)
}

pub fn parse_arp(text: &str, ip: IpAddr) -> Option<String> {
    let want = ip.to_string();
    text.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 4 && f[0] == want && f[3] != "00:00:00:00:00:00").then(|| normalize_mac(f[3])).flatten()
    })
}

pub struct ScanOptions {
    pub timeout: std::time::Duration,
    pub sweep: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { timeout: std::time::Duration::from_secs(4), sweep: true }
    }
}

/// Run every discovery mechanism concurrently and merge the results.
pub fn scan(opts: &ScanOptions, log: &crate::logs::Logger) -> Inventory {
    let (tx, rx) = std::sync::mpsc::channel::<Found>();
    let mut handles = vec![];
    macro_rules! spawn {
        ($name:literal, $body:expr) => {{
            let tx = tx.clone();
            let log = log.clone();
            handles.push(std::thread::spawn(move || match $body(&tx) {
                Ok(n) => log.debug(format!("discovery {}: {} result(s)", $name, n)),
                Err(e) => log.warn(format!("discovery {} failed: {e}", $name)),
            }));
        }};
    }
    let t = opts.timeout;
    spawn!("synology-udp", move |tx: &std::sync::mpsc::Sender<Found>| syno_udp::scan(t, tx));
    spawn!("mdns", move |tx: &std::sync::mpsc::Sender<Found>| mdns::scan(t, tx));
    spawn!("ssdp", move |tx: &std::sync::mpsc::Sender<Found>| ssdp::scan(t, tx));
    if opts.sweep {
        spawn!("sweep", move |tx: &std::sync::mpsc::Sender<Found>| probe::sweep(tx));
    }
    drop(tx);
    let mut inv = Inventory::default();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for mut f in rx {
        f.last_seen = now.clone();
        if f.mac.is_none()
            && let Some(ip) = f.primary_ip()
        {
            f.mac = arp_lookup(ip);
        }
        inv.merge(f);
    }
    for h in handles {
        let _ = h.join();
    }
    // second pass: ARP is populated after traffic
    for d in &mut inv.devices {
        if d.mac.is_none()
            && let Some(ip) = d.primary_ip()
        {
            d.mac = arp_lookup(ip);
            if d.mac.as_deref().is_some_and(is_synology_mac) {
                d.synology = true;
                d.sources.push("oui".into());
            }
        }
    }
    inv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(ip: &str, mac: Option<&str>, model: &str, src: &str) -> Found {
        Found {
            ips: vec![ip.parse().unwrap()],
            mac: mac.map(String::from),
            model: model.into(),
            sources: vec![src.into()],
            ..Default::default()
        }
    }

    #[test]
    fn mac_normalization_and_oui() {
        assert_eq!(normalize_mac("00-11-32-AA-BB-CC").as_deref(), Some("00:11:32:aa:bb:cc"));
        assert_eq!(normalize_mac("001132aabbcc").as_deref(), Some("00:11:32:aa:bb:cc"));
        assert!(normalize_mac("zz").is_none());
        assert!(is_synology_mac("00:11:32:AA:BB:CC"));
        assert!(!is_synology_mac("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn merging_by_mac_ip_and_serial() {
        let mut inv = Inventory::default();
        inv.merge(f("192.168.1.10", Some("00:11:32:aa:bb:cc"), "", "mdns"));
        inv.merge(Found {
            model: "DS918+".into(),
            serial: "SN1".into(),
            ips: vec!["192.168.1.10".parse().unwrap(), "fe80::1".parse().unwrap()],
            sources: vec!["udp".into()],
            ..Default::default()
        });
        inv.merge(f("192.168.1.99", Some("00-11-32-AA-BB-CC"), "", "ssdp")); // same MAC, different IP
        inv.merge(f("192.168.1.50", None, "Other", "ssdp"));
        assert_eq!(inv.devices.len(), 2);
        let d = &inv.devices[0];
        assert_eq!(d.model, "DS918+");
        assert_eq!(d.ips.len(), 3);
        assert!(d.synology, "OUI marks it as Synology");
        assert_eq!(d.sources.len(), 3);
        assert_eq!(inv.synology_devices().count(), 1);
        assert!(d.title().contains("DS918+"));
    }

    #[test]
    fn arp_parsing() {
        let t = "IP address       HW type     Flags       HW address            Mask     Device\n192.168.1.10    0x1         0x2         00:11:32:aa:bb:cc     *        eth0\n192.168.1.11    0x1         0x0         00:00:00:00:00:00     *        eth0\n";
        assert_eq!(parse_arp(t, "192.168.1.10".parse().unwrap()).as_deref(), Some("00:11:32:aa:bb:cc"));
        assert!(parse_arp(t, "192.168.1.11".parse().unwrap()).is_none());
    }

    #[test]
    fn inventory_roundtrip() {
        let mut inv = Inventory::default();
        inv.merge(f("10.0.0.5", Some("00:11:32:00:00:01"), "DS220+", "udp"));
        let p = std::env::temp_dir().join(format!("sf-inv-{}.json", std::process::id()));
        inv.save(&p).unwrap();
        assert_eq!(Inventory::load(&p).devices.len(), 1);
    }
}
