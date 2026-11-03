//! Demo mode (`synofurb gui --demo --screen <name>`): built-in units and scripted install events so every screen can
//! be shown and screenshotted without hardware. Nothing here touches the network or a NAS.

use super::addhost::{self, AddedHost, AddedHosts, CheckResult, CheckState, ProbeReport, Target};
use super::worker::{DEMO_TICK, Msg, Tx};
use std::net::IpAddr;
use synofurb_core::compat::Status;
use synofurb_core::discovery::{Found, Inventory};
use synofurb_core::inspect::{fixtures, parse_inspect};
use synofurb_core::logs::{Event, Level, LogLine, Phase, Source};
use synofurb_core::model::DeviceFacts;
use synofurb_core::session::Outcome;
use synofurb_core::updater::{AssetInfo, Channel, UpdateInfo};

/// Which built-in unit demo mode inspects. Chosen with the `SYNOFURB_DEMO_UNIT` environment variable
/// (`ds918` default, `ds1621` for the AMD + boot-flash + raw-console variant, `rs816` for ARM / U-Boot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoUnit {
    Ds918,
    Ds1621,
    Rs816,
    /// DS918+ with one larger disk (mixed sizes: Quick setup asks Hybrid vs RAID).
    Mixed,
    /// The owner's real DS918+: 4x 8 TB, two DSM volumes (ext4 and btrfs), md2 resyncing.
    Volumes,
    /// The same unit with a USB stick attached (a disk that is listed but not touched).
    VolumesUsb,
    /// The same unit with one disk missing from md3 (a degraded array): the Alert on Inspect.
    VolumesDegraded,
}

impl DemoUnit {
    pub fn from_env() -> DemoUnit {
        match std::env::var("SYNOFURB_DEMO_UNIT").unwrap_or_default().to_ascii_lowercase().as_str() {
            "ds1621" | "amd" | "dom" => DemoUnit::Ds1621,
            "rs816" | "arm" | "uboot" => DemoUnit::Rs816,
            "mixed" => DemoUnit::Mixed,
            "volumes" | "ds918-volumes" => DemoUnit::Volumes,
            "volumes-usb" | "usb" => DemoUnit::VolumesUsb,
            "volumes-degraded" | "degraded" => DemoUnit::VolumesDegraded,
            _ => DemoUnit::Ds918,
        }
    }

