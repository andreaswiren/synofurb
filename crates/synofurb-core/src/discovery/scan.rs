//! A scan over chosen networks, with progress events and cancellation.
//!
//! [`scan_with`] runs the selected discovery methods on the selected networks only and reports what it is doing as
//! [`ScanEvent`]s, so a UI can show "Bonjour/mDNS on Ethernet 192.168.69.0/24: running" and a progress bar for the port
//! sweep. A [`CancelToken`] stops it: the listening methods notice within 300 ms, the sweep within about a second.
//!
//! What runs where:
//!
//! | method | uses |
//! |---|---|
//! | `synology_udp` | one UDP broadcast to each selected network's broadcast address, from the adapter on that network |
//! | `mdns` | multicast DNS on the adapter of each selected network (not for a custom network we are not attached to) |
//! | `ssdp` | UPnP M-SEARCH on the adapter of each selected network (same limit) |
//! | `probe_sweep` | TCP connect to DSM port 5000 on every address of each selected network, never our own |

use super::networks::{self, NetworkError, ResolvedNetwork, ScanNetwork};
use super::{Found, Inventory, arp_lookup, is_synology_mac, mdns, probe, ssdp, syno_udp};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// Ask a running scan to stop. Cloning shares the flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMethod {
    Mdns,
    Ssdp,
    SynologyUdp,
    ProbeSweep,
}

