//! Which networks the Find NAS scan covers: the person's choice, remembered in `ui-settings.json`, resolved against the networks this
//! computer is on right now.
//!
//! * Nothing chosen yet means "the recommended networks" (real LAN adapters; never VPN or virtual ones).
//! * A remembered network that is gone (an unplugged adapter, a VPN that is off) is dropped from the scan; when nothing remains the
//!   recommended networks are used again, and the dialog says so.
//! * A custom network is a CIDR the person typed, at most a /20 (the core's rule, its message is shown as is).
//!
//! No GPUI types here.

use serde::{Deserialize, Serialize};
use synofurb_core::discovery::networks::{NetworkError, NetworkInfo, ScanNetwork};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomNet {
    pub cidr: String,
    pub on: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetSelection {
    /// Ticked networks of this computer by `NetworkInfo::id`; `None` = the recommended ones (nothing chosen yet).
    pub ids: Option<Vec<String>>,
    pub custom: Vec<CustomNet>,
}

/// What a scan will use right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Effective {
    /// Ids of the ticked networks that exist now.
    pub ids: Vec<String>,
    /// Ticked custom networks.
    pub custom: Vec<String>,
    /// Remembered ids that no longer exist.
    pub dropped: Vec<String>,
    /// Everything remembered is gone, so the recommended networks stand in.
    pub fell_back: bool,
}

impl NetSelection {
    pub fn effective(&self, avail: &[NetworkInfo]) -> Effective {
        let recommended: Vec<String> = avail.iter().filter(|n| n.recommended).map(|n| n.id.clone()).collect();
        let custom: Vec<String> = self.custom.iter().filter(|c| c.on).map(|c| c.cidr.clone()).collect();
        let (mut ids, dropped) = match &self.ids {
            None => (recommended.clone(), vec![]),
            Some(chosen) => {
                let (keep, gone): (Vec<_>, Vec<_>) = chosen.iter().cloned().partition(|i| avail.iter().any(|n| &n.id == i));
                (keep, gone)
            }
        };
        let mut fell_back = false;
        if ids.is_empty() && custom.is_empty() {
            fell_back = self.ids.is_some() && !dropped.is_empty();
            ids = recommended;
        }
        Effective { ids, custom, dropped, fell_back }
    }

    /// The scan selection for the core.
    pub fn to_scan(&self, avail: &[NetworkInfo]) -> Vec<ScanNetwork> {
        let e = self.effective(avail);
        let mut v: Vec<ScanNetwork> = e.ids.into_iter().map(ScanNetwork::id).collect();
        v.extend(e.custom.iter().filter_map(|c| ScanNetwork::custom(c).ok()));
        v
    }

    /// `(selected, total)` for the chip "Networks: 1 of 5".
    pub fn counts(&self, avail: &[NetworkInfo]) -> (usize, usize) {
        let e = self.effective(avail);
        (e.ids.len() + e.custom.len(), avail.len() + self.custom.len())
    }

    pub fn is_on(&self, avail: &[NetworkInfo], id: &str) -> bool {
        self.effective(avail).ids.iter().any(|i| i == id)
    }

    /// Turn one network of this computer on or off. The first change writes the recommended choice out so the rest stays as it was.
    pub fn toggle(&mut self, avail: &[NetworkInfo], id: &str) {
        let mut ids = self.effective(avail).ids;
        match ids.iter().position(|i| i == id) {
            Some(p) => {
                ids.remove(p);
            }
            None => ids.push(id.to_string()),
        }
        self.ids = Some(ids);
    }

    pub fn toggle_custom(&mut self, cidr: &str) {
        if let Some(c) = self.custom.iter_mut().find(|c| c.cidr == cidr) {
            c.on = !c.on;
        }
    }

    pub fn remove_custom(&mut self, cidr: &str) {
        self.custom.retain(|c| c.cidr != cidr);
    }

    pub fn select_recommended(&mut self) {
        self.ids = None;
        for c in &mut self.custom {
            c.on = false;
        }
    }

    pub fn select_all(&mut self, avail: &[NetworkInfo]) {
        self.ids = Some(avail.iter().map(|n| n.id.clone()).collect());
        for c in &mut self.custom {
            c.on = true;
        }
    }

    /// Add a network the person typed (it is ticked). The error is the sentence to show under the field.
    pub fn add_custom(&mut self, text: &str) -> Result<String, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("Enter a network such as 192.168.1.0/24.".into());
        }
        let ScanNetwork::Custom { cidr } = ScanNetwork::custom(t).map_err(error_text)? else {
            return Err("That is not a network.".into());
        };
        let c = cidr.to_string();
        match self.custom.iter_mut().find(|x| x.cidr == c) {
            Some(x) => x.on = true,
            None => self.custom.push(CustomNet { cidr: c.clone(), on: true }),
        }
        Ok(c)
    }
}