    /// The raw `inspect` output for the unit, as the NAS would send it.
    pub fn raw(self) -> String {
        let base = fixtures::ds_4bay();
        match self {
            DemoUnit::Volumes | DemoUnit::Ds918 => {
                fixtures::ds918_two_volumes().replace("KV|uefi|0\n", "KV|uefi|0\nKV|cpu_vendor|GenuineIntel\n")
            }
            DemoUnit::VolumesDegraded => fixtures::ds918_two_volumes()
                .replace("KV|uefi|0\n", "KV|uefi|0\nKV|cpu_vendor|GenuineIntel\n")
                // md3 lost sdb6: one member missing, the other state words as the kernel reports them
                .replace("MD|md3|raid5|clean|35162439168|sda6,sdd6,sdc6,sdb6|u3|awss01:3|1.2|idle|none|0", "MD|md3|raid5|clean, degraded|35162439168|sda6,sdd6,sdc6|u3|awss01:3|1.2|idle|none|1")
                .replace("MD|md2|raid5|active|11692677888|sdd5,sda5,sdb5,sdc5|u2|awss01:2|1.2|resync|1770222464_/_1948779648|0", "MD|md2|raid5|clean|11692677888|sdd5,sda5,sdb5,sdc5|u2|awss01:2|1.2|idle|none|0"),
            DemoUnit::VolumesUsb => {
                let usb = "DISK|sde|SanDisk Ultra|4C530001|125045424|512|1|1|gpt|/sys/devices/pci0000:00/0000:00:15.0/usb1/1-2/1-2:1.0/host5/target5:0:0/5:0:0:0\nPART|sde|sde1|1|2048|125043376\n";
                fixtures::ds918_two_volumes()
                    .replace("KV|uefi|0\n", "KV|uefi|0\nKV|cpu_vendor|GenuineIntel\n")
                    .replace("MD|md0", &format!("{usb}MD|md0"))
            }
            DemoUnit::Mixed => {
                let mut out = base.replace("KV|uefi|0\n", "KV|uefi|0\nKV|cpu_vendor|GenuineIntel\n");
                for d in ["sdc", "sdd"] {
                    let old = format!("|{}|512|1|0|gpt\n", 7_814_037_168u64);
                    let marker = format!("DISK|{d}|");
                    if let Some(i) = out.find(&marker) {
                        let j = i + out[i..].find(&old).unwrap_or(0);
                        out.replace_range(j..j + old.len(), "|15628053168|512|1|0|gpt\n");
                    }
                }
                out
            }
            DemoUnit::Ds1621 => base
                .replace("KV|unique|synology_apollolake_918+", "KV|unique|synology_v1000_1621+")
                .replace("KV|model|DS918+", "KV|model|DS1621+")
                .replace("KV|cpu|Intel(R) Celeron(R) CPU J3455 @ 1.50GHz", "KV|cpu|AMD Ryzen Embedded V1500B with Radeon Vega Gfx")
                .replace("KV|uefi|0\n", "KV|uefi|1\nKV|cpu_vendor|AuthenticAMD\n")
                .replace(
                    "KV|cmdline|root=/dev/md0 console=ttyS0,115200n8 quiet",
                    "KV|cmdline|root=/dev/md0 console=tty0 console=uart8250,mmio32,0xfedc9000,115200n8",
                )
                .replace("NIC|eth0", "DOM|262144|65536|196608|1|1\nNIC|eth0"),
            DemoUnit::Rs816 => base
                .replace("KV|uname_m|x86_64", "KV|uname_m|armv7l")
                .replace("KV|unique|synology_apollolake_918+", "KV|unique|synology_armada38x_rs816")
                .replace("KV|model|DS918+", "KV|model|RS816")
                .replace("KV|cpu|Intel(R) Celeron(R) CPU J3455 @ 1.50GHz", "KV|cpu|Marvell Armada 385 (ARMv7)")
                .replace("KV|kexec|1", "KV|kexec|0")
                .replace("KV|dsm_version|7.2.1-69057", "KV|dsm_version|6.2.4-25556")
                .replace("KV|mem_kib|8051240", "KV|mem_kib|1048576")
                .replace("KV|cmdline|root=/dev/md0 console=ttyS0,115200n8 quiet", "KV|cmdline|root=/dev/md0 console=ttyS0,115200n8"),
        }
    }

    pub fn facts(self) -> DeviceFacts {
        parse_inspect(&self.raw()).expect("demo fixture parses")
    }

    pub fn address(self) -> &'static str {
        match self {
            DemoUnit::Ds918 | DemoUnit::Mixed | DemoUnit::Volumes | DemoUnit::VolumesUsb | DemoUnit::VolumesDegraded => "192.168.1.20",
            DemoUnit::Ds1621 => "192.168.1.31",
            DemoUnit::Rs816 => "192.168.1.44",
        }
    }
}

/// The compatibility status the demo shows for a model (the real list has no Verified or Known-issues entries
/// yet, and the screenshots should show every badge).
pub fn status_override(model: &str) -> Option<Status> {
    Some(match model {
        // The default demo unit is untested like every real model; one other row shows the Verified badge so the list has them all.
        "DS918+" => Status::Untested,
        "DS1621+" => Status::Community,
        "DS923+" => Status::Verified,
        "RS816" => Status::KnownIssues,
        "DS220j" => Status::Unsupported,
        _ => return None,
    })
}

