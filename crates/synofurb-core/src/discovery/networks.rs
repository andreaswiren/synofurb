//! Which networks this computer is on, and which of them a scan should cover.
//!
//! A PC usually has more IPv4 addresses than networks a NAS could be on: Docker, WSL, Hyper-V, VMware and
//! VirtualBox add adapters, and a VPN (Tailscale, WireGuard) adds another. Scanning those wastes time and
//! shows nothing useful, so [`networks`] lists every local IPv4 network with a [`NetworkKind`] and marks only
//! the real LAN ones as `recommended`: that is the default selection. [`ScanNetwork`] is one entry of the
//! selection, either an enumerated network (by `id`) or a custom CIDR the user typed.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

/// Largest custom network (`/20` = 4094 hosts); a bigger one would take minutes to sweep and is almost never intended.
pub const MAX_CUSTOM_PREFIX: u8 = 20;
/// A network of one of our own adapters that is larger than `/20` is swept only in the `/24` around our address.
pub const SWEEP_FALLBACK_PREFIX: u8 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkKind {
    /// A private (RFC 1918) network on a real adapter: where a NAS lives.
    Lan,
    /// A VPN or overlay network (Tailscale/CGNAT `100.64.0.0/10`, WireGuard, OpenVPN, ZeroTier).
    Vpn,
    /// A virtual adapter of Docker, WSL, Hyper-V, VMware, VirtualBox, libvirt and the like.
    Virtual,
    /// `169.254.0.0/16`: no DHCP answered; nothing useful is reachable.
    LinkLocal,
    /// Anything else (a public address, a range we do not know).
    Other,
}

impl NetworkKind {
    pub fn label(self) -> &'static str {
        match self {
            NetworkKind::Lan => "LAN",
            NetworkKind::Vpn => "VPN",
            NetworkKind::Virtual => "Virtual",
            NetworkKind::LinkLocal => "Link-local",
            NetworkKind::Other => "Other",
        }
    }
    /// One sentence for a tooltip or the CLI: why this network is or is not scanned by default.
    pub fn explanation(self) -> &'static str {
        match self {
            NetworkKind::Lan => "A local network: scanned by default.",
            NetworkKind::Vpn => "A VPN or overlay network: a NAS is rarely here and sweeping it is slow. Select it to scan it anyway.",
            NetworkKind::Virtual => {
                "A virtual adapter (Docker, WSL, Hyper-V, VMware, VirtualBox): no NAS lives here. Select it to scan it anyway."
            }
            NetworkKind::LinkLocal => "A self-assigned 169.254.x.x address: the adapter has no real network.",
            NetworkKind::Other => "A network that is not a known private range. Select it to scan it anyway.",
        }
    }
}

/// One IPv4 network this computer is connected to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInfo {
    /// Stable identifier: `"<interface>|<ip>/<prefix>"`. Pass it back in [`ScanNetwork::Id`].
    pub id: String,
    pub interface: String,
    /// This computer's address on the network.
    pub ip: Ipv4Addr,
    pub prefix: u8,
    /// The network in CIDR form, e.g. `192.168.69.0/24` (host bits cleared).
    pub cidr: String,
    pub broadcast: Ipv4Addr,
    pub kind: NetworkKind,
    /// Part of the default selection (real LAN networks only).
    pub recommended: bool,
    /// How many other addresses the port sweep would probe (our own addresses excluded).
    pub sweep_hosts: u32,
}

// ------------------------------------------------------------------------------------------------ CIDR

/// An IPv4 network: the network address and the prefix length. Parses `a.b.c.d/p` (host bits are cleared)
/// and a bare address (a `/32`). Serializes as a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4Cidr {
    addr: Ipv4Addr,
    prefix: u8,
}

