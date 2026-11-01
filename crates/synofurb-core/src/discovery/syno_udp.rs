//! Synology Assistant style discovery: UDP broadcast on port 9999 with a TLV query; DSM answers with
//! its hostname, MAC, IP, ports, DSM build, model, platform and serial.
//!
//! The packet layout comes from third-party reimplementations of the protocol (magic `12 34 56 78 'S' 'Y' 'N' 'O'`
//! followed by `type, length, value` records). It is NOT published by Synology and was not verified
//! against hardware by this project, so this source is best-effort; mDNS, SSDP and the Web API probe
//! are the primary mechanisms.

use super::{Found, normalize_mac};
use anyhow::Result;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub const MAGIC: [u8; 8] = [0x12, 0x34, 0x56, 0x78, b'S', b'Y', b'N', b'O'];
pub const PORT: u16 = 9999;

fn tlv(out: &mut Vec<u8>, id: u8, data: &[u8]) {
    out.push(id);
    out.push(data.len().min(255) as u8);
    out.extend_from_slice(&data[..data.len().min(255)]);
}

/// The query packet. The MAC string is a locally administered address of ours (the query asks
/// "who answers me"); it is not a Synology or VMware address.
pub fn build_query() -> Vec<u8> {
    let mut p = MAGIC.to_vec();
    tlv(&mut p, 0xa4, &[0x00, 0x00, 0x02, 0x01]);
    tlv(&mut p, 0xa6, &[0x78, 0x00, 0x00, 0x00]);
    tlv(&mut p, 0x01, &[0x01, 0x00, 0x00, 0x00]);
    tlv(&mut p, 0xb0, &[0xc0, 0x01, 0, 0, 0, 0, 0, 0]);
    tlv(&mut p, 0xb1, &[0; 8]);
    tlv(&mut p, 0xb8, &[0xc0, 0x01, 0, 0, 0, 0, 0, 0]);
    tlv(&mut p, 0xb9, &[0; 8]);
    for _ in 0..4 {
        tlv(&mut p, 0x7c, b"02:53:59:4e:4f:01");
    }
    p
}

