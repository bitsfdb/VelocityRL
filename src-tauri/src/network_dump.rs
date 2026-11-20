/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 *
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

use base64::Engine;
use serde::Serialize;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tauri::Manager;

#[derive(Serialize)]
struct DumpPacket {
    timestamp: String,
    direction: String,
    endpoint: String,
    encrypted: bool,
    decrypted: bool,
    payload_b64: String,
    /// Exact bytes that crossed the wire before decryption (base64).
    raw_wire_b64: String,
}

/// Write the current monitor events and the buffered UDP datagrams to a folder
/// containing JSON logs plus a Wireshark-compatible pcap.
pub fn dump_network_capture(app: &tauri::AppHandle) -> Result<String, String> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let base_dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| format!("Could not resolve an output directory: {e}"))?;
    let out_dir = base_dir.join(format!("velocityrl-network-{stamp}"));
    fs::create_dir_all(&out_dir).map_err(|e| format!("Could not create dump folder: {e}"))?;

    let events = crate::proxy::get_network_events();
    let events_json = serde_json::to_vec_pretty(&events)
        .map_err(|e| format!("Could not serialize network events: {e}"))?;
    fs::write(out_dir.join("network-events.json"), events_json)
        .map_err(|e| format!("Could not write network-events.json: {e}"))?;

    let packets = crate::udp_proxy::get_udp_capture();
    let dump_packets: Vec<DumpPacket> = packets
        .iter()
        .map(|packet| DumpPacket {
            timestamp: packet.timestamp.clone(),
            direction: packet.direction.clone(),
            endpoint: packet.endpoint.clone(),
            encrypted: packet.encrypted,
            decrypted: packet.decrypted,
            payload_b64: base64::engine::general_purpose::STANDARD.encode(&packet.payload),
            raw_wire_b64: base64::engine::general_purpose::STANDARD.encode(&packet.raw_wire),
        })
        .collect();
    let packets_json = serde_json::to_vec_pretty(&dump_packets)
        .map_err(|e| format!("Could not serialize UDP packets: {e}"))?;
    fs::write(out_dir.join("udp-packets.json"), packets_json)
        .map_err(|e| format!("Could not write udp-packets.json: {e}"))?;

    fs::write(out_dir.join("udp-capture.pcap"), build_pcap(&packets))
        .map_err(|e| format!("Could not write udp-capture.pcap: {e}"))?;

    let readme = format!(
        "VelocityRL network dump\n\
         Created: {}\n\
         Monitor events: {}\n\
         UDP datagrams: {}\n\n\
         network-events.json\n\
         \x20 Monitor rows with their captured payload text.\n\n\
         udp-packets.json\n\
         \x20 One object per datagram. payload_b64 is decrypted plaintext when\n\
         \x20 decrypted=true, otherwise the raw wire bytes.\n\n\
         udp-capture.pcap\n\
         \x20 Wireshark-compatible capture with synthetic Ethernet/IPv4/UDP\n\
         \x20 headers. The UDP payload is the decrypted data when available.\n",
        chrono::Utc::now().to_rfc3339(),
        events.len(),
        packets.len()
    );
    fs::write(out_dir.join("README.txt"), readme)
        .map_err(|e| format!("Could not write README.txt: {e}"))?;

    Ok(out_dir.to_string_lossy().to_string())
}

fn build_pcap(packets: &[crate::udp_proxy::UdpCaptureEntry]) -> Vec<u8> {
    let mut out = Vec::new();

    // pcap global header: little-endian, Ethernet link type.
    out.extend_from_slice(&0xA1B2C3D4u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&65_535u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());

    let local_port = crate::udp_proxy::get_local_port().unwrap_or(7777);

    for (index, packet) in packets.iter().enumerate() {
        let (src, dst) = packet_endpoints(packet, local_port, index);
        let stamp = packet_timestamp(&packet.timestamp, index);
        let frame = build_udp_frame(src, dst, index as u16, &packet.payload);

        out.extend_from_slice(&stamp.0.to_le_bytes());
        out.extend_from_slice(&stamp.1.to_le_bytes());
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(&frame);
    }

    out
}

fn packet_endpoints(
    packet: &crate::udp_proxy::UdpCaptureEntry,
    local_port: u16,
    index: usize,
) -> (SocketAddr, SocketAddr) {
    let fallback = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)),
        7000 + (index % 1000) as u16,
    );
    let remote = packet
        .endpoint
        .parse::<SocketAddr>()
        .map(to_ipv4)
        .unwrap_or(fallback);
    let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), local_port);

    if packet.direction == "C2S" {
        (local, remote)
    } else {
        (remote, local)
    }
}