impl Ipv4Cidr {
    pub fn new(ip: Ipv4Addr, prefix: u8) -> Result<Ipv4Cidr, NetworkError> {
        if prefix > 32 {
            return Err(NetworkError::Invalid(format!("{ip}/{prefix}: the prefix length must be between 0 and 32")));
        }
        Ok(Ipv4Cidr { addr: Ipv4Addr::from(u32::from(ip) & mask(prefix)), prefix })
    }
    pub fn network(&self) -> Ipv4Addr {
        self.addr
    }
    pub fn prefix(&self) -> u8 {
        self.prefix
    }
    pub fn broadcast(&self) -> Ipv4Addr {
        if self.prefix >= 31 {
            return self.addr;
        }
        Ipv4Addr::from(u32::from(self.addr) | !mask(self.prefix))
    }
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & mask(self.prefix) == u32::from(self.addr)
    }
    /// Number of addresses a host can have in the network (`/24` = 254, `/31` = 2, `/32` = 1).
    pub fn host_count(&self) -> u32 {
        match self.prefix {
            32 => 1,
            31 => 2,
            p => ((1u64 << (32 - p)) - 2) as u32,
        }
    }
    /// Every host address, in order (network and broadcast addresses left out for prefixes up to `/30`).
    pub fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> + use<> {
        let first = u32::from(self.addr);
        let (start, end) = match self.prefix {
            32 => (first, first),
            31 => (first, first + 1),
            _ => (first + 1, u32::from(self.broadcast()) - 1),
        };
        (start..=end).map(Ipv4Addr::from)
    }
    /// The same network widened or narrowed to `prefix` around this network's address.
    pub fn with_prefix(&self, prefix: u8) -> Ipv4Cidr {
        Ipv4Cidr { addr: Ipv4Addr::from(u32::from(self.addr) & mask(prefix.min(32))), prefix: prefix.min(32) }
    }
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix.min(32))) }
}

impl fmt::Display for Ipv4Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl FromStr for Ipv4Cidr {
    type Err = NetworkError;
    fn from_str(s: &str) -> Result<Self, NetworkError> {
        let s = s.trim();
        let bad = |why: &str| NetworkError::Invalid(format!("'{s}' is not a network: {why}. Write it like 192.168.1.0/24"));
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, p.trim().parse::<u8>().map_err(|_| bad("the part after '/' must be a number from 0 to 32"))?),
            None => (s, 32),
        };
        let ip: Ipv4Addr = ip.trim().parse().map_err(|_| bad("the address must be four numbers like 192.168.1.0"))?;
        Ipv4Cidr::new(ip, prefix).map_err(|_| bad("the prefix length must be from 0 to 32"))
    }
}

impl Serialize for Ipv4Cidr {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Ipv4Cidr {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetworkError {
    #[error("{0}")]
    Invalid(String),
    #[error(
        "{cidr} is too large: a /{prefix} has {hosts} addresses and the largest network a scan accepts is a /{MAX_CUSTOM_PREFIX} (4094 addresses). Split it into smaller networks, e.g. {example}"
    )]
    TooLarge { cidr: String, prefix: u8, hosts: u64, example: String },
    #[error("{0}")]
    NotScannable(String),
    #[error("no network matches '{0}'. Run `synofurb-cli scan --list-networks` to see the choices, or give a network like 192.168.1.0/24")]
    Unknown(String),
}