fn ascii(d: &[u8]) -> String {
    String::from_utf8_lossy(d).chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn le32(d: &[u8]) -> Option<u32> {
    (d.len() >= 4).then(|| u32::from_le_bytes([d[0], d[1], d[2], d[3]]))
}

/// Parse a reply. Returns `None` for packets without the magic prefix.
pub fn parse_response(data: &[u8], src: IpAddr) -> Option<Found> {
    if data.len() < 8 || data[..8] != MAGIC {
        return None;
    }
    let mut f = Found { sources: vec!["synology-udp".into()], synology: true, ..Default::default() };
    let mut build = None;
    let mut ver = String::new();
    let (mut i, mut ip_in_packet) = (8usize, None);
    while i + 2 <= data.len() {
        let (id, len) = (data[i], data[i + 1] as usize);
        i += 2;
        if i + len > data.len() {
            break;
        }
        let v = &data[i..i + len];
        i += len;
        match id {
            0x11 => f.hostname = ascii(v),
            0x19 => f.mac = normalize_mac(&ascii(v)),
            0x12 if len == 4 => ip_in_packet = Some(IpAddr::V4(Ipv4Addr::new(v[0], v[1], v[2], v[3]))),
            0x75 => f.http_port = le32(v).and_then(|p| u16::try_from(p).ok()),
            0x76 => f.https_port = le32(v).and_then(|p| u16::try_from(p).ok()),
            0x49 => build = le32(v),
            0x77 => ver = ascii(v),
            0x78 => f.model = ascii(v),
            0x70 => f.platform = ascii(v),
            0xc0 => f.serial = ascii(v),
            _ => {}
        }
    }
    f.dsm_version = match (ver.is_empty(), build) {
        (false, Some(b)) => format!("{ver}-{b}"),
        (false, None) => ver,
        (true, Some(b)) => format!("build {b}"),
        _ => String::new(),
    };
    f.http_port = f.http_port.or(Some(5000));
    f.https_port = f.https_port.or(Some(5001));
    f.ips.push(ip_in_packet.unwrap_or(src));
    if let IpAddr::V4(s) = src
        && !f.ips.contains(&IpAddr::V4(s))
    {
        f.ips.push(IpAddr::V4(s));
    }
    Some(f)
}

fn broadcast_targets() -> Vec<Ipv4Addr> {
    let mut v = vec![Ipv4Addr::BROADCAST];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for i in ifs {
            if let if_addrs::IfAddr::V4(a) = i.addr
                && !a.ip.is_loopback()
                && let Some(b) = a.broadcast
            {
                v.push(b);
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

pub fn scan(timeout: Duration, tx: &Sender<Found>) -> Result<usize> {
    // Replies come back to the source port; some units reply to 9999, so try to bind it but fall back.
    let sock = UdpSocket::bind("0.0.0.0:9999").or_else(|_| UdpSocket::bind("0.0.0.0:0"))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    let q = build_query();
    for t in broadcast_targets() {
        let _ = sock.send_to(&q, SocketAddr::new(IpAddr::V4(t), PORT));
    }
    let (start, mut n, mut buf) = (Instant::now(), 0, [0u8; 2048]);
    let mut resent = false;
    while start.elapsed() < timeout {
        if !resent && start.elapsed() > timeout / 2 {
            for t in broadcast_targets() {
                let _ = sock.send_to(&q, SocketAddr::new(IpAddr::V4(t), PORT));
            }
            resent = true;
        }
        if let Ok((len, src)) = sock.recv_from(&mut buf)
            && let Some(f) = parse_response(&buf[..len], src.ip())
        {
            n += 1;
            let _ = tx.send(f);
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply() -> Vec<u8> {
        let mut p = MAGIC.to_vec();
        tlv(&mut p, 0x11, b"diskstation");
        tlv(&mut p, 0x19, b"00:11:32:AA:BB:CC");
        tlv(&mut p, 0x12, &[192, 168, 1, 20]);
        tlv(&mut p, 0x75, &5000u32.to_le_bytes());
        tlv(&mut p, 0x76, &5001u32.to_le_bytes());
        tlv(&mut p, 0x49, &69057u32.to_le_bytes());
        tlv(&mut p, 0x77, b"7.2.1\0");
        tlv(&mut p, 0x78, b"DS918+");
        tlv(&mut p, 0x70, b"apollolake");
        tlv(&mut p, 0xc0, b"1920PDN123456");
        p
    }

    #[test]
    fn query_has_magic_and_valid_tlvs() {
        let q = build_query();
        assert_eq!(&q[..8], &MAGIC);
        let mut i = 8;
        while i < q.len() {
            i += 2 + q[i + 1] as usize;
        }
        assert_eq!(i, q.len(), "TLV lengths must tile the packet exactly");
        assert!(!String::from_utf8_lossy(&q).contains("00:50:56"), "must not impersonate VMware");
    }

    #[test]
    fn parses_reply() {
        let f = parse_response(&reply(), "192.168.1.20".parse().unwrap()).unwrap();
        assert_eq!(f.hostname, "diskstation");
        assert_eq!(f.mac.as_deref(), Some("00:11:32:aa:bb:cc"));
        assert_eq!(f.model, "DS918+");
        assert_eq!(f.platform, "apollolake");
        assert_eq!(f.serial, "1920PDN123456");
        assert_eq!(f.dsm_version, "7.2.1-69057");
        assert_eq!((f.http_port, f.https_port), (Some(5000), Some(5001)));
        assert_eq!(f.ips, vec!["192.168.1.20".parse::<IpAddr>().unwrap()]);
    }

    #[test]
    fn rejects_garbage_and_truncated_packets() {
        assert!(parse_response(b"hello world!", "1.2.3.4".parse().unwrap()).is_none());
        let mut r = reply();
        r.truncate(r.len() - 5);
        // truncated final record: still returns what was parsed before it
        let f = parse_response(&r, "1.2.3.4".parse().unwrap()).unwrap();
        assert_eq!(f.model, "DS918+");
        assert!(parse_response(&MAGIC[..4], "1.2.3.4".parse().unwrap()).is_none());
    }
}
