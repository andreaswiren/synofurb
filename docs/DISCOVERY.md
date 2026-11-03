# Finding the NAS: what is scanned

Find NAS (the app) and `synofurb-cli scan` look for Synology units with four methods and merge what they see:

| Method (`id`) | What it does | Needs |
|---|---|---|
| Synology Assistant broadcast (`synology_udp`) | UDP query to port 9999 on the **broadcast address of each selected network**, sent from this computer's address on that network | a unit on the same network; best effort (the packet format is not published by Synology) |
| Bonjour/mDNS (`mdns`) | browses `_diskstation._tcp` and `_synofurb._tcp` **on the adapter of each selected network only** | this computer on that network |
| UPnP/SSDP (`ssdp`) | M-SEARCH out of the adapter of each selected network; the device description must say "Synology" | this computer on that network |
| Port sweep (`probe_sweep`) | TCP connect to port 5000 on every address of each selected network, then the unauthenticated `SYNO.API.Info` call proves DSM. **Never probes this computer's own addresses** | nothing (works when multicast is blocked) |

## Which networks

`synofurb_core::discovery::networks()` lists the local IPv4 networks (loopback excluded) with a stable `id` (`<interface>|<ip>/<prefix>`), `cidr`, `broadcast`, `kind` and `recommended`:

| kind | what | scanned by default |
|---|---|---|
| `lan` | a private (RFC 1918) address on a real adapter | yes |
| `vpn` | 100.64.0.0/10 (Tailscale and other overlays), or an adapter named tailscale / wireguard / openvpn / zerotier / tun / tap | no |
| `virtual` | adapters named like Docker, WSL, Hyper-V (`vEthernet`), VMware, VirtualBox, libvirt, container bridges (`br-`, `veth`); Docker's 172.17.0.0/16, VirtualBox host-only 192.168.56.0/24, and any 172.16.0.0/12 network of /20 or larger (what WSL, Hyper-V and Docker allocate) | no |
| `link_local` | 169.254.0.0/16 (no DHCP answered) | no |
| `other` | public or unknown ranges | no |

Example, the owner's PC: LAN `192.168.69.100/24` is recommended; Tailscale `100.64.248.160` is `vpn`; `172.18.192.1`, `172.31.16.1` and `172.31.82.101` are `virtual`. The classifier is
a pure function (`classify(name, ip, prefix)`) and is a heuristic: a Hyper-V *external* switch carries the real LAN address on a `vEthernet` adapter and is classified `virtual`, so select
that network by hand when the NAS is there. Anything can be selected: the list is the choice, `recommended` only fills the default.

A custom network is a CIDR the user types (`ScanNetwork::custom("10.20.0.0/22")`, at most `/20`; a larger one is refused with `NetworkError::TooLarge` and a suggestion). A network of this
computer larger than a `/20` is swept in the `/24` around our address (`NetworkInfo::sweep_hosts` says how many addresses will be probed).

## API (for the app and the CLI)

```rust
let nets = discovery::networks();                       // Vec<NetworkInfo>
let opts = ScanOptions::default()                       // recommended networks, all methods, 4 s
    .with_networks(vec![ScanNetwork::id(&nets[0].id), ScanNetwork::custom("10.20.0.0/24")?]);
let cancel = CancelToken::new();                        // cancel.cancel() from another thread
let (tx, rx) = std::sync::mpsc::channel();
let inv = discovery::scan_with(&opts, &tx, &cancel)?;   // or scan_with_callback(&opts, &cancel, |event| ...)
```

`ScanEvent`: `Started{networks,methods,total_hosts}`, `MethodStarted{method,network}`, `MethodProgress{method,network,done,total}` (addresses for the sweep, milliseconds for the listening
methods), `MethodFinished{method,network,found,error}`, `Skipped{method,network,reason}`, `Found(Found)` (merged with earlier sightings), `Finished{found_total,elapsed}`. `network` is a
`ResolvedNetwork::key`. A method that fails reports its error in `MethodFinished`; it is never swallowed. `scan_with` returns `Err(NetworkError)` without scanning when the selection cannot be
used (an adapter that has gone away, an invalid or oversized custom network). `CancelToken` stops the listening methods within 300 ms and the sweep within about a second (a probe already in
flight finishes); `Finished` is still the last event. The old `discovery::scan(&opts, &log)` and the per-method `scan` functions remain and cover the recommended networks.

Verified by unit tests (classification of the owner's five adapters, CIDR parsing and the /20 limit, sweep progress and cancel over the loopback range with a stub probe) and CLI tests. Not
verified on real hardware: the multicast interface selection (`IP_MULTICAST_IF` for SSDP, mDNS interface filtering) and the directed broadcasts on Windows and macOS.