/// Why a demo unit has its status (shown after the badge when it is not Verified).
pub fn status_reason(model: &str) -> Option<&'static str> {
    Some(match model {
        "DS1621+" => "one user reported a successful install through the boot flash",
        "DS918+" => "no report yet; read-only checks run first",
        "RS816" => "ARM U-Boot board: a serial console is mandatory",
        "DS220j" => "512 MiB RAM and no documented Debian path",
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn found(model: &str, ip: &str, mac: &str, dsm: &str, serial: &str, sources: &[&str], debian: bool, host: &str) -> Found {
    Found {
        ips: vec![ip.parse::<IpAddr>().expect("demo ip")],
        mac: Some(mac.into()),
        hostname: host.into(),
        model: model.into(),
        serial: serial.into(),
        dsm_version: dsm.into(),
        platform: String::new(),
        http_port: Some(5000),
        https_port: Some(5001),
        synology: true,
        synofurb_debian: debian,
        sources: sources.iter().map(|s| s.to_string()).collect(),
        last_seen: "2026-11-01 09:41:07".into(),
    }
}

/// A second DS918+ with another serial number: the twin of the default demo unit.
pub fn twin_of_ds918() -> Found {
    found("DS918+", "192.168.1.21", "00:11:32:aa:bb:02", "7.2.1-69057", "1920PDN654321", &["udp", "mdns"], false, "diskstation2")
}

pub fn inventory() -> Inventory {
    let mut inv = Inventory::default();
    inv.merge(found(
        "DS918+",
        DemoUnit::Ds918.address(),
        "00:11:32:aa:bb:01",
        "7.2.1-69057",
        "18A0PDN014408",
        &["udp", "mdns", "ssdp", "dsm-api"],
        false,
        "diskstation",
    ));
    inv.merge(found(
        "DS1621+",
        DemoUnit::Ds1621.address(),
        "90:09:d0:12:34:56",
        "7.2.1-69057",
        "2110VBA000731",
        &["udp", "mdns"],
        false,
        "vault",
    ));
    inv.merge(found(
        "DS923+",
        "192.168.1.37",
        "90:09:d0:65:43:21",
        "7.2.2-72806",
        "2260TCN004412",
        &["mdns", "dsm-api"],
        false,
        "office-nas",
    ));
    inv.merge(found(
        "RS816",
        DemoUnit::Rs816.address(),
        "00:11:32:7a:7b:7c",
        "6.2.4-25556",
        "1650MXN200118",
        &["udp", "oui"],
        false,
        "rack",
    ));
    inv.merge(found("DS220j", "192.168.1.52", "00:11:32:11:22:33", "7.1.1-42962", "2030ABC100871", &["ssdp"], false, "guest"));
    inv.merge(found("DS218+", "192.168.1.60", "00:11:32:aa:aa:aa", "", "1850XYZ000212", &["mdns"], true, "attic"));
    inv
}

// ------------------------------------------------------------------------------------------------ scan and Add by IP

/// The networks the demo computer is on: one real LAN, a VPN and three virtual adapters ("Networks: 1 of 5").
pub fn networks() -> Vec<synofurb_core::discovery::networks::NetworkInfo> {
    use synofurb_core::discovery::networks::{LocalAddr, networks_from};
    let a = |n: &str, ip: &str, p: u8| LocalAddr { interface: n.into(), ip: ip.parse().expect("demo ip"), prefix: p };
    networks_from(&[
        a("Ethernet", "192.168.1.10", 24),
        a("Tailscale", "100.101.4.7", 10),
        a("vEthernet (WSL)", "172.28.16.1", 20),
        a("docker0", "172.17.0.1", 16),
        a("VMware Network Adapter VMnet8", "192.168.136.1", 24),
    ])
}

fn demo_network(kind: super::state::DemoScan) -> synofurb_core::discovery::networks::ResolvedNetwork {
    use super::state::DemoScan;
    use synofurb_core::discovery::networks::{Ipv4Cidr, NetworkKind, ResolvedNetwork};
    let cidr: Ipv4Cidr = "192.168.1.0/24".parse().expect("demo cidr");
    let mut n = ResolvedNetwork {
        key: "Ethernet|192.168.1.10/24".into(),
        label: "Ethernet 192.168.1.0/24".into(),
        cidr,
        interface: Some("Ethernet".into()),
        local_ip: "192.168.1.10".parse().ok(),
        broadcast: cidr.broadcast(),
        kind: Some(NetworkKind::Lan),
        custom: false,
        sweep: cidr,
        sweep_hosts: 253,
    };
    match kind {
        // a /16 is swept in the /24 around this computer only
        DemoScan::Wide => {
            let wide: Ipv4Cidr = "192.168.0.0/16".parse().expect("demo cidr");
            (n.cidr, n.broadcast, n.label) = (wide, wide.broadcast(), "Ethernet 192.168.0.0/16".into());
        }
        // a VPN address: nothing to sweep
        DemoScan::Vpn32 => {
            let one: Ipv4Cidr = "100.101.4.7/32".parse().expect("demo cidr");
            n = ResolvedNetwork {
                key: "Tailscale|100.101.4.7/32".into(),
                label: "Tailscale 100.101.4.7/32".into(),
                cidr: one,
                interface: Some("Tailscale".into()),
                local_ip: "100.101.4.7".parse().ok(),
                broadcast: one.broadcast(),
                kind: Some(NetworkKind::Vpn),
                custom: false,
                sweep: one,
                sweep_hosts: 0,
            };
        }
        _ => {}
    }
    n
}

/// The scripted scan: `(milliseconds to wait before the event, event)`. `Running` stops half way (the progress panel);
/// the others end. `Failed` has one method that failed; `Empty` finds nothing.
pub fn scan_script(kind: super::state::DemoScan) -> Vec<(u64, synofurb_core::discovery::ScanEvent)> {
    use super::state::DemoScan;
    use synofurb_core::discovery::{ScanEvent as E, ScanMethod as M};
    let net = demo_network(kind);
    let key = net.key.clone();
    let hosts = net.sweep_hosts;
    let mut v: Vec<(u64, E)> =
        vec![(150, E::Started { networks: vec![net], methods: vec![M::SynologyUdp, M::Mdns, M::Ssdp, M::ProbeSweep], total_hosts: hosts })];
    for m in [M::SynologyUdp, M::Mdns, M::Ssdp, M::ProbeSweep] {
        v.push((40, E::MethodStarted { method: m, network: key.clone() }));
    }
    let empty = matches!(kind, DemoScan::Empty | DemoScan::AllFailed | DemoScan::Vpn32);
    let found: Vec<Found> = if empty { vec![] } else { inventory().devices.into_iter().filter(|d| !d.synofurb_debian).collect() };
    let failed = kind == DemoScan::Failed;
    if kind == DemoScan::AllFailed {
        // nothing ran: every method ends with its own reason
        let why = [
            (M::SynologyUdp, "UDP socket: permission denied (os error 13)"),
            (M::Mdns, "multicast is blocked on this adapter (os error 10013)"),
            (M::Ssdp, "multicast is blocked on this adapter (os error 10013)"),
            (M::ProbeSweep, "network is unreachable (os error 10051)"),
        ];
        for (m, e) in why {
            v.push((120, E::MethodFinished { method: m, network: key.clone(), found: 0, error: Some(e.to_string()) }));
        }
        v.push((50, E::Finished { found_total: 0, elapsed: std::time::Duration::from_millis(1200) }));
        return v;
    }
    // listening methods: 4 s of listening, shown in steps
    for step in 1..=3u32 {
        for m in [M::SynologyUdp, M::Mdns, M::Ssdp] {
            v.push((120, E::MethodProgress { method: m, network: key.clone(), done: step * 1000, total: 4000 }));
        }
        if hosts > 0 {
            v.push((60, E::MethodProgress { method: M::ProbeSweep, network: key.clone(), done: step * 28, total: hosts }));
        }
        if let Some(f) = found.get(step as usize - 1) {
            v.push((30, E::Found(f.clone())));
        }
    }
    v.push((100, E::MethodFinished { method: M::SynologyUdp, network: key.clone(), found: found.len().min(3), error: None }));
    v.push((
        50,
        E::MethodFinished {
            method: M::Mdns,
            network: key.clone(),
            found: if failed { 0 } else { found.len().min(2) },
            error: failed.then(|| "multicast is blocked on this adapter (os error 10013)".to_string()),
        },
    ));
    if matches!(kind, DemoScan::Running | DemoScan::Stopped) {
        // stuck here: 87 of 253 hosts, two units found, the sweep still running
        v.push((50, E::MethodProgress { method: M::ProbeSweep, network: key.clone(), done: 87, total: hosts }));
        return v;
    }
    v.push((100, E::MethodFinished { method: M::Ssdp, network: key.clone(), found: found.len().min(2), error: None }));
    for f in found.iter().skip(3) {
        v.push((60, E::Found(f.clone())));
    }
    if hosts > 0 {
        v.push((100, E::MethodProgress { method: M::ProbeSweep, network: key.clone(), done: hosts, total: hosts }));
    }
    v.push((50, E::MethodFinished { method: M::ProbeSweep, network: key.clone(), found: found.len().min(2), error: None }));
    v.push((50, E::Finished { found_total: found.len(), elapsed: std::time::Duration::from_millis(5600) }));
    v
}

/// Play [`scan_script`] on a thread. A stopped scan ends with `Finished` at once, like the real one.
pub fn spawn_scan_script(tx: Tx, kind: super::state::DemoScan, cancel: synofurb_core::discovery::CancelToken) {
    use synofurb_core::discovery::ScanEvent;
    std::thread::spawn(move || {
        let script = scan_script(kind);
        let t0 = std::time::Instant::now();
        for (wait, ev) in script {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(wait);
            while std::time::Instant::now() < end && !cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if cancel.is_cancelled() {
                break;
            }
            if tx.unbounded_send(Msg::ScanEv(ev)).is_err() {
                return;
            }
        }
        if matches!(kind, super::state::DemoScan::Running | super::state::DemoScan::Stopped) {
            while !cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if cancel.is_cancelled() {
            let _ = tx.unbounded_send(Msg::ScanEv(ScanEvent::Finished { found_total: 2, elapsed: t0.elapsed() }));
        }
    });
}

fn chk(check: &str, state: CheckState, detail: &str) -> CheckResult {
    CheckResult { check: check.into(), state, detail: detail.into() }
}

/// What the demo's checks find for an address: `.99` is a Synology unit, `.88` a router (an unknown device), anything else is silent.
pub fn add_report(t: &Target) -> ProbeReport {
    let names = addhost::check_names(t);
    let ip = t.ip().unwrap_or_else(|| "192.168.1.99".parse().expect("ip"));
    let last = t.host.rsplit('.').next().unwrap_or("");
    match last {
        "99" => ProbeReport {
            checks: vec![
                chk(&names[0], CheckState::Answered, "SSH-2.0-OpenSSH_8.2"),
                chk(&names[1], CheckState::Answered, "DSM answered its Web API"),
                chk(&names[2], CheckState::Answered, "open (DSM's secure web interface)"),
                chk(&names[3], CheckState::Answered, "DS220+ answered"),
            ],
            found: Some(Found {
                ips: vec![ip],
                model: "DS220+".into(),
                dsm_version: "7.2.2-72806".into(),
                serial: "2150TCN001234".into(),
                hostname: "guest-room".into(),
                http_port: Some(5000),
                https_port: Some(5001),
                synology: true,
                sources: vec!["added".into()],
                ..Default::default()
            }),
        },
        "88" => ProbeReport {
            checks: vec![
                chk(&names[0], CheckState::Closed, "connection refused (the host is there, the port is closed)"),
                chk(&names[1], CheckState::Closed, "a web server answered (HTTP/1.1 401 Unauthorized), but not DSM"),
                chk(&names[2], CheckState::NoReply, "no reply in 1.5 s"),
                chk(&names[3], CheckState::NoReply, "no reply in 1.5 s"),
            ],
            found: None,
        },
        _ => ProbeReport {
            checks: vec![
                chk(&names[0], CheckState::NoReply, "no reply in 1.5 s"),
                chk(&names[1], CheckState::NoReply, "no reply in 1.5 s"),
                chk(&names[2], CheckState::NoReply, "no reply in 1.5 s"),
                chk(&names[3], CheckState::NoReply, "no reply in 1.5 s"),
            ],
            found: None,
        },
    }
}

/// The three kinds of added row, for the Find screenshots.
pub fn added_hosts() -> AddedHosts {
    let mut a = AddedHosts::default();
    for ip in ["192.168.1.99", "192.168.1.88", "192.168.1.77"] {
        let target = addhost::parse_target(ip).expect("demo target");
        let report = add_report(&target);
        a.upsert(AddedHost { target, report, checked: "2026-11-03 10:12".into() });
    }
    a
}

/// A scripted check: the steps arrive one by one; `.150` never ends (the dialog's "checking" state) until it is cancelled.
pub fn spawn_add_script(tx: Tx, ticket: u64, target: Target, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let report = add_report(&target);
        let hold = target.host.ends_with(".150");
        for (i, r) in report.checks.iter().enumerate() {
            if hold && i >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(if hold { 150 } else { 400 }));
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let _ = tx.unbounded_send(Msg::AddStep { ticket, idx: i, result: r.clone() });
        }
        if hold {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !cancel.load(Ordering::Relaxed) {
            let _ = tx.unbounded_send(Msg::AddDone { ticket, target, report });
        }
    });
}

pub fn outcome(unit: DemoUnit, hostname: &str, username: &str) -> Outcome {
    Outcome {
        hostname: hostname.to_string(),
        ip: unit.address().parse().ok(),
        ssh_fingerprint: Some("SHA256:3t4mVjQd0sXbJ8N5w1o9y2cQeF0zHk7uA6pLr1nXb4E".into()),
        mgmt_url: Some(format!("https://{}:8443", unit.address())),
        mgmt_reachable: true,
        username: username.to_string(),
        log_path: Some("~/.local/share/synofurb/logs/synofurb-demo.log".into()),
    }
}

pub fn update_info() -> UpdateInfo {
    UpdateInfo {
        current: env!("CARGO_PKG_VERSION").into(),
        latest: "0.3.0".into(),
        tag: "v0.3.0".into(),
        update_available: true,
        channel: Channel::Stable,
        prerelease: false,
        published_at: Some("2026-10-28T14:02:11Z".into()),
        notes: "## What's new\n- New desktop interface (GPUI): one codebase for Windows, macOS and Linux\n- Boot manager mode for x86 units with an internal boot flash\n- Compatibility list shown per unit, refreshable from the project repository\n\n## Fixes\n- The data-loss confirmation now shows what is destroyed and what is preserved side by side\n- Serial consoles on AMD boards (raw uart8250) are detected and configured".into(),
        html_url: Some("https://github.com/andreaswiren/synofurb/releases/tag/v0.3.0".into()),
        asset: Some(AssetInfo { name: "synofurb-0.3.0-linux-x86_64.AppImage".into(), url: "https://example.invalid/a".into(), size: 38_400_000 }),
        checksums: Some(AssetInfo { name: "SHA256SUMS".into(), url: "https://example.invalid/s".into(), size: 400 }),
        signature: Some(AssetInfo { name: "SHA256SUMS.minisig".into(), url: "https://example.invalid/m".into(), size: 300 }),
        problems: vec![],
    }
}

/// The update state a demo screen starts in.
pub fn update_state(d: super::state::DemoUpdate) -> super::app::UpdateState {
    use super::app::UpdateState;
    use super::state::DemoUpdate as D;
    match d {
        D::None => UpdateState::Unknown,
        D::Checking => UpdateState::Checking,
        D::Current => UpdateState::UpToDate,
        D::Available | D::Unsigned => UpdateState::Available(Box::new(update_info())),
        D::NotFound | D::Offline => UpdateState::CheckFailed(update_failure(d)),
    }
}

/// What a check ends in for a demo state (the click on "Check for updates").
pub fn update_check_result(d: super::state::DemoUpdate) -> Result<Box<UpdateInfo>, super::updates::CheckFailure> {
    use super::state::DemoUpdate as D;
    match d {
        D::Current => Ok(Box::new(UpdateInfo { update_available: false, latest: env!("CARGO_PKG_VERSION").into(), ..update_info() })),
        D::NotFound | D::Offline => Err(update_failure(d)),
        _ => Ok(Box::new(update_info())),
    }
}

fn update_failure(d: super::state::DemoUpdate) -> super::updates::CheckFailure {
    use super::updates::{CheckFailure, FailKind};
    if d == super::state::DemoUpdate::Offline {
        CheckFailure {
            kind: FailKind::Offline,
            detail: "request to https://api.github.com/repos/andreaswiren/synofurb/releases failed: dns error".into(),
        }
    } else {
        CheckFailure { kind: FailKind::NotFound, detail: "repository or releases not found: andreaswiren/synofurb".into() }
    }
}

// ------------------------------------------------------------------------------------------------ scripted install

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    /// Runs to step 6 after the point of no return and then idles (the "install" screenshot).
    Running,
    /// Stops at step 2, before the point of no return, and idles.
    RunningEarly,
    /// Fails at step 11 after the point of no return.
    Fails,
    /// Fails at step 2, before the point of no return (the disks were not touched).
    FailsEarly,
    /// Runs through to the end.
    Succeeds,
}