/// The core's sentence for a refused network, shortened where it carries advice for the command line.
pub fn error_text(e: NetworkError) -> String {
    match e {
        NetworkError::TooLarge { prefix, .. } => {
            format!("A /{prefix} is too large: a scan takes a /20 (4094 addresses) at most. Split it into smaller networks.")
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::discovery::networks::{LocalAddr, networks_from};

    fn avail() -> Vec<NetworkInfo> {
        let a = |n: &str, ip: &str, p: u8| LocalAddr { interface: n.into(), ip: ip.parse().unwrap(), prefix: p };
        networks_from(&[
            a("Ethernet", "192.168.69.10", 24),
            a("Tailscale", "100.101.4.7", 10),
            a("vEthernet (WSL)", "172.28.16.1", 20),
            a("docker0", "172.17.0.1", 16),
            a("VMware Network Adapter VMnet8", "192.168.136.1", 24),
        ])
    }

    #[test]
    fn nothing_chosen_means_the_real_lan_and_never_vpn_or_virtual() {
        let av = avail();
        let s = NetSelection::default();
        let e = s.effective(&av);
        assert_eq!(e.ids.len(), 1, "{av:?}");
        assert!(e.ids[0].starts_with("Ethernet|"));
        assert_eq!(s.counts(&av), (1, 5), "the chip reads 'Networks: 1 of 5'");
        assert_eq!(s.to_scan(&av).len(), 1);
    }

    #[test]
    fn the_first_toggle_keeps_the_recommended_choice_and_adds_to_it() {
        let av = avail();
        let mut s = NetSelection::default();
        let vpn = av.iter().find(|n| n.interface == "Tailscale").unwrap().id.clone();
        s.toggle(&av, &vpn);
        assert_eq!(s.effective(&av).ids.len(), 2);
        assert!(s.is_on(&av, &vpn));
        s.toggle(&av, &vpn);
        assert_eq!(s.effective(&av).ids.len(), 1);
    }

    #[test]
    fn select_all_and_select_recommended() {
        let av = avail();
        let mut s = NetSelection::default();
        s.select_all(&av);
        assert_eq!(s.counts(&av), (5, 5));
        s.select_recommended();
        assert_eq!(s.counts(&av), (1, 5));
        assert_eq!(s.ids, None, "recommended is stored as 'nothing chosen' so a new adapter follows the rule");
    }

    #[test]
    fn a_network_that_disappeared_is_dropped_and_the_recommended_ones_stand_in() {
        let av = avail();
        let s = NetSelection { ids: Some(vec!["Gone|10.9.9.9/24".into()]), custom: vec![] };
        let e = s.effective(&av);
        assert_eq!(e.dropped, vec!["Gone|10.9.9.9/24".to_string()]);
        assert!(e.fell_back);
        assert_eq!(e.ids.len(), 1, "falls back to the recommended network");
        let s2 = NetSelection { ids: Some(vec![av[0].id.clone(), "Gone|10.9.9.9/24".into()]), custom: vec![] };
        let e2 = s2.effective(&av);
        assert_eq!((e2.ids.len(), e2.dropped.len(), e2.fell_back), (1, 1, false), "the rest of the choice is kept");
    }

    #[test]
    fn custom_networks_are_validated_with_the_cores_rules() {
        let mut s = NetSelection::default();
        assert_eq!(s.add_custom("10.20.0.0/22").unwrap(), "10.20.0.0/22");
        assert_eq!(s.add_custom("10.20.0.7/22").unwrap(), "10.20.0.0/22", "host bits are cleared, and it is one entry");
        assert_eq!(s.custom.len(), 1);
        assert!(s.add_custom("10.0.0.0/8").unwrap_err().contains("/20"), "larger than a /20 is refused with the reason");
        assert!(s.add_custom("not a network").is_err());
        assert!(s.add_custom("").is_err());
        assert!(s.add_custom("224.0.0.0/24").is_err(), "multicast is not a network a NAS is on");
        s.toggle_custom("10.20.0.0/22");
        assert!(!s.custom[0].on);
        s.remove_custom("10.20.0.0/22");
        assert!(s.custom.is_empty());
    }

    #[test]
    fn a_custom_network_alone_is_a_valid_choice() {
        let av = avail();
        let mut s = NetSelection { ids: Some(vec![]), custom: vec![] };
        s.add_custom("172.16.5.0/24").unwrap();
        let e = s.effective(&av);
        assert_eq!((e.ids.len(), e.custom.len(), e.fell_back), (0, 1, false));
        assert_eq!(s.to_scan(&av).len(), 1);
    }

    #[test]
    fn the_choice_survives_a_restart() {
        let av = avail();
        let mut s = NetSelection::default();
        s.toggle(&av, &av[1].id);
        s.add_custom("10.1.0.0/24").unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<NetSelection>(&json).unwrap(), s);
        assert_eq!(serde_json::from_str::<NetSelection>("{}").unwrap(), NetSelection::default(), "an old settings file means the default");
    }
}
