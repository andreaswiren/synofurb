//! `scan` and `inventory`.

use super::args::{InventoryCmd, ScanArgs};
use super::exit::{CliError, CliResult};
use super::facts::config_dir;
use super::out::Out;
use serde_json::{Value, json};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use synofurb_core::discovery::{self, CancelToken, Found, Inventory, NetworkInfo, ScanEvent, ScanMethod, ScanNetwork, ScanOptions, probe};

pub fn inventory_path() -> PathBuf {
    config_dir().join("inventory.json")
}

/// Outcome of one discovery method on one network, for `-v`.
pub struct MethodResult {
    pub name: &'static str,
    pub network: String,
    pub result: Result<usize, String>,
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Run the scan and stream the merged inventory (a thin layer over [`discovery::scan_with_callback`], kept for the wizard).
/// `on_update` gets the merged device after each sighting.
pub fn scan_stream(
    opts: &ScanOptions,
    known: Inventory,
    mut on_update: impl FnMut(&Found),
    mut on_method: impl FnMut(MethodResult),
) -> Inventory {
    let r = discovery::scan_with_callback(opts, &CancelToken::new(), |e| match e {
        ScanEvent::Found(d) => on_update(&d),
        ScanEvent::MethodFinished { method, network, found, error } => {
            on_method(MethodResult { name: method.label(), network, result: error.map_or(Ok(found), Err) });
        }
        _ => {}
    });
    let mut inv = known;
    for d in r.map(|i| i.devices).unwrap_or_default() {
        inv.merge(d);
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

/// `Ethernet 192.168.69.0/24 [LAN]` for the "scanning" lines.
fn net_line(n: &discovery::ResolvedNetwork) -> String {
    format!("{} [{}]", n.label, n.kind.map_or("not attached to this computer", |k| k.label()))
}

fn network_row(n: &NetworkInfo) -> String {
    format!(
        "{} {:<28} {:<18} {:<19} {:<10} {}",
        if n.recommended { "[x]" } else { "[ ]" },
        n.interface,
        format!("{}/{}", n.ip, n.prefix),
        n.cidr,
        n.kind.label(),
        n.id
    )
}

pub fn networks_json(nets: &[NetworkInfo]) -> Value {
    json!({"networks": nets})
}

fn list_networks(out: &Out, json: bool) -> CliResult<()> {
    let nets = discovery::networks();
    if json {
        out.json_doc(&networks_json(&nets));
        return Ok(());
    }
    if nets.is_empty() {
        out.data("no IPv4 network found on this computer");
        return Ok(());
    }
    out.data(&format!("{:<3} {:<28} {:<18} {:<19} {:<10} {}", "", "INTERFACE", "ADDRESS", "NETWORK", "KIND", "ID"));
    for n in &nets {
        out.data(&network_row(n));
    }
    out.say(
        "[x] = scanned by default (real LAN networks). Pick others with --network <CIDR|interface|id> (repeatable) or use --all-networks.",
    );
    Ok(())
}

/// The networks `scan` will cover: `--network` entries, or `--all-networks`, or the recommended ones.
fn selection(a: &ScanArgs, avail: &[NetworkInfo]) -> CliResult<Vec<ScanNetwork>> {
    let mut sel: Vec<ScanNetwork> = vec![];
    if !a.network.is_empty() {
        for t in &a.network {
            for n in discovery::parse_selector(t, avail).map_err(|e| CliError::usage(e.to_string()))? {
                if !sel.contains(&n) {
                    sel.push(n);
                }
            }
        }
    } else {
        sel = avail.iter().filter(|n| a.all_networks || n.recommended).map(ScanNetwork::from_info).collect();
    }
    if sel.is_empty() {
        let hint = if avail.is_empty() {
            "this computer has no IPv4 network"
        } else {
            "none of this computer's networks is a real LAN (see `scan --list-networks`); name one with --network or use --all-networks"
        };
        return Err(CliError::usage(format!("no network to scan: {hint}")));
    }
    Ok(sel)
}

pub fn scan(out: &Out, a: ScanArgs) -> CliResult<()> {
    if a.list_networks {
        return list_networks(out, a.json);
    }
    let avail = discovery::networks();
    let sel = selection(&a, &avail)?;
    let mut opts =
        ScanOptions { networks: sel, methods: discovery::ScanMethods::default(), timeout: Duration::from_secs(a.timeout), sweep: true };
    opts.methods.probe_sweep = !a.no_sweep;
    let path = inventory_path();
    let known = Inventory::load(&path);
    let live = out.tty_err && !out.quiet && !a.ndjson;
    let mut seen_live: Vec<IpAddr> = vec![];
    let (all, ndjson) = (a.all, a.ndjson);
    let mut scanned: Vec<discovery::ResolvedNetwork> = vec![];
    let mut method_errors: Vec<Value> = vec![];
    let mut method_ok = 0usize;
    let mut sweep_step: std::collections::HashMap<String, u32> = Default::default();
    let label_of =
        |nets: &[discovery::ResolvedNetwork], key: &str| nets.iter().find(|n| n.key == key).map_or(key.to_string(), |n| n.label.clone());
    let found = discovery::scan_with_callback(&opts, &CancelToken::new(), |e| {
        if ndjson {
            let skip = matches!(&e, ScanEvent::Found(d) if !is_listed(d, all));
            if !skip {
                out.data(&serde_json::to_string(&e).unwrap_or_default());
            }
        }
        match &e {
            ScanEvent::Started { networks, methods, total_hosts } => {
                scanned = networks.clone();
                let names: Vec<&str> = methods.iter().map(|m| m.label()).collect();
                out.info(&format!(
                    "Scanning {} network(s) with {} ({} s per listening method{}):",
                    networks.len(),
                    names.join(", "),
                    a.timeout,
                    if methods.contains(&ScanMethod::ProbeSweep) {
                        format!(", {total_hosts} addresses to probe")
                    } else {
                        ", no port sweep".into()
                    }
                ));
                for n in networks {
                    out.info(&format!("  {}", net_line(n)));
                }
                let left: Vec<String> = avail
                    .iter()
                    .filter(|n| !networks.iter().any(|r| r.key == n.id))
                    .map(|n| format!("{} {} [{}]", n.interface, n.cidr, n.kind.label()))
                    .collect();
                if !left.is_empty() {
                    out.info(&format!("Not scanned: {} (use --network or --all-networks to include)", left.join("; ")));
                }
            }
            ScanEvent::MethodStarted { method, network } => {
                out.detail(&format!("  [{}] {}: started", method.label(), label_of(&scanned, network)));
            }
            ScanEvent::MethodProgress { method: ScanMethod::ProbeSweep, network, done, total } if *total > 0 => {
                let step = done * 4 / total; // 25 %, 50 %, 75 %, 100 %
                if step > 0 && sweep_step.get(network) != Some(&step) {
                    sweep_step.insert(network.clone(), step);
                    out.info(&format!("  [{}] {}: {done}/{total} addresses", ScanMethod::ProbeSweep.label(), label_of(&scanned, network)));
                }
            }
            ScanEvent::MethodFinished { method, network, found, error } => {
                let label = label_of(&scanned, network);
                match error {
                    Some(err) => {
                        out.warn(&format!("[{}] {label} failed: {err}", method.label()));
                        method_errors.push(json!({"method": method.id(), "network": network, "error": err}));
                    }
                    None => {
                        method_ok += 1;
                        out.detail(&format!("  [{}] {label}: {found} result(s)", method.label()));
                    }
                }
            }
            ScanEvent::Skipped { method, network, reason } => {
                out.info(&format!("  [{}] {}: skipped, {reason}", method.label(), label_of(&scanned, network)));
            }
            ScanEvent::Found(d) => {
                if live
                    && is_listed(d, all)
                    && let Some(ip) = d.primary_ip()
                    && !seen_live.contains(&ip)
                {
                    seen_live.push(ip);
                    out.say(&format!("  + {}", device_line(d)));
                }
            }
            ScanEvent::MethodProgress { .. } | ScanEvent::Finished { .. } => {}
        }
    })
    .map_err(|e| CliError::usage(e.to_string()))?;
    if method_ok == 0 && !method_errors.is_empty() {
        out.warn("every discovery method failed (no network?); the reasons are above");
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
        let mut doc = inventory_json(&listed);
        doc["networks"] = json!(scanned);
        doc["errors"] = json!(method_errors);
        out.json_doc(&doc);
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