fn log(level: Level, source: Source, msg: &str, ts: &str) -> Event {
    Event::Log(LogLine { ts: ts.into(), level, source, msg: msg.into() })
}

/// The events of a plausible install, in order, with a pause marker `None` between phases.
pub fn script_events(script: Script, unit: DemoUnit, model: &str) -> Vec<Event> {
    let mut v = vec![];
    let mut sec = 0u32;
    let mut ts = move || {
        sec += 1;
        format!("09:4{}:{:02}", 2 + sec / 60, sec % 60)
    };
    let l = |v: &mut Vec<Event>, level, src, msg: &str, ts: &mut dyn FnMut() -> String| v.push(log(level, src, msg, &ts()));

    v.push(Event::Phase(Phase::Connecting));
    l(
        &mut v,
        Level::Info,
        Source::App,
        &format!("synofurb {} starting install on {model} ({})", env!("CARGO_PKG_VERSION"), unit.address()),
        &mut ts,
    );
    l(&mut v, Level::Info, Source::App, "this computer reaches the NAS from 192.168.1.10", &mut ts);
    l(&mut v, Level::Debug, Source::Remote, "ssh: connected, host key SHA256:3t4mVjQd0sXbJ8N5w1o9y2cQeF0zHk7uA6pLr1nXb4E", &mut ts);
    v.push(Event::Phase(Phase::Inspecting));
    l(&mut v, Level::Info, Source::App, "disks still match the plan: sda sdb sdc sdd", &mut ts);
    l(
        &mut v,
        Level::Warn,
        Source::App,
        "disk metadata backup (partition tables, not your files): ~/.local/share/synofurb/backups/disk-metadata-18A0PDN014408",
        &mut ts,
    );
    v.push(Event::Phase(Phase::Staging));
    l(&mut v, Level::Info, Source::App, "fetching the Debian installer (cached)", &mut ts);
    for f in [0.15, 0.4, 0.7, 1.0] {
        v.push(Event::Progress { what: "Uploading the installer to the NAS".into(), fraction: f });
    }
    v.push(Event::Phase(Phase::Booting));
    l(&mut v, Level::Info, Source::App, "kexec: loading the installer kernel", &mut ts);
    l(&mut v, Level::Warn, Source::Remote, "ssh: connection closed by the NAS while rebooting into the installer (expected)", &mut ts);
    v.push(Event::Phase(Phase::Installing));
    let steps = crate::ui::state::INSTALL_STEPS;
    let last = match script {
        Script::Running => 6,
        Script::RunningEarly | Script::FailsEarly => 2,
        Script::Fails => 11,
        Script::Succeeds => 12,
    };
    for n in 1..=last as u32 {
        if n == crate::ui::state::PONR_STEP {
            l(&mut v, Level::Warn, Source::Installer, "POINT OF NO RETURN: from here the disks are changed", &mut ts);
            v.push(Event::PointOfNoReturn);
        }
        v.push(Event::Step { n, total: 12, name: steps[n as usize - 1].into() });
        l(&mut v, Level::Info, Source::Installer, &format!("#{n} {}", steps[n as usize - 1]), &mut ts);
        l(&mut v, Level::Debug, Source::Installer, &format!("CMD step {n}: running helper commands"), &mut ts);
        if n == 4 {
            l(&mut v, Level::Warn, Source::Installer, "WARN sdc: write cache is disabled, partitioning will be slow", &mut ts);
        }
        if n == 9 {
            for f in [0.25, 0.5, 0.8] {
                v.push(Event::Progress { what: "Installing the Debian base system".into(), fraction: f });
            }
        }
        if unit == DemoUnit::Rs816 && n == 11 {
            for c in [
                "setenv bootargs 'console=ttyS0,115200 root=/dev/md0'",
                "setenv bootcmd 'sata init; ext4load sata 0:1 0x2000000 /vmlinuz'",
                "saveenv",
            ] {
                l(&mut v, Level::Info, Source::Installer, &format!("uboot-env| {c}"), &mut ts);
            }
        }
    }
    match script {
        Script::FailsEarly => {
            l(&mut v, Level::Error, Source::Installer, "ERROR preflight: /dev/sdc is not the disk the plan expects", &mut ts);
            l(&mut v, Level::Error, Source::Installer, "@@FAIL preflight: disks differ from the plan", &mut ts);
            v.push(Event::InstallerFailed("preflight: disks differ from the plan".into()));
            v.push(Event::Phase(Phase::Failed));
        }
        Script::Fails => {
            l(&mut v, Level::Error, Source::Installer, "ERROR grub-install: cannot write to /dev/sdc: Input/output error", &mut ts);
            l(&mut v, Level::Error, Source::Installer, "@@FAIL grub-install failed on sdc", &mut ts);
            v.push(Event::InstallerFailed("grub-install failed on sdc: Input/output error".into()));
            v.push(Event::Phase(Phase::Failed));
        }
        Script::Succeeds => {
            v.push(Event::InstallerDone);
            v.push(Event::Phase(Phase::WaitingForReboot));
            l(&mut v, Level::Info, Source::App, "the installer finished; waiting for the new system", &mut ts);
            v.push(Event::Phase(Phase::Verifying));
            l(&mut v, Level::Info, Source::App, "new system answers on ssh and https://192.168.1.20:8443", &mut ts);
            v.push(Event::Phase(Phase::Finished));
        }
        Script::Running | Script::RunningEarly => {}
    }
    v
}