fn to_ipv4(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(_) => addr,
        SocketAddr::V6(v6) => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), v6.port())
        }
    }
}

fn packet_timestamp(timestamp: &str, index: usize) -> (u32, u32) {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        let secs = parsed.timestamp().max(0) as u32;
        return (secs, parsed.timestamp_subsec_micros());
    }

    // Keep record timestamps monotonic if a packet lacks a parseable timestamp.
    let now = chrono::Utc::now().timestamp().max(0) as u32;
    (now, (index % 1_000_000) as u32)
}

fn build_udp_frame(
    src: SocketAddr,
    dst: SocketAddr,
    ip_id: u16,
    payload: &[u8],
) -> Vec<u8> {
    const CLIENT_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    const SERVER_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];

    let src_mac = if src.ip().is_loopback() {
        CLIENT_MAC
    } else {
        SERVER_MAC
    };
    let dst_mac = if dst.ip().is_loopback() {
        CLIENT_MAC
    } else {
        SERVER_MAC
    };

    let src_ip = match src.ip() {
        IpAddr::V4(ip) => ip.octets(),
        IpAddr::V6(_) => [192, 0, 2, 1],
    };
    let dst_ip = match dst.ip() {
        IpAddr::V4(ip) => ip.octets(),
        IpAddr::V6(_) => [192, 0, 2, 2],
    };

    let udp_len = (8 + payload.len()) as u16;
    let total_len = 20 + udp_len as usize;

    let mut frame = Vec::with_capacity(14 + total_len);
    frame.extend_from_slice(&dst_mac);
    frame.extend_from_slice(&src_mac);
    frame.extend_from_slice(&0x0800u16.to_be_bytes());

    let mut ip = [0u8; 20];
    ip[0] = 0x45;
    ip[1] = 0;
    ip[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&ip_id.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 17; // UDP
    ip[12..16].copy_from_slice(&src_ip);
    ip[16..20].copy_from_slice(&dst_ip);
    let checksum = ipv4_checksum(&ip);
    ip[10..12].copy_from_slice(&checksum.to_be_bytes());
    frame.extend_from_slice(&ip);

    frame.extend_from_slice(&src.port().to_be_bytes());
    frame.extend_from_slice(&dst.port().to_be_bytes());
    frame.extend_from_slice(&udp_len.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes()); // checksum optional for IPv4
    frame.extend_from_slice(payload);

    frame
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < header.len() {
        sum += u16::from_be_bytes([header[i], header[i + 1]]) as u32;
        i += 2;
    }
    if i < header.len() {
        sum += (header[i] as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp_proxy::UdpCaptureEntry;

    fn entry(direction: &str, endpoint: &str, payload: &[u8]) -> UdpCaptureEntry {
        UdpCaptureEntry {
            timestamp: "2026-11-18T12:00:00Z".into(),
            direction: direction.into(),
            endpoint: endpoint.into(),
            encrypted: true,
            decrypted: true,
            payload: payload.to_vec(),
            raw_wire: payload.to_vec(),
        }
    }

    #[test]
    fn test_pcap_has_header_and_records() {
        let packets = vec![
            entry("C2S", "203.0.113.10:7000", b"hello"),
            entry("S2C", "203.0.113.10:7000", b"world"),
        ];
        let pcap = build_pcap(&packets);
        assert_eq!(&pcap[..4], &0xA1B2C3D4u32.to_le_bytes());
        assert_eq!(&pcap[20..24], &1u32.to_le_bytes());

        let mut offset = 24;
        for expected in [b"hello".as_slice(), b"world".as_slice()] {
            let incl_len = u32::from_le_bytes(pcap[offset + 8..offset + 12].try_into().unwrap()) as usize;
            assert_eq!(incl_len, 14 + 20 + 8 + expected.len());
            let frame = &pcap[offset + 16..offset + 16 + incl_len];
            assert_eq!(&frame[14 + 20 + 8..], expected);
            offset += 16 + incl_len;
        }
    }

    #[test]
    fn test_ipv4_checksum_is_valid() {
        let mut header = [0u8; 20];
        header[0] = 0x45;
        header[2..4].copy_from_slice(&40u16.to_be_bytes());
        header[8] = 64;
        header[9] = 17;
        let checksum = ipv4_checksum(&header);
        header[10..12].copy_from_slice(&checksum.to_be_bytes());
        assert_eq!(ipv4_checksum(&header), 0);
    }
}