/// Validate a custom network typed by the user: at most `/20`, and an address range that can hold a NAS.
pub fn validate_custom(c: &Ipv4Cidr) -> Result<(), NetworkError> {
    if c.prefix() < MAX_CUSTOM_PREFIX {
        let example = Ipv4Cidr::new(c.network(), MAX_CUSTOM_PREFIX).map(|e| e.to_string()).unwrap_or_default();
        return Err(NetworkError::TooLarge { cidr: c.to_string(), prefix: c.prefix(), hosts: (1u64 << (32 - c.prefix())) - 2, example });
    }
    let n = c.network();
    if n.is_multicast() || n.is_unspecified() || n.octets()[0] >= 240 {
        return Err(NetworkError::NotScannable(format!(
            "{c} is not a network a NAS can be on (multicast, reserved or unspecified addresses)"
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ classification

fn in_range(ip: Ipv4Addr, base: [u8; 4], prefix: u8) -> bool {
    u32::from(ip) & mask(prefix) == u32::from(Ipv4Addr::from(base)) & mask(prefix)
}

/// RFC 1918: 10/8, 172.16/12, 192.168/16.
pub fn is_private(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// 100.64.0.0/10 (carrier-grade NAT; Tailscale and other overlays use it).
pub fn is_cgnat(ip: Ipv4Addr) -> bool {
    in_range(ip, [100, 64, 0, 0], 10)
}

const VIRTUAL_NAME_HINTS: &[&str] = &[
    "vethernet",
    "wsl",
    "docker",
    "hyper-v",
    "vmware",
    "vmnet",
    "virtualbox",
    "vbox",
    "virtual",
    "virbr",
    "lxcbr",
    "lxdbr",
    "podman",
    "default switch",
    "cni",
    "flannel",
    "kube",
    "bridge100",
    "bridge101",
    "utm",
];
const VIRTUAL_NAME_PREFIXES: &[&str] = &["br-", "veth", "vnet", "vmnet", "vboxnet", "cali", "docker", "virbr", "cbr", "lxc"];
const VPN_NAME_HINTS: &[&str] =
    &["tailscale", "zerotier", "wireguard", "openvpn", "vpn", "nordlynx", "wintun", "ipsec", "hamachi", "tap-windows"];
const VPN_NAME_PREFIXES: &[&str] = &["tun", "tap", "utun", "wg", "zt", "ppp", "tailscale"];
const PHYSICAL_NAME_PREFIXES: &[&str] = &["eth", "en", "wl", "ww", "wi-fi", "wifi", "lan", "local area"];

/// Does an adapter name (Linux `eth0`, macOS `en0`, Windows `vEthernet (WSL)`) say it is virtual?
pub fn name_is_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    VIRTUAL_NAME_PREFIXES.iter().any(|p| n.starts_with(p)) || VIRTUAL_NAME_HINTS.iter().any(|h| n.contains(h))
}

/// Does an adapter name say it is a VPN or overlay adapter?
pub fn name_is_vpn(name: &str) -> bool {
    let n = name.to_lowercase();
    VPN_NAME_PREFIXES.iter().any(|p| n.starts_with(p)) || VPN_NAME_HINTS.iter().any(|h| n.contains(h))
}

fn name_is_physical(name: &str) -> bool {
    let n = name.to_lowercase();
    PHYSICAL_NAME_PREFIXES.iter().any(|p| n.starts_with(p)) || n.contains("ethernet") || n.contains("wi-fi") || n.contains("wireless")
}

/// Classify one local address. Pure: depends only on the adapter name, the address and the prefix length.
///
/// * `169.254.0.0/16` is link-local, whatever the adapter.
/// * An adapter named like a Docker/WSL/Hyper-V/VMware/VirtualBox/libvirt/container-network one is virtual; one named like a
///   VPN (Tailscale, WireGuard, OpenVPN, ZeroTier, tun/tap) or addressed in `100.64.0.0/10` is a VPN.
/// * By range: Docker's `172.17.0.0/16`, VirtualBox's host-only `192.168.56.0/24`, and a `172.16.0.0/12` network of `/20` or
///   larger (what WSL, Hyper-V and Docker allocate) are virtual; a `172.16.0.0/12` address on an adapter that does not
///   look like a physical one is virtual too.
/// * Any other RFC 1918 address is a LAN; everything else is "other".
///
/// A Hyper-V *external* switch shows the host's real LAN address on a `vEthernet` adapter and is therefore classified virtual;
/// select it by hand when the NAS is on that network.
pub fn classify(name: &str, ip: Ipv4Addr, prefix: u8) -> NetworkKind {
    if ip.is_link_local() {
        return NetworkKind::LinkLocal;
    }
    if name_is_virtual(name) {
        return NetworkKind::Virtual;
    }
    if name_is_vpn(name) || is_cgnat(ip) {
        return NetworkKind::Vpn;
    }
    if in_range(ip, [172, 17, 0, 0], 16) || in_range(ip, [192, 168, 56, 0], 24) {
        return NetworkKind::Virtual;
    }
    if in_range(ip, [172, 16, 0, 0], 12) && (prefix <= 20 || !name_is_physical(name)) {
        return NetworkKind::Virtual;
    }
    if is_private(ip) {
        return NetworkKind::Lan;
    }
    NetworkKind::Other
}

fn kind_order(k: NetworkKind) -> u8 {
    match k {
        NetworkKind::Lan => 0,
        NetworkKind::Other => 1,
        NetworkKind::Vpn => 2,
        NetworkKind::Virtual => 3,
        NetworkKind::LinkLocal => 4,
    }
}

/// The stable id of a network: `"<interface>|<ip>/<prefix>"`.
pub fn network_id(interface: &str, ip: Ipv4Addr, prefix: u8) -> String {
    format!("{interface}|{ip}/{prefix}")
}

/// One local IPv4 address as the operating system reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAddr {
    pub interface: String,
    pub ip: Ipv4Addr,
    pub prefix: u8,
}

/// Build the network list from raw local addresses (pure; [`networks`] feeds it the real ones). Loopback addresses are left out,
/// duplicates (same id) are merged, and the order is: recommended first, then by kind, then by interface name.
pub fn networks_from(addrs: &[LocalAddr]) -> Vec<NetworkInfo> {
    let own: Vec<Ipv4Addr> = addrs.iter().map(|a| a.ip).collect();
    let mut v: Vec<NetworkInfo> = vec![];
    for a in addrs.iter().filter(|a| !a.ip.is_loopback() && !a.ip.is_unspecified() && a.prefix <= 32) {
        let id = network_id(&a.interface, a.ip, a.prefix);
        if v.iter().any(|n| n.id == id) {
            continue;
        }
        let Ok(cidr) = Ipv4Cidr::new(a.ip, a.prefix) else { continue };
        let kind = classify(&a.interface, a.ip, a.prefix);
        let sweep = sweep_range(&cidr, a.ip, false);
        v.push(NetworkInfo {
            id,
            interface: a.interface.clone(),
            ip: a.ip,
            prefix: a.prefix,
            cidr: cidr.to_string(),
            broadcast: cidr.broadcast(),
            kind,
            recommended: kind == NetworkKind::Lan,
            sweep_hosts: sweep_hosts(&sweep, &own).len() as u32,
        });
    }
    v.sort_by(|a, b| {
        (!a.recommended, kind_order(a.kind), &a.interface, a.ip).cmp(&(!b.recommended, kind_order(b.kind), &b.interface, b.ip))
    });
    v
}

/// The range the port sweep covers for a network: a custom network entirely, one of our own adapters' networks entirely up to
/// `/20`, and the `/24` around our address for anything larger (a `/16` LAN is not swept address by address).
pub fn sweep_range(cidr: &Ipv4Cidr, own_ip: Ipv4Addr, custom: bool) -> Ipv4Cidr {
    if custom || cidr.prefix() >= MAX_CUSTOM_PREFIX {
        return *cidr;
    }
    Ipv4Cidr::new(own_ip, SWEEP_FALLBACK_PREFIX).unwrap_or(*cidr)
}

/// The addresses to probe in `range`, without our own. A `/32` or `/31` network of one of our adapters (a VPN point-to-point
/// address) yields nothing but the peer.
pub fn sweep_hosts(range: &Ipv4Cidr, own: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    range.hosts().filter(|h| !own.contains(h)).collect()
}

/// Every local, non-loopback IPv4 network that is up, classified. Empty when the interfaces cannot be read.
pub fn networks() -> Vec<NetworkInfo> {
    networks_from(&local_addrs())
}

/// The default selection: real LAN networks only.
pub fn recommended() -> Vec<NetworkInfo> {
    networks().into_iter().filter(|n| n.recommended).collect()
}

/// All local IPv4 addresses, including loopback (used to recognise our own packets).
pub fn local_addrs() -> Vec<LocalAddr> {
    use if_addrs::IfOperStatus::{Down, LowerLayerDown, NotPresent};
    let Ok(ifs) = if_addrs::get_if_addrs() else { return vec![] };
    ifs.into_iter()
        .filter(|i| !matches!(i.oper_status, Down | NotPresent | LowerLayerDown))
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) => Some(LocalAddr { interface: i.name, ip: a.ip, prefix: a.prefixlen }),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------ selection

/// One entry of the networks a scan covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScanNetwork {
    /// A network of this computer, by [`NetworkInfo::id`].
    Id { id: String },
    /// A network the user typed (at most `/20`, see [`validate_custom`]).
    Custom { cidr: Ipv4Cidr },
}

impl ScanNetwork {
    pub fn id(id: impl Into<String>) -> ScanNetwork {
        ScanNetwork::Id { id: id.into() }
    }
    pub fn from_info(n: &NetworkInfo) -> ScanNetwork {
        ScanNetwork::Id { id: n.id.clone() }
    }
    /// A custom network from text such as `10.20.0.0/22`; refused when it is invalid or larger than `/20`.
    pub fn custom(text: &str) -> Result<ScanNetwork, NetworkError> {
        let cidr: Ipv4Cidr = text.parse()?;
        validate_custom(&cidr)?;
        Ok(ScanNetwork::Custom { cidr })
    }
}

/// A network ready to scan: what [`ScanNetwork`] resolves to against the current interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedNetwork {
    /// Identifies the network in scan events: the `id` of an enumerated network, the CIDR of a custom one.
    pub key: String,
    /// Short text for progress lines, e.g. `Ethernet 192.168.69.0/24`.
    pub label: String,
    pub cidr: Ipv4Cidr,
    pub interface: Option<String>,
    /// Our address on this network, when one of our adapters is on it (mDNS and SSDP need it; broadcasts leave from it).
    pub local_ip: Option<Ipv4Addr>,
    pub broadcast: Ipv4Addr,
    pub kind: Option<NetworkKind>,
    pub custom: bool,
    /// The range the port sweep covers.
    pub sweep: Ipv4Cidr,
    /// How many addresses the sweep probes.
    pub sweep_hosts: u32,
}

/// Resolve a selection against `available` (the current [`networks`]) and the full list of our own addresses.
/// An id that no longer exists (adapter unplugged) is an error naming it.
pub fn resolve_networks(
    selection: &[ScanNetwork],
    available: &[NetworkInfo],
    own: &[Ipv4Addr],
) -> Result<Vec<ResolvedNetwork>, NetworkError> {
    let mut out: Vec<ResolvedNetwork> = vec![];
    for s in selection {
        let r = match s {
            ScanNetwork::Id { id } => {
                let n = available.iter().find(|n| &n.id == id).ok_or_else(|| NetworkError::Unknown(id.clone()))?;
                let cidr = Ipv4Cidr::new(n.ip, n.prefix)?;
                let sweep = sweep_range(&cidr, n.ip, false);
                ResolvedNetwork {
                    key: n.id.clone(),
                    label: format!("{} {}", n.interface, n.cidr),
                    cidr,
                    interface: Some(n.interface.clone()),
                    local_ip: Some(n.ip),
                    broadcast: n.broadcast,
                    kind: Some(n.kind),
                    custom: false,
                    sweep,
                    sweep_hosts: sweep_hosts(&sweep, own).len() as u32,
                }
            }
            ScanNetwork::Custom { cidr } => {
                validate_custom(cidr)?;
                let local = available.iter().find(|n| cidr.contains(n.ip));
                ResolvedNetwork {
                    key: cidr.to_string(),
                    label: format!("{cidr} (custom)"),
                    cidr: *cidr,
                    interface: local.map(|n| n.interface.clone()),
                    local_ip: local.map(|n| n.ip),
                    broadcast: cidr.broadcast(),
                    kind: local.map(|n| n.kind),
                    custom: true,
                    sweep: *cidr,
                    sweep_hosts: sweep_hosts(cidr, own).len() as u32,
                }
            }
        };
        if !out.iter().any(|o| o.key == r.key) {
            out.push(r);
        }
    }
    Ok(out)
}

/// Turn what a user typed into selection entries: a network id, an interface name (all its networks), one of our own
/// addresses, or a CIDR (`192.168.1.0/24`; a bare address is a single host). A custom network larger than `/20` is refused.
pub fn parse_selector(input: &str, available: &[NetworkInfo]) -> Result<Vec<ScanNetwork>, NetworkError> {
    let t = input.trim();
    if t.is_empty() {
        return Err(NetworkError::Invalid("an empty network name; give an interface, a network id or a CIDR like 192.168.1.0/24".into()));
    }
    if let Some(n) = available.iter().find(|n| n.id == t) {
        return Ok(vec![ScanNetwork::from_info(n)]);
    }
    if let Ok(ip) = t.parse::<Ipv4Addr>()
        && let Some(n) = available.iter().find(|n| n.ip == ip)
    {
        return Ok(vec![ScanNetwork::from_info(n)]);
    }
    if t.contains('/') || t.parse::<Ipv4Addr>().is_ok() {
        let c: Ipv4Cidr = t.parse()?;
        // the exact CIDR of one of our networks is that network
        if let Some(n) = available.iter().find(|n| n.cidr == c.to_string()) {
            return Ok(vec![ScanNetwork::from_info(n)]);
        }
        return ScanNetwork::custom(t).map(|n| vec![n]);
    }
    let same: Vec<ScanNetwork> = available.iter().filter(|n| n.interface.eq_ignore_ascii_case(t)).map(ScanNetwork::from_info).collect();
    if same.is_empty() { Err(NetworkError::Unknown(t.to_string())) } else { Ok(same) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }
    fn la(name: &str, addr: &str, prefix: u8) -> LocalAddr {
        LocalAddr { interface: name.into(), ip: ip(addr), prefix }
    }

    /// The five adapters of the owner's PC, with the names Windows gives them.
    fn owners_pc() -> Vec<LocalAddr> {
        vec![
            la("Ethernet", "192.168.69.100", 24),
            la("Tailscale", "100.64.248.160", 32),
            la("vEthernet (WSL (Hyper-V firewall))", "172.18.192.1", 20),
            la("vEthernet (Default Switch)", "172.31.16.1", 20),
            la("VirtualBox Host-Only Network", "172.31.82.101", 24),
            la("Loopback Pseudo-Interface 1", "127.0.0.1", 8),
        ]
    }

    #[test]
    fn the_owners_adapters_only_the_lan_is_recommended() {
        let nets = networks_from(&owners_pc());
        assert_eq!(nets.len(), 5, "loopback is excluded");
        let kind = |i: &str| nets.iter().find(|n| n.ip == ip(i)).unwrap().kind;
        assert_eq!(kind("192.168.69.100"), NetworkKind::Lan);
        assert_eq!(kind("100.64.248.160"), NetworkKind::Vpn);
        assert_eq!(kind("172.18.192.1"), NetworkKind::Virtual);
        assert_eq!(kind("172.31.16.1"), NetworkKind::Virtual);
        assert_eq!(kind("172.31.82.101"), NetworkKind::Virtual);
        let rec: Vec<_> = nets.iter().filter(|n| n.recommended).collect();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].id, "Ethernet|192.168.69.100/24");
        assert_eq!(rec[0].cidr, "192.168.69.0/24");
        assert_eq!(rec[0].broadcast, ip("192.168.69.255"));
        assert_eq!(rec[0].sweep_hosts, 253, "our own address is not swept");
        assert_eq!(nets[0].id, rec[0].id, "recommended networks come first");
    }

    #[test]
    fn the_owners_adapters_are_virtual_even_with_unhelpful_names() {
        // a generic Windows name must not make the 172.x adapters look like a LAN
        assert_eq!(classify("Unidentified network", ip("172.31.82.101"), 24), NetworkKind::Virtual);
        assert_eq!(
            classify("Ethernet 5", ip("172.18.192.1"), 20),
            NetworkKind::Virtual,
            "a /20 in 172.16/12 is what WSL and Hyper-V allocate"
        );
        assert_eq!(classify("Ethernet 3", ip("172.31.16.1"), 20), NetworkKind::Virtual);
        assert_eq!(classify("Ethernet", ip("100.64.248.160"), 32), NetworkKind::Vpn, "100.64/10 is a VPN whatever the name");
        // but a real corporate 172.16/12 /24 on a real adapter is a LAN
        assert_eq!(classify("Ethernet", ip("172.20.5.10"), 24), NetworkKind::Lan);
        assert_eq!(classify("eth0", ip("172.20.5.10"), 24), NetworkKind::Lan);
    }

    #[test]
    fn classification_by_name_and_range() {
        use NetworkKind::*;
        for (name, addr, p, want) in [
            ("eth0", "192.168.1.5", 24, Lan),
            ("enp3s0", "10.0.0.7", 8, Lan),
            ("wlp2s0", "192.168.0.20", 24, Lan),
            ("en0", "10.1.2.3", 24, Lan),
            ("Wi-Fi", "192.168.178.20", 24, Lan),
            ("Ethernet", "10.10.0.4", 16, Lan),
            ("docker0", "172.17.0.1", 16, Virtual),
            ("br-1a2b3c4d5e6f", "172.19.0.1", 16, Virtual),
            ("veth1234", "10.0.3.1", 24, Virtual),
            ("virbr0", "192.168.122.1", 24, Virtual),
            ("vEthernet (WSL)", "172.28.0.1", 20, Virtual),
            ("VMware Network Adapter VMnet8", "192.168.174.1", 24, Virtual),
            ("VMware Network Adapter VMnet1", "192.168.40.1", 24, Virtual),
            ("Ethernet 2", "192.168.56.1", 24, Virtual),
            ("VirtualBox Host-Only Ethernet Adapter", "192.168.99.1", 24, Virtual),
            ("Hyper-V Virtual Ethernet Adapter", "192.168.100.1", 24, Virtual),
            ("tailscale0", "100.100.1.2", 32, Vpn),
            ("wg0", "10.66.66.2", 32, Vpn),
            ("tun0", "10.8.0.6", 24, Vpn),
            ("OpenVPN TAP-Windows6", "10.8.0.2", 24, Vpn),
            ("ZeroTier One [8056c2e21c]", "10.147.17.5", 24, Vpn),
            ("Ethernet", "169.254.12.3", 16, LinkLocal),
            ("Local Area Connection* 9", "169.254.9.1", 16, LinkLocal),
            ("eth0", "203.0.113.9", 24, Other),
            ("Ethernet", "100.63.255.1", 24, Other),
            ("Ethernet", "100.128.0.1", 24, Other),
        ] {
            assert_eq!(classify(name, ip(addr), p), want, "{name} {addr}/{p}");
        }
    }

    #[test]
    fn network_list_shape_and_order() {
        let nets = networks_from(&[
            la("docker0", "172.17.0.1", 16),
            la("eth1", "10.0.0.2", 24),
            la("eth0", "192.168.1.5", 24),
            la("eth0", "192.168.1.5", 24),
            la("lo", "127.0.0.1", 8),
            la("tailscale0", "100.100.1.2", 32),
        ]);
        let ids: Vec<&str> = nets.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["eth0|192.168.1.5/24", "eth1|10.0.0.2/24", "tailscale0|100.100.1.2/32", "docker0|172.17.0.1/16"]);
        let ts = &nets[2];
        assert_eq!((ts.prefix, ts.cidr.as_str(), ts.sweep_hosts), (32, "100.100.1.2/32", 0), "a /32 VPN address has nothing to sweep");
        assert_eq!(nets[3].sweep_hosts, 253, "a /16 is swept in the /24 around us only");
        let json = serde_json::to_value(&nets[0]).unwrap();
        assert_eq!(json["kind"], "lan");
        assert_eq!(json["recommended"], true);
        assert_eq!(serde_json::from_value::<NetworkInfo>(json).unwrap(), nets[0]);
    }

    #[test]
    fn the_real_interface_list_never_contains_loopback() {
        for n in networks() {
            assert!(!n.ip.is_loopback(), "{n:?}");
            assert!(n.id.contains('|'));
        }
    }

    #[test]
    fn cidr_parsing_and_math() {
        let c: Ipv4Cidr = "192.168.69.77/24".parse().unwrap();
        assert_eq!(c.to_string(), "192.168.69.0/24", "host bits are cleared");
        assert_eq!((c.broadcast(), c.host_count()), (ip("192.168.69.255"), 254));
        assert!(c.contains(ip("192.168.69.1")) && !c.contains(ip("192.168.70.1")));
        assert_eq!(c.hosts().count(), 254);
        assert_eq!(c.hosts().next(), Some(ip("192.168.69.1")));
        assert_eq!(c.hosts().last(), Some(ip("192.168.69.254")));
        assert_eq!("10.0.0.5".parse::<Ipv4Cidr>().unwrap().to_string(), "10.0.0.5/32");
        assert_eq!("10.0.0.5/32".parse::<Ipv4Cidr>().unwrap().hosts().collect::<Vec<_>>(), [ip("10.0.0.5")]);
        assert_eq!("10.0.0.4/31".parse::<Ipv4Cidr>().unwrap().hosts().count(), 2);
        assert_eq!("10.0.0.0/20".parse::<Ipv4Cidr>().unwrap().host_count(), 4094);
        for bad in ["", "abc", "1.2.3/24", "1.2.3.4/33", "1.2.3.4/x", "300.1.1.1/24", "1.2.3.4/-1", "::1/64", "1.2.3.4/24/2"] {
            assert!(bad.parse::<Ipv4Cidr>().is_err(), "{bad:?}");
        }
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"192.168.69.0/24\"");
        assert_eq!(serde_json::from_str::<Ipv4Cidr>(&json).unwrap(), c);
    }

    #[test]
    fn custom_networks_larger_than_a_slash_20_are_refused() {
        assert!(ScanNetwork::custom("10.0.0.0/20").is_ok());
        assert!(ScanNetwork::custom("10.0.0.0/24").is_ok());
        assert!(ScanNetwork::custom("10.0.0.7/32").is_ok());
        for big in ["10.0.0.0/19", "10.0.0.0/16", "192.168.0.0/8", "0.0.0.0/0"] {
            let e = ScanNetwork::custom(big).unwrap_err();
            assert!(matches!(e, NetworkError::TooLarge { .. }), "{big}: {e}");
            let msg = e.to_string();
            assert!(msg.contains("too large") && msg.contains("/20") && msg.contains("Split"), "{msg}");
        }
        assert!(ScanNetwork::custom("224.0.0.0/24").is_err(), "multicast");
        assert!(ScanNetwork::custom("not a network").unwrap_err().to_string().contains("192.168.1.0/24"));
    }

    #[test]
    fn selectors_accept_id_interface_address_and_cidr() {
        let nets = networks_from(&owners_pc());
        let lan = ScanNetwork::id("Ethernet|192.168.69.100/24");
        assert_eq!(parse_selector("Ethernet|192.168.69.100/24", &nets).unwrap(), vec![lan.clone()]);
        assert_eq!(parse_selector("ethernet", &nets).unwrap(), vec![lan.clone()], "interface name, any case");
        assert_eq!(parse_selector("192.168.69.100", &nets).unwrap(), vec![lan.clone()], "one of our own addresses");
        assert_eq!(parse_selector("192.168.69.0/24", &nets).unwrap(), vec![lan], "the CIDR of one of our networks");
        assert_eq!(parse_selector("10.9.0.0/22", &nets).unwrap(), vec![ScanNetwork::Custom { cidr: "10.9.0.0/22".parse().unwrap() }]);
        assert_eq!(parse_selector("10.9.0.5", &nets).unwrap(), vec![ScanNetwork::Custom { cidr: "10.9.0.5/32".parse().unwrap() }]);
        assert!(matches!(parse_selector("10.0.0.0/16", &nets), Err(NetworkError::TooLarge { .. })));
        assert!(matches!(parse_selector("wlan9", &nets), Err(NetworkError::Unknown(_))));
        assert!(parse_selector("  ", &nets).is_err());
        // an interface with two networks selects both
        let two = networks_from(&[la("eth0", "192.168.1.5", 24), la("eth0", "10.0.0.5", 24)]);
        assert_eq!(parse_selector("eth0", &two).unwrap().len(), 2);
    }

    #[test]
    fn resolving_a_selection() {
        let nets = networks_from(&owners_pc());
        let own: Vec<Ipv4Addr> = owners_pc().iter().map(|a| a.ip).collect();
        let sel = vec![
            ScanNetwork::id("Ethernet|192.168.69.100/24"),
            ScanNetwork::custom("192.168.69.0/25").unwrap(),
            ScanNetwork::custom("10.9.0.0/30").unwrap(),
            ScanNetwork::id("Ethernet|192.168.69.100/24"),
        ];
        let r = resolve_networks(&sel, &nets, &own).unwrap();
        assert_eq!(r.len(), 3, "duplicates are merged");
        assert_eq!((r[0].key.as_str(), r[0].sweep_hosts, r[0].local_ip), ("Ethernet|192.168.69.100/24", 253, Some(ip("192.168.69.100"))));
        assert!(
            r[1].custom && r[1].local_ip == Some(ip("192.168.69.100")),
            "a custom network that holds our address is attached to that adapter"
        );
        assert_eq!(r[1].interface.as_deref(), Some("Ethernet"));
        assert_eq!((r[2].local_ip, r[2].sweep_hosts, r[2].broadcast), (None, 2, ip("10.9.0.3")));
        let gone = resolve_networks(&[ScanNetwork::id("eth9|1.2.3.4/24")], &nets, &own).unwrap_err();
        assert!(matches!(gone, NetworkError::Unknown(_)));
        // serde shape for the GUI
        let v = serde_json::to_value(&sel[1]).unwrap();
        assert_eq!(v["type"], "custom");
        assert_eq!(v["cidr"], "192.168.69.0/25");
        assert_eq!(serde_json::from_value::<ScanNetwork>(v).unwrap(), sel[1]);
    }
}
