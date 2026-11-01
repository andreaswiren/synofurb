//! `scan` and `inventory`.

use super::args::{InventoryCmd, ScanArgs};
use super::exit::{CliError, CliResult};
use super::facts::config_dir;
use super::out::Out;
use serde_json::{Value, json};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::Duration;
use synofurb_core::discovery::{self, Found, Inventory, ScanOptions, mdns, probe, ssdp, syno_udp};

pub fn inventory_path() -> PathBuf {
    config_dir().join("inventory.json")
}

/// Outcome of one discovery method, for `-v`.
pub struct MethodResult {
    pub name: &'static str,
    pub result: Result<usize, String>,
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Run every discovery method on its own thread and stream the merged inventory. (The core's `discovery::scan` only returns at the
/// end; this is the same orchestration with a hook per sighting, like the app's.) `on_update` gets the merged device after each sighting.
pub fn scan_stream(
    opts: &ScanOptions,
    known: Inventory,
    mut on_update: impl FnMut(&Found),
    mut on_method: impl FnMut(MethodResult),
) -> Inventory {
    let (ftx, frx) = channel::<Found>();
    let (mtx, mrx) = channel::<MethodResult>();
    let t = opts.timeout;
    let mut handles = vec![];
    macro_rules! method {
        ($name:literal, $f:expr) => {{
            let (ftx, mtx) = (ftx.clone(), mtx.clone());
            handles.push(std::thread::spawn(move || {
                let r = $f(&ftx).map_err(|e| format!("{e:#}"));
                let _ = mtx.send(MethodResult { name: $name, result: r });
            }));
        }};
    }
    method!("Synology Assistant broadcast", move |s: &Sender<Found>| syno_udp::scan(t, s));
    method!("Bonjour/mDNS", move |s: &Sender<Found>| mdns::scan(t, s));
    method!("UPnP/SSDP", move |s: &Sender<Found>| ssdp::scan(t, s));
    if opts.sweep {
        method!("Subnet sweep (DSM ports)", |s: &Sender<Found>| probe::sweep(s));
    }
    drop(ftx);
    drop(mtx);
    let mut inv = known;
    let now = now_stamp();
    loop {
        for m in mrx.try_iter() {
            on_method(m);
        }
        match frx.recv_timeout(Duration::from_millis(100)) {
            Ok(mut f) => {
                f.last_seen = now.clone();
                if f.mac.is_none()
                    && let Some(ip) = f.primary_ip()
                {
                    f.mac = discovery::arp_lookup(ip);
                }
                let ips = f.ips.clone();
                let mac = f.mac.clone();
                inv.merge(f);
                if let Some(d) = inv.devices.iter().find(|d| d.ips.iter().any(|i| ips.contains(i)) || (mac.is_some() && d.mac == mac)) {
                    on_update(d);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    for h in handles {
        let _ = h.join();
    }
    for m in mrx.try_iter() {
        on_method(m);
    }
    // ARP is populated after the traffic: a second chance to recognise units by their Synology MAC prefix
    for d in &mut inv.devices {
        if d.mac.is_none()
            && let Some(ip) = d.primary_ip()
        {
            d.mac = discovery::arp_lookup(ip);
            if d.mac.as_deref().is_some_and(discovery::is_synology_mac) {
                d.synology = true;
                d.sources.push("oui".into());
            }
        }
    }
    inv
}

pub fn device_line(d: &Found) -> String {
    format!(
        "{:28} mac {:17} DSM {:12} serial {:14} via {}",
        d.title(),
        d.mac.clone().unwrap_or_default(),
        d.dsm_version,
        d.serial,
        d.sources.join(",")
    )
}

fn is_listed(d: &Found, all: bool) -> bool {
    all || d.synology || d.synofurb_debian || d.sources.iter().any(|s| s == "manual")
}

pub fn inventory_json(devs: &[&Found]) -> Value {
    json!({"devices": devs})
}

pub fn scan(out: &Out, a: ScanArgs) -> CliResult<()> {
    let opts = ScanOptions { timeout: Duration::from_secs(a.timeout), sweep: !a.no_sweep };
    let path = inventory_path();
    let known = Inventory::load(&path);
    out.info(&format!("Scanning for Synology units ({} s per method{}) ...", a.timeout, if a.no_sweep { ", no port sweep" } else { "" }));
    let live = out.tty_err && !out.quiet && !a.ndjson;
    let mut seen_live: Vec<IpAddr> = vec![];
    let all = a.all;
    let ndjson = a.ndjson;
    let mut methods: Vec<MethodResult> = vec![];
    let found = scan_stream(
        &opts,
        Inventory::default(),
        |d| {
            if !is_listed(d, all) {
                return;
            }
            if ndjson {
                out.data(&serde_json::to_string(d).unwrap_or_default());
            } else if live
                && let Some(ip) = d.primary_ip()
                && !seen_live.contains(&ip)
            {
                seen_live.push(ip);
                out.say(&format!("  + {}", device_line(d)));
            }
        },
        |m| {
            if let Err(e) = &m.result {
                out.detail(&format!("discovery method '{}' failed: {e}", m.name));
            } else if let Ok(n) = &m.result {
                out.detail(&format!("discovery method '{}': {n} result(s)", m.name));
            }
            methods.push(m);
        },
    );
    if !methods.is_empty() && methods.iter().all(|m| m.result.is_err()) {
        out.warn("every discovery method failed (no network?); run with -v for the reasons");
    }
    let listed: Vec<&Found> = found.devices.iter().filter(|d| is_listed(d, a.all)).collect();
    if !a.no_save {
        let mut merged = known;
        for d in &found.devices {
            merged.merge(d.clone());
        }
        if let Err(e) = merged.save(&path) {
            out.warn(&format!("could not save the inventory to {}: {e}", path.display()));
        }
    }
    if a.json {
        out.json_doc(&inventory_json(&listed));
    } else if !a.ndjson {
        let n = found.synology_devices().count();
        out.data(&format!("{n} Synology device(s) found"));
        for d in &listed {
            out.data(&format!("  {}", device_line(d)));
        }
    }
    Ok(())
}

pub fn inventory(out: &Out, action: InventoryCmd) -> CliResult<()> {
    let path = inventory_path();
    match action {
        InventoryCmd::List { json, all } => {
            let inv = Inventory::load(&path);
            let listed: Vec<&Found> = inv.devices.iter().filter(|d| is_listed(d, all)).collect();
            if json {
                out.json_doc(&inventory_json(&listed));
            } else {
                out.data(&format!("{} remembered device(s) ({})", listed.len(), path.display()));
                for d in listed {
                    out.data(&format!("  {}   last seen {}", device_line(d), d.last_seen));
                }
            }
            Ok(())
        }
        InventoryCmd::Forget { pattern, all } => {
            let mut inv = Inventory::load(&path);
            let before = inv.devices.len();
            if all {
                inv.devices.clear();
            } else {
                let p =
                    pattern.ok_or_else(|| CliError::usage("say what to forget: an address, MAC, serial, model or host name, or --all"))?;
                inv.devices.retain(|d| !matches_pattern(d, &p));
            }
            let removed = before - inv.devices.len();
            inv.save(&path)?;
            out.data(&format!("forgot {removed} device(s)"));
            if removed == 0 && !all {
                return Err(CliError::Other(anyhow::anyhow!("nothing in the inventory matches that")));
            }
            Ok(())
        }
        InventoryCmd::Add { ip, name } => {
            let mut inv = Inventory::load(&path);
            out.info(&format!("Probing {ip} for a DSM web API ..."));
            let mut f = match probe::probe_dsm(ip) {
                Some(f) => {
                    out.info("a DSM answered");
                    f
                }
                None => {
                    out.warn(&format!("{ip} did not answer like a DSM (port 5000/5001); added anyway"));
                    Found { ips: vec![ip], ..Default::default() }
                }
            };
            f.sources.push("manual".into());
            if let Some(n) = name {
                f.hostname = n;
            }
            f.last_seen = now_stamp();
            if f.mac.is_none() {
                f.mac = discovery::arp_lookup(ip);
            }
            inv.merge(f);
            inv.save(&path)?;
            out.data(&format!("added {ip}; {} device(s) in the inventory", inv.devices.len()));
            Ok(())
        }
    }
}

/// Does `pattern` name this device? Address, MAC (any separator), serial, model or host name, case-insensitive.
pub fn matches_pattern(d: &Found, pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() {
        return false;
    }
    if let Ok(ip) = p.parse::<IpAddr>() {
        return d.ips.contains(&ip);
    }
    if let Some(mac) = discovery::normalize_mac(p) {
        return d.mac.as_deref() == Some(mac.as_str());
    }
    d.serial.eq_ignore_ascii_case(p) || d.model.eq_ignore_ascii_case(p) || d.hostname.eq_ignore_ascii_case(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> Found {
        Found {
            ips: vec!["192.168.1.20".parse().unwrap()],
            mac: Some("00:11:32:aa:bb:cc".into()),
            model: "DS923+".into(),
            serial: "2150ABC".into(),
            hostname: "attic".into(),
            synology: true,
            sources: vec!["mdns".into()],
            ..Default::default()
        }
    }

    #[test]
    fn forget_patterns() {
        let d = dev();
        for p in ["192.168.1.20", "00-11-32-AA-BB-CC", "001132aabbcc", "ds923+", "2150abc", "ATTIC"] {
            assert!(matches_pattern(&d, p), "{p}");
        }
        for p in ["192.168.1.21", "00:11:32:aa:bb:cd", "ds918+", "", "att"] {
            assert!(!matches_pattern(&d, p), "{p:?}");
        }
    }

    #[test]
    fn listing_rules_and_json_shape() {
        let mut other = Found { ips: vec!["10.0.0.9".parse().unwrap()], ..Default::default() };
        assert!(!is_listed(&other, false) && is_listed(&other, true));
        other.sources.push("manual".into());
        assert!(is_listed(&other, false), "manual entries are always listed");
        let d = dev();
        let v = inventory_json(&[&d]);
        assert_eq!(v["devices"][0]["model"], "DS923+");
        assert_eq!(v["devices"][0]["ips"][0], "192.168.1.20");
        assert!(v["devices"][0]["synology"].as_bool().unwrap());
        assert!(device_line(&d).contains("DS923+ (192.168.1.20)"));
    }
}