/// Replay [`script_events`] on a thread at a lively pace, then finish like the real worker would.
pub fn spawn_script(tx: Tx, script: Script, unit: DemoUnit, model: String, out: Option<Outcome>) {
    std::thread::spawn(move || {
        for e in script_events(script, unit, &model) {
            let pause = !matches!(e, Event::Log(_));
            let _ = tx.unbounded_send(Msg::Ev(e));
            std::thread::sleep(if pause { DEMO_TICK } else { DEMO_TICK / 3 });
        }
        match script {
            Script::Fails => {
                let _ = tx.unbounded_send(Msg::Finished(Err(
                    "the installer reported a failure: grub-install failed on sdc: Input/output error".into(),
                )));
            }
            Script::FailsEarly => {
                let _ =
                    tx.unbounded_send(Msg::Finished(Err("the installer reported a failure: preflight: disks differ from the plan".into())));
            }
            Script::Succeeds => {
                if let Some(o) = out {
                    let _ = tx.unbounded_send(Msg::Finished(Ok(o)));
                }
            }
            Script::Running | Script::RunningEarly => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_demo_unit_parses_with_the_expected_traits() {
        let ds = DemoUnit::Ds918.facts();
        assert_eq!(ds.model, "DS918+");
        let amd = DemoUnit::Ds1621.facts();
        assert_eq!(amd.firmware, synofurb_core::model::Firmware::Dom);
        assert!(amd.serial_console().raw.is_some(), "raw uart8250 console detected");
        let arm = DemoUnit::Rs816.facts();
        assert_eq!(arm.arch, synofurb_core::model::Arch::Armhf);
        assert_eq!(arm.firmware, synofurb_core::model::Firmware::UBoot);
        assert!(!arm.kexec);
    }

    #[test]
    fn demo_inventory_has_one_unit_per_status_and_a_converted_unit() {
        let inv = inventory();
        let statuses: Vec<Status> = inv.devices.iter().filter(|d| !d.synofurb_debian).filter_map(|d| status_override(&d.model)).collect();
        for s in Status::ALL {
            assert!(statuses.contains(&s), "{s:?} missing");
        }
        assert_eq!(inv.devices.iter().filter(|d| d.synofurb_debian).count(), 1);
    }

    #[test]
    fn scripts_have_the_right_shape() {
        let run = script_events(Script::Running, DemoUnit::Ds918, "DS918+");
        assert!(run.iter().any(|e| matches!(e, Event::PointOfNoReturn)));
        assert!(!run.iter().any(|e| matches!(e, Event::InstallerFailed(_) | Event::InstallerDone)));
        assert!(matches!(run.iter().rev().find(|e| matches!(e, Event::Step { .. })), Some(Event::Step { n: 6, .. })));
        let fail = script_events(Script::Fails, DemoUnit::Ds918, "DS918+");
        assert!(fail.iter().any(|e| matches!(e, Event::InstallerFailed(_))));
        let early = script_events(Script::FailsEarly, DemoUnit::Ds918, "DS918+");
        assert!(early.iter().any(|e| matches!(e, Event::InstallerFailed(_))));
        assert!(!early.iter().any(|e| matches!(e, Event::PointOfNoReturn)), "the early failure is before the point of no return");
        let before = script_events(Script::RunningEarly, DemoUnit::Ds918, "DS918+");
        assert!(!before.iter().any(|e| matches!(e, Event::PointOfNoReturn)));
        let ok = script_events(Script::Succeeds, DemoUnit::Rs816, "RS816");
        assert!(ok.iter().any(|e| matches!(e, Event::InstallerDone)));
        assert!(ok.iter().any(|e| matches!(e, Event::Log(l) if l.msg.contains("uboot-env| "))), "ARM script prints U-Boot commands");
    }
}
