//! SSDP/UPnP: DSM answers `M-SEARCH`; the device description XML names manufacturer and model.

use super::Found;
use anyhow::Result;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const MSEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: ssdp:all\r\n\r\n";

/// Header value (case-insensitive name) from an SSDP/HTTP response.
pub fn header<'a>(resp: &'a str, name: &str) -> Option<&'a str> {
    resp.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// Text of the first `<tag>..</tag>`.
pub fn xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = xml.find(&open)? + open.len();
    let e = xml[s..].find(&close)? + s;
    Some(xml[s..e].trim())
}

/// Build a `Found` from a UPnP device description, only if the manufacturer is Synology.
pub fn from_description(xml: &str, ip: IpAddr) -> Option<Found> {
    let maker = xml_tag(xml, "manufacturer")?;
    if !maker.to_lowercase().contains("synology") {
        return None;
    }
    Some(Found {
        ips: vec![ip],
        hostname: xml_tag(xml, "friendlyName").unwrap_or("").to_string(),
        model: xml_tag(xml, "modelName").unwrap_or("").to_string(),
        serial: xml_tag(xml, "serialNumber").unwrap_or("").to_string(),
        synology: true,
        sources: vec!["ssdp".into()],
        ..Default::default()
    })
}

pub fn scan(timeout: Duration, tx: &Sender<Found>) -> Result<usize> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    let dst: SocketAddr = "239.255.255.250:1900".parse()?;
    sock.send_to(MSEARCH.as_bytes(), dst)?;
    let (start, mut buf) = (Instant::now(), [0u8; 4096]);
    let mut seen = std::collections::HashSet::new();
    let mut n = 0;
    while start.elapsed() < timeout {
        let Ok((len, src)) = sock.recv_from(&mut buf) else { continue };
        let resp = String::from_utf8_lossy(&buf[..len]).to_string();
        let Some(loc) = header(&resp, "location") else { continue };
        if !seen.insert(loc.to_string()) || !loc.starts_with("http://") {
            continue;
        }
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).build();
        if let Ok(r) = agent.get(loc).call()
            && let Ok(xml) = r.into_string()
            && let Some(f) = from_description(&xml, src.ip())
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

    #[test]
    fn parses_description() {
        let xml = "<root><device><friendlyName>DiskStation</friendlyName><manufacturer>Synology</manufacturer><modelName>DS220+</modelName><serialNumber>2010SN</serialNumber></device></root>";
        let f = from_description(xml, "10.0.0.5".parse().unwrap()).unwrap();
        assert_eq!((f.model.as_str(), f.serial.as_str(), f.hostname.as_str()), ("DS220+", "2010SN", "DiskStation"));
        assert!(from_description("<manufacturer>Netgear</manufacturer>", "10.0.0.6".parse().unwrap()).is_none());
    }

    #[test]
    fn headers_are_case_insensitive() {
        let r = "HTTP/1.1 200 OK\r\nLocation: http://10.0.0.5:50001/desc.xml\r\nST: upnp:rootdevice\r\n\r\n";
        assert_eq!(header(r, "LOCATION"), Some("http://10.0.0.5:50001/desc.xml"));
        assert_eq!(header(r, "nope"), None);
    }
}