impl ScanMethod {
    pub const ALL: [ScanMethod; 4] = [ScanMethod::SynologyUdp, ScanMethod::Mdns, ScanMethod::Ssdp, ScanMethod::ProbeSweep];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            ScanMethod::SynologyUdp => "Synology Assistant broadcast",
            ScanMethod::Mdns => "Bonjour/mDNS",
            ScanMethod::Ssdp => "UPnP/SSDP",
            ScanMethod::ProbeSweep => "Port sweep (DSM ports)",
        }
    }
    /// The stable machine name (`mdns`, `ssdp`, `synology_udp`, `probe_sweep`), as in JSON events and `--methods`.
    pub fn id(self) -> &'static str {
        match self {
            ScanMethod::SynologyUdp => "synology_udp",
            ScanMethod::Mdns => "mdns",
            ScanMethod::Ssdp => "ssdp",
            ScanMethod::ProbeSweep => "probe_sweep",
        }
    }
    pub fn from_id(s: &str) -> Option<ScanMethod> {
        ScanMethod::ALL.into_iter().find(|m| m.id() == s.trim().to_lowercase().replace('-', "_"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanMethods {
    pub mdns: bool,
    pub ssdp: bool,
    pub synology_udp: bool,
    pub probe_sweep: bool,
}

impl Default for ScanMethods {
    fn default() -> Self {
        ScanMethods { mdns: true, ssdp: true, synology_udp: true, probe_sweep: true }
    }
}

impl ScanMethods {
    pub fn none() -> ScanMethods {
        ScanMethods { mdns: false, ssdp: false, synology_udp: false, probe_sweep: false }
    }
    pub fn contains(&self, m: ScanMethod) -> bool {
        match m {
            ScanMethod::Mdns => self.mdns,
            ScanMethod::Ssdp => self.ssdp,
            ScanMethod::SynologyUdp => self.synology_udp,
            ScanMethod::ProbeSweep => self.probe_sweep,
        }
    }
    pub fn set(&mut self, m: ScanMethod, on: bool) {
        match m {
            ScanMethod::Mdns => self.mdns = on,
            ScanMethod::Ssdp => self.ssdp = on,
            ScanMethod::SynologyUdp => self.synology_udp = on,
            ScanMethod::ProbeSweep => self.probe_sweep = on,
        }
    }
    /// The enabled methods in the order they are listed (`ScanMethod::ALL`).
    pub fn list(&self) -> Vec<ScanMethod> {
        ScanMethod::ALL.into_iter().filter(|m| self.contains(*m)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// The networks to scan. `Default` fills it with the recommended (real LAN) networks of this computer.
    pub networks: Vec<ScanNetwork>,
    pub methods: ScanMethods,
    /// How long each listening method (broadcast, mDNS, SSDP) waits for answers.
    pub timeout: Duration,
    /// Legacy switch kept for callers written before [`ScanOptions::methods`]: `false` turns the port sweep off even when
    /// `methods.probe_sweep` is on. New code sets `methods.probe_sweep` and leaves this `true`.
    pub sweep: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions::recommended()
    }
}

impl ScanOptions {
    /// Every method, on the recommended networks (real LAN networks only), 4 s per listening method.
    pub fn recommended() -> ScanOptions {
        ScanOptions {
            networks: networks::recommended().iter().map(ScanNetwork::from_info).collect(),
            methods: ScanMethods::default(),
            timeout: Duration::from_secs(4),
            sweep: true,
        }
    }
    /// Every method on every network of this computer (including VPN and virtual ones).
    pub fn all_networks() -> ScanOptions {
        ScanOptions { networks: networks::networks().iter().map(ScanNetwork::from_info).collect(), ..ScanOptions::recommended() }
    }
    pub fn with_networks(mut self, n: Vec<ScanNetwork>) -> ScanOptions {
        self.networks = n;
        self
    }
    pub fn with_methods(mut self, m: ScanMethods) -> ScanOptions {
        self.methods = m;
        self
    }
    pub fn with_timeout(mut self, t: Duration) -> ScanOptions {
        self.timeout = t;
        self
    }
    /// The methods that will run (`methods` and the legacy `sweep` switch combined).
    pub fn effective_methods(&self) -> Vec<ScanMethod> {
        self.methods.list().into_iter().filter(|m| *m != ScanMethod::ProbeSweep || self.sweep).collect()
    }
}

/// What a scan is doing. Serializes with an `event` tag (`started`, `method_started`, `method_progress`, `method_finished`,
/// `skipped`, `found`, `finished`); a `found` event is a device object with `"event":"found"` added.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ScanEvent {
    /// The scan begins. `total_hosts` is how many addresses the port sweep will probe (0 without the sweep).
    Started { networks: Vec<ResolvedNetwork>, methods: Vec<ScanMethod>, total_hosts: u32 },
    /// `network` is a [`ResolvedNetwork::key`].
    MethodStarted { method: ScanMethod, network: String },
    /// Port sweep: addresses probed so far out of the total for the network. Listening methods: milliseconds waited out of the timeout.
    MethodProgress { method: ScanMethod, network: String, done: u32, total: u32 },
    /// `found` counts sightings that came from this method on this network; `error` is the reason the method failed.
    MethodFinished { method: ScanMethod, network: String, found: usize, error: Option<String> },
    /// This method was not run on this network, and why (e.g. multicast cannot reach a custom network we are not attached to).
    Skipped { method: ScanMethod, network: String, reason: String },
    /// A device was seen (merged with what earlier methods saw of it, so the latest event is the most complete).
    Found(Found),
    /// The last event, also after a cancel.
    Finished {
        found_total: usize,
        #[serde(rename = "elapsed_ms", with = "duration_ms")]
        elapsed: Duration,
    },
}

mod duration_ms {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

/// Where a listening method or the sweep reports progress: `(done, total)`.
pub type Progress<'a> = &'a (dyn Fn(u32, u32) + Sync);

enum Msg {
    Ev(ScanEvent),
    Found(Found),
}

/// Broadcast/unicast targets of the Synology UDP query on a network: its broadcast address, or the host(s) of a `/31` or `/32`.
pub fn udp_targets(n: &ResolvedNetwork, own: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    let v: Vec<Ipv4Addr> = if n.cidr.prefix() >= 31 { n.cidr.hosts().collect() } else { vec![n.broadcast] };
    v.into_iter().filter(|a| !own.contains(a)).collect()
}

/// Run the scan and report every step to `events`. Returns the devices seen (not merged with any saved inventory).
/// An error means the selection could not be used (an adapter that is gone, an invalid custom network); nothing was scanned.
pub fn scan_with(opts: &ScanOptions, events: &Sender<ScanEvent>, cancel: &CancelToken) -> Result<Inventory, NetworkError> {
    scan_with_callback(opts, cancel, |e| {
        let _ = events.send(e);
    })
}

/// [`scan_with`] with a callback instead of a channel. The callback runs on the calling thread.
pub fn scan_with_callback(opts: &ScanOptions, cancel: &CancelToken, on_event: impl FnMut(ScanEvent)) -> Result<Inventory, NetworkError> {
    run(opts, cancel, on_event, &probe::probe_host)
}

fn run(
    opts: &ScanOptions,
    cancel: &CancelToken,
    mut on_event: impl FnMut(ScanEvent),
    host_probe: &(dyn Fn(Ipv4Addr) -> Option<Found> + Sync),
) -> Result<Inventory, NetworkError> {
    let t0 = Instant::now();
    let available = networks::networks();
    let mut own_v4: Vec<Ipv4Addr> = networks::local_addrs().iter().map(|a| a.ip).collect();
    if !own_v4.contains(&Ipv4Addr::LOCALHOST) {
        own_v4.push(Ipv4Addr::LOCALHOST);
    }
    let own: Vec<IpAddr> = own_v4.iter().copied().map(IpAddr::V4).collect();
    let nets = networks::resolve_networks(&opts.networks, &available, &own_v4)?;
    let methods = opts.effective_methods();

    // the sweep list per network; an address belongs to the first network that holds it
    let mut sweep_lists: Vec<Vec<Ipv4Addr>> = vec![];
    let mut seen = std::collections::HashSet::new();
    for n in &nets {
        let l: Vec<Ipv4Addr> = if methods.contains(&ScanMethod::ProbeSweep) {
            networks::sweep_hosts(&n.sweep, &own_v4).into_iter().filter(|h| seen.insert(*h)).collect()
        } else {
            vec![]
        };
        sweep_lists.push(l);
    }
    let total_hosts: u32 = sweep_lists.iter().map(|l| l.len() as u32).sum();
    on_event(ScanEvent::Started { networks: nets.clone(), methods: methods.clone(), total_hosts });

    let (tx, rx) = channel::<Msg>();
    let mut inv = Inventory::default();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let timeout = opts.timeout;
    std::thread::scope(|s| {
        for &m in &methods {
            if m == ScanMethod::ProbeSweep {
                let (tx, nets, lists) = (tx.clone(), &nets, &sweep_lists);
                s.spawn(move || sweep_worker(nets, lists, cancel, &tx, host_probe));
                continue;
            }
            for n in &nets {
                let ev = |e: ScanEvent| {
                    let _ = tx.send(Msg::Ev(e));
                };
                let skip = |reason: &str| ev(ScanEvent::Skipped { method: m, network: n.key.clone(), reason: reason.to_string() });
                let udp = if m == ScanMethod::SynologyUdp { udp_targets(n, &own_v4) } else { vec![] };
                match m {
                    ScanMethod::SynologyUdp if udp.is_empty() => {
                        skip("no other address to ask on this network");
                        continue;
                    }
                    ScanMethod::Mdns | ScanMethod::Ssdp if n.local_ip.is_none() => {
                        skip("this computer is not on that network, and multicast does not cross routers");
                        continue;
                    }
                    _ => {}
                }
                let (tx, own) = (tx.clone(), &own);
                s.spawn(move || {
                    let send = |e: ScanEvent| {
                        let _ = tx.send(Msg::Ev(e));
                    };
                    send(ScanEvent::MethodStarted { method: m, network: n.key.clone() });
                    let total = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
                    let progress = |done: u32, _t: u32| {
                        send(ScanEvent::MethodProgress { method: m, network: n.key.clone(), done, total });
                    };
                    let emit = |f: Found| {
                        let _ = tx.send(Msg::Found(f));
                    };
                    let r = match m {
                        ScanMethod::SynologyUdp => syno_udp::scan_network(n.local_ip, &udp, own, timeout, &emit, cancel, &progress),
                        ScanMethod::Mdns => n.local_ip.map_or(Ok(0), |ip| mdns::scan_network(ip, timeout, &emit, cancel, &progress)),
                        ScanMethod::Ssdp => n.local_ip.map_or(Ok(0), |ip| ssdp::scan_network(ip, timeout, &emit, cancel, &progress)),
                        ScanMethod::ProbeSweep => Ok(0),
                    };
                    let (found, error) = match r {
                        Ok(n) => (n, None),
                        Err(e) => (0, Some(format!("{e:#}"))),
                    };
                    send(ScanEvent::MethodFinished { method: m, network: n.key.clone(), found, error });
                });
            }
        }
        drop(tx);
        for msg in rx {
            match msg {
                Msg::Ev(e) => on_event(e),
                Msg::Found(f) => merge_found(&mut inv, f, &now, &mut on_event),
            }
        }
    });
    // ARP is populated after the traffic: a second chance to recognise units by their Synology MAC prefix
    for i in 0..inv.devices.len() {
        let d = &mut inv.devices[i];
        if d.mac.is_none()
            && let Some(ip) = d.primary_ip()
        {
            d.mac = arp_lookup(ip);
            if d.mac.is_some() {
                if d.mac.as_deref().is_some_and(is_synology_mac) {
                    d.synology = true;
                    d.sources.push("oui".into());
                }
                on_event(ScanEvent::Found(d.clone()));
            }
        }
    }
    on_event(ScanEvent::Finished { found_total: inv.devices.len(), elapsed: t0.elapsed() });
    Ok(inv)
}

fn merge_found(inv: &mut Inventory, mut f: Found, now: &str, on_event: &mut impl FnMut(ScanEvent)) {
    f.last_seen = now.to_string();
    if f.mac.is_none()
        && let Some(ip) = f.primary_ip()
    {
        f.mac = arp_lookup(ip);
    }
    let (ips, mac) = (f.ips.clone(), f.mac.clone());
    inv.merge(f);
    let mac = mac.as_deref().and_then(super::normalize_mac);
    if let Some(d) = inv.devices.iter().find(|d| d.ips.iter().any(|i| ips.contains(i)) || (mac.is_some() && d.mac == mac)) {
        on_event(ScanEvent::Found(d.clone()));
    }
}

/// The port sweep over all networks at once (one pool of workers), reporting progress per network.
fn sweep_worker(
    nets: &[ResolvedNetwork],
    lists: &[Vec<Ipv4Addr>],
    cancel: &CancelToken,
    tx: &Sender<Msg>,
    host_probe: &(dyn Fn(Ipv4Addr) -> Option<Found> + Sync),
) {
    let ev = |e: ScanEvent| {
        let _ = tx.send(Msg::Ev(e));
    };
    let mut active = vec![];
    for (i, n) in nets.iter().enumerate() {
        if lists[i].is_empty() {
            ev(ScanEvent::Skipped {
                method: ScanMethod::ProbeSweep,
                network: n.key.clone(),
                reason: "no other address to probe on this network".into(),
            });
        } else {
            ev(ScanEvent::MethodStarted { method: ScanMethod::ProbeSweep, network: n.key.clone() });
            active.push(i);
        }
    }
    let found = probe::sweep_lists(
        lists,
        cancel,
        host_probe,
        &|_i, f| {
            let _ = tx.send(Msg::Found(f));
        },
        &|i, done, total| {
            ev(ScanEvent::MethodProgress { method: ScanMethod::ProbeSweep, network: nets[i].key.clone(), done, total });
        },
    );
    for i in active {
        ev(ScanEvent::MethodFinished { method: ScanMethod::ProbeSweep, network: nets[i].key.clone(), found: found[i], error: None });
    }
}

/// Drain a channel of events into a vector (handy in tests and simple callers).
pub fn collect_events(rx: &Receiver<ScanEvent>) -> Vec<ScanEvent> {
    rx.try_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lo_net(cidr: &str) -> ScanOptions {
        ScanOptions {
            networks: vec![ScanNetwork::custom(cidr).unwrap()],
            methods: ScanMethods::none(),
            timeout: Duration::from_millis(600),
            sweep: true,
        }
    }

    #[test]
    fn methods_ids_and_lists() {
        let mut m = ScanMethods::default();
        assert_eq!(m.list().len(), 4);
        m.set(ScanMethod::Ssdp, false);
        assert!(!m.contains(ScanMethod::Ssdp) && m.contains(ScanMethod::Mdns));
        assert_eq!(ScanMethod::from_id("probe-sweep"), Some(ScanMethod::ProbeSweep));
        assert_eq!(ScanMethod::from_id("MDNS"), Some(ScanMethod::Mdns));
        assert_eq!(ScanMethod::from_id("nope"), None);
        let o = ScanOptions { sweep: false, ..ScanOptions::default() };
        assert!(!o.effective_methods().contains(&ScanMethod::ProbeSweep));
        assert_eq!(serde_json::to_value(ScanMethod::SynologyUdp).unwrap(), "synology_udp");
    }

    #[test]
    fn events_serialize_with_an_event_tag() {
        let e = ScanEvent::MethodFinished { method: ScanMethod::Mdns, network: "n".into(), found: 2, error: Some("boom".into()) };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(
            (v["event"].as_str(), v["method"].as_str(), v["found"].as_u64(), v["error"].as_str()),
            (Some("method_finished"), Some("mdns"), Some(2), Some("boom"))
        );
        let f = ScanEvent::Found(Found { ips: vec!["10.0.0.5".parse().unwrap()], model: "DS918+".into(), ..Default::default() });
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!((v["event"].as_str(), v["model"].as_str()), (Some("found"), Some("DS918+")), "a device object plus the event tag");
        let fin = ScanEvent::Finished { found_total: 1, elapsed: Duration::from_millis(1500) };
        let v = serde_json::to_value(&fin).unwrap();
        assert_eq!(v["elapsed_ms"], 1500);
        assert!(matches!(serde_json::from_value::<ScanEvent>(v).unwrap(), ScanEvent::Finished { found_total: 1, .. }));
        let back: ScanEvent = serde_json::from_value(serde_json::to_value(&f).unwrap()).unwrap();
        assert!(matches!(back, ScanEvent::Found(d) if d.model == "DS918+"));
    }

    #[test]
    fn udp_targets_follow_the_network() {
        let nets =
            networks::networks_from(&[networks::LocalAddr { interface: "eth0".into(), ip: "192.168.1.5".parse().unwrap(), prefix: 24 }]);
        let own = [Ipv4Addr::new(192, 168, 1, 5)];
        let r = networks::resolve_networks(
            &[
                ScanNetwork::id("eth0|192.168.1.5/24"),
                ScanNetwork::custom("10.0.0.4/31").unwrap(),
                ScanNetwork::custom("192.168.1.5/32").unwrap(),
            ],
            &nets,
            &own,
        )
        .unwrap();
        assert_eq!(udp_targets(&r[0], &own), [Ipv4Addr::new(192, 168, 1, 255)], "only the broadcast address of the selected network");
        assert_eq!(udp_targets(&r[1], &own).len(), 2);
        assert!(udp_targets(&r[2], &own).is_empty(), "never ourselves");
    }

    #[test]
    fn an_unusable_selection_is_an_error_and_scans_nothing() {
        let o = ScanOptions { networks: vec![ScanNetwork::id("no-such-adapter|1.2.3.4/24")], ..lo_net("127.0.0.0/30") };
        let mut got = vec![];
        let r = scan_with_callback(&o, &CancelToken::new(), |e| got.push(e));
        assert!(matches!(r, Err(NetworkError::Unknown(_))));
        assert!(got.is_empty());
    }

    #[test]
    fn sweep_over_loopback_reports_progress_and_finishes() {
        // 127.0.0.2 .. 127.0.0.254 of 127.0.0.0/24 (127.0.0.1 is us); a stub probe so nothing depends on port 5000
        let mut o = lo_net("127.0.0.0/24");
        o.methods.probe_sweep = true;
        let probed = std::sync::Mutex::new(vec![]);
        let stub = |ip: Ipv4Addr| {
            probed.lock().unwrap().push(ip);
            (ip == Ipv4Addr::new(127, 0, 0, 9)).then(|| Found {
                ips: vec![IpAddr::V4(ip)],
                synology: true,
                model: "stub".into(),
                ..Default::default()
            })
        };
        let mut ev = vec![];
        let inv = run(&o, &CancelToken::new(), |e| ev.push(e), &stub).unwrap();
        assert_eq!(inv.devices.len(), 1);
        let ScanEvent::Started { total_hosts, networks, methods } = &ev[0] else { panic!("first event: {:?}", ev[0]) };
        assert_eq!((*total_hosts, networks.len(), methods.as_slice()), (253, 1, &[ScanMethod::ProbeSweep][..]));
        let own_lo = Ipv4Addr::LOCALHOST;
        assert!(!probed.lock().unwrap().contains(&own_lo), "never our own address");
        assert_eq!(probed.lock().unwrap().len(), 253);
        assert!(matches!(ev[1], ScanEvent::MethodStarted { method: ScanMethod::ProbeSweep, .. }));
        let last_progress =
            ev.iter().rev().find_map(|e| if let ScanEvent::MethodProgress { done, total, .. } = e { Some((*done, *total)) } else { None });
        assert_eq!(last_progress, Some((253, 253)));
        assert!(ev.iter().any(|e| matches!(e, ScanEvent::Found(d) if d.model == "stub")));
        assert!(ev.iter().any(|e| matches!(e, ScanEvent::MethodFinished { found: 1, error: None, .. })));
        assert!(matches!(ev.last(), Some(ScanEvent::Finished { found_total: 1, .. })));
    }

    #[test]
    fn a_cancelled_sweep_stops_within_a_second() {
        let mut o = lo_net("127.0.0.0/20");
        o.methods.probe_sweep = true;
        let cancel = CancelToken::new();
        let slow = |_ip: Ipv4Addr| {
            std::thread::sleep(Duration::from_millis(100));
            None
        };
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.cancel();
        });
        let t = Instant::now();
        let mut ev = vec![];
        run(&o, &cancel, |e| ev.push(e), &slow).unwrap();
        let took = t.elapsed();
        // 4093 hosts at 100 ms with 128 workers would take about 3 s; cancelled after 0.3 s it ends well inside a second
        assert!(took < Duration::from_millis(1300), "took {took:?}");
        let done =
            ev.iter().rev().find_map(|e| if let ScanEvent::MethodProgress { done, .. } = e { Some(*done) } else { None }).unwrap_or(0);
        assert!(done < 4093, "{done}");
        assert!(matches!(ev.last(), Some(ScanEvent::Finished { .. })), "a cancelled scan still ends with Finished");
    }

    #[test]
    fn unreachable_methods_are_skipped_with_a_reason_not_failed() {
        // a custom network we are not on: multicast cannot reach it; the sweep list is empty for a /32 of ourselves
        let o = ScanOptions {
            networks: vec![ScanNetwork::custom("203.0.113.0/30").unwrap()],
            methods: ScanMethods { mdns: true, ssdp: true, synology_udp: false, probe_sweep: false },
            timeout: Duration::from_millis(300),
            sweep: true,
        };
        let mut ev = vec![];
        run(&o, &CancelToken::new(), |e| ev.push(e), &|_| None).unwrap();
        let skipped = ev.iter().filter(|e| matches!(e, ScanEvent::Skipped { reason, .. } if reason.contains("multicast"))).count();
        assert_eq!(skipped, 2);
        assert!(!ev.iter().any(|e| matches!(e, ScanEvent::MethodStarted { .. })));
    }

    #[test]
    fn the_listening_methods_report_a_cancel_quickly() {
        let o = ScanOptions {
            networks: vec![ScanNetwork::custom("127.0.0.0/30").unwrap()],
            methods: ScanMethods { mdns: false, ssdp: false, synology_udp: true, probe_sweep: false },
            timeout: Duration::from_secs(30),
            sweep: true,
        };
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.cancel();
        });
        let t = Instant::now();
        let mut ev = vec![];
        run(&o, &cancel, |e| ev.push(e), &|_| None).unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert!(ev.iter().any(|e| matches!(e, ScanEvent::MethodFinished { method: ScanMethod::SynologyUdp, .. })), "{ev:?}");
    }
}
