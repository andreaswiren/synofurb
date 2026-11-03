//! What the Find NAS screen shows while a scan runs: one line per method and network, an overall bar and the words around it.
//!
//! [`ScanView`] folds the core's [`ScanEvent`]s into that state; it holds no GPUI types, so every sentence and number is a unit test.

use std::collections::HashSet;
use std::time::Duration;
use synofurb_core::discovery::networks::ResolvedNetwork;
use synofurb_core::discovery::{ScanEvent, ScanMethod};

#[derive(Clone, Debug, PartialEq)]
pub enum LineState {
    Waiting,
    Running,
    Done,
    /// The method could not run or ended with an error; the text is the reason (shown in an info popover).
    Failed(String),
    /// Not run on this network, with the reason.
    Skipped(String),
    /// The scan was stopped before this line finished.
    Stopped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub method: ScanMethod,
    /// `ResolvedNetwork::key`.
    pub network: String,
    pub label: String,
    pub state: LineState,
    pub done: u32,
    pub total: u32,
    pub found: usize,
}

impl Line {
    /// How far this line is, 0 to 1. Finished lines count as complete whatever way they ended.
    pub fn fraction(&self) -> f32 {
        match self.state {
            LineState::Waiting => 0.0,
            LineState::Running => {
                if self.total == 0 {
                    0.0
                } else {
                    (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
                }
            }
            _ => 1.0,
        }
    }
    /// Port sweep lines count addresses; listening methods count milliseconds.
    pub fn is_sweep(&self) -> bool {
        self.method == ScanMethod::ProbeSweep
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanView {
    pub running: bool,
    /// Stop was pressed; the scan ends within about a second.
    pub stopping: bool,
    pub networks: Vec<ResolvedNetwork>,
    pub methods: Vec<ScanMethod>,
    pub total_hosts: u32,
    pub lines: Vec<Line>,
    /// Units seen so far (addresses of Synology units).
    found: HashSet<String>,
    /// `(devices seen, elapsed)` once the scan ended.
    pub finished: Option<(usize, Duration)>,
    /// The scan could not start (the selection is unusable); nothing was scanned.
    pub error: Option<String>,
    /// The scan ended because it was stopped.
    pub was_stopped: bool,
}

impl ScanView {
    /// A scan is about to start: clear everything, show the bar at once.
    pub fn begin(&mut self) {
        *self = ScanView { running: true, ..ScanView::default() };
    }

    pub fn stop_requested(&mut self) {
        if self.running {
            self.stopping = true;
        }
    }

    pub fn fail(&mut self, why: String) {
        self.running = false;
        self.stopping = false;
        self.error = Some(why);
    }

    fn line_mut(&mut self, m: ScanMethod, net: &str) -> Option<&mut Line> {
        self.lines.iter_mut().find(|l| l.method == m && l.network == net)
    }

    pub fn apply(&mut self, ev: &ScanEvent) {
        match ev {
            ScanEvent::Started { networks, methods, total_hosts } => {
                self.networks = networks.clone();
                self.methods = methods.clone();
                self.total_hosts = *total_hosts;
                self.lines = networks
                    .iter()
                    .flat_map(|n| {
                        methods.iter().map(move |m| Line {
                            method: *m,
                            network: n.key.clone(),
                            label: n.label.clone(),
                            state: LineState::Waiting,
                            done: 0,
                            total: 0,
                            found: 0,
                        })
                    })
                    .collect();
            }
            ScanEvent::MethodStarted { method, network } => {
                if let Some(l) = self.line_mut(*method, network) {
                    l.state = LineState::Running;
                }
            }
            ScanEvent::MethodProgress { method, network, done, total } => {
                if let Some(l) = self.line_mut(*method, network) {
                    l.state = LineState::Running;
                    l.done = *done;
                    l.total = *total;
                }
            }
            ScanEvent::MethodFinished { method, network, found, error } => {
                if let Some(l) = self.line_mut(*method, network) {
                    l.found = *found;
                    l.done = l.total.max(l.done);
                    l.state = match error {
                        Some(e) => LineState::Failed(e.clone()),
                        None => LineState::Done,
                    };
                }
            }
            ScanEvent::Skipped { method, network, reason } => {
                if let Some(l) = self.line_mut(*method, network) {
                    l.state = LineState::Skipped(reason.clone());
                }
            }
            ScanEvent::Found(f) => {
                if f.synology || f.synofurb_debian {
                    self.found.insert(f.primary_ip().map(|i| i.to_string()).unwrap_or_else(|| f.serial.clone()));
                }
            }
            ScanEvent::Finished { found_total, elapsed } => {
                self.was_stopped = self.stopping;
                self.running = false;
                self.stopping = false;
                self.finished = Some((*found_total, *elapsed));
                // lines that never ended: the scan was stopped under them
                for l in &mut self.lines {
                    if matches!(l.state, LineState::Waiting | LineState::Running) {
                        l.state = LineState::Stopped;
                    }
                }
            }
        }
    }

    /// Units found so far.
    pub fn found_units(&self) -> usize {
        self.found.len()
    }

    /// Addresses probed so far, over all networks, and the total the sweep will probe.
    pub fn sweep_progress(&self) -> (u32, u32) {
        let done: u32 = self
            .lines
            .iter()
            .filter(|l| l.is_sweep())
            .map(|l| if matches!(l.state, LineState::Done) { l.total.max(l.done) } else { l.done })
            .sum();
        (if self.total_hosts > 0 { done.min(self.total_hosts) } else { done }, self.total_hosts)
    }

    /// Overall progress, 0 to 1: the mean of the lines (a scan with no lines yet is at 0).
    pub fn fraction(&self) -> f32 {
        if self.lines.is_empty() {
            return 0.0;
        }
        self.lines.iter().map(Line::fraction).sum::<f32>() / self.lines.len() as f32
    }

    /// What the scan names as its target: the range that was really swept (`192.168.69.0/24`, `10.0.1.0/24 (part of 10.0.0.0/16)`,
    /// `100.100.1.2/32 (nothing to sweep)`), or `3 networks`, with how many of them were swept only in part.
    pub fn networks_text(&self) -> String {
        match self.networks.as_slice() {
            [] => "your networks".to_string(),
            [n] => swept_label(n),
            many => {
                let limited = many.iter().filter(|n| is_limited(n)).count();
                if limited == 0 {
                    format!("{} networks", many.len())
                } else {
                    format!("{} networks, {limited} swept in part", many.len())
                }
            }
        }
    }

    /// The sweep label of the network a line belongs to.
    pub fn network_text(&self, key: &str) -> String {
        self.networks.iter().find(|n| n.key == key).map(swept_label).unwrap_or_default()
    }

    /// "Scanning 192.168.69.0/24 · 87 of 254 hosts · 2 found"
    pub fn headline(&self) -> String {
        let mut parts = vec![format!("Scanning {}", self.networks_text())];
        let (done, total) = self.sweep_progress();
        if total > 0 {
            parts.push(format!("{done} of {total} hosts"));
        } else if !self.lines.is_empty() {
            parts.push("listening for replies".to_string());
        }
        parts.push(format!("{} found", self.found_units()));
        parts.join(" \u{00b7} ")
    }

    /// "Scanned 192.168.69.0/24 in 6 s · 1 unit found", or "Stopped after 3 s · 1 unit found".
    pub fn summary(&self) -> Option<String> {
        let (_, elapsed) = self.finished?;
        let n = self.found_units();
        let units = if n == 1 { "1 unit found".to_string() } else { format!("{n} units found") };
        Some(if self.was_stopped {
            format!("Stopped after {} \u{00b7} {units}", secs_text(elapsed))
        } else {
            format!("Scanned {} in {} \u{00b7} {units}", self.networks_text(), secs_text(elapsed))
        })
    }

    /// What an empty result says was scanned: "Scanned 192.168.69.0/24 with 4 methods in 6 s." When no address could be probed it says
    /// so (a /32 VPN address has none), so that "no NAS found" is not read as "every address was tried".
    pub fn empty_line(&self) -> String {
        let m = self.methods.len();
        let how = if m == 1 { "1 method".to_string() } else { format!("{m} methods") };
        match self.finished {
            Some((_, e)) if self.was_stopped => format!("Stopped after {} on {}.", secs_text(e), self.networks_text()),
            Some((_, e)) if self.total_hosts == 0 => {
                format!("Scanned {} with {how} in {}. No address was probed, only broadcasts were heard.", self.networks_text(), secs_text(e))
            }
            Some((_, e)) => format!("Scanned {} with {how} in {}.", self.networks_text(), secs_text(e)),
            None => String::new(),
        }
    }

    /// The lines grouped by network, for the progress panel: a network caption only when there is more than one network.
    pub fn show_network_captions(&self) -> bool {
        self.networks.len() > 1
    }

    /// Methods that failed (not merely skipped), for a warning count next to the summary.
    pub fn failed_lines(&self) -> Vec<&Line> {
        self.lines.iter().filter(|l| matches!(l.state, LineState::Failed(_))).collect()
    }
}

/// Is this network swept less than its whole range (a /16 around our address only), or not at all?
pub fn is_limited(n: &ResolvedNetwork) -> bool {
    n.sweep != n.cidr || n.sweep_hosts == 0
}

/// The network as the scan text names it: the range that was really swept. A network of this computer larger than a /20 is swept
/// only in the /24 around our address; a /32 (a VPN address) has nothing to sweep.
pub fn swept_label(n: &ResolvedNetwork) -> String {
    if n.sweep_hosts == 0 {
        format!("{} (nothing to sweep)", n.cidr)
    } else if n.sweep != n.cidr {
        format!("{} (part of {})", n.sweep, n.cidr)
    } else {
        n.cidr.to_string()
    }
}

/// `6 s`, `1 min 5 s`; under a second reads `<1 s`.
pub fn secs_text(d: Duration) -> String {
    let s = d.as_secs_f32();
    if s < 1.0 {
        "<1 s".to_string()
    } else if s < 60.0 {
        format!("{} s", s.round() as u32)
    } else {
        format!("{} min {} s", (s / 60.0) as u32, (s % 60.0).round() as u32)
    }
}

/// The short name of a method, as the progress lines show it.
pub fn method_name(m: ScanMethod) -> &'static str {
    m.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::discovery::Found;
    use synofurb_core::discovery::networks::{Ipv4Cidr, NetworkKind};

    fn net(key: &str, cidr: &str, label: &str) -> ResolvedNetwork {
        let c: Ipv4Cidr = cidr.parse().unwrap();
        ResolvedNetwork {
            key: key.into(),
            label: label.into(),
            cidr: c,
            interface: Some("Ethernet".into()),
            local_ip: Some("192.168.69.10".parse().unwrap()),
            broadcast: c.broadcast(),
            kind: Some(NetworkKind::Lan),
            custom: false,
            sweep: c,
            sweep_hosts: c.host_count(),
        }
    }

    fn started(v: &mut ScanView, nets: Vec<ResolvedNetwork>, hosts: u32) {
        v.begin();
        v.apply(&ScanEvent::Started { networks: nets, methods: vec![ScanMethod::Mdns, ScanMethod::ProbeSweep], total_hosts: hosts });
    }

    #[test]
    fn a_scan_has_one_line_per_method_and_network_all_waiting() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "Ethernet 192.168.69.0/24"), net("b", "10.0.0.0/24", "Wi-Fi 10.0.0.0/24")], 506);
        assert_eq!(v.lines.len(), 4);
        assert!(v.lines.iter().all(|l| l.state == LineState::Waiting));
        assert_eq!(v.fraction(), 0.0);
        assert_eq!(v.headline(), "Scanning 2 networks \u{00b7} 0 of 506 hosts \u{00b7} 0 found");
    }

    #[test]
    fn the_headline_names_the_network_the_hosts_and_what_was_found() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "Ethernet 192.168.69.0/24")], 253);
        v.apply(&ScanEvent::MethodStarted { method: ScanMethod::ProbeSweep, network: "a".into() });
        v.apply(&ScanEvent::MethodProgress { method: ScanMethod::ProbeSweep, network: "a".into(), done: 87, total: 253 });
        v.apply(&ScanEvent::Found(Found { ips: vec!["192.168.69.20".parse().unwrap()], synology: true, ..Default::default() }));
        v.apply(&ScanEvent::Found(Found {
            ips: vec!["192.168.69.20".parse().unwrap()],
            synology: true,
            model: "DS918+".into(),
            ..Default::default()
        }));
        v.apply(&ScanEvent::Found(Found { ips: vec!["192.168.69.1".parse().unwrap()], synology: false, ..Default::default() }));
        v.apply(&ScanEvent::Found(Found { ips: vec!["192.168.69.21".parse().unwrap()], synology: true, ..Default::default() }));
        assert_eq!(
            v.headline(),
            "Scanning 192.168.69.0/24 \u{00b7} 87 of 253 hosts \u{00b7} 2 found",
            "a unit seen twice counts once; a router is not a unit"
        );
        assert!(v.running);
    }

    #[test]
    fn the_bar_is_the_mean_of_the_lines_and_never_goes_back() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "x")], 100);
        v.apply(&ScanEvent::MethodProgress { method: ScanMethod::Mdns, network: "a".into(), done: 2000, total: 4000 });
        v.apply(&ScanEvent::MethodProgress { method: ScanMethod::ProbeSweep, network: "a".into(), done: 50, total: 100 });
        assert!((v.fraction() - 0.5).abs() < 1e-6);
        v.apply(&ScanEvent::MethodFinished { method: ScanMethod::Mdns, network: "a".into(), found: 1, error: None });
        assert!((v.fraction() - 0.75).abs() < 1e-6, "a finished line counts as complete");
    }

    #[test]
    fn failures_and_skips_keep_their_reason() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "x")], 100);
        v.apply(&ScanEvent::MethodFinished {
            method: ScanMethod::Mdns,
            network: "a".into(),
            found: 0,
            error: Some("socket: permission denied".into()),
        });
        v.apply(&ScanEvent::Skipped { method: ScanMethod::ProbeSweep, network: "a".into(), reason: "no other address".into() });
        assert_eq!(v.lines[0].state, LineState::Failed("socket: permission denied".into()));
        assert_eq!(v.lines[1].state, LineState::Skipped("no other address".into()));
        assert_eq!(v.failed_lines().len(), 1);
    }

    #[test]
    fn the_summary_says_what_was_scanned_and_how_long_it_took() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "x")], 100);
        v.apply(&ScanEvent::Found(Found { ips: vec!["192.168.69.20".parse().unwrap()], synology: true, ..Default::default() }));
        assert_eq!(v.summary(), None, "no summary while it runs");
        v.apply(&ScanEvent::Finished { found_total: 1, elapsed: Duration::from_millis(5600) });
        assert_eq!(v.summary().as_deref(), Some("Scanned 192.168.69.0/24 in 6 s \u{00b7} 1 unit found"));
        assert!(!v.running);
        assert!(v.lines.iter().all(|l| l.state == LineState::Stopped), "lines that never ended are marked, not left spinning");
    }

    #[test]
    fn an_empty_scan_names_what_it_looked_at() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "x")], 100);
        v.apply(&ScanEvent::Finished { found_total: 0, elapsed: Duration::from_secs(6) });
        assert_eq!(v.empty_line(), "Scanned 192.168.69.0/24 with 2 methods in 6 s.");
        assert_eq!(v.summary().as_deref(), Some("Scanned 192.168.69.0/24 in 6 s \u{00b7} 0 units found"));
    }

    #[test]
    fn stop_is_remembered_and_the_summary_says_so() {
        let mut v = ScanView::default();
        started(&mut v, vec![net("a", "192.168.69.0/24", "x")], 100);
        v.stop_requested();
        assert!(v.stopping && v.running);
        v.apply(&ScanEvent::Finished { found_total: 0, elapsed: Duration::from_secs(3) });
        assert_eq!(v.summary().as_deref(), Some("Stopped after 3 s \u{00b7} 0 units found"));
        assert!(v.was_stopped && !v.running);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(secs_text(Duration::from_millis(300)), "<1 s");
        assert_eq!(secs_text(Duration::from_secs(6)), "6 s");
        assert_eq!(secs_text(Duration::from_secs(65)), "1 min 5 s");
    }

    #[test]
    fn a_selection_the_core_refused_is_an_error_not_a_scan() {
        let mut v = ScanView::default();
        v.begin();
        v.fail("no network matches 'x'".into());
        assert!(!v.running && v.error.is_some() && v.summary().is_none());
    }

    fn limited(key: &str, cidr: &str, sweep: &str, hosts: u32) -> ResolvedNetwork {
        ResolvedNetwork { sweep: sweep.parse().unwrap(), sweep_hosts: hosts, ..net(key, cidr, key) }
    }

    #[test]
    fn a_scan_says_the_range_it_really_swept() {
        // a /16 of this computer is swept in the /24 around it: the text must not claim the /16
        let mut v = ScanView::default();
        started(&mut v, vec![limited("lan", "10.0.0.0/16", "10.0.1.0/24", 253)], 253);
        v.apply(&ScanEvent::Finished { found_total: 0, elapsed: Duration::from_secs(6) });
        assert_eq!(v.networks_text(), "10.0.1.0/24 (part of 10.0.0.0/16)");
        assert_eq!(v.summary().as_deref(), Some("Scanned 10.0.1.0/24 (part of 10.0.0.0/16) in 6 s \u{00b7} 0 units found"));
        assert_eq!(v.empty_line(), "Scanned 10.0.1.0/24 (part of 10.0.0.0/16) with 2 methods in 6 s.");
        assert!(!v.empty_line().contains("10.0.0.0/16 with"), "never the whole /16 as the thing that was scanned");
        let mut w = ScanView::default();
        started(&mut w, vec![limited("lan", "10.0.0.0/16", "10.0.1.0/24", 253)], 253);
        assert!(w.headline().starts_with("Scanning 10.0.1.0/24 (part of 10.0.0.0/16) \u{00b7} 0 of 253 hosts"), "{}", w.headline());
    }

    #[test]
    fn a_slash_32_sweeps_nothing_and_the_text_says_so() {
        let mut v = ScanView::default();
        started(&mut v, vec![limited("ts", "100.100.1.2/32", "100.100.1.2/32", 0)], 0);
        assert_eq!(v.headline(), "Scanning 100.100.1.2/32 (nothing to sweep) \u{00b7} listening for replies \u{00b7} 0 found");
        v.apply(&ScanEvent::Finished { found_total: 0, elapsed: Duration::from_secs(4) });
        assert_eq!(
            v.empty_line(),
            "Scanned 100.100.1.2/32 (nothing to sweep) with 2 methods in 4 s. No address was probed, only broadcasts were heard."
        );
        assert!(v.summary().unwrap().starts_with("Scanned 100.100.1.2/32 (nothing to sweep) in 4 s"));
    }

    #[test]
    fn several_networks_count_how_many_were_swept_in_part_and_lines_show_the_network_once() {
        let mut v = ScanView::default();
        started(
            &mut v,
            vec![net("a", "192.168.69.0/24", "x"), limited("b", "10.0.0.0/16", "10.0.1.0/24", 253), limited("c", "100.1.1.1/32", "100.1.1.1/32", 0)],
            506,
        );
        assert_eq!(v.networks_text(), "3 networks, 2 swept in part");
        assert!(v.show_network_captions());
        assert_eq!(v.network_text("b"), "10.0.1.0/24 (part of 10.0.0.0/16)");
        assert_eq!(v.network_text("zzz"), "");
        let mut one = ScanView::default();
        started(&mut one, vec![net("a", "192.168.69.0/24", "x")], 253);
        assert!(!one.show_network_captions(), "one network is named in the headline, not on every line");
        let mut plain = ScanView::default();
        started(&mut plain, vec![net("a", "192.168.69.0/24", "x"), net("b", "10.0.0.0/24", "y")], 506);
        assert_eq!(plain.networks_text(), "2 networks");
    }
}
