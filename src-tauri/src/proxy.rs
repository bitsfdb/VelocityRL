/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

use base64::Engine;
use hmac::{Hmac, Mac};
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha2::Sha256;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkEventEntry {
    pub id: u64,
    pub timestamp: String,
    pub protocol: String, // "HTTP", "WS_C2S", "WS_S2C", "UDP", "CONFIG_CDN", "UPK"
    pub direction: String, // "INBOUND", "OUTBOUND", "LOOPBACK"
    pub service_or_url: String,
    pub status_or_type: String,
    pub summary: String,
    pub payload_snippet: String,
    pub payload_len: usize,
    pub intercepted: bool,
}

static NETWORK_EVENT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static NETWORK_EVENT_RING: std::sync::Mutex<Option<std::collections::VecDeque<NetworkEventEntry>>> =
    std::sync::Mutex::new(None);

pub fn record_network_event(
    protocol: &str,
    direction: &str,
    service_or_url: &str,
    status_or_type: &str,
    summary: &str,
    payload: &str,
    intercepted: bool,
) {
    let id = NETWORK_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = chrono::Utc::now().to_rfc3339();
    let payload_len = payload.len();
    let entry = NetworkEventEntry {
        id,
        timestamp: now,
        protocol: protocol.to_string(),
        direction: direction.to_string(),
        service_or_url: service_or_url.to_string(),
        status_or_type: status_or_type.to_string(),
        summary: summary.to_string(),
        payload_snippet: payload.to_string(), // Store complete full payload without truncation
        payload_len,
        intercepted,
    };
    if let Ok(mut lock) = NETWORK_EVENT_RING.lock() {
        let deque = lock.get_or_insert_with(|| std::collections::VecDeque::with_capacity(512));
        if deque.len() >= 500 {
            deque.pop_front();
        }
        deque.push_back(entry);
    }
}

pub fn get_network_events() -> Vec<NetworkEventEntry> {
    if let Ok(lock) = NETWORK_EVENT_RING.lock() {
        if let Some(deque) = lock.as_ref() {
            return deque.iter().cloned().collect();
        }
    }
    Vec::new()
}

// Keep summary creation separate so payload size is bounded before IPC serialization.
fn network_event_summary(event: &NetworkEventEntry) -> NetworkEventEntry {
    NetworkEventEntry {
        id: event.id, timestamp: event.timestamp.clone(), protocol: event.protocol.clone(),
        direction: event.direction.clone(), service_or_url: event.service_or_url.clone(),
        status_or_type: event.status_or_type.clone(), summary: event.summary.clone(),
        payload_len: event.payload_len, intercepted: event.intercepted,
        payload_snippet: event.payload_snippet.chars().take(512).collect(),
    }
}

pub fn get_network_event_summaries() -> Vec<NetworkEventEntry> {
    if let Ok(lock) = NETWORK_EVENT_RING.lock() {
        if let Some(events) = lock.as_ref() {
            return events.iter().map(network_event_summary).collect();
        }
    }
    Vec::new()
}

pub fn get_network_event(id: u64) -> Option<NetworkEventEntry> {
    NETWORK_EVENT_RING.lock().ok()?.as_ref()?.iter().find(|event| event.id == id).cloned()
}

pub fn clear_network_events() {
    if let Ok(mut lock) = NETWORK_EVENT_RING.lock() {
        if let Some(deque) = lock.as_mut() {
            deque.clear();
        }
    }
}

const LEAF_CONFIG_CERT_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_config.psynet.gg.crt");
const LEAF_CONFIG_KEY_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_config.psynet.gg.key");
const LEAF_WS_CERT_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_ws.rlpp.psynet.gg.crt");
const LEAF_WS_KEY_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_ws.rlpp.psynet.gg.key");
const LEAF_EPIC_CERT_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_epic.crt");
const LEAF_EPIC_KEY_PEM: &[u8] =
    include_bytes!("../resources/certs/leaf_epic.key");
const CA_CERT_PEM: &[u8] = include_bytes!("../resources/certs/velocityrl_ca.crt");
const CA_CRL_DER: &[u8] = include_bytes!("../resources/certs/velocityrl.crl");

pub fn leaf_epic_cert_bytes() -> &'static [u8] {
    LEAF_EPIC_CERT_PEM
}

pub const SYSTEM_PROXY_PORT: u16 = 8080;

/// EOS account/profile API hosts. On Linux these are hosts-redirected to the
/// local :443 MITM (Wine WinINET proxies are ignored by Proton/EOS).
pub const EOS_ACCOUNT_HOSTS: &[&str] = &["api.epicgames.dev"];

pub fn hostname_only(host: &str) -> &str {
    host.split(':').next().unwrap_or(host).trim()
}

pub fn host_header_port(host_hdr: &str, default: u16) -> u16 {
    if let Some(p) = host_hdr.rsplit_once(':').map(|(_, p)| p.trim()) {
        if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
            return p.parse().unwrap_or(default);
        }
    }
    default
}

pub fn is_eos_account_host(host: &str) -> bool {
    let h = hostname_only(host).to_ascii_lowercase();
    h == "api.epicgames.dev" || h.ends_with(".epicgames.dev")
}

pub fn is_intercept_target(host: &str) -> bool {
    let h = hostname_only(host).to_ascii_lowercase();
    let is_epic = h.contains("epicgames.dev") || h.contains("epicgames.com");
    if is_epic {
        let spoof_cfg = crate::psynet::load_active_spoof_from_disk();
        let name_spoof_on = spoof_cfg
            .as_ref()
            .and_then(|c| c.name_spoof.as_ref())
            .map(|n| n.enabled)
            .unwrap_or(false);
        if !name_spoof_on {
            return false;
        }
    }
    h.contains("epicgames.dev")
        || h.contains("psyonix.com")
        || h.contains("live.psynet.gg")
        || h.contains("rlpp.psynet.gg")
        || h.contains("psynet.gg")
}

static RESOLVED_IPS_CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, std::net::SocketAddr>>> = std::sync::Mutex::new(None);

/// Resolve an A record via public DNS, bypassing /etc/hosts (so MITM loopback
/// entries cannot poison our upstream client).
pub fn resolve_ipv4_public(host: &str) -> Option<std::net::SocketAddr> {
    let host = hostname_only(host);
    if host.is_empty() || host == "localhost" {
        return None;
    }
    if let Ok(guard) = RESOLVED_IPS_CACHE.lock() {
        if let Some(map) = guard.as_ref() {
            if let Some(addr) = map.get(host) {
                return Some(*addr);
            }
        }
    }

    // 1. Attempt system DNS resolution first (if not loopback, e.g. host is not redirected in hosts file)
    use std::net::ToSocketAddrs;
    if let Ok(addrs) = format!("{host}:443").to_socket_addrs() {
        for addr in addrs {
            if let std::net::SocketAddr::V4(v4) = addr {
                if !v4.ip().is_loopback() && !v4.ip().is_unspecified() {
                    crate::applog::event(&format!("proxy: system DNS {host} -> {}", v4.ip()));
                    let out = std::net::SocketAddr::from((*v4.ip(), 443));
                    if let Ok(mut guard) = RESOLVED_IPS_CACHE.lock() {
                        guard.get_or_insert_with(std::collections::HashMap::new).insert(host.to_string(), out);
                    }
                    return Some(out);
                }
            }
        }
    }

    // 2. Query public DNS resolvers via UDP
    for server in ["1.1.1.1:53", "8.8.8.8:53", "9.9.9.9:53", "208.67.222.222:53"] {
        if let Some(ip) = dns_query_a(host, server) {
            crate::applog::event(&format!("proxy: public DNS UDP {host} -> {ip} via {server}"));
            let addr = std::net::SocketAddr::from((ip, 443));
            if let Ok(mut guard) = RESOLVED_IPS_CACHE.lock() {
                guard.get_or_insert_with(std::collections::HashMap::new).insert(host.to_string(), addr);
            }
            return Some(addr);
        }
    }

    // 3. Query public DNS resolvers via TCP (RFC 1035 fallback when UDP 53 is blocked by VPN/firewall)
    for server in ["1.1.1.1:53", "8.8.8.8:53", "9.9.9.9:53"] {
        if let Some(ip) = dns_query_a_tcp(host, server) {
            crate::applog::event(&format!("proxy: public DNS TCP {host} -> {ip} via {server}"));
            let addr = std::net::SocketAddr::from((ip, 443));
            if let Ok(mut guard) = RESOLVED_IPS_CACHE.lock() {
                guard.get_or_insert_with(std::collections::HashMap::new).insert(host.to_string(), addr);
            }
            return Some(addr);
        }
    }

    crate::applog::event(&format!("proxy: public DNS lookup failed for {host}"));
    None
}

fn dns_query_a_tcp(host: &str, server: &str) -> Option<std::net::Ipv4Addr> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let mut q = Vec::with_capacity(16 + host.len());
    q.extend_from_slice(&[0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

    let server_addr: std::net::SocketAddr = server.parse().ok()?;
    let mut sock = TcpStream::connect_timeout(&server_addr, Duration::from_millis(1500)).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
    sock.set_write_timeout(Some(Duration::from_millis(1500))).ok()?;

    let len = (q.len() as u16).to_be_bytes();
    sock.write_all(&len).ok()?;
    sock.write_all(&q).ok()?;

    let mut len_buf = [0u8; 2];
    sock.read_exact(&mut len_buf).ok()?;
    let resp_len = u16::from_be_bytes(len_buf) as usize;
    if resp_len < 12 || resp_len > 4096 {
        return None;
    }
    let mut buf = vec![0u8; resp_len];
    sock.read_exact(&mut buf).ok()?;
    parse_dns_a_answer(&buf)
}

fn dns_query_a(host: &str, server: &str) -> Option<std::net::Ipv4Addr> {
    use std::net::UdpSocket;
    use std::time::Duration;

    let mut q = Vec::with_capacity(16 + host.len());
    q.extend_from_slice(&[0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(900))).ok()?;
    sock.send_to(&q, server).ok()?;
    let mut buf = [0u8; 512];
    let (n, _) = sock.recv_from(&mut buf).ok()?;
    parse_dns_a_answer(&buf[..n])
}

fn skip_dns_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        if pos >= buf.len() {
            return None;
        }
        let len = buf[pos];
        if len == 0 {
            return Some(pos + 1);
        }
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= buf.len() {
                return None;
            }
            return Some(pos + 2);
        }
        pos = pos.checked_add(1 + len as usize)?;
    }
}

fn parse_dns_a_answer(buf: &[u8]) -> Option<std::net::Ipv4Addr> {
    if buf.len() < 12 {
        return None;
    }
    let ancount = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    if ancount == 0 {
        return None;
    }
    let mut i = skip_dns_name(buf, 12)?;
    i = i.checked_add(4)?;
    for _ in 0..ancount {
        i = skip_dns_name(buf, i)?;
        if i.checked_add(10)? > buf.len() {
            return None;
        }
        let typ = u16::from_be_bytes([buf[i], buf[i + 1]]);
        let class = u16::from_be_bytes([buf[i + 2], buf[i + 3]]);
        let rdlen = u16::from_be_bytes([buf[i + 8], buf[i + 9]]) as usize;
        i += 10;
        if i.checked_add(rdlen)? > buf.len() {
            return None;
        }
        if typ == 1 && class == 1 && rdlen == 4 {
            return Some(std::net::Ipv4Addr::new(buf[i], buf[i + 1], buf[i + 2], buf[i + 3]));
        }
        i += rdlen;
    }
    None
}

fn pin_host_on_builder(mut builder: reqwest::ClientBuilder, host: &str) -> reqwest::ClientBuilder {
    if let Some(addr) = resolve_ipv4_public(host) {
        builder = builder.resolve(host, addr);
    }
    builder
}

fn pin_eos_hosts_on_builder(mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    for host in EOS_ACCOUNT_HOSTS {
        builder = pin_host_on_builder(builder, host);
    }
    builder
}

fn http_client_pinned_for(host: &str) -> reqwest::Client {
    pin_host_on_builder(
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none()),
        host,
    )
    .build()
    .unwrap_or_default()
}

fn build_intercept_http_client() -> reqwest::Client {
    pin_eos_hosts_on_builder(
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none()),
    )
    .build()
    .unwrap_or_default()
}

static LEARNED_PLAYER_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static LEARNED_REAL_NAME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn get_learned_player_id() -> Option<String> {
    let mut lock = LEARNED_PLAYER_ID.lock().unwrap();
    if lock.is_none() {
        if let Some(cfg) = crate::psynet::load_active_spoof_from_disk() {
            if let Some(ns) = cfg.name_spoof {
                if let Some(pid) = ns.player_id {
                    let clean = pid.trim();
                    if !clean.is_empty() && !clean.contains("|temp|") {
                        *lock = Some(clean.to_string());
                    }
                }
            }
        }
        if lock.is_none() {
            let dir = crate::psynet::config_dir();
            let last_file = dir.join("last_player_id.txt");
            if let Ok(content) = std::fs::read_to_string(&last_file) {
                let clean = content.trim();
                if !clean.is_empty() && !clean.contains("|temp|") {
                    *lock = Some(clean.to_string());
                }
            }
        }
    }
    lock.clone()
}

pub fn get_learned_real_name() -> Option<String> {
    let mut lock = LEARNED_REAL_NAME.lock().unwrap();
    if lock.is_none() {
        if let Some(cfg) = crate::psynet::load_active_spoof_from_disk() {
            if let Some(ns) = cfg.name_spoof {
                if let Some(rn) = ns.real_name {
                    let clean = rn.trim();
                    if !clean.is_empty() {
                        *lock = Some(clean.to_string());
                    }
                }
            }
        }
        if lock.is_none() {
            let dir = crate::psynet::config_dir();
            let last_file = dir.join("last_real_name.txt");
            if let Ok(content) = std::fs::read_to_string(&last_file) {
                let clean = content.trim();
                if !clean.is_empty() {
                    *lock = Some(clean.to_string());
                }
            }
        }
    }
    lock.clone()
}

pub fn set_learned_player_id(id: &str) {
    let clean = id.trim();
    if clean.is_empty() || clean.to_ascii_lowercase().contains("|temp|") {
        return;
    }
    let mut lock = LEARNED_PLAYER_ID.lock().unwrap();
    if lock.as_deref() != Some(clean) {
        crate::applog::event(&format!("proxy: learned own PlayerID: '{clean}'"));
        *lock = Some(clean.to_string());
        persist_learned_identity_to_disk(Some(clean), None);
        if let Ok(mut spoof_lock) = SPOOF_CONFIG.try_write() {
            if let Some(cfg) = spoof_lock.as_mut() {
                if let Some(ns) = cfg.name_spoof.as_mut() {
                    ns.player_id = Some(clean.to_string());
                }
            }
        }
    }
}

pub fn set_learned_real_name(name: &str) {
    let clean = name.trim();
    if clean.is_empty() {
        return;
    }
    let mut lock = LEARNED_REAL_NAME.lock().unwrap();
    if lock.as_deref() != Some(clean) {
        crate::applog::event(&format!("proxy: learned real displayName: '{clean}'"));
        *lock = Some(clean.to_string());
        persist_learned_identity_to_disk(None, Some(clean));
    }
}

pub fn persist_learned_identity_to_disk(learned_id: Option<&str>, learned_name: Option<&str>) {
    let dir = crate::psynet::config_dir();
    let _ = std::fs::create_dir_all(&dir);

    if let Some(id) = learned_id {
        let clean = id.trim();
        if !clean.is_empty() && !clean.contains("|temp|") {
            let _ = std::fs::write(dir.join("last_player_id.txt"), clean);
        }
    }
    if let Some(name) = learned_name {
        let clean = name.trim();
        if !clean.is_empty() {
            let _ = std::fs::write(dir.join("last_real_name.txt"), clean);
        }
    }

    let path = dir.join("psynet_config.json");
    let mut v = if path.is_file() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = v.as_object_mut() {
        let mut changed = false;
        let ns_entry = obj.entry("name_spoof").or_insert_with(|| serde_json::json!({}));
        if let Some(ns_map) = ns_entry.as_object_mut() {
            if let Some(id) = learned_id {
                let clean = id.trim();
                if !clean.is_empty() && !clean.contains("|temp|") {
                    if ns_map.get("player_id").and_then(|v| v.as_str()) != Some(clean) {
                        ns_map.insert("player_id".into(), serde_json::json!(clean));
                        changed = true;
                    }
                }
            }
            if let Some(name) = learned_name {
                let clean = name.trim();
                if !clean.is_empty() {
                    if ns_map.get("real_name").and_then(|v| v.as_str()) != Some(clean) {
                        ns_map.insert("real_name".into(), serde_json::json!(clean));
                        changed = true;
                    }
                }
            }
        }

        if changed {
            if let Ok(formatted) = serde_json::to_string_pretty(&v) {
                let _ = std::fs::write(&path, formatted);
                crate::applog::event(&format!("psynet: persisted learned identity to {}", path.display()));
            }
        }
    }
}

pub fn normalize_player_id(id: &str) -> String {
    let lower = id.trim().to_ascii_lowercase();
    let s = if let Some(first_bar) = lower.find('|') {
        let after_first = &lower[first_bar + 1..];
        if let Some(second_bar) = after_first.find('|') {
            &after_first[..second_bar]
        } else {
            after_first
        }
    } else {
        lower.as_str()
    };
    s.trim().to_string()
}

pub fn patch_eos_accounts_json(
    val: &mut serde_json::Value,
    new_name: &str,
    target_pid: Option<&str>,
    filter_real_name: Option<&str>,
) -> (bool, Option<String>, Option<String>) {
    let mut modified = false;
    let mut learned_pid = None;
    let mut learned_name = None;

    let clean_target = target_pid
        .map(normalize_player_id)
        .filter(|s| !s.is_empty() && s != "temp");

    if let serde_json::Value::Array(arr) = val {
        let single_item = arr.len() == 1;
        for user_data in arr.iter_mut().filter_map(|v| v.as_object_mut()) {
            let acc: Option<String> = user_data
                .get("accountId")
                .or_else(|| user_data.get("account_id"))
                .or_else(|| user_data.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let current_disp: Option<String> = user_data
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut is_match = false;
            if let Some(ref target) = clean_target {
                if let Some(ref a) = acc {
                    if normalize_player_id(a) == *target {
                        is_match = true;
                    }
                }
            } else if single_item {
                is_match = true;
                if let Some(ref a) = acc {
                    learned_pid = Some(a.clone());
                }
            }
            if !is_match {
                if let Some(real) = filter_real_name {
                    if let Some(ref disp) = current_disp {
                        if disp.eq_ignore_ascii_case(real) {
                            is_match = true;
                        }
                    }
                }
            }

            if is_match {
                if let Some(old_name) = current_disp {
                    if old_name != new_name {
                        learned_name = Some(old_name);
                    }
                }
                user_data.insert("displayName".to_string(), serde_json::json!(new_name));
                user_data.insert("sanitizedDisplayName".to_string(), serde_json::json!(new_name));
                user_data.insert("SanitizedDisplayName".to_string(), serde_json::json!(new_name));
                user_data.insert("DisplayNameSanitized".to_string(), serde_json::json!(new_name));
                user_data.insert("sanitized_display_name".to_string(), serde_json::json!(new_name));
                if let Some(c) = user_data.get_mut("cabined") {
                    *c = serde_json::Value::Bool(false);
                }
                modified = true;
                if let Some(ref a) = acc {
                    learned_pid = Some(a.clone());
                }
            }
        }
    } else if let serde_json::Value::Object(user_data) = val {
        let acc: Option<String> = user_data
            .get("accountId")
            .or_else(|| user_data.get("account_id"))
            .or_else(|| user_data.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let current_disp: Option<String> = user_data
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut is_match = false;
        if let Some(ref target) = clean_target {
            if let Some(ref a) = acc {
                if normalize_player_id(a) == *target {
                    is_match = true;
                }
            }
        } else {
            is_match = true;
            if let Some(ref a) = acc {
                learned_pid = Some(a.clone());
            }
        }
        if !is_match {
            if let Some(real) = filter_real_name {
                if let Some(ref disp) = current_disp {
                    if disp.eq_ignore_ascii_case(real) {
                        is_match = true;
                    }
                }
            }
        }

        if is_match {
            if let Some(old_name) = current_disp {
                if old_name != new_name {
                    learned_name = Some(old_name);
                }
            }
            user_data.insert("displayName".to_string(), serde_json::json!(new_name));
            user_data.insert("sanitizedDisplayName".to_string(), serde_json::json!(new_name));
            user_data.insert("SanitizedDisplayName".to_string(), serde_json::json!(new_name));
            user_data.insert("DisplayNameSanitized".to_string(), serde_json::json!(new_name));
            user_data.insert("sanitized_display_name".to_string(), serde_json::json!(new_name));
            if let Some(c) = user_data.get_mut("cabined") {
                *c = serde_json::Value::Bool(false);
            }
            modified = true;
            if let Some(ref a) = acc {
                learned_pid = Some(a.clone());
            }
        }
    }

    (modified, learned_pid, learned_name)
}

fn patch_ws_name_fields(
    body: &[u8],
    new_name: &str,
    target_pid: Option<&str>,
) -> (Vec<u8>, bool) {
    if new_name.trim().is_empty() {
        return (body.to_vec(), false);
    }
    let Ok(mut root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (body.to_vec(), false);
    };

    let clean_pid = target_pid
        .map(normalize_player_id)
        .filter(|s| !s.is_empty() && s != "temp")
        .or_else(|| get_learned_player_id().map(|s| normalize_player_id(&s)));

    let learned_real_name = get_learned_real_name();

    let mut changed = false;
    patch_ws_names_value(
        &mut root,
        new_name.trim(),
        clean_pid.as_deref(),
        learned_real_name.as_deref(),
        &mut changed,
    );

    if !changed {
        return (body.to_vec(), false);
    }

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn patch_ws_names_value(
    val: &mut serde_json::Value,
    new_name: &str,
    clean_pid: Option<&str>,
    clean_real: Option<&str>,
    changed: &mut bool,
) {
    const NAME_KEYS: &[&str] = &[
        "VerifiedPlayerName",
        "PlayerName",
        "DisplayName",
        "displayName",
        "epicDisplayName",
        "PlayerNickName",
        "NickName",
        "username",
        "UserName",
        "TargetName",
        "AccountName",
        "PersonaName",
        "sanitizedDisplayName",
        "sanitized_display_name",
        "SanitizedDisplayName",
    ];

    match val {
        serde_json::Value::Object(map) => {
            let mut is_own_obj = false;
            let mut has_other_id = false;

            for id_key in &[
                "PlayerID",
                "UserID",
                "FromUserID",
                "ForUserID",
                "FromEpicUserID",
                "PlayerId",
                "AccountID",
                "AccountId",
                "EpicAccountId",
                "id",
                "Id",
                "ID",
            ] {
                if let Some(id_val) = map.get(*id_key) {
                    if let Some(s) = id_val.as_str() {
                        let norm = normalize_player_id(s);
                        if let Some(target) = clean_pid {
                            if norm == target {
                                is_own_obj = true;
                                break;
                            } else if !norm.is_empty() && norm != "0" {
                                has_other_id = true;
                            }
                        }
                    } else if let Some(sub_obj) = id_val.as_object() {
                        for sub_k in &["ID", "Id", "id", "PlayerID", "AccountId"] {
                            if let Some(s) = sub_obj.get(*sub_k).and_then(|v| v.as_str()) {
                                let norm = normalize_player_id(s);
                                if let Some(target) = clean_pid {
                                    if norm == target {
                                        is_own_obj = true;
                                        break;
                                    } else if !norm.is_empty() && norm != "0" {
                                        has_other_id = true;
                                    }
                                }
                            }
                        }
                        if is_own_obj {
                            break;
                        }
                    }
                }
            }

            if !is_own_obj && !has_other_id && clean_pid.is_none() {
                is_own_obj = true;
            }

            // If this object is explicitly marked as primary/local player or non-remote
            if !is_own_obj && !has_other_id {
                let is_primary = map.get("bPrimaryPlayer").and_then(|v| v.as_bool()).unwrap_or(false);
                let is_remote = map.get("bRemotePlayer").and_then(|v| v.as_bool()).unwrap_or(true);
                if is_primary || !is_remote {
                    is_own_obj = true;
                }
            }

            for (k, v) in map.iter_mut() {
                let is_name_key = NAME_KEYS.iter().any(|nk| nk.eq_ignore_ascii_case(k));
                if is_name_key {
                    let mut should_patch = is_own_obj;
                    if let Some(real) = clean_real {
                        if let Some(s) = v.as_str() {
                            if s.eq_ignore_ascii_case(real) {
                                should_patch = true;
                            }
                        }
                    }
                    if should_patch {
                        if let Some(s) = v.as_str() {
                            if s != new_name && !s.trim().is_empty() {
                                *v = serde_json::json!(new_name);
                                *changed = true;
                            }
                        }
                    }
                } else {
                    if let Some(s) = v.as_str() {
                        let trimmed = s.trim();
                        if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                        {
                            if let Ok(mut inner_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                let mut inner_changed = false;
                                patch_ws_names_value(&mut inner_val, new_name, clean_pid, clean_real, &mut inner_changed);
                                if inner_changed {
                                    if let Ok(serialized) = serde_json::to_string(&inner_val) {
                                        *v = serde_json::json!(serialized);
                                        *changed = true;
                                    }
                                }
                            }
                        }
                    }
                    patch_ws_names_value(v, new_name, clean_pid, clean_real, changed);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                patch_ws_names_value(v, new_name, clean_pid, clean_real, changed);
            }
        }
        _ => {}
    }
}

fn is_loadout_sensitive(svc: &str, body: &[u8]) -> bool {
    let s = svc.to_ascii_lowercase();

    // Players/GetProfile carries player profile data and display names for the local user.
    // It must be allowed through to name patching even if PresenceState is present.
    if s.contains("players/getprofile") {
        return false;
    }

    for needle in &[
        "products/getloadoutproducts",
        "products/matchcomplete",
        "products/matchcompletefte",
        "products/playerhasloadouttemplate",
        "products/getcontainerdroptable",
        "products/getdestructionproductvalues",
        "products/productupgradelevel",
        "products/schematicstradein",
        "products/unlockcontainer",
        "products/tradein",
        "products/crossentitlement",
        "genericstorage/getplayergenericstorage",
        "genericstorage/setplayergenericstorage",
        "rocketpass/getplayerprestigerewards",
        "rocketpass/getrewardcontent",
        "microtransaction/claimentitlements",
        "microtransaction/getcatalog",
        // Club services - preserve club metadata, rosters, and invitations
        "clubs/",
        // AuthPlayer must never be patched with name spoofing
        "authplayer",
        // Only tournament brackets and tournament listings - preserve authentic names
        "tournaments/getbracket",
        "tournaments/getactivebracket",
        "tournaments/gettournament",
        "tournaments/gettournaments",
        // Leaderboard services - preserve authentic names
        "skills/getskillleaderboard",
        "stats/getstatleaderboard",
        "leaderboards/getleaderboard",
        "leaderboard",
    ] {
        if s.contains(needle) {
            return true;
        }
    }

    for cat in &[
        b"ProfileLoadoutSave_TA" as &[u8],
        b"ProductsSave_TA",
        b"ExhibitionMatchSettingsSave_TA",
        b"PrivateMatchSettingsSave_TA",
        // Presence and party frames — these carry friends' online state and must not be modified
        b"PresenceState",
        b"PartyMember",
        b"RichPresence",
        b"SocialBeacon",
        b"FriendStatus",
        // Tournament brackets and leaderboard payloads - preserve authentic names
        b"TournamentBracket",
        b"Tournament_TA",
        b"GetSkillLeaderboard",
        b"GetStatLeaderboard",
    ] {
        if find_bytes(body, cat).is_some() {
            return true;
        }
    }

    false
}

const PSY_CDN_KEY: &[u8] = b"cqhyz50f3c3j2pxhwo6b1kypxikah0wh";
const PSY_RESP_KEY: &[u8] = b"3b932153785842ac927744b292e40e52";
const PSY_REQ_KEY: &[u8] = b"c338bd36fb8c42b1a431d30add939fc7";

static PROXY_RUNNING: AtomicBool = AtomicBool::new(false);
static PROXY_STOP_TX: std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>> =
    std::sync::Mutex::new(None);
static BROKER_STOP_TX: std::sync::Mutex<Option<oneshot::Sender<()>>> = std::sync::Mutex::new(None);
static SPOOF_CONFIG: std::sync::LazyLock<Arc<RwLock<Option<crate::psynet::SpoofPayload>>>> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(None)));

#[derive(Clone, Debug)]
struct AuthWSCreds {
    token: String,
    session_id: String,
    timestamp: std::time::Instant,
}
static LAST_AUTH_WS: std::sync::Mutex<Option<AuthWSCreds>> = std::sync::Mutex::new(None);
static LAST_GAME_BUILD_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn parse_build_id(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for window in segments.windows(2) {
        if window[0].eq_ignore_ascii_case("battlecars") {
            return Some(window[1].to_string());
        }
    }
    None
}

fn cache_auth_ws(body: &[u8]) {
    let token = find_json_value(body, "PsyToken");
    let session = find_json_value(body, "SessionID");
    if let (Some(t), Some(s)) = (token, session) {
        if !t.is_empty() && !s.is_empty() {
            let session_prefix = if s.len() > 8 { &s[..8] } else { &s };
            crate::applog::log_i18n(
                "auth_cached",
                "broker: cached AuthPlayer PsyToken & session ({session}...) for WS",
                &[("session", session_prefix)],
            );
            let mut lock = LAST_AUTH_WS.lock().unwrap();
            *lock = Some(AuthWSCreds {
                token: t,
                session_id: s,
                timestamp: std::time::Instant::now(),
            });
        }
    }
}

fn find_json_value(body: &[u8], key: &str) -> Option<String> {
    let prefix = format!("\"{key}\":\"");
    let prefix_bytes = prefix.as_bytes();
    let i = find_bytes(body, prefix_bytes)?;
    let val_start = i + prefix_bytes.len();
    let j = json_string_end(body, val_start)?;
    String::from_utf8(body[val_start..j].to_vec()).ok()
}

/// OS-assigned port for the plain-HTTP WS/RPC broker (`0` = not listening).
/// Config MITM + AuthPlayer rewrites point Rocket League here so we never need
/// a fixed port like 15759 (avoids conflicts / "broker already in use").
static BROKER_PORT: AtomicU16 = AtomicU16::new(0);

pub fn broker_port() -> Option<u16> {
    let p = BROKER_PORT.load(Ordering::SeqCst);
    if p == 0 {
        None
    } else {
        Some(p)
    }
}

fn broker_http_base() -> Option<String> {
    broker_port().map(|p| format!("http://127.0.0.1:{p}"))
}

pub fn ca_cert_bytes() -> &'static [u8] {
    CA_CERT_PEM
}

pub fn leaf_config_cert_bytes() -> &'static [u8] {
    LEAF_CONFIG_CERT_PEM
}

pub fn leaf_ws_cert_bytes() -> &'static [u8] {
    LEAF_WS_CERT_PEM
}

pub fn ca_crl_bytes() -> &'static [u8] {
    CA_CRL_DER
}

pub fn is_proxy_running() -> bool {
    PROXY_RUNNING.load(Ordering::SeqCst)
}

pub async fn set_spoof_config(cfg: crate::psynet::SpoofPayload) {
    if let Some(ns) = &cfg.name_spoof {
        if let Some(pid) = &ns.player_id {
            if !pid.trim().is_empty() {
                let mut lock = LEARNED_PLAYER_ID.lock().unwrap();
                if lock.is_none() {
                    *lock = Some(pid.trim().to_string());
                }
            }
        }
        if let Some(rn) = &ns.real_name {
            if !rn.trim().is_empty() {
                let mut lock = LEARNED_REAL_NAME.lock().unwrap();
                if lock.is_none() {
                    *lock = Some(rn.trim().to_string());
                }
            }
        }
    }
    let mut lock = SPOOF_CONFIG.write().await;
    *lock = Some(cfg);
}

pub async fn get_spoof_config() -> Option<crate::psynet::SpoofPayload> {
    SPOOF_CONFIG.read().await.clone()
}

#[derive(Debug)]
struct SniCertResolver {
    config_key: Arc<CertifiedKey>,
    ws_key: Arc<CertifiedKey>,
    epic_key: Arc<CertifiedKey>,
}

impl ResolvesServerCert for SniCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        let sni = client_hello.server_name();
        crate::applog::event(&format!("proxy: ClientHello SNI={sni:?}"));
        if let Some(s) = sni {
            let lower = s.to_ascii_lowercase();
            if lower.contains("ws.rlpp.psynet.gg") {
                return Some(self.ws_key.clone());
            }
            if lower.contains("config.psynet.gg") {
                return Some(self.config_key.clone());
            }
            return Some(self.epic_key.clone());
        }
        Some(self.epic_key.clone())
    }
}

fn load_certified_key(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<CertifiedKey>, String> {
    let mut cert_reader = std::io::Cursor::new(cert_pem);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("invalid cert pem: {e}"))?;

    let mut key_reader = std::io::Cursor::new(key_pem);
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| format!("invalid key pem read: {e}"))?
        .ok_or_else(|| "no private key found in pem".to_string())?;

    let signing_key = tokio_rustls::rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|e| format!("failed to parse signing key: {e}"))?;

    Ok(Arc::new(CertifiedKey::new(certs, signing_key)))
}

fn create_tls_acceptor() -> Result<TlsAcceptor, String> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let config_key = load_certified_key(LEAF_CONFIG_CERT_PEM, LEAF_CONFIG_KEY_PEM)?;
    let ws_key = load_certified_key(LEAF_WS_CERT_PEM, LEAF_WS_KEY_PEM)?;
    let epic_key = load_certified_key(LEAF_EPIC_CERT_PEM, LEAF_EPIC_KEY_PEM)?;

    let mut server_config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[
        &tokio_rustls::rustls::version::TLS12,
        &tokio_rustls::rustls::version::TLS13,
    ])
    .map_err(|e| format!("failed to set TLS protocol versions: {e}"))?
    .with_no_client_auth()
    .with_cert_resolver(Arc::new(SniCertResolver { config_key, ws_key, epic_key }));

    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

#[derive(Debug)]
struct NoCertVerifier;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        tokio_rustls::rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn create_upstream_tls_connector() -> tokio_rustls::TlsConnector {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let client_config = tokio_rustls::rustls::ClientConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("safe default protocols")
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
    .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(client_config))
}

type ResponseBoxBody = BoxBody<Bytes, std::convert::Infallible>;

fn full_body<T: Into<Bytes>>(chunk: T) -> ResponseBoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn empty_body() -> ResponseBoxBody {
    Empty::new()
        .map_err(|never| match never {})
        .boxed()
}

async fn handle_crl_or_http(
    req: Request<Incoming>,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let path = req.uri().path();
    crate::applog::event(&format!("proxy: HTTP port 80 request: {} {}", req.method(), path));
    if path == "/crl/velocityrl.crl" || path == "/velocityrl.crl" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/pkix-crl")
            .header("Content-Length", CA_CRL_DER.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(CA_CRL_DER.to_vec()))
            .unwrap());
    }
    if path == "/health" || path == "/vrl-health" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain")
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body("OK"))
            .unwrap());
    }
    if path == "/proxy.pac" || path == "/wpad.dat" {
        let pac = format!(
            "function FindProxyForURL(url, host) {{\n\
             \x20   if (\n\
             \x20       shExpMatch(host, \"*.epicgames.dev\") ||\n\
             \x20       host == \"api.epicgames.dev\" ||\n\
             \x20       shExpMatch(host, \"*account-public-service*\") ||\n\
             \x20       shExpMatch(host, \"*.psynet.gg\") ||\n\
             \x20       host == \"config.psynet.gg\" ||\n\
             \x20       shExpMatch(host, \"*.psyops.psynet.gg\") ||\n\
             \x20       shExpMatch(host, \"*.rocketleague.com\")\n\
             \x20   ) {{\n\
             \x20       return \"PROXY 127.0.0.1:{SYSTEM_PROXY_PORT}; DIRECT\";\n\
             \x20   }}\n\
             \x20   return \"DIRECT\";\n\
             }}"
        );
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/x-ns-proxy-autoconfig")
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(pac))
            .unwrap());
    }
    Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(full_body("Not Found"))
        .unwrap())
}

pub async fn start_native_proxy() -> Result<(), String> {
    if is_proxy_running() {
        crate::applog::event("proxy: already running");
        return Ok(());
    }

    if broker_port().is_none() {
        if let Err(e) = start_ws_broker().await {
            crate::applog::event(&format!("proxy: warning: failed to pre-start broker: {e}"));
        }
    }

    let acceptor = create_tls_acceptor()?;

    let listener_v4 = match tokio::net::TcpListener::bind("127.0.0.1:443").await {
        Ok(l) => l,
        Err(e) => {
            #[cfg(target_os = "linux")]
            let msg = if e.kind() == std::io::ErrorKind::PermissionDenied {
                "Failed to bind 127.0.0.1:443 (Permission denied). To allow port 443 on Linux, run: sudo sysctl -w net.ipv4.ip_unprivileged_port_start=80".to_string()
            } else {
                format!("Failed to bind 127.0.0.1:443: {e}")
            };
            #[cfg(not(target_os = "linux"))]
            let msg = format!("Failed to bind 127.0.0.1:443: {e}");

            crate::applog::event(&format!("proxy: {msg}"));
            return Err(msg);
        }
    };

    crate::applog::event("proxy: listening on 127.0.0.1:443 (IPv4 HTTPS)");

    let (stop_tx, mut stop_rx_443) = tokio::sync::watch::channel(false);
    let mut stop_rx_80 = stop_tx.subscribe();
    let mut stop_rx_forward = stop_tx.subscribe();
    *PROXY_STOP_TX.lock().unwrap() = Some(stop_tx);
    PROXY_RUNNING.store(true, Ordering::SeqCst);

    if let Ok(listener_forward) = tokio::net::TcpListener::bind(format!("127.0.0.1:{SYSTEM_PROXY_PORT}")).await {
        crate::applog::event(&format!("proxy: listening on 127.0.0.1:{SYSTEM_PROXY_PORT} (System Forward Proxy)"));
        let acceptor_forward = acceptor.clone();
        let forward_client = pin_eos_hosts_on_builder(
            reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none()),
        )
        .build()
        .unwrap_or_default();

        tokio::spawn(async move {
            loop {
                let conn = tokio::select! {
                    _ = stop_rx_forward.changed() => {
                        crate::applog::event("proxy: forward proxy shutting down");
                        break;
                    }
                    res = listener_forward.accept() => match res {
                        Ok(c) => c,
                        Err(e) => {
                            log::debug!("forward proxy accept error: {e}");
                            continue;
                        }
                    },
                };

                let (stream, peer_addr) = conn;
                let acc = acceptor_forward.clone();
                let client = forward_client.clone();

                tokio::spawn(async move {
                    handle_forward_proxy_connection(stream, peer_addr, acc, client).await;
                });
            }
        });
    } else {
        crate::applog::event(&format!("proxy: failed to bind forward proxy on 127.0.0.1:{SYSTEM_PROXY_PORT}"));
    }

    if let Ok(listener_http) = tokio::net::TcpListener::bind("127.0.0.1:80").await {
        crate::applog::event("proxy: listening on 127.0.0.1:80 (IPv4 HTTP CRL responder)");
        tokio::spawn(async move {
            loop {
                let conn = tokio::select! {
                    _ = stop_rx_80.changed() => {
                        crate::applog::event("proxy: HTTP 80 CRL responder shutting down");
                        break;
                    }
                    res = listener_http.accept() => match res {
                        Ok(c) => c,
                        Err(e) => {
                            log::debug!("proxy http 80 accept error: {e}");
                            continue;
                        }
                    },
                };

                let (stream, peer_addr) = conn;
                let io = TokioIo::new(stream);
                let service = service_fn(move |req: Request<Incoming>| async move {
                    handle_crl_or_http(req).await
                });

                tokio::spawn(async move {
                    if let Err(e) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await
                    {
                        let err_str = e.to_string();
                        let is_benign = err_str.contains("unexpected EOF")
                            || err_str.contains("error shutting down connection");
                        if !is_benign {
                            log::debug!("proxy http 80 error from {peer_addr}: {e}");
                        }
                    }
                });
            }
        });
    } else {
        crate::applog::event("proxy: port 80 unavailable for HTTP CRL responder (non-critical; store & port 443 active)");
    }

    let intercept_client = build_intercept_http_client();
    let mut builder = reqwest::Client::builder()
        .http1_only()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .redirect(reqwest::redirect::Policy::none());
    if let Some(addr) = resolve_ipv4_public("config.psynet.gg") {
        builder = builder.resolve("config.psynet.gg", addr);
    }
    if let Some(addr) = resolve_ipv4_public("api.rlpp.psynet.gg") {
        builder = builder.resolve("api.rlpp.psynet.gg", addr);
    }
    let client = pin_eos_hosts_on_builder(builder)
        .build()
        .map_err(|e| format!("failed to create reqwest client: {e}"))?;

    tokio::spawn(async move {
        loop {
            let conn = tokio::select! {
                _ = stop_rx_443.changed() => {
                    crate::applog::event("proxy: stop signal received, shutting down listener");
                    None
                }
                res = listener_v4.accept() => match res {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log::debug!("proxy v4 accept error: {e}");
                        continue;
                    }
                },
            };

            let Some((stream, peer_addr)) = conn else {
                break;
            };

            crate::applog::event(&format!("proxy: accepted connection from {peer_addr}"));

            let acceptor = acceptor.clone();
            let client = client.clone();
            let intercept_client = intercept_client.clone();

            tokio::spawn(async move {
                let tls_stream = match acceptor.accept(stream).await {
                    Ok(s) => {
                        crate::applog::event(&format!("proxy: TLS handshake OK from {peer_addr}"));
                        s
                    }
                    Err(e) => {
                        crate::applog::event(&format!("proxy: TLS handshake FAILED from {peer_addr}: {e}"));
                        return;
                    }
                };

                let io = TokioIo::new(tls_stream);
                let service = service_fn(move |req: Request<Incoming>| {
                    let client = client.clone();
                    let intercept_client = intercept_client.clone();
                    async move {
                        handle_request(req, client, intercept_client).await
                    }
                });

                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                {
                    let err_str = e.to_string();
                    let is_benign = err_str.contains("unexpected EOF")
                        || err_str.contains("error shutting down connection");
                    if !is_benign {
                        crate::applog::event(&format!("proxy connection error from {peer_addr}: {e}"));
                    }
                }
            });
        }
        PROXY_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

async fn handle_forward_proxy_connection(
    mut client_stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    tls_acceptor: TlsAcceptor,
    client: reqwest::Client,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = [0u8; 4096];
    let n = match client_stream.peek(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let head = match std::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(_) => return,
    };

    let first_line = match head.lines().next() {
        Some(l) => l,
        None => return,
    };

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    if parts[0].eq_ignore_ascii_case("CONNECT") {
        if parts.len() < 2 {
            return;
        }
        let target = parts[1];
        let header_end = if let Some(pos) = head.find("\r\n\r\n") {
            pos + 4
        } else if let Some(pos) = head.find("\n\n") {
            pos + 2
        } else {
            return;
        };

        let mut discard = vec![0u8; header_end];
        if client_stream.read_exact(&mut discard).await.is_err() {
            return;
        }

        let (host, port) = match target.split_once(':') {
            Some((h, p)) => (h.trim(), p.trim().parse::<u16>().unwrap_or(443)),
            None => (target.trim(), 443),
        };

        if is_intercept_target(host) {
            crate::applog::event(&format!("forward proxy: intercepting CONNECT {host}:{port} from {peer_addr}"));
            if client_stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_err() {
                return;
            }

            let tls_stream = match tls_acceptor.accept(client_stream).await {
                Ok(s) => s,
                Err(e) => {
                    crate::applog::event(&format!("forward proxy: TLS handshake failed for {host}: {e}"));
                    return;
                }
            };

            let io = TokioIo::new(tls_stream);
            let up_host = host.to_string();
            let service = service_fn(move |req: Request<Incoming>| {
                let client = client.clone();
                let up_h = up_host.clone();
                async move {
                    handle_forward_intercepted_request(req, up_h, port, client).await
                }
            });

            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                let err_str = e.to_string();
                if !err_str.contains("unexpected EOF") && !err_str.contains("error shutting down connection") {
                    log::debug!("forward proxy http error: {e}");
                }
            }
        } else {
            // Tunnel raw TCP bytes bidirectionally to destination
            let mut upstream_conn = match tokio::net::TcpStream::connect((host, port)).await {
                Ok(c) => c,
                Err(e) => {
                    log::debug!("forward proxy tunnel connect failed for {host}:{port}: {e}");
                    let _ = client_stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                    return;
                }
            };

            if client_stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_err() {
                return;
            }

            let _ = tokio::io::copy_bidirectional(&mut client_stream, &mut upstream_conn).await;
        }
    } else {
        // Plain HTTP proxy request (e.g. GET http://api.velocityrl.tech/ HTTP/1.1)
        let io = TokioIo::new(client_stream);
        let service = service_fn(move |mut req: Request<Incoming>| {
            let client = client.clone();
            async move {
                let path = req.uri().path();
                if path == "/proxy.pac" || path == "/wpad.dat" || path == "/health" || path == "/vrl-health" {
                    return handle_crl_or_http(req).await;
                }
                let uri = req.uri().clone();
                let host = uri.host().or_else(|| {
                    req.headers().get(hyper::header::HOST).and_then(|h| h.to_str().ok()).and_then(|s| s.split(':').next())
                }).unwrap_or("127.0.0.1").to_string();
                let port = uri.port_u16().unwrap_or(80);

                let mut target_url = uri.to_string();
                if !target_url.starts_with("http://") && !target_url.starts_with("https://") {
                    let path_and_query = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
                    target_url = format!("http://{host}:{port}{path_and_query}");
                }

                let mut req_builder = client.request(req.method().clone(), &target_url);
                for (k, v) in req.headers() {
                    let k_lower = k.as_str().to_ascii_lowercase();
                    if k_lower != "host" && k_lower != "proxy-connection" && k_lower != "connection" {
                        req_builder = req_builder.header(k.as_str(), v.as_bytes());
                    }
                }

                let body_bytes = match req.body_mut().collect().await {
                    Ok(collected) => collected.to_bytes().to_vec(),
                    Err(_) => Vec::new(),
                };
                if !body_bytes.is_empty() {
                    req_builder = req_builder.body(body_bytes);
                }

                let resp = match req_builder.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        return Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::BAD_GATEWAY)
                            .body(full_body(format!("Proxy upstream error: {e}").into_bytes()))
                            .unwrap());
                    }
                };

                let mut resp_builder = Response::builder().status(resp.status());
                for (k, v) in resp.headers() {
                    let k_lower = k.as_str().to_ascii_lowercase();
                    if k_lower != "content-length" && k_lower != "transfer-encoding" && k_lower != "content-encoding" {
                        resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
                    }
                }
                let resp_bytes = resp.bytes().await.unwrap_or_default().to_vec();
                resp_builder = resp_builder.header("Content-Length", resp_bytes.len().to_string());
                Ok(resp_builder.body(full_body(resp_bytes)).unwrap())
            }
        });

        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(io, service)
            .await;
    }
}

async fn handle_forward_websocket(
    req: Request<Incoming>,
    upstream_host: &str,
    upstream_port: u16,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    if upstream_host.contains("ws.rlpp.psynet.gg") || upstream_host.contains("psynet.gg") {
        return handle_websocket(req).await;
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let connector = create_upstream_tls_connector();
    let addr = format!("{upstream_host}:{upstream_port}");
    let tcp_conn = if let Some(mut sa) = resolve_ipv4_public(upstream_host) {
        sa.set_port(upstream_port);
        match tokio::net::TcpStream::connect(sa).await {
            Ok(t) => t,
            Err(_) => match tokio::net::TcpStream::connect(&addr).await {
                Ok(t) => t,
                Err(e) => {
                    crate::applog::event(&format!("forward proxy ws: connect to {addr} failed: {e}"));
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .body(full_body(format!("connect error: {e}")))
                        .unwrap());
                }
            },
        }
    } else {
        match tokio::net::TcpStream::connect(&addr).await {
            Ok(t) => t,
            Err(e) => {
                crate::applog::event(&format!("forward proxy ws: connect to {addr} failed: {e}"));
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(full_body(format!("connect error: {e}")))
                    .unwrap());
            }
        }
    };

    let server_name = match tokio_rustls::rustls::pki_types::ServerName::try_from(upstream_host.to_string()) {
        Ok(sn) => sn,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("invalid server name: {e}")))
                .unwrap());
        }
    };

    let mut tls_upstream = match connector.connect(server_name, tcp_conn).await {
        Ok(s) => s,
        Err(e) => {
            crate::applog::event(&format!("forward proxy ws: tls handshake error with {upstream_host}: {e}"));
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream tls error: {e}")))
                .unwrap());
        }
    };

    let method = req.method().clone();
    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_string();
    let mut req_raw = format!("{method} {path_and_query} HTTP/1.1\r\n");
    req_raw.push_str(&format!("Host: {upstream_host}\r\n"));
    for (k, v) in req.headers() {
        if !k.as_str().eq_ignore_ascii_case("host") {
            if let Ok(v_str) = v.to_str() {
                req_raw.push_str(&format!("{}: {}\r\n", k.as_str(), v_str));
            }
        }
    }
    req_raw.push_str("\r\n");

    if let Err(e) = tls_upstream.write_all(req_raw.as_bytes()).await {
        crate::applog::event(&format!("forward proxy ws: failed to write upgrade req: {e}"));
        return Ok(Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(full_body(format!("write upgrade req error: {e}")))
            .unwrap());
    }

    let mut resp_buf = vec![0u8; 4096];
    let mut total_read = 0;
    let header_end;
    loop {
        let n = match tls_upstream.read(&mut resp_buf[total_read..]).await {
            Ok(n) if n > 0 => n,
            _ => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(full_body("empty upstream upgrade response"))
                    .unwrap());
            }
        };
        total_read += n;
        if let Some(pos) = find_bytes(&resp_buf[..total_read], b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if total_read >= resp_buf.len() {
            resp_buf.resize(resp_buf.len() * 2, 0);
        }
    }

    let header_bytes = &resp_buf[..header_end];
    let extra_data = resp_buf[header_end..total_read].to_vec();

    let header_str = String::from_utf8_lossy(header_bytes);
    let first_line = header_str.lines().next().unwrap_or("HTTP/1.1 101 Switching Protocols");
    let status_code = first_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(101);

    let mut resp_builder = Response::builder().status(status_code);
    for line in header_str.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() {
                resp_builder = resp_builder.header(k, v);
            }
        }
    }

    tokio::spawn(async move {
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                let mut client_stream = TokioIo::new(upgraded);
                if !extra_data.is_empty() {
                    let _ = client_stream.write_all(&extra_data).await;
                }
                let _ = tokio::io::copy_bidirectional(&mut client_stream, &mut tls_upstream).await;
            }
            Err(e) => {
                log::debug!("forward proxy ws upgrade on(req) failed: {e}");
            }
        }
    });

    Ok(resp_builder.body(empty_body()).unwrap())
}

async fn handle_forward_intercepted_request(
    req: Request<Incoming>,
    upstream_host: String,
    upstream_port: u16,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    if is_upgrade {
        if upstream_host.contains("ws.rlpp.psynet.gg") || upstream_host.contains("psynet.gg") {
            return handle_websocket(req).await;
        }
        return handle_forward_websocket(req, &upstream_host, upstream_port).await;
    }
    let client = if EOS_ACCOUNT_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(&upstream_host)) {
        client
    } else if resolve_ipv4_public(&upstream_host).is_some() {
        http_client_pinned_for(&upstream_host)
    } else {
        client
    };
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("https://{upstream_host}:{upstream_port}{path}{query}");

    let headers = req.headers().clone();
    let req_body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes().to_vec(),
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(empty_body())
                .unwrap());
        }
    };

    let path_lower = path.to_ascii_lowercase();
    if path_lower.contains("products/getcontainerdroptable") || path_lower.contains("getcontainerdroptable") {
        let resp_bytes = get_authoritative_container_drop_table_response_bytes();
        let psy_time = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1755399374));
        let sig = resign_rpc_response(&psy_time, &resp_bytes);
        crate::applog::event("proxy: intercepted forward HTTPS Products/GetContainerDropTable -> authoritative drop table");
        record_network_event(
            "HTTP",
            "INBOUND",
            &path_lower,
            "200 OK",
            "GetContainerDropTable (Authoritative)",
            &String::from_utf8_lossy(&resp_bytes),
            true,
        );
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("PsyTime", &psy_time)
            .header("PsySig", &sig)
            .header("Psysignature", &sig)
            .header("Content-Length", resp_bytes.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(resp_bytes))
            .unwrap());
    }

    if path_lower.contains("products/unlockcontainer")
        || path_lower.contains("unlockcontainer")
        || path_lower.contains("products/opencontainer")
        || path_lower.contains("opencontainer")
    {
        if let Some(resp_json) = generate_simulated_unlock_container_payload(&req_body_bytes) {
            let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
            let psy_time = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1755399374));
            let sig = resign_rpc_response(&psy_time, &resp_bytes);
            crate::applog::event("proxy: intercepted forward HTTPS Products/UnlockContainer -> simulated crate open drop");
            record_network_event(
                "HTTP",
                "INBOUND",
                &path_lower,
                "200 OK",
                "UnlockContainer Drop Grant (Simulated)",
                &String::from_utf8_lossy(&resp_bytes),
                true,
            );
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header("PsyTime", &psy_time)
                .header("PsySig", &sig)
                .header("Psysignature", &sig)
                .header("Content-Length", resp_bytes.len().to_string())
                .header("Cache-Control", "no-cache, no-store")
                .body(full_body(resp_bytes))
                .unwrap());
        }
    }

    let mut up_req = client.request(method, &url);
    for (k, v) in headers.iter() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower != "content-length"
            && k_lower != "accept-encoding"
            && k_lower != "if-none-match"
            && k_lower != "if-modified-since"
            && k_lower != "if-match"
            && k_lower != "if-unmodified-since"
            && k_lower != "if-range"
            && k_lower != "host"
            && k_lower != "connection"
            && k_lower != "keep-alive"
            && k_lower != "proxy-connection"
            && k_lower != "proxy-authorization"
            && k_lower != "proxy-authenticate"
            && k_lower != "te"
            && k_lower != "trailers"
            && k_lower != "transfer-encoding"
            && k_lower != "upgrade"
        {
            up_req = up_req.header(k.as_str(), v.as_bytes());
        }
    }
    if !req_body_bytes.is_empty() {
        up_req = up_req.body(req_body_bytes);
    }

    let resp = match up_req.send().await {
        Ok(r) => r,
        Err(e) => {
            crate::applog::event(&format!("forward proxy upstream error to {url}: {e}"));
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream error: {e}")))
                .unwrap());
        }
    };

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let is_json = resp_headers
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("application/json") || ct.contains("+json"))
        .unwrap_or(false);

    if is_json {
        let resp_bytes = match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(_) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(empty_body())
                    .unwrap());
            }
        };

        let spoof_cfg = match crate::psynet::load_active_spoof_from_disk() {
            Some(c) => Some(c),
            None => get_spoof_config().await,
        };
        let name_spoof = spoof_cfg
            .as_ref()
            .and_then(|c| c.name_spoof.as_ref())
            .filter(|n| n.enabled && !n.display_name.trim().is_empty());

        let mut final_body = resp_bytes;
        let mut body_modified = false;
        if let Some(ns) = name_spoof {
            if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&final_body) {
                let learned_pid_store = get_learned_player_id();
                let target_pid = learned_pid_store.as_deref().or(ns.player_id.as_deref());
                let learned_name_store = get_learned_real_name();
                let filter_real = ns.real_name.as_deref().or(learned_name_store.as_deref());

                let (modified, learned_pid, learned_name) = patch_eos_accounts_json(
                    &mut json_val,
                    ns.display_name.trim(),
                    target_pid,
                    filter_real,
                );

                if let Some(pid) = learned_pid {
                    set_learned_player_id(&pid);
                }
                if let Some(rn) = learned_name {
                    set_learned_real_name(&rn);
                }

                if modified {
                    if let Ok(serialized) = serde_json::to_vec(&json_val) {
                        final_body = serialized;
                        body_modified = true;
                    }
                }

                if !path.to_ascii_lowercase().contains("authplayer") {
                    let (ws_patched, ws_changed) = patch_ws_name_fields(
                        &final_body,
                        ns.display_name.trim(),
                        target_pid,
                    );
                    if ws_changed {
                        final_body = ws_patched;
                        body_modified = true;
                    }
                }
            }
        }

        let path_lower = path.to_ascii_lowercase();
        let is_reservation = path_lower.contains("reservation")
            || path_lower.contains("reservationsready")
            || path_lower.contains("clientreservation")
            || path_lower.contains("addreservation")
            || path_lower.contains("dsr/")
            || find_bytes(&final_body, b"ServerAddress").is_some()
            || find_bytes(&final_body, b"GameURL").is_some()
            || find_bytes(&final_body, b"PingURL").is_some();
        if is_reservation {
            let swap_rules = load_active_swap_rules(spoof_cfg.as_ref());
            let (new_body, changed) = patch_reservation_json_with_rules(&final_body, &swap_rules).await;
            if changed {
                final_body = new_body;
                body_modified = true;
            }
        }

        let is_skill_rpc = path_lower.contains("getplayerskill")
            || path_lower.contains("skills")
            || find_bytes(&final_body, b"\"Skills\"").is_some();
        if is_skill_rpc {
            extract_and_save_real_skills(&final_body);
            if let Some(fr) = spoof_cfg.as_ref().and_then(|c| c.fake_ranks.as_ref()) {
                if fr.enabled {
                    let (new_body, did_patch) = patch_get_player_skill_json(&final_body, fr);
                    if did_patch {
                        final_body = new_body;
                        body_modified = true;
                    }
                }
            }
        }

        if let Some(cs) = spoof_cfg.as_ref().and_then(|c| c.credit_spoof.as_ref()).filter(|c| c.enabled) {
            let path_lower = path.to_ascii_lowercase();
            let is_wallet = path_lower.contains("wallet")
                || path_lower.contains("tournament")
                || path_lower.contains("currencies")
                || path_lower.contains("shops")
                || find_bytes(&final_body, b"\"Currencies\"").is_some()
                || find_bytes(&final_body, b"TournamentCredit").is_some()
                || find_bytes(&final_body, b"TournamentPoint").is_some();
            if is_wallet {
                let (new_body, did_patch) = patch_player_wallet_json(&final_body, cs);
                if did_patch {
                    final_body = new_body;
                    body_modified = true;
                }
            }
        }

        let is_ecom_ownership = path_lower.contains("ecom")
            || path_lower.contains("entitlement")
            || path_lower.contains("ownership")
            || path_lower.contains("dlc")
            || path_lower.contains("crossentitlement")
            || path_lower.contains("checkownership")
            || path_lower.contains("verifyownership")
            || path_lower.contains("getplayerproducts")
            || path_lower.contains("getloadoutproducts")
            || path_lower.contains("getuserproducts")
            || find_bytes(&final_body, b"\"ProductData\"").is_some()
            || find_bytes(&final_body, b"\"Products\"").is_some()
            || (find_bytes(&final_body, b"\"ProductID\"").is_some()
                && find_bytes(&final_body, b"\"InstanceID\"").is_some());
        if is_ecom_ownership {
            let spawned_items = load_active_spawned_items(spoof_cfg.as_ref());
            if !spawned_items.is_empty() || !spawned_title_ids().is_empty() {
                let (new_body_prod, did_patch_prod) = patch_player_products_json(&final_body, &spawned_items);
                if did_patch_prod {
                    final_body = new_body_prod;
                    body_modified = true;
                }
                let (new_body_cross, did_patch_cross) = patch_cross_entitlement_json(&final_body, &spawned_items);
                if did_patch_cross {
                    final_body = new_body_cross;
                    body_modified = true;
                }
            }
            let (new_body_own, did_patch_own) = patch_check_ownership_json(&final_body, &spawned_items);
            if did_patch_own {
                final_body = new_body_own;
                body_modified = true;
            }
        }

        let was_denied = response_denies_access(&final_body);
        let (new_body_sync, did_patch_sync) = neutralize_inventory_out_of_sync_json(&final_body);
        if did_patch_sync {
            final_body = new_body_sync;
            body_modified = true;
        }
        let effective_status = if was_denied && did_patch_sync { StatusCode::OK } else { status };

        let is_avatar_rpc = path_lower.contains("canshowavatar")
            || find_bytes(&final_body, b"\"AllowedPlayerIDs\"").is_some()
            || find_bytes(&final_body, b"\"HiddenPlayerIDs\"").is_some();
        if is_avatar_rpc {
            let (new_body, did_patch) = patch_users_can_show_avatar_json(&final_body);
            if did_patch {
                final_body = new_body;
                body_modified = true;
            }
        }

        let mut resp_builder = Response::builder().status(effective_status);
        for (k, v) in resp_headers.iter() {
            let k_lower = k.as_str().to_ascii_lowercase();
            if k_lower != "content-length"
                && k_lower != "content-encoding"
                && k_lower != "transfer-encoding"
                && (!body_modified || (k_lower != "psysig" && k_lower != "psysignature"))
            {
                resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
            }
        }

        if body_modified {
            let psy_time = resp_headers
                .get("psytime")
                .or_else(|| resp_headers.get("PsyTime"))
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(1755399374)
                    )
                });
            let sig = resign_rpc_response(&psy_time, &final_body);
            resp_builder = resp_builder.header("PsyTime", &psy_time);
            resp_builder = resp_builder.header("PsySig", &sig);
            resp_builder = resp_builder.header("Psysignature", &sig);
        }

        record_network_event(
            "HTTP",
            "INBOUND",
            &url,
            &effective_status.to_string(),
            if body_modified { "Forward Intercepted RPC (Patched)" } else { "Forward Intercepted RPC" },
            &String::from_utf8_lossy(&final_body),
            body_modified,
        );

        resp_builder = resp_builder.header("Content-Length", final_body.len().to_string());
        Ok(resp_builder.body(full_body(final_body)).unwrap())
    } else {
        use futures_util::StreamExt;
        let stream = resp.bytes_stream().filter_map(|r| async {
            r.ok().map(hyper::body::Frame::data).map(Ok::<_, std::convert::Infallible>)
        });
        let body_stream = http_body_util::StreamBody::new(stream);
        let boxed_body = http_body_util::BodyExt::boxed(body_stream);

        let mut resp_builder = Response::builder().status(status);
        for (k, v) in resp_headers.iter() {
            let k_lower = k.as_str().to_ascii_lowercase();
            if k_lower != "transfer-encoding" {
                resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
            }
        }
        Ok(resp_builder.body(boxed_body).unwrap())
    }
}

/// Active loopback health probe: initiates a TLS handshake and HTTP/1.1 request to
/// https://127.0.0.1:443/health (with SNI config.psynet.gg).
/// Verifies that port 443 is bound, accepting TLS connections, using the VelocityRL certificate,
/// and successfully processing requests before hosts redirection occurs.
pub async fn check_loopback_health() -> Result<(), String> {
    if !is_proxy_running() {
        return Err("Proxy is not marked running".into());
    }

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .resolve("config.psynet.gg", "127.0.0.1:443".parse().unwrap())
        .no_proxy()
        .timeout(std::time::Duration::from_millis(2000))
        .build()
        .map_err(|e| format!("failed to build loopback probe client: {e}"))?;

    // Give the spawned TLS accept loop a moment to start before first probe.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    for attempt in 1..=8 {
        match client.get("https://config.psynet.gg/health").send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    crate::applog::log_i18n(
                        "health_ok",
                        "proxy: loopback TLS health check on 127.0.0.1:443 verified OK on attempt {attempt}",
                        &[("attempt", &attempt.to_string())],
                    );
                    return Ok(());
                } else {
                    crate::applog::event(&format!(
                        "proxy: loopback health check returned HTTP {}",
                        resp.status()
                    ));
                }
            }
            Err(e) => {
                crate::applog::log_i18n(
                    "health_fail",
                    "proxy: loopback health probe attempt {attempt}/8 failed: {err}",
                    &[("attempt", &attempt.to_string()), ("err", &e.to_string())],
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    Err("Proxy loopback TLS health check on 127.0.0.1:443 failed after 8 attempts. Run VelocityRL as Administrator — binding port 443 requires elevated privileges on Windows.".into())
}

pub async fn verify_proxy_loopback_health() -> Result<(), String> {
    check_loopback_health().await
}

pub fn stop_native_proxy(_revert_hosts_file: bool) {
    if let Some(tx) = PROXY_STOP_TX.lock().unwrap().take() {
        let _ = tx.send(true);
    }
    if let Some(tx) = BROKER_STOP_TX.lock().unwrap().take() {
        let _ = tx.send(());
    }
    BROKER_PORT.store(0, Ordering::SeqCst);
    PROXY_RUNNING.store(false, Ordering::SeqCst);
    crate::applog::event("proxy: stopped");
}

/// Start a plain-HTTP broker on `127.0.0.1:<ephemeral>`.
/// Rocket League connects here after `PsyNetUrl` / `PerConURL*` are rewritten to
/// that host:port. Returns the bound port.
pub async fn start_ws_broker() -> Result<u16, String> {
    if let Some(existing) = broker_port() {
        crate::applog::event(&format!(
            "broker: already listening on 127.0.0.1:{existing} — reuse"
        ));
        return Ok(existing);
    }

    let listener = match tokio::net::TcpListener::bind("127.0.0.1:15759").await {
        Ok(l) => l,
        Err(_) => match tokio::net::TcpListener::bind("127.0.0.1:49320").await {
            Ok(l) => l,
            Err(_) => match tokio::net::TcpListener::bind("127.0.0.1:7546").await {
                Ok(l) => l,
                Err(_) => match tokio::net::TcpListener::bind("127.0.0.1:49325").await {
                    Ok(l) => l,
                    Err(_) => match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                        Ok(l) => l,
                        Err(e) => {
                            let msg = format!("broker: Failed to bind 127.0.0.1: {e}");
                            crate::applog::event(&msg);
                            return Err(msg);
                        }
                    },
                },
            },
        },
    };
    let port = listener
        .local_addr()
        .map_err(|e| format!("broker: local_addr failed: {e}"))?
        .port();
    BROKER_PORT.store(port, Ordering::SeqCst);
    crate::applog::event(&format!(
        "broker: listening on http://127.0.0.1:{port} (PsyNetUrl RPC + local WS forward)"
    ));

    let mut builder = reqwest::Client::builder()
        .http1_only()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate();
    if let Some(addr) = resolve_ipv4_public("api.rlpp.psynet.gg") {
        builder = builder.resolve("api.rlpp.psynet.gg", addr);
    }
    let client = builder
        .build()
        .map_err(|e| format!("failed to create reqwest client for broker: {e}"))?;

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    *BROKER_STOP_TX.lock().unwrap() = Some(stop_tx);

    let client_main = client.clone();
    tokio::spawn(async move {
        loop {
            let conn = tokio::select! {
                _ = &mut stop_rx => {
                    crate::applog::event("broker: stop signal received");
                    None
                }
                res = listener.accept() => match res {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log::debug!("broker accept error: {e}");
                        continue;
                    }
                },
            };

            let Some((stream, peer_addr)) = conn else { break };

            let client = client_main.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req: Request<Incoming>| {
                    let client = client.clone();
                    async move {
                        handle_broker_request(req, client).await
                    }
                });
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                {
                    log::debug!("broker connection error from {peer_addr}: {e}");
                }
            });
        }
        BROKER_PORT.store(0, Ordering::SeqCst);
    });

    // Auxiliary listeners on remaining known ports (15759, 17895, 7546, 49320, 49325)
    let aux_ports = [15759u16, 17895u16, 7546u16, 49320u16, 49325u16];
    for &alt_port in &aux_ports {
        if alt_port == port {
            continue;
        }
        let client_aux = client.clone();
        tokio::spawn(async move {
            match tokio::net::TcpListener::bind(format!("127.0.0.1:{alt_port}")).await {
                Ok(aux_listener) => {
                    crate::applog::event(&format!(
                        "broker: auxiliary listener active on http://127.0.0.1:{alt_port}"
                    ));
                    loop {
                        let Ok((stream, peer_addr)) = aux_listener.accept().await else { break };
                        let c = client_aux.clone();
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let service = service_fn(move |req: Request<Incoming>| {
                                let c2 = c.clone();
                                async move {
                                    handle_broker_request(req, c2).await
                                }
                            });
                            if let Err(e) = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, service)
                                .with_upgrades()
                                .await
                            {
                                log::debug!("aux broker connection error on port {alt_port} from {peer_addr}: {e}");
                            }
                        });
                    }
                }
                Err(e) => {
                    log::debug!("broker: auxiliary port {alt_port} skipped ({e}) — primary broker active on {port}");
                }
            }
        });
    }

    Ok(port)
}


static CONTAINER_DROPS_JSON: &str = include_str!("../../resources/container_drops.json");

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct ContainerDropEntry {
    #[serde(rename = "ProductID")]
    pub product_id: i32,
    #[serde(rename = "SeriesID")]
    pub series_id: i32,
    #[serde(rename = "Drops")]
    pub drops: Vec<ContainerDropItem>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct ContainerDropItem {
    #[serde(rename = "ProductID")]
    pub product_id: i32,
    #[serde(rename = "SeriesID")]
    pub series_id: Option<i32>,
    #[serde(rename = "Paintable")]
    pub paintable: Option<bool>,
}

static CONTAINER_DROPS: std::sync::LazyLock<Vec<ContainerDropEntry>> = std::sync::LazyLock::new(|| {
    serde_json::from_str(CONTAINER_DROPS_JSON).unwrap_or_default()
});

static PRODUCT_SERIES_MAP: std::sync::LazyLock<std::collections::HashMap<i32, i32>> = std::sync::LazyLock::new(|| {
    let mut map = std::collections::HashMap::new();
    for c in CONTAINER_DROPS.iter() {
        if c.series_id > 0 {
            map.insert(c.product_id, c.series_id);
        }
        for d in &c.drops {
            let sid = d.series_id.unwrap_or(c.series_id);
            if sid > 0 {
                map.insert(d.product_id, sid);
            }
        }
    }
    map
});

pub fn lookup_series_id_for_product(pid: i32) -> i32 {
    PRODUCT_SERIES_MAP.get(&pid).copied().unwrap_or(0)
}

pub fn is_container_or_drop(product_id: i32) -> bool {
    CONTAINER_DROPS.iter().any(|c| c.product_id == product_id)
        || product_id == 1353
        || (5298..=5303).contains(&product_id)
        || (8413..=8421).contains(&product_id)
        || (10666..=10673).contains(&product_id)
}

pub fn get_authoritative_container_drop_table_response_bytes() -> Vec<u8> {
    static CACHED_DROP_TABLE_RESP: std::sync::LazyLock<Vec<u8>> = std::sync::LazyLock::new(|| {
        let mut drops_val: serde_json::Value = serde_json::from_str(CONTAINER_DROPS_JSON).unwrap_or_default();
        if let Some(arr) = drops_val.as_array_mut() {
            for entry in arr {
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert("bSkipRoulette".to_string(), serde_json::json!(true));
                    obj.insert("bUnlocked".to_string(), serde_json::json!(true));
                }
            }
        }
        let resp_obj = serde_json::json!({
            "ContainerDrops": drops_val,
            "Result": {
                "ContainerDrops": drops_val
            },
            "bSuccess": true
        });
        serde_json::to_vec(&resp_obj).unwrap_or_default()
    });
    CACHED_DROP_TABLE_RESP.clone()
}

fn roll_drop_tier(min_tier: &str, rng: &mut impl rand::Rng) -> &'static str {
    let roll: u32 = rng.gen_range(0..100);
    match min_tier {
        // Sport Drop (Uncommon min) → boosted: VeryRare 50%, Import 30%, Exotic 15%, BM 5%
        "Uncommon" => {
            if roll < 50 { "VeryRare" }
            else if roll < 80 { "Import" }
            else if roll < 95 { "Exotic" }
            else { "BlackMarket" }
        }
        // Special Drop (Rare min) → Import 45%, Exotic 35%, BM 20%
        "Rare" => {
            if roll < 45 { "Import" }
            else if roll < 80 { "Exotic" }
            else { "BlackMarket" }
        }
        // Deluxe Drop (VeryRare min) → Import 30%, Exotic 45%, BM 25%
        "VeryRare" => {
            if roll < 30 { "Import" }
            else if roll < 75 { "Exotic" }
            else { "BlackMarket" }
        }
        // Import Drop → Exotic 55%, BM 45%
        "Import" => {
            if roll < 55 { "Exotic" }
            else { "BlackMarket" }
        }
        // Exotic Drop → Exotic 40%, BM 60%
        "Exotic" => {
            if roll < 40 { "Exotic" }
            else { "BlackMarket" }
        }
        // Black Market Drop → always BM
        "BlackMarket" => "BlackMarket",
        // Golden egg/gift default → Import 30%, Exotic 40%, BM 30%
        _ => {
            if roll < 30 { "Import" }
            else if roll < 70 { "Exotic" }
            else { "BlackMarket" }
        }
    }
}

fn pick_authentic_non_crate_item(tier: &str, rng: &mut impl rand::Rng) -> (i32, bool) {
    match tier {
        // VeryRare — painted wheels and painted toppers only, nothing base
        "VeryRare" => {
            let pool: &[(i32, bool)] = &[
                (386, true),   // Cristiano
                (1452, true),  // Ink
                (1166, true),  // Frostbite
                (1951, true),  // Yamane
                (4127, false), // Anodized Pearl
                (45, true),    // Ion
                (52, true),    // Plasma
                (363, true),   // Dieci
                (373, true),   // Veloce
                (383, true),   // Sunburst
            ];
            let idx = rng.gen_range(0..pool.len());
            pool[idx]
        }
        // Import — quality bodies and boost/decals
        "Import" => {
            let pool: &[(i32, bool)] = &[
                (23, true),    // Octane
                (4284, true),  // Fennec
                (403, true),   // Dominus
                (22, true),    // Breakout
                (36, true),    // Flamethrower
                (53, true),    // Thermal
                (35, true),    // Datastream
                (44, true),    // Sacred
                (2355, true),  // Reaper
                (30, true),    // Merc
            ];
            let idx = rng.gen_range(0..pool.len());
            pool[idx]
        }
        // Exotic — premium wheels only
        "Exotic" => {
            let pool: &[(i32, bool)] = &[
                (1580, true),  // Zomba
                (1661, true),  // Draco
                (731, true),   // Voltaic
                (380, true),   // Invader
                (381, true),   // Vortex
                (368, true),   // Stern
                (363, true),   // Dieci
                (383, true),   // Sunburst
                (373, true),   // Veloce
                (359, true),   // Tunica
            ];
            let idx = rng.gen_range(0..pool.len());
            pool[idx]
        }
        // BlackMarket — the best decals/goals/boosts
        _ => {
            let pool: &[(i32, bool)] = &[
                (2854, false), // Dissolver
                (2044, true),  // Dueling Dragons
                (3239, true),  // Mainframe
                (1435, false), // Heatwave
                (2966, false), // Fire God
                (1684, true),  // 20XX
                (1888, false), // Bubbly
                (1908, true),  // Hellfire
                (4549, true),  // Gravity Bomb
                (2532, true),  // Wonderment
            ];
            let idx = rng.gen_range(0..pool.len());
            pool[idx]
        }
    }
}

fn generate_simulated_unlock_container_payload(body: &[u8]) -> Option<serde_json::Value> {
    let req_val: Option<serde_json::Value> = serde_json::from_slice(body).ok();
    let mut consumed_instance_ids: Vec<String> = Vec::new();
    let mut crate_pid_opt: Option<i32> = None;

    if let Some(ref root) = req_val {
        if let Some(arr) = root.get("InstanceIDs").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    consumed_instance_ids.push(s.to_string());
                    if crate_pid_opt.is_none() && s.len() >= 8 {
                        if let Ok(pid) = u32::from_str_radix(&s[..8], 16) {
                            if pid > 0 {
                                crate_pid_opt = Some(pid as i32);
                            }
                        }
                    }
                }
            }
        }
        if let Some(arr) = root.get("KeyInstanceIDs").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    consumed_instance_ids.push(s.to_string());
                }
            }
        }
    }

    if crate_pid_opt.is_none() {
        if let Ok(g) = DYNAMIC_SPAWNED_ITEMS.read() {
            for &(pid, _) in g.iter() {
                if CONTAINER_DROPS.iter().any(|c| c.product_id == pid) {
                    crate_pid_opt = Some(pid);
                    break;
                }
            }
        }
    }

    let container_entry = crate_pid_opt.and_then(|pid| {
        CONTAINER_DROPS.iter().find(|c| c.product_id == pid)
    });

    use rand::Rng;
    let mut rng = rand::thread_rng();

    // Check if a ProductID is an internal Psyonix drop tier indicator (5364..=5369)
    let is_tier_placeholder = |pid: i32| (5364..=5369).contains(&pid);
    let owned_spawned = load_active_spawned_items(None);
    let is_owned = |pid: i32, paint: i32| owned_spawned.iter().any(|&(opid, opaint)| opid == pid && (paint == 0 || opaint == paint));

    let (dropped_pid, dropped_series_id, can_be_painted) = if let Some(container) = container_entry {
        // Collect authentic specific drops from this container (excluding tier indicator placeholders
        // and excluding base/common items — bodies with no quality and default paints).
        // Base product IDs to skip: common bodies and default decals shipped with the base game.
        let is_base_item = |pid: i32| matches!(pid,
            63 | 62 | 36 | 53 | 35 | 44 | 2355 | 30 | 25 | 29 | 31 | // common bodies/decals
            100..=200 |  // early common topper/decal range
            7000..=9999   // internal placeholder range
        );
        let specific_drops: Vec<&ContainerDropItem> = container.drops.iter()
            .filter(|d| !is_tier_placeholder(d.product_id) && !is_base_item(d.product_id))
            .collect();

        if !specific_drops.is_empty() {
            // Classic series crate: strictly draw from the authentic preview contents,
            // prioritizing items not already owned.
            let unowned_drops: Vec<&ContainerDropItem> = specific_drops.iter()
                .filter(|d| !is_owned(d.product_id, 0))
                .copied()
                .collect();
            let target_pool = if !unowned_drops.is_empty() { &unowned_drops } else { &specific_drops };
            let idx = rng.gen_range(0..target_pool.len());
            let d = target_pool[idx];
            let sid = d.series_id.unwrap_or(container.series_id);
            let paintable = d.paintable.unwrap_or(true);
            (d.product_id, sid, paintable)
        } else {
            // Tier-based drop container (Uncommon Drop, Rare Drop, Very Rare Drop, Golden Eggs/Gifts, etc.)
            let (min_tier, container_sid) = match container.product_id {
                5302 => ("Uncommon", 0),    // Sport Drop (Uncommon)
                5301 => ("Rare", 0),        // Special Drop (Rare)
                5303 => ("VeryRare", 0),    // Deluxe Drop (Very Rare)
                5300 => ("Import", 0),      // Import Drop
                5299 => ("Exotic", 0),      // Exotic Drop
                5298 => ("BlackMarket", 0), // Black Market Drop
                _ => ("Rare", container.series_id), // Golden Eggs/Gifts, Cups, etc.
            };

            let rolled_tier = roll_drop_tier(min_tier, &mut rng);
            let (pid, paintable) = pick_authentic_non_crate_item(rolled_tier, &mut rng);
            let sid = if container_sid > 0 { container_sid } else { lookup_series_id_for_product(pid) };
            (pid, sid, paintable)
        }
    } else {
        // High tier fallback items pool if opened container was not matched
        let drop_candidates: &[(i32, i32, bool)] = &[
            (2854, 21, false), // Dissolver (Black Market)
            (2044, 21, true),  // Dueling Dragons (Black Market)
            (3239, 21, true),  // Mainframe (Black Market)
            (1435, 0, false),  // Heatwave (Black Market)
            (1684, 0, true),   // 20XX (Black Market)
            (1580, 2, true),   // Zomba (Exotic)
            (1661, 4, true),   // Draco (Exotic)
            (4549, 0, true),   // Gravity Bomb (Black Market)
            (4284, 0, true),   // Fennec (Import)
        ];
        let idx = rng.gen_range(0..drop_candidates.len());
        drop_candidates[idx]
    };

    let is_painted = can_be_painted && rng.gen_bool(0.55); // 55% chance painted if paintable (premium drops bias)
    let dropped_paint = if is_painted {
        let unowned_paints: Vec<i32> = (1..=14).filter(|&p| !is_owned(dropped_pid, p)).collect();
        if !unowned_paints.is_empty() {
            if unowned_paints.contains(&12) && rng.gen_bool(0.40) {
                12 // Titanium White prioritized if unowned
            } else {
                unowned_paints[rng.gen_range(0..unowned_paints.len())]
            }
        } else if rng.gen_bool(0.40) {
            12
        } else {
            rng.gen_range(1..=14)
        }
    } else {
        0
    };

    // Consume the opened crate from active spawned items and persistent storage
    if let Some(crate_pid) = crate_pid_opt {
        crate::applog::event(&format!("proxy: crate unboxing consuming opened crate ProductID={crate_pid}"));
        unregister_spawned_items(&[crate_pid]);
    }

    // Register drop into broker memory & disk so garage inventory immediately recognizes it.
    // Pre-insert into NOTIFIED_SPAWNED_ITEMS so the background inventory reconciler doesn't
    // blast a duplicate concurrent WebSocket Reward/RewardResult frame at the exact moment
    // the game client is receiving this HandleUnlockContainerResponse (which would freeze
    // the unboxing reveal sequence).
    if let Ok(mut set) = NOTIFIED_SPAWNED_ITEMS.write() {
        set.insert((dropped_pid, dropped_paint));
    }
    let _ = register_spawned_items(&[(dropped_pid, dropped_paint)]);
    if let Some(crate_pid) = crate_pid_opt {
        decrement_container_quantity(crate_pid);
    }
    crate::applog::event(&format!(
        "proxy: crate unboxing granted authentic drop ProductID={dropped_pid}, Paint={dropped_paint}, SeriesID={dropped_series_id}"
    ));

    let mut attributes: Vec<serde_json::Value> = Vec::new();
    if dropped_paint > 0 {
        attributes.push(serde_json::json!({
            "Key": "Painted",
            "Value": dropped_paint.to_string()
        }));
        attributes.push(serde_json::json!({
            "Key": "Paint",
            "Value": dropped_paint.to_string()
        }));
    }
    attributes.push(serde_json::json!({
        "Key": "bSkipRoulette",
        "Value": "true"
    }));
    attributes.push(serde_json::json!({
        "Key": "bUnlocked",
        "Value": "true"
    }));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(1700000000);
    let inst_hex = format!("{:08x}{:08x}0000000000000000", dropped_pid as u32, dropped_paint.max(0) as u32);

    let mut drop_obj = serde_json::json!({
        "ProductID": dropped_pid,
        "InstanceID": inst_hex,
        "Attributes": attributes,
        "SeriesID": dropped_series_id,
        "Amount": 1,
        "ExpirationTime": -1,
        "IsTradable": false,
        "TradeHold": -1,
        "AddedTimestamp": now,
        "UpdatedTimestamp": now,
        "bSkipRoulette": true,
        "bUnlocked": true,
    });
    if dropped_paint > 0 {
        drop_obj["Paint"] = serde_json::json!(dropped_paint);
        drop_obj["PaintID"] = serde_json::json!(dropped_paint);
    }

    let resp_obj = serde_json::json!({
        "Drops": [drop_obj.clone()],
        "ProductData": [drop_obj.clone()],
        "RewardDrops": [drop_obj.clone()],
        "InstanceIDs": consumed_instance_ids,
        "bSkipRoulette": true,
        "bUnlocked": true,
        "Result": {
            "Drops": [drop_obj.clone()],
            "ProductData": [drop_obj.clone()],
            "RewardDrops": [drop_obj],
            "InstanceIDs": consumed_instance_ids,
            "bSkipRoulette": true,
            "bUnlocked": true
        },
        "bSuccess": true
    });

    Some(resp_obj)
}

/// Pull the requested player's Steam ID out of an avatar request path.
/// Understands the sidecar avatar URL scheme:
/// `/Content/Images/Players/Avatars/Large/<id>/0?buildSecret=..&tab=1`
/// as well as the simpler VelocityRL routes (`/rlim/avatar?id=...`).
pub fn avatar_player_id_from_path(path: &str) -> Option<u64> {
    for part in path.split(&['/', '?', '&'][..]) {
        let candidate = part
            .strip_suffix(".png")
            .or_else(|| part.strip_suffix(".jpg"))
            .unwrap_or(part);
        if let Some(id) = crate::avatar::parse_steam_id(candidate) {
            return Some(id);
        }
    }
    None
}

/// Match only an identified local player, including encoded lobby URLs and image suffixes.
fn avatar_path_matches_player(path: &str, player_id: &str) -> bool {
    let account = normalize_player_id(player_id);
    if account.is_empty() {
        return false;
    }
    path.split(&['/', '?', '&'][..]).any(|part| {
        let decoded = crate::avatar::percent_decode(part);
        let value = decoded.split_once('=').map(|(_, value)| value).unwrap_or(&decoded);
        let value = value.strip_suffix(".png")
            .or_else(|| value.strip_suffix(".jpg"))
            .unwrap_or(value);
        normalize_player_id(value) == account
    })
}

fn avatar_query_has_player(query: Option<&str>) -> bool {
    query.map(|query| query.split('&').any(|part| {
        let key = crate::avatar::percent_decode(part.split('=').next().unwrap_or_default())
            .to_ascii_lowercase().replace('_', "");
        matches!(key.as_str(), "id" | "steamid" | "playerid" | "accountid")
    })).unwrap_or(false)
}

fn is_local_avatar_route(path: &str, query: Option<&str>) -> bool {
    matches!(path, "/rlim/pfp" | "/rlim/avatar" | "/rlim/avatar.png" | "/a.png")
        && !avatar_query_has_player(query)
}

/// Handle incoming requests on the plain-HTTP broker (ephemeral local port).
/// Upgrades WebSocket connections to the game server, and proxies HTTP RPC/Services calls to api.rlpp.psynet.gg.
async fn handle_broker_request(
    mut req: Request<Incoming>,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let path_raw = req.uri().path().to_ascii_lowercase();
    let path_str = if let Some(stripped) = path_raw.strip_prefix("/r/") {
        format!("/rlim/{stripped}")
    } else if let Some(stripped) = path_raw.strip_prefix("/s/") {
        format!("/rlim/{stripped}")
    } else {
        path_raw
    };
    if path_str == "/crl/velocityrl.crl" || path_str == "/velocityrl.crl" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/pkix-crl")
            .header("Content-Length", CA_CRL_DER.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(CA_CRL_DER.to_vec()))
            .unwrap());
    }

    if path_str == "/rlim/name" || path_str.starts_with("/rlim/name?") || path_str == "/s/name" || path_str.starts_with("/s/name?") {
        let spoof_cfg = match crate::psynet::load_active_spoof_from_disk() {
            Some(c) => Some(c),
            None => get_spoof_config().await,
        };
        let name = spoof_cfg
            .as_ref()
            .and_then(|c| c.name_spoof.as_ref())
            .filter(|ns| ns.enabled && !ns.display_name.trim().is_empty())
            .map(|ns| ns.display_name.trim().to_string())
            .or_else(|| {
                spoof_cfg.as_ref()
                    .filter(|c| !c.custom_name.trim().is_empty())
                    .map(|c| c.custom_name.trim().to_string())
            })
            .unwrap_or_default();
        crate::applog::event(&format!("broker: {path_str} queried -> responding '{name}' for GFxData_LocalPlayer_TA::ChangeName hook"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Content-Length", name.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(name.into_bytes()))
            .unwrap());
    }

    if path_str == "/rlim/title" || path_str.starts_with("/rlim/title?") || path_str == "/s/title" || path_str.starts_with("/s/title?") {
        let (title, glow) = crate::psynet::load_active_spoof_from_disk()
            .map(|c| {
                let t = if !c.equip_title_id.trim().is_empty() {
                    c.equip_title_id.trim().to_string()
                } else if !c.custom_text.trim().is_empty() {
                    c.custom_text.trim().to_string()
                } else {
                    c.display_title_id.trim().to_string()
                };
                let g = c.title_color.map(|tc| tc.glow_color).unwrap_or_default();
                (t, g)
            })
            .unwrap_or_default();
        let body_str = if !title.is_empty() || !glow.is_empty() {
            format!("{title}|{glow}")
        } else {
            "|".to_string()
        };
        crate::applog::event(&format!("broker: {path_str} queried -> responding '{body_str}'"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Content-Length", body_str.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(body_str.into_bytes()))
            .unwrap());
    }

    if path_str == "/rlim/ping" || path_str.starts_with("/rlim/ping?")
        || path_str == "/rlim/pingspoof" || path_str.starts_with("/rlim/pingspoof?")
        || path_str == "/s/ping" || path_str.starts_with("/s/ping?") {
        let ping_val = "0".to_string();
        crate::applog::event(&format!("broker: {path_str} queried -> responding '{ping_val}'"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Content-Length", ping_val.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(ping_val.into_bytes()))
            .unwrap());
    }

    if path_str == "/network/events" || path_str == "/api/network/events" {
        let events = get_network_events();
        let payload = serde_json::to_vec(&events).unwrap_or_default();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .header("Content-Length", payload.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(payload))
            .unwrap());
    }

    if path_str == "/network/clear" || path_str == "/api/network/clear" {
        clear_network_events();
        let payload = b"{\"success\":true}";
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .header("Content-Length", payload.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(payload.to_vec()))
            .unwrap());
    }

    if path_str == "/network/status" || path_str == "/api/network/status" {
        let cfg = crate::psynet::load_active_spoof_from_disk().unwrap_or_default();
        let udp_port = crate::udp_proxy::get_local_port().unwrap_or(0);
        let broker_port = BROKER_PORT.load(Ordering::Relaxed);
        let info = serde_json::json!({
            "broker_port": broker_port,
            "udp_match_proxy_port": udp_port,
            "spoof_active": cfg.enabled,
            "name_spoof": cfg.name_spoof.as_ref().map(|n| n.display_name.clone()).unwrap_or_default(),
            "fake_rank_enabled": cfg.fake_ranks.as_ref().map(|s| s.enabled).unwrap_or(false),
            "title_id": cfg.equip_title_id,
            "credits": cfg.credit_spoof,
        });
        let payload = serde_json::to_vec(&info).unwrap_or_default();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .header("Content-Length", payload.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(payload))
            .unwrap());
    }

    if path_str == "/rlim/gear" || path_str.starts_with("/rlim/gear?")
        || path_str == "/rlim/gea" || path_str.starts_with("/rlim/gea?")
        || path_str == "/rlim/geam" || path_str.starts_with("/rlim/geam?")
        || path_str == "/s/gear" || path_str.starts_with("/s/gear?")
        || path_str == "/s/geam" || path_str.starts_with("/s/geam?") {
        let rules = load_active_swap_rules(None);
        let spawned = load_active_spawned_items(None);
        let mut pids: Vec<String> = rules.iter().map(|r| r.wanted_id.to_string()).collect();
        for (pid, _) in spawned {
            if pid > 0 && !pids.contains(&pid.to_string()) {
                pids.push(pid.to_string());
            }
        }
        let gear_body = pids.join(",");
        crate::applog::event(&format!("broker: {path_str} queried -> responding '{gear_body}'"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Content-Length", gear_body.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(gear_body.into_bytes()))
            .unwrap());
    }

    // Plain-text variant for the in-game hook: body is just the loadout IDs,
    // title travels in a response header so no JSON parser is needed in bytecode.
    if path_str == "/loadout/raw" || path_str.starts_with("/loadout/raw?")
        || path_str == "/rlim/loadout/raw" || path_str.starts_with("/rlim/loadout/raw?") {
        let rules = load_active_swap_rules(None);
        let mut pids: Vec<String> = rules.iter()
            .filter(|r| r.wanted_id > 0)
            .map(|r| if r.paint_id > 0 { format!("{}:{}", r.wanted_id, r.paint_id) }
                     else { r.wanted_id.to_string() })
            .collect();
        for (pid, paint) in load_active_spawned_items(None) {
            if pid > 0 {
                let s = if paint > 0 { format!("{pid}:{paint}") } else { pid.to_string() };
                if !pids.contains(&s) { pids.push(s); }
            }
        }
        let body = pids.join(",");
        crate::applog::event(&format!("broker: {path_str} -> raw loadout '{}'", body));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .header("Content-Length", body.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(body.into_bytes()))
            .unwrap());
    }

    if path_str == "/rlim/loadout" || path_str.starts_with("/rlim/loadout?")
        || path_str == "/s/lo" || path_str.starts_with("/s/lo?")
        || path_str == "/loadout" || path_str.starts_with("/loadout?") {
        let rules = load_active_swap_rules(None);
        let mut pids: Vec<String> = rules.iter()
            .filter(|r| r.wanted_id > 0)
            .map(|r| if r.paint_id > 0 {
                format!("{}:{}", r.wanted_id, r.paint_id)
            } else {
                r.wanted_id.to_string()
            })
            .collect();
        for (pid, paint) in load_active_spawned_items(None) {
            if pid > 0 {
                let s = if paint > 0 { format!("{pid}:{paint}") } else { pid.to_string() };
                if !pids.contains(&s) { pids.push(s); }
            }
        }
        let loadout_value = pids.join(",");
        let (title_id, title_text) = configured_title();
        let body = serde_json::json!({
            "Loadout": loadout_value,
            "Title": title_id,
            "TitleText": title_text,
            "Rules": rules,
        });
        let loadout_json = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
        crate::applog::event(&format!("broker: {path_str} queried -> loadout '{}' + title '{title_id}'", loadout_value));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .header("Content-Length", loadout_json.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(loadout_json.into_bytes()))
            .unwrap());
    }

    if path_str == "/rlim/sp" || path_str.starts_with("/rlim/sp?")
        || path_str == "/s/sp" || path_str.starts_with("/s/sp?") {
        let rules = load_active_swap_rules(None);
        let spawned = load_active_spawned_items(None);
        let mut sp_ids: Vec<String> = Vec::new();
        for (pid, paint) in &spawned {
            if *pid > 0 {
                let id_str = if *paint > 0 {
                    format!("{pid}:{paint}")
                } else {
                    pid.to_string()
                };
                if !sp_ids.contains(&id_str) {
                    sp_ids.push(id_str);
                }
            }
        }
        for r in &rules {
            if r.wanted_id > 0 {
                let id_str = if r.paint_id > 0 {
                    format!("{}:{}", r.wanted_id, r.paint_id)
                } else {
                    r.wanted_id.to_string()
                };
                if !sp_ids.contains(&id_str) {
                    sp_ids.push(id_str);
                }
            }
        }
        let sp_body = sp_ids.join(",");
        crate::applog::event(&format!("broker: {path_str} queried -> responding '{sp_body}'"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Content-Length", sp_body.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(sp_body.into_bytes()))
            .unwrap());
    }

    if path_str == "/rlim/pk" || path_str.starts_with("/rlim/pk?")
        || path_str == "/rlim/sk" || path_str.starts_with("/rlim/sk?") {
        crate::applog::event(&format!("broker: {path_str} queried"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain")
            .header("Content-Length", "0")
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(Vec::new()))
            .unwrap());
    }

    // B1 preset key -> decimal loadout records for the in-game listener.
    if path_str == "/rlim/lo" || path_str.starts_with("/rlim/lo?") {
        let query = req.uri().query().unwrap_or_default().to_string();
        let body_bytes = match req.body_mut().collect().await {
            Ok(c) => c.to_bytes().to_vec(),
            Err(_) => Vec::new(),
        };
        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();
        let key = crate::loadout_wire::find_key(&query, &body_text);

        let digits = key
            .as_deref()
            .and_then(crate::loadout_wire::key_to_digits)
            .unwrap_or_else(|| {
                let mut slots: Vec<(u32, Vec<(String, String)>)> =
                    vec![(0, Vec::new()); crate::presets::COSMETIC_SLOTS_COUNT + 7];

                for rule in load_active_swap_rules(None) {
                    let idx = crate::presets::upk_slot_index_from_str(&rule.slot) as usize;
                    if idx < slots.len() && rule.wanted_id > 0 {
                        let mut attrs = Vec::new();
                        if rule.paint_id > 0 {
                            attrs.push(("Painted".to_string(), rule.paint_id.to_string()));
                        }
                        slots[idx] = (rule.wanted_id as u32, attrs);
                    }
                }
                let mut spawned = load_active_spawned_items(None);
                for (product, paint) in spawned.drain(..) {
                    if product <= 0 || slots.iter().any(|(p, _)| *p == product as u32) {
                        continue;
                    }
                    if let Some(slot) = slots.iter_mut().find(|(p, _)| *p == 0) {
                        let mut attrs = Vec::new();
                        if paint > 0 {
                            attrs.push(("Painted".to_string(), paint.to_string()));
                        }
                        *slot = (product as u32, attrs);
                    }
                }

                let title = equipped_title_id();
                if !title.is_empty() && !slots.is_empty() {
                    slots[0].1.push(("TitleId".to_string(), title));
                }

                let encoded = crate::loadout_wire::encode_loadout(&slots, 1);
                crate::applog::event(&format!("broker: {path_str} rebuilt loadout key {encoded}"));
                crate::loadout_wire::key_to_digits(&encoded).unwrap_or_else(|| "D".to_string())
            });

        crate::applog::event(&format!(
            "broker: {path_str} -> {}... ({} bytes)",
            &digits[..digits.len().min(96)],
            digits.len()
        ));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .header("Content-Length", digits.len().to_string())
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(full_body(digits.into_bytes()))
            .unwrap());
    }

    if path_str == "/rlim/steam_callback" || path_str.starts_with("/rlim/steam_callback?")
        || path_str == "/steam/callback" || path_str.starts_with("/steam/callback?") {
        let query_str = req.uri().query().unwrap_or_default().to_string();
        crate::applog::event("broker: handling Steam OpenID callback");

        match crate::avatar::verify_steam_openid_assertion(&query_str).await {
            Ok(steam_id) => {
                crate::applog::event(&format!("broker: Steam OpenID verified successfully for SteamID {steam_id}"));

                let (persona, avatar_url) = crate::avatar::fetch_verified_steam_profile(steam_id)
                    .await
                    .unwrap_or_else(|_| (format!("Player_{}", steam_id % 10000), "https://avatars.steamstatic.com/fef49e7fa7e1997310d705b2a6158ff8dc1cdfeb_full.jpg".to_string()));

                // Download and process the Steam avatar for the profile card.
                // Never let a Steam login replace an avatar the user already
                // chose, otherwise signing in silently wipes the custom PFP.
                let mut avatar_b64 = String::new();
                if let Ok(img_resp) = client.get(&avatar_url).send().await {
                    if let Ok(bytes) = img_resp.bytes().await {
                        if let Ok((_bgra, png_bytes)) = crate::avatar::process_avatar_image(&bytes) {
                            // Steam login only caches the linked account's PFP
                            // for the profile card. It must never become the
                            // active in-game avatar -- only the user's chosen
                            // avatar is allowed there.
                            let cache_dir = std::path::PathBuf::from("/root/.config/com.velocityrl.app/custom_avatars");
                            let _ = std::fs::create_dir_all(&cache_dir);
                            let _ = std::fs::write(cache_dir.join(format!("steam_{steam_id}.png")), &png_bytes);

                            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
                            avatar_b64 = format!("data:image/png;base64,{b64}");
                        }
                    }
                }

                let auth_user = crate::avatar::SteamAuthenticatedUser {
                    steam_id: steam_id.to_string(),
                    persona_name: persona.clone(),
                    avatar_url: avatar_url.clone(),
                    avatar_data_url: avatar_b64.clone(),
                };
                crate::avatar::set_authenticated_steam_user(auth_user);

                let html_body = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>VelocityRL — Steam Linked</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            background: #121212; color: #e8e8e8;
            font-family: system-ui, -apple-system, Segoe UI, sans-serif;
            display: flex; align-items: center; justify-content: center; min-height: 100vh;
            padding: 24px;
        }}
        .card {{
            background: #1a1a1a; border: 1px solid #333333; border-radius: 6px;
            padding: 32px; text-align: center; max-width: 400px; width: 100%;
        }}
        .avatar-img {{
            width: 80px; height: 80px; border-radius: 4px; border: 1px solid #333333;
            margin-bottom: 20px; object-fit: cover; display: inline-block;
        }}
        h1 {{ font-size: 20px; font-weight: 600; margin: 0 0 10px; color: #e8e8e8; }}
        p {{ color: #a0a0a0; font-size: 14px; line-height: 1.5; margin: 6px 0; }}
        b {{ color: #ffffff; }}
        .badge {{
            display: inline-block; margin-top: 16px; padding: 4px 10px;
            background: #222222; color: #4a9eff;
            border: 1px solid #333333; border-radius: 4px;
            font-size: 12px; font-weight: 500;
        }}
        .hint {{ margin-top: 24px; font-size: 13px; color: #888888; }}
    </style>
</head>
<body>
    <div class="card">
        <img class="avatar-img" src="{avatar_b64}" alt="Avatar">
        <h1>Steam Account Linked</h1>
        <p>Authenticated as <b>{persona}</b></p>
        <p>Your custom Steam avatar is now active in Rocket League.</p>
        <div class="badge">Verified via Steam OpenID</div>
        <div class="hint">You can safely close this browser window and return to VelocityRL.</div>
    </div>
    <script>
        setTimeout(() => {{ try {{ window.close(); }} catch(e){{}} }}, 3500);
    </script>
</body>
</html>"#);

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .header("Content-Length", html_body.len().to_string())
                    .body(full_body(html_body.into_bytes()))
                    .unwrap());
            }
            Err(e) => {
                crate::applog::event(&format!("broker: Steam OpenID verification failed: {e}"));
                let err_html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>VelocityRL — Authentication Failed</title>
    <style>
        body {{ margin: 0; background: #121212; color: #e8e8e8; font-family: sans-serif; display: flex; align-items: center; justify-content: center; height: 100vh; }}
        .card {{ background: #1a1a1a; border: 1px solid #e74c3c; border-radius: 12px; padding: 32px; text-align: center; max-width: 400px; }}
        h1 {{ color: #e74c3c; font-size: 20px; margin-bottom: 12px; }}
        p {{ color: #a0a0a0; font-size: 13.5px; }}
    </style>
</head>
<body>
    <div class="card">
        <h1>Authentication Failed</h1>
        <p>{e}</p>
        <p style="margin-top: 20px; font-size: 12px; color: #666;">You can close this tab and try again.</p>
    </div>
</body>
</html>"#);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .header("Content-Length", err_html.len().to_string())
                    .body(full_body(err_html.into_bytes()))
                    .unwrap());
            }
        }
    }

    if path_str == "/rlim/avatar" || path_str.starts_with("/rlim/avatar/") || path_str.starts_with("/rlim/avatar?")
        || path_str.starts_with("/rlim/avatar.png")
        || path_str.starts_with("/profile/") || path_str.starts_with("/rlim/profile/")
        || path_str == "/a.png" || path_str.starts_with("/a.png?")
        || path_str == "/rlim/pfp" || path_str == "/s/pfp" || path_str.starts_with("/s/pfp/") || path_str.starts_with("/rlim/pfp/")
        || path_str.contains("/avatars/")
        || path_str.contains("/content/images/players/avatars/") {
        crate::applog::event(&format!("Rlim AvatarRequest: {path_str}"));
        // 1. Check if a specific Steam ID is requested in path or query.
        //    Sidecar avatar URLs look like
        //    /Content/Images/Players/Avatars/Large/<id>/0?buildSecret=..&tab=1
        //    so scan every path/query component rather than relying on a
        //    single fixed position.
        let query_str = req.uri().query().unwrap_or_default();
        let mut target_steam_id = avatar_player_id_from_path(&path_str);
        if target_steam_id.is_none() {
            for q_part in query_str.split('&') {
                if let Some((k, v)) = q_part.split_once('=') {
                    if k.eq_ignore_ascii_case("id")
                        || k.eq_ignore_ascii_case("steamid")
                        || k.eq_ignore_ascii_case("playerid")
                        || k.eq_ignore_ascii_case("accountid")
                    {
                        if let Some(id) = crate::avatar::parse_steam_id(v) {
                            target_steam_id = Some(id);
                            break;
                        }
                    }
                }
            }
        }

        // Explicit paths used for the local player's own avatar. Other avatar
        // routes are treated as per-player requests and must never fall back to
        // the local image, otherwise every player shows the same picture.
        let is_local_route = is_local_avatar_route(&path_str, req.uri().query());

        let authenticated_steam_id = crate::avatar::get_authenticated_steam_user_internal()
            .and_then(|user| crate::avatar::parse_steam_id(&user.steam_id));
        // Steam login is optional, but the proxy also learns the local player's
        // own ID from intercepted PsyNet responses. Use it as a fallback so the
        // custom avatar still goes to the right player without a login.
        let learned_steam_id = crate::proxy::get_learned_player_id()
            .map(|id| normalize_player_id(&id))
            .and_then(|id| crate::avatar::parse_steam_id(&id));

        // Epic PlayerIDs look like "Epic|<32-hex account id>|0". They are not
        // Steam IDs, so parse_steam_id() returns None for them. Match the
        // account id from the request path against the learned local PlayerID
        // directly so the custom avatar resolves for those requests too.
        let learned_player_id = crate::proxy::get_learned_player_id().unwrap_or_default();
        let config_player_id = crate::proxy::get_spoof_config().await
            .and_then(|c| c.name_spoof)
            .and_then(|ns| ns.player_id)
            .unwrap_or_default();
        let target_is_local_pid = avatar_path_matches_player(&req.uri().to_string(), &learned_player_id)
            || (!config_player_id.is_empty() && avatar_path_matches_player(&req.uri().to_string(), &config_player_id))
            || avatar_path_matches_player(&req.uri().to_string(), "0")
            || avatar_path_matches_player(&req.uri().to_string(), "unknown");
        if target_is_local_pid {
            if let Some(png_bytes) = crate::avatar::get_active_avatar_png() {
                crate::applog::event(&format!(
                    "broker: {path_str} queried for local avatar (pid_match=true) -> active avatar PNG ({} bytes)",
                    png_bytes.len()
                ));
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "image/png")
                    .header("Content-Length", png_bytes.len().to_string())
                    .header("Cache-Control", "no-cache, no-store")
                    .body(full_body(png_bytes))
                    .unwrap());
            }
        }

        if let Some(steam_id) = target_steam_id {
            // The linked account's own ID always resolves to the local avatar.
            if Some(steam_id) == authenticated_steam_id || Some(steam_id) == learned_steam_id {
                if let Some(png_bytes) = crate::avatar::get_active_avatar_png() {
                    crate::applog::event(&format!("broker: {path_str} queried for local Steam ID {steam_id} -> responding with active avatar PNG ({} bytes)", png_bytes.len()));
                    return Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "image/png")
                        .header("Content-Length", png_bytes.len().to_string())
                        .header("Cache-Control", "no-cache, no-store")
                        .body(full_body(png_bytes))
                        .unwrap());
                }
            }

            // The sidecar URL patched into TAGame.upk is now the game's primary
            // avatar path for *every* player. Other players must keep their real
            // Steam avatars regardless of the "load other PFPs" toggle (it only
            // controls the older in-menu broker routes).
            let is_sidecar_route = path_str.contains("/content/images/players/avatars/");
            let allow_other_pfps = is_sidecar_route
                || (crate::avatar::is_load_other_pfps_enabled_internal()
                    && crate::avatar::get_authenticated_steam_user_internal().is_some());
            if allow_other_pfps {
                if let Some(steam_png) = crate::avatar::fetch_steam_avatar_png(steam_id).await {
                    crate::applog::event(&format!("broker: {path_str} queried for Steam ID {steam_id} -> responding with Steam avatar ({} bytes)", steam_png.len()));
                    return Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "image/png")
                        .header("Content-Length", steam_png.len().to_string())
                        .header("Cache-Control", "public, max-age=86400")
                        .body(full_body(steam_png))
                        .unwrap());
                }
            } else {
                crate::applog::event(&format!("broker: {path_str} queried for Steam ID {steam_id} -> rejected (load other pfps disabled or unauthenticated)"));
            }

            // A request for another player must never receive the local avatar.
            crate::applog::event(&format!("broker: {path_str} queried for Steam ID {steam_id} -> no per-player avatar available"));
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "text/plain")
                .header("Content-Length", "0")
                .header("Cache-Control", "no-cache, no-store")
                .body(full_body(Vec::new()))
                .unwrap());
        }

        if is_local_route {
            if let Some(png_bytes) = crate::avatar::get_active_avatar_png() {
                crate::applog::event(&format!("broker: {path_str} queried -> responding with active avatar PNG ({} bytes)", png_bytes.len()));
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "image/png")
                    .header("Content-Length", png_bytes.len().to_string())
                    .header("Cache-Control", "no-cache, no-store")
                    .body(full_body(png_bytes))
                    .unwrap());
            }
        }

        crate::applog::event(&format!("broker: {path_str} queried -> no local avatar for this route"));
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .header("Content-Length", "0")
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(Vec::new()))
            .unwrap());
    }



    if path_str == "/rlim/status" || path_str.starts_with("/rlim/status?") {
        let cfg = crate::psynet::load_active_spoof_from_disk().unwrap_or_default();
        let has_avatar = crate::avatar::get_active_avatar_png().is_some();
        let status_json = serde_json::json!({
            "active": true,
            "port": BROKER_PORT.load(Ordering::Relaxed),
            "name": cfg.name_spoof.as_ref().map(|n| n.display_name.clone()).unwrap_or_default(),
            "has_avatar": has_avatar,
        }).to_string();
        crate::applog::event(&format!("broker: /rlim/status queried"));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Content-Length", status_json.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(status_json.into_bytes()))
            .unwrap());
    }

    if path_str.starts_with("/rlim/") {
        let res_body = "{\"status\":\"ok\"}";
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Content-Length", res_body.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(res_body.as_bytes().to_vec()))
            .unwrap());
    }
    if is_upgrade || path_str.starts_with("/ws") {
        return handle_websocket(req).await;
    }

    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_string();
    let path_lower = path_and_query.to_ascii_lowercase();

    let method = req.method().clone();
    let req_headers = req.headers().clone();
    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes().to_vec(),
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full_body(format!("broker read body error: {e}")))
                .unwrap());
        }
    };

    if path_lower.contains("products/getcontainerdroptable") || path_lower.contains("getcontainerdroptable") {
        let resp_bytes = get_authoritative_container_drop_table_response_bytes();
        let psy_time = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1755399374));
        let sig = resign_rpc_response(&psy_time, &resp_bytes);
        crate::applog::event("broker: intercepted Products/GetContainerDropTable -> authoritative drop table");
        record_network_event(
            "HTTP",
            "INBOUND",
            &path_lower,
            "200 OK",
            "GetContainerDropTable (Authoritative)",
            &String::from_utf8_lossy(&resp_bytes),
            true,
        );
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("PsyTime", &psy_time)
            .header("PsySig", &sig)
            .header("Psysignature", &sig)
            .header("Content-Length", resp_bytes.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(resp_bytes))
            .unwrap());
    }

    if path_lower.contains("products/unlockcontainer")
        || path_lower.contains("unlockcontainer")
        || path_lower.contains("products/opencontainer")
        || path_lower.contains("opencontainer")
    {
        crate::applog::event(&format!("broker: intercepted unlock container request {path_lower} body={}", String::from_utf8_lossy(&body_bytes)));
        if let Some(resp_json) = generate_simulated_unlock_container_payload(&body_bytes) {
            let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
            let psy_time = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1755399374));
            let sig = resign_rpc_response(&psy_time, &resp_bytes);
            crate::applog::event(&format!("broker: simulated Products/UnlockContainer -> {}", String::from_utf8_lossy(&resp_bytes)));
            record_network_event(
                "HTTP",
                "INBOUND",
                &path_lower,
                "200 OK",
                "UnlockContainer Drop Grant (Simulated)",
                &String::from_utf8_lossy(&resp_bytes),
                true,
            );
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header("PsyTime", &psy_time)
                .header("PsySig", &sig)
                .header("Psysignature", &sig)
                .header("Content-Length", resp_bytes.len().to_string())
                .header("Cache-Control", "no-cache, no-store")
                .body(full_body(resp_bytes))
                .unwrap());
        }
    }

    let upstream_url = format!("https://api.rlpp.psynet.gg{path_and_query}");

    let mut up_builder = client.request(method, &upstream_url);
    for (k, v) in req_headers.iter() {
        let k_str = k.as_str().to_ascii_lowercase();
        // Skip hop-by-hop headers and headers that reqwest manages
        if k_str != "host"
            && k_str != "content-length"
            && k_str != "accept-encoding"
            && k_str != "connection"
            && k_str != "keep-alive"
            && k_str != "proxy-authenticate"
            && k_str != "proxy-authorization"
            && k_str != "te"
            && k_str != "trailers"
            && k_str != "transfer-encoding"
            && k_str != "upgrade"
        {
            up_builder = up_builder.header(k.as_str(), v.as_bytes());
        }
    }
    // Note: Reqwest automatically sets the Host / :authority header to the upstream hostname.
    if !body_bytes.is_empty() {
        up_builder = up_builder.body(body_bytes.clone());
    }

    let up_resp = match up_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            crate::applog::event(&format!("broker upstream error for {upstream_url}: {e:?}"));
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("broker upstream error: {e}")))
                .unwrap());
        }
    };

    let status = up_resp.status();
    let resp_headers = up_resp.headers().clone();
    let resp_bytes = match up_resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("broker read resp error: {e}")))
                .unwrap());
        }
    };

    let mut out_body = resp_bytes;
    let mut patched = false;

    let path_lower = path_and_query.to_ascii_lowercase();
    let mut effective_status = status;
    if path_lower.contains("authplayer") {
        cache_auth_ws(&out_body);
        for k in &["VerifiedPlayerName", "PlayerName", "DisplayName"] {
            if let Some(rn) = find_json_value(&out_body, k) {
                set_learned_real_name(&rn);
                break;
            }
        }
        for k in &["PlayerID", "PlayerId", "UserID", "FromUserID"] {
            if let Some(pid) = find_json_value(&out_body, k) {
                set_learned_player_id(&pid);
                break;
            }
        }
        for k in &["PlayerName", "DisplayName"] {
            if let Some(rn) = find_json_value(&body_bytes, k) {
                set_learned_real_name(&rn);
                break;
            }
        }
        for k in &["PlayerID", "PlayerId", "UserID"] {
            if let Some(pid) = find_json_value(&body_bytes, k) {
                set_learned_player_id(&pid);
                break;
            }
        }

        // PerConURL / PerConURLv2 rewriting in AuthPlayer disabled; ProjectX.upk routes WebSocket directly.
    } else {
        let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
            Some(c) => Some(c),
            None => get_spoof_config().await,
        };

        let is_reservation_rpc = path_lower.contains("reservation")
            || path_lower.contains("reservationsready")
            || path_lower.contains("clientreservation")
            || path_lower.contains("addreservation")
            || path_lower.contains("dsr/")
            || find_bytes(&out_body, b"ServerAddress").is_some()
            || find_bytes(&out_body, b"GameURL").is_some()
            || find_bytes(&out_body, b"PingURL").is_some();
        if is_reservation_rpc {
            let swap_rules = load_active_swap_rules(cfg_opt.as_ref());
            let (new_body, did_patch) = patch_reservation_json_with_rules(&out_body, &swap_rules).await;
            if did_patch {
                out_body = new_body;
                patched = true;
            }
        }

        let is_products_rpc = path_lower.contains("getplayerproducts")
            || path_lower.contains("getloadoutproducts")
            || path_lower.contains("checkownership")
            || path_lower.contains("verifyownership")
            || path_lower.contains("getuserproducts")
            || path_lower.contains("crossentitlement")
            || path_lower.contains("entitlement")
            || path_lower.contains("ecom")
            || path_lower.contains("dlc")
            || path_lower.contains("ownership")
            || find_bytes(&out_body, b"\"ProductData\"").is_some()
            || find_bytes(&out_body, b"\"Products\"").is_some()
            || (find_bytes(&out_body, b"\"ProductID\"").is_some()
                && find_bytes(&out_body, b"\"InstanceID\"").is_some());
        if is_products_rpc {
            let spawned_items = load_active_spawned_items(cfg_opt.as_ref());
            if !spawned_items.is_empty() || !spawned_title_ids().is_empty() {
                let (new_body, did_patch) = patch_player_products_json(&out_body, &spawned_items);
                if did_patch {
                    out_body = new_body;
                    patched = true;
                }
                let (new_body_cross, did_patch_cross) = patch_cross_entitlement_json(&out_body, &spawned_items);
                if did_patch_cross {
                    out_body = new_body_cross;
                    patched = true;
                }
            }
            let (new_body_own, did_patch_own) = patch_check_ownership_json(&out_body, &spawned_items);
            if did_patch_own {
                out_body = new_body_own;
                patched = true;
            }
        }

        let was_denied = response_denies_access(&out_body);
        let (new_body_sync, did_patch_sync) = neutralize_inventory_out_of_sync_json(&out_body);
        if did_patch_sync {
            out_body = new_body_sync;
            patched = true;
        }
        if was_denied && did_patch_sync {
            effective_status = StatusCode::OK;
        }

        let is_skill_rpc = path_lower.contains("getplayerskill")
            || path_lower.contains("skills")
            || find_bytes(&out_body, b"\"Skills\"").is_some();
        if is_skill_rpc {
            extract_and_save_real_skills(&out_body);
            if let Some(fr) = cfg_opt.as_ref().and_then(|c| c.fake_ranks.as_ref()) {
                if fr.enabled {
                    let (new_body, did_patch) = patch_get_player_skill_json(&out_body, fr);
                    if did_patch {
                        out_body = new_body;
                        patched = true;
                    }
                }
            }
        }

        if let Some(cfg) = &cfg_opt {
            let is_wallet_rpc = path_lower.contains("getplayerwallet")
                || path_lower.contains("wallet")
                || path_lower.contains("tournament")
                || path_lower.contains("currencies")
                || path_lower.contains("shops")
                || find_bytes(&out_body, b"\"Currencies\"").is_some()
                || find_bytes(&out_body, b"TournamentCredit").is_some()
                || find_bytes(&out_body, b"TournamentPoint").is_some();
            if is_wallet_rpc {
                if let Some(cs) = &cfg.credit_spoof {
                    if cs.enabled {
                        let (new_body, did_patch) = patch_player_wallet_json(&out_body, cs);
                        if did_patch {
                            out_body = new_body;
                            patched = true;
                        }
                    }
                }
            }

            let is_avatar_rpc = path_lower.contains("canshowavatar")
                || find_bytes(&out_body, b"\"AllowedPlayerIDs\"").is_some()
                || find_bytes(&out_body, b"\"HiddenPlayerIDs\"").is_some();
            if is_avatar_rpc {
                let (new_body, did_patch) = patch_users_can_show_avatar_json(&out_body);
                if did_patch {
                    out_body = new_body;
                    patched = true;
                }
            }

            if let Some(ns) = &cfg.name_spoof {
                if ns.enabled && !ns.display_name.trim().is_empty() {
                    let learned_pid = get_learned_player_id();
                    let target_pid = learned_pid.as_deref().or(ns.player_id.as_deref());

                    let (new_body, did_patch) = patch_ws_name_fields(
                        &out_body,
                        ns.display_name.trim(),
                        target_pid,
                    );
                    if did_patch {
                        out_body = new_body;
                        patched = true;
                    }
                }
            }


        }
    }

    let mut resp_builder = Response::builder().status(effective_status.as_u16());
    let mut psy_time = String::new();
    for (k, v) in resp_headers.iter() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower == "psytime" {
            if let Ok(s) = v.to_str() {
                psy_time = s.to_string();
            }
        }
        if k_lower != "content-length"
            && k_lower != "transfer-encoding"
            && k_lower != "content-encoding"
            && (!patched || (k_lower != "psysig" && k_lower != "psysignature"))
        {
            resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
        }
    }

    if patched {
        let sig = resign_rpc_response(&psy_time, &out_body);
        resp_builder = resp_builder.header("PsySig", &sig);
        resp_builder = resp_builder.header("Psysignature", &sig);
    }

    record_network_event(
        "HTTP",
        "INBOUND",
        &path_lower,
        &effective_status.to_string(),
        if patched { "Broker Services RPC (Patched)" } else { "Broker Services RPC" },
        &String::from_utf8_lossy(&out_body),
        patched,
    );

    resp_builder = resp_builder.header("Content-Length", out_body.len().to_string());
    Ok(resp_builder.body(full_body(out_body)).unwrap())
}

async fn handle_request(
    req: Request<Incoming>,
    client: reqwest::Client,
    intercept_client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let path = req.uri().path();
    if path == "/health" || path == "/vrl-health" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain")
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body("OK"))
            .unwrap());
    }

    if path == "/crl/velocityrl.crl" || path == "/velocityrl.crl" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/pkix-crl")
            .header("Content-Length", CA_CRL_DER.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(CA_CRL_DER.to_vec()))
            .unwrap());
    }

    let host_hdr = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    if host_hdr.contains("ws.rlpp.psynet.gg") {
        return handle_websocket(req).await;
    }

    if host_hdr.contains("api.rlpp.psynet.gg") || host_hdr.contains("rlpp.psynet.gg") {
        return handle_broker_request(req, client).await;
    }

    let is_eos = is_eos_account_host(&host_hdr);
    if is_eos || host_hdr.contains("psyonix.com") || host_hdr.contains("live.psynet.gg") {
        let host = hostname_only(&host_hdr).to_string();
        let port = host_header_port(&host_hdr, 443);
        crate::applog::event(&format!(
            "proxy: MITM host {host}:{port} {}",
            req.uri().path()
        ));
        let up_client = if EOS_ACCOUNT_HOSTS
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&host))
        {
            intercept_client
        } else {
            http_client_pinned_for(&host)
        };
        return handle_forward_intercepted_request(req, host, port, up_client).await;
    }

    if is_upgrade {
        let host = hostname_only(&host_hdr).to_string();
        let port = host_header_port(&host_hdr, 443);
        if !host.is_empty() {
            return handle_forward_websocket(req, &host, port).await;
        }
        return handle_websocket(req).await;
    }

    handle_http_config(req, client).await
}

async fn handle_websocket(
    req: Request<Incoming>,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let uri = req.uri();
    let sec_key = match req.headers().get("sec-websocket-key").and_then(|v| v.to_str().ok()) {
        Some(k) => k.to_string(),
        None => {
            let resp = Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full_body("Missing Sec-WebSocket-Key"))
                .unwrap();
            return Ok(resp);
        }
    };

    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(sec_key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept_hash = hasher.finalize();
    let accept_val = base64::engine::general_purpose::STANDARD.encode(accept_hash);

    let mut path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/ws/gc2").to_string();
    if path_and_query.is_empty() || path_and_query == "/" || path_and_query == "/ws" {
        path_and_query = "/ws/gc2".to_string();
    } else if !path_and_query.starts_with('/') {
        path_and_query = format!("/{path_and_query}");
    }

    let upstream_url = format!("wss://ws.rlpp.psynet.gg{}", path_and_query);
    let mut up_builder = match tokio_tungstenite::tungstenite::handshake::client::Request::builder()
        .uri(&upstream_url)
        .header("Host", "ws.rlpp.psynet.gg")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())
    {
        Ok(r) => r,
        Err(e) => {
            crate::applog::event(&format!("proxy: failed to build upstream ws req: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("build ws req error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    let mut has_psy_token = false;
    let mut has_psy_session = false;
    let mut has_psy_build = false;
    let mut has_psy_env = false;

    for (k, v) in req.headers() {
        let name = k.as_str();
        if !name.eq_ignore_ascii_case("host")
            && !name.eq_ignore_ascii_case("connection")
            && !name.eq_ignore_ascii_case("upgrade")
            && !name.eq_ignore_ascii_case("sec-websocket-key")
            && !name.eq_ignore_ascii_case("sec-websocket-version")
            && !name.eq_ignore_ascii_case("sec-websocket-extensions")
            && !name.eq_ignore_ascii_case("origin")
        {
            if name.eq_ignore_ascii_case("psytoken") {
                has_psy_token = true;
            } else if name.eq_ignore_ascii_case("psysessionid") {
                has_psy_session = true;
            } else if name.eq_ignore_ascii_case("psybuildid") {
                has_psy_build = true;
            } else if name.eq_ignore_ascii_case("psyenvironment") {
                has_psy_env = true;
            }
            up_builder.headers_mut().insert(k.clone(), v.clone());
        }
    }

    if !has_psy_token || !has_psy_session {
        if let Some(creds) = LAST_AUTH_WS.lock().unwrap().clone() {
            if creds.timestamp.elapsed().as_secs() < 300 {
                if !has_psy_token {
                    if let Ok(val) = hyper::header::HeaderValue::from_str(&creds.token) {
                        up_builder.headers_mut().insert("PsyToken", val);
                    }
                }
                if !has_psy_session {
                    if let Ok(val) = hyper::header::HeaderValue::from_str(&creds.session_id) {
                        up_builder.headers_mut().insert("PsySessionID", val);
                    }
                }
                crate::applog::event("proxy: injected cached AuthPlayer PsyToken/PsySessionID into upstream WS");
            }
        }
    }

    if !has_psy_build {
        let fallback_build = LAST_GAME_BUILD_ID
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "-1887694083".to_string());
        if let Ok(val) = hyper::header::HeaderValue::from_str(&fallback_build) {
            up_builder.headers_mut().insert("PsyBuildID", val);
            crate::applog::event(&format!("proxy: set WebSocket PsyBuildID: {fallback_build}"));
        }
    }
    if !has_psy_env {
        up_builder.headers_mut().insert("PsyEnvironment", hyper::header::HeaderValue::from_static("Prod"));
    }

    up_builder.headers_mut().insert(
        hyper::header::ORIGIN,
        hyper::header::HeaderValue::from_static("https://ws.rlpp.psynet.gg"),
    );

    let connector = create_upstream_tls_connector();
    let upstream_addr = resolve_ipv4_public("ws.rlpp.psynet.gg");
    let tcp_conn = match upstream_addr {
        Some(addr) => match tokio::net::TcpStream::connect(addr).await {
            Ok(t) => t,
            Err(e) => {
                crate::applog::event(&format!("proxy: connect to resolved ws {addr} failed ({e}), trying DNS ws.rlpp.psynet.gg:443"));
                match tokio::net::TcpStream::connect("ws.rlpp.psynet.gg:443").await {
                    Ok(t) => t,
                    Err(e2) => {
                        crate::applog::event(&format!("proxy: failed to connect to upstream ws: {e2}"));
                        let resp = Response::builder()
                            .status(StatusCode::BAD_GATEWAY)
                            .body(full_body(format!("connect ws upstream error: {e2}")))
                            .unwrap();
                        return Ok(resp);
                    }
                }
            }
        },
        None => {
            match tokio::net::TcpStream::connect("ws.rlpp.psynet.gg:443").await {
                Ok(t) => t,
                Err(e) => {
                    crate::applog::event(&format!("proxy: failed to connect to upstream ws: {e}"));
                    let resp = Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .body(full_body(format!("connect ws upstream error: {e}")))
                        .unwrap();
                    return Ok(resp);
                }
            }
        }
    };

    let server_name = match tokio_rustls::rustls::pki_types::ServerName::try_from("ws.rlpp.psynet.gg".to_string()) {
        Ok(sn) => sn,
        Err(e) => {
            crate::applog::event(&format!("proxy: invalid ServerName: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body("invalid server name"))
                .unwrap();
            return Ok(resp);
        }
    };

    let tls_upstream = match connector.connect(server_name, tcp_conn).await {
        Ok(s) => s,
        Err(e) => {
            crate::applog::event(&format!("proxy: upstream ws tls handshake error: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream ws tls error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    let (upstream_ws, _) = match tokio_tungstenite::client_async(up_builder, tls_upstream).await {
        Ok(pair) => pair,
        Err(e) => {
            crate::applog::event(&format!("proxy: upstream ws handshake failed: {e}"));
            let resp = Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("upstream ws handshake error: {e}")))
                .unwrap();
            return Ok(resp);
        }
    };

    crate::applog::event("proxy: WebSocket upstream connected to ws.rlpp.psynet.gg");

    tokio::spawn(async move {
        let upgraded = match hyper::upgrade::on(req).await {
            Ok(u) => u,
            Err(e) => {
                crate::applog::event(&format!("proxy: client ws upgrade error: {e}"));
                return;
            }
        };

        let client_ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        ).await;

        crate::applog::event("proxy: WebSocket client tunnel established");
        tunnel_websocket(client_ws, upstream_ws).await;
    });

    let resp = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Accept", accept_val)
        .body(empty_body())
        .unwrap();

    Ok(resp)
}

async fn tunnel_websocket<S1, S2>(
    client_ws: tokio_tungstenite::WebSocketStream<S1>,
    upstream_ws: tokio_tungstenite::WebSocketStream<S2>,
) where
    S1: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    S2: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use futures_util::{SinkExt, StreamExt};
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut up_tx, mut up_rx) = upstream_ws.split();

    let (downstream_tx, mut downstream_rx) = tokio::sync::mpsc::unbounded_channel::<tokio_tungstenite::tungstenite::Message>();
    register_client_ws_sender(downstream_tx);

    let c2u = tokio::spawn(async move {
        while let Some(msg_res) = client_rx.next().await {
            match msg_res {
                Ok(msg) => {
                    let out_msg = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => {
                            let (patched_text, did_patch) = patch_ws_frame_text(&t, false).await;
                            record_network_event(
                                "WS_C2S",
                                "OUTBOUND",
                                "ws.rlpp.psynet.gg",
                                "FRAME_TEXT",
                                if did_patch { "WebSocket C2S Frame (Patched)" } else { "WebSocket C2S Frame" },
                                &patched_text,
                                did_patch,
                            );
                            tokio_tungstenite::tungstenite::Message::Text(patched_text.into())
                        }
                        tokio_tungstenite::tungstenite::Message::Binary(b) => {
                            let (patched_b, did_patch) = patch_ws_frame_binary(&b, false).await;
                            record_network_event(
                                "WS_C2S",
                                "OUTBOUND",
                                "ws.rlpp.psynet.gg",
                                "FRAME_BINARY",
                                if did_patch { "WebSocket C2S Binary (Patched)" } else { "WebSocket C2S Binary" },
                                &String::from_utf8_lossy(&patched_b),
                                did_patch,
                            );
                            tokio_tungstenite::tungstenite::Message::Binary(patched_b.into())
                        }
                        other => other,
                    };
                    if up_tx.send(out_msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let u2c = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(msg_res) = up_rx.next() => {
                    match msg_res {
                        Ok(msg) => {
                            let out_msg = match msg {
                                tokio_tungstenite::tungstenite::Message::Text(t) => {
                                    if t.contains("ReservationsReadyMessage") || t.contains("ClientReservationMessage") {
                                        record_network_event("WS_S2C", "INBOUND", "ws.rlpp.psynet.gg",
                                            "ORIGINAL_FRAME_TEXT", "Original server reservation before local changes", &t, false);
                                    }
                                    let (patched_text, did_patch) = patch_ws_frame_text(&t, true).await;
                                    record_network_event(
                                        "WS_S2C",
                                        "INBOUND",
                                        "ws.rlpp.psynet.gg",
                                        "FRAME_TEXT",
                                        if did_patch { "WebSocket S2C Frame (Patched)" } else { "WebSocket S2C Frame" },
                                        &patched_text,
                                        did_patch,
                                    );
                                    tokio_tungstenite::tungstenite::Message::Text(patched_text.into())
                                }
                                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                                    let (patched_b, did_patch) = patch_ws_frame_binary(&b, true).await;
                                    record_network_event(
                                        "WS_S2C",
                                        "INBOUND",
                                        "ws.rlpp.psynet.gg",
                                        "FRAME_BINARY",
                                        if did_patch { "WebSocket S2C Binary (Patched)" } else { "WebSocket S2C Binary" },
                                        &String::from_utf8_lossy(&patched_b),
                                        did_patch,
                                    );
                                    tokio_tungstenite::tungstenite::Message::Binary(patched_b.into())
                                }
                                other => other,
                            };
                            if client_tx.send(out_msg).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                Some(inject_msg) = downstream_rx.recv() => {
                    if client_tx.send(inject_msg).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    tokio::select! {
        _ = c2u => {},
        _ = u2c => {},
    }
    unregister_client_ws_sender();
    crate::applog::event("proxy: WebSocket tunnel closed");
}

async fn patch_ws_frame_text(text: &str, from_server: bool) -> (String, bool) {
    let (patched_bytes, changed) = patch_ws_frame_binary(text.as_bytes(), from_server).await;
    if changed {
        if let Ok(s) = String::from_utf8(patched_bytes) {
            return (s, true);
        }
    }
    (text.to_string(), false)
}

static PENDING_UNLOCK_REQUESTS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub async fn patch_ws_frame_binary(frame: &[u8], from_server: bool) -> (Vec<u8>, bool) {
    let Some(hdr_end) = find_bytes(frame, b"\r\n\r\n") else {
        return (frame.to_vec(), false);
    };

    let headers_part = &frame[..hdr_end];
    let body_part = &frame[hdr_end + 4..];

    let mut current_headers = headers_part.to_vec();
    let mut current_body = body_part.to_vec();
    let mut any_changed = false;

    let req_id = get_ws_header_value(headers_part, "PsyRequestID");
    let resp_id = get_ws_header_value(headers_part, "PsyResponseID");

    let svc = get_ws_header_value(headers_part, "PsyService");
    let svc_lower = svc.to_ascii_lowercase();
    let is_skill = svc_lower.contains("getplayerskill")
        || svc_lower.contains("getplayerskills")
        || svc_lower.contains("skills/")
        || find_bytes(body_part, b"\"Skills\"").is_some()
        || (find_bytes(body_part, b"\"Mu\"").is_some()
            && (find_bytes(body_part, b"\"Tier\"").is_some() || find_bytes(body_part, b"\"Playlist\"").is_some()));

    if is_skill {
        extract_and_save_real_skills(body_part);
    }

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };

    let is_reservation = find_bytes(body_part, b"ServerAddress").is_some()
        || find_bytes(body_part, b"GameURL").is_some()
        || find_bytes(body_part, b"PingURL").is_some()
        || find_bytes(body_part, b"ReservationsReadyMessage_X").is_some()
        || find_bytes(body_part, b"ClientReservationMessage_X").is_some()
        || find_bytes(body_part, b"AddReservationMessage").is_some()
        || find_bytes(body_part, b"DSR/ClientMessage").is_some()
        || find_bytes(body_part, b"DSR/RelayToServer").is_some()
        || find_bytes(body_part, b"PlayWithLoadout").is_some()
        || find_bytes(body_part, b"\"Loadout\"").is_some()
        || find_bytes(body_part, b"\\\"Loadout\\\"").is_some();
    if is_reservation {
        let swap_rules = load_active_swap_rules(cfg_opt.as_ref());
        let (new_body, changed) = patch_reservation_json_with_rules(&current_body, &swap_rules).await;
        if changed {
            current_body = new_body;
            any_changed = true;
        }
    }

    let body_lower = String::from_utf8_lossy(body_part).to_ascii_lowercase();
    let is_unlock_container_req = svc_lower.contains("unlockcontainer")
        || svc_lower.contains("opencontainer")
        || body_lower.contains("unlockcontainer")
        || body_lower.contains("opencontainer")
        || (body_lower.contains("instanceids") && body_lower.contains("keyinstanceids"));

    if !from_server && is_unlock_container_req && !req_id.is_empty() {
        crate::applog::event(&format!("proxy: tracked outbound UnlockContainer request ID '{req_id}' service='{svc}'"));
        if let Ok(mut pending) = PENDING_UNLOCK_REQUESTS.lock() {
            pending.insert(req_id.clone(), body_part.to_vec());
        }
    }

    let pending_req_body = if from_server && !resp_id.is_empty() {
        if let Ok(mut pending) = PENDING_UNLOCK_REQUESTS.lock() {
            pending.remove(&resp_id)
        } else {
            None
        }
    } else {
        None
    };

    let is_unlock_container_resp = pending_req_body.is_some()
        || (from_server && (svc_lower.contains("unlockcontainer")
            || svc_lower.contains("opencontainer")
            || body_lower.contains("unlockcontainer")
            || body_lower.contains("opencontainer")));

    if is_unlock_container_resp {
        let unlock_input_body = pending_req_body.as_deref().unwrap_or(body_part);
        crate::applog::event(&format!(
            "proxy: WS unlock container response intercepted (resp_id='{resp_id}') -> rewriting to 200 OK with authentic drop"
        ));
        if let Some(resp_json) = generate_simulated_unlock_container_payload(unlock_input_body) {
            let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
            current_body = resp_bytes;
            current_headers = replace_ws_header_value(&current_headers, "PsyStatus", "200");
            let now_str = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1755399374));
            current_headers = replace_ws_header_value(&current_headers, "PsyTime", &now_str);
            any_changed = true;
            crate::applog::event("proxy: WS authentic Products/UnlockContainer -> drop granted & registered to inventory (PsyStatus 200)");
        }
    }

    if from_server && (svc_lower.contains("getcontainerdroptable") || body_lower.contains("getcontainerdroptable")) {
        let resp_bytes = get_authoritative_container_drop_table_response_bytes();
        current_body = resp_bytes;
        current_headers = replace_ws_header_value(&current_headers, "PsyStatus", "200");
        let now_str = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1755399374));
        current_headers = replace_ws_header_value(&current_headers, "PsyTime", &now_str);
        any_changed = true;
        crate::applog::event("proxy: WS intercepted Products/GetContainerDropTable -> served authoritative table (PsyStatus 200)");
    }

    let (new_body_sync, did_patch_sync) = neutralize_inventory_out_of_sync_json(&current_body);
    if did_patch_sync {
        current_body = new_body_sync;
        any_changed = true;
    }

    let is_products = svc_lower.contains("getplayerproducts")
        || svc_lower.contains("getloadoutproducts")
        || svc_lower.contains("checkownership")
        || svc_lower.contains("verifyownership")
        || svc_lower.contains("getuserproducts")
        || svc_lower.contains("crossentitlement")
        || svc_lower.contains("entitlement")
        || svc_lower.contains("ecom")
        || svc_lower.contains("dlc")
        || svc_lower.contains("ownership")
        || find_bytes(body_part, b"\"ProductData\"").is_some()
        || find_bytes(body_part, b"\"Products\"").is_some()
        || (find_bytes(body_part, b"\"ProductID\"").is_some()
            && find_bytes(body_part, b"\"InstanceID\"").is_some());

    if is_products {
        let spawned_items = load_active_spawned_items(cfg_opt.as_ref());
        if !spawned_items.is_empty() || !spawned_title_ids().is_empty() {
            let (new_body, changed) = patch_player_products_json(&current_body, &spawned_items);
            if changed {
                current_body = new_body;
                any_changed = true;
            }
        }
        let (new_body_own, changed_own) = patch_check_ownership_json(&current_body, &spawned_items);
        if changed_own {
            current_body = new_body_own;
            any_changed = true;
        }
    }

    let is_cross_entitlement = svc_lower.contains("crossentitlement")
        || find_bytes(body_part, b"CrossEntitledProductIDs").is_some();

    if is_cross_entitlement {
        let spawned_items = load_active_spawned_items(cfg_opt.as_ref());
        if !spawned_items.is_empty() {
            let (new_body, changed) = patch_cross_entitlement_json(&current_body, &spawned_items);
            if changed {
                current_body = new_body;
                any_changed = true;
            }
        }
    }

    if is_skill {
        if let Some(fr) = cfg_opt.as_ref().and_then(|c| c.fake_ranks.as_ref()) {
            if fr.enabled {
                let (new_body, changed) = patch_get_player_skill_json(&current_body, fr);
                if changed {
                    current_body = new_body;
                    any_changed = true;
                }
            }
        }
    }

    if let Some(cfg) = &cfg_opt {
        let is_wallet = svc_lower.contains("getplayerwallet")
        || svc_lower.contains("wallet")
        || svc_lower.contains("tournament")
        || svc_lower.contains("currencies")
        || svc_lower.contains("shops")
        || find_bytes(body_part, b"\"Currencies\"").is_some()
        || find_bytes(body_part, b"TournamentCredit").is_some()
        || find_bytes(body_part, b"TournamentPoint").is_some();

    if is_wallet {
        if let Some(cs) = &cfg.credit_spoof {
            if cs.enabled {
                let (new_body, changed) = patch_player_wallet_json(&current_body, cs);
                if changed {
                    current_body = new_body;
                    any_changed = true;
                }
            }
        }
    }

    let is_can_show_avatar = svc_lower.contains("canshowavatar")
        || find_bytes(body_part, b"\"AllowedPlayerIDs\"").is_some()
        || find_bytes(body_part, b"\"HiddenPlayerIDs\"").is_some();

    if is_can_show_avatar {
        let (new_body, changed) = patch_users_can_show_avatar_json(&current_body);
        if changed {
            current_body = new_body;
            any_changed = true;
        }
    }

    // Leaderboard spoofing disabled per user request
    let is_leaderboard = false;

    if is_leaderboard {
        let (new_body, changed) = patch_leaderboard_json(&current_body, &cfg);
        if changed {
            current_body = new_body;
            any_changed = true;
        }
    }

    if let Some(ns) = &cfg.name_spoof {
        if ns.enabled && !ns.display_name.trim().is_empty() {
            if !is_loadout_sensitive(&svc, &current_body) {
                let learned_pid = get_learned_player_id();
                let target_pid = learned_pid.as_deref().or(ns.player_id.as_deref());

                let (new_body, changed) = patch_ws_name_fields(
                    &current_body,
                    ns.display_name.trim(),
                    target_pid,
                );
                if changed {
                    current_body = new_body;
                    any_changed = true;
                }
            }
        }
    }
    }

    if !any_changed {
        return (frame.to_vec(), false);
    }

    let new_headers = resign_ws_headers(&current_headers, &current_body);
    let mut out = Vec::with_capacity(new_headers.len() + 4 + current_body.len());
    out.extend_from_slice(&new_headers);
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(&current_body);

    (out, true)
}

fn get_ws_header_value(headers: &[u8], key: &str) -> String {
    let text = String::from_utf8_lossy(headers);
    let key_lower = key.to_ascii_lowercase();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(&key_lower) {
                return v.trim().to_string();
            }
        }
    }
    String::new()
}

fn replace_ws_header_value(headers: &[u8], key: &str, new_val: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(headers);
    let mut out_lines = Vec::new();
    let key_lower = key.to_ascii_lowercase();
    let mut replaced = false;

    for line in text.lines() {
        if let Some((k, _)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(&key_lower) {
                out_lines.push(format!("{key}: {new_val}"));
                replaced = true;
                continue;
            }
        }
        out_lines.push(line.to_string());
    }

    if !replaced {
        out_lines.push(format!("{key}: {new_val}"));
    }

    out_lines.join("\r\n").into_bytes()
}

fn resign_ws_headers(headers: &[u8], body: &[u8]) -> Vec<u8> {
    let mut hdrs = headers.to_vec();
    if !get_ws_header_value(&hdrs, "Content-Length").is_empty() {
        hdrs = replace_ws_header_value(&hdrs, "Content-Length", &body.len().to_string());
    }

    let psy_time = get_ws_header_value(&hdrs, "PsyTime");
    let has_psysig = find_bytes(&hdrs, b"PsySig:").is_some() || find_bytes(&hdrs, b"psysig:").is_some();
    let has_psysignature = find_bytes(&hdrs, b"Psysignature:").is_some() || find_bytes(&hdrs, b"psysignature:").is_some();

    if !has_psysig && !has_psysignature {
        return hdrs;
    }

    let sig = if !psy_time.is_empty() {
        let mut m = Hmac::<Sha256>::new_from_slice(PSY_RESP_KEY).expect("valid hmac key");
        m.update(format!("{psy_time}-").as_bytes());
        m.update(body);
        base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
    } else {
        let mut m = Hmac::<Sha256>::new_from_slice(PSY_REQ_KEY).expect("valid hmac key");
        m.update(b"-");
        m.update(body);
        base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
    };

    if has_psysig {
        hdrs = replace_ws_header_value(&hdrs, "PsySig", &sig);
    }
    if has_psysignature {
        hdrs = replace_ws_header_value(&hdrs, "Psysignature", &sig);
    }
    hdrs
}

#[allow(dead_code)]
fn is_leaderboard_body(body: &[u8]) -> bool {
    let trim = body.trim_ascii();
    find_bytes(trim, b"\"LeaderboardID\"").is_some()
        || find_bytes(trim, b"\"LeaderboardRows\"").is_some()
        || find_bytes(trim, b"\"Leaderboard\"").is_some()
        || find_bytes(trim, b"\"Leaderboards\"").is_some()
        || find_bytes(trim, b"\"TopPlayers\"").is_some()
        || find_bytes(trim, b"\"Platforms\"").is_some()
        || find_bytes(trim, b"\"bHasSkill\"").is_some()
        || find_bytes(trim, b"\"bHasValue\"").is_some()
        || (find_bytes(trim, b"\"Rows\"").is_some() && (find_bytes(trim, b"\"Rank\"").is_some() || find_bytes(trim, b"\"Value\"").is_some() || find_bytes(trim, b"\"Rating\"").is_some()))
        || (find_bytes(trim, b"\"Entries\"").is_some() && (find_bytes(trim, b"\"Rank\"").is_some() || find_bytes(trim, b"\"Value\"").is_some() || find_bytes(trim, b"\"Rating\"").is_some()))
        || (find_bytes(trim, b"\"Players\"").is_some() && (find_bytes(trim, b"\"MMR\"").is_some() || find_bytes(trim, b"\"Value\"").is_some()))
}

fn patch_leaderboard_json(
    body: &[u8],
    cfg: &crate::psynet::SpoofPayload,
) -> (Vec<u8>, bool) {
    let fake_ranks_opt = cfg.fake_ranks.as_ref().filter(|fr| fr.enabled);
    let lb_spoof_opt = cfg.leaderboard_spoof.as_ref().filter(|lb| lb.enabled);

    if fake_ranks_opt.is_none() && lb_spoof_opt.is_none() {
        return (body.to_vec(), false);
    }

    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let result_obj = if let Some(r) = root.get_mut("Result") {
        r
    } else if let Some(d) = root.get_mut("Data") {
        d
    } else {
        &mut root
    };

    let mut pl = 11; // Default to Doubles 2v2
    if let Some(id_str) = result_obj.get("LeaderboardID").and_then(|v| v.as_str()) {
        let num_str = id_str.trim_start_matches("Skill").trim_start_matches("skill");
        if let Ok(n) = num_str.parse::<i32>() {
            pl = n;
        }
    } else if let Some(p_val) = result_obj.get("Playlist").or_else(|| result_obj.get("PlaylistID")) {
        if let Some(n) = p_val.as_i64() {
            pl = n as i32;
        } else if let Some(s) = p_val.as_str() {
            if let Ok(n) = s.parse::<i32>() {
                pl = n;
            }
        }
    }

    let mut target_display_mmr = 2150.0;
    let mut target_tier = 22; // Supersonic Legend default
    let mut custom_rank: Option<i64> = None;
    let mut found_override = false;

    if let Some(lb) = lb_spoof_opt {
        found_override = true;
        if let Some(cr) = lb.custom_rank {
            custom_rank = Some(cr as i64);
        }
        if !lb.sync_from_fake_ranks {
            if let Some(cm) = lb.custom_mmr {
                target_display_mmr = cm as f64;
            }
        }
    }

    if let Some(fr) = fake_ranks_opt {
        if let Some(ov) = get_playlist_override(fr, pl) {
            found_override = true;
            if let Some(disp) = ov.display_mmr {
                target_display_mmr = disp;
            } else if let Some(mu) = ov.mu {
                target_display_mmr = (mu * 20.0 + 100.0).max(0.0);
            }
            if let Some(tier) = ov.tier {
                target_tier = tier;
            }
        }
    }

    if !found_override && lb_spoof_opt.is_none() {
        return (body.to_vec(), false);
    }

    let target_mu = mu_from_display(target_display_mmr);
    let disp_int = target_display_mmr.round() as i64;

    // Calculate auto rank if custom rank is not explicitly set
    let assigned_rank = if let Some(cr) = custom_rank {
        cr
    } else {
        // Inspect Platforms, Players, or Rows in leaderboard to find where target ranks
        let mut computed_rank = 1;
        let mut found_slot = false;

        let players_opt = result_obj.get("Platforms")
            .and_then(|p| p.as_array())
            .and_then(|p_arr| p_arr.first())
            .and_then(|p0| p0.get("Players").and_then(|pl| pl.as_array()))
            .or_else(|| result_obj.get("Players").and_then(|pl| pl.as_array()));

        if let Some(players) = players_opt {
            for (i, p) in players.iter().enumerate() {
                let p_mmr = p.get("MMR").and_then(|v| v.as_f64())
                    .or_else(|| p.get("Value").and_then(|v| v.as_f64()))
                    .unwrap_or(0.0);
                if target_mu >= p_mmr || target_display_mmr >= p_mmr {
                    computed_rank = (i + 1) as i64;
                    found_slot = true;
                    break;
                }
            }
            if !found_slot {
                computed_rank = (players.len() + 1) as i64;
            }
            computed_rank
        } else {
            let rows_opt = result_obj.get("Rows").and_then(|r| r.as_array())
                .or_else(|| result_obj.get("Entries").and_then(|r| r.as_array()))
                .or_else(|| result_obj.get("LeaderboardRows").and_then(|r| r.as_array()));

            if let Some(rows) = rows_opt {
                for (i, row) in rows.iter().enumerate() {
                    let row_val = row.get("Value").and_then(|v| v.as_f64())
                        .or_else(|| row.get("Rating").and_then(|v| v.as_f64()))
                        .or_else(|| row.get("Score").and_then(|v| v.as_f64()))
                        .or_else(|| {
                            row.get("MMR").and_then(|v| v.as_f64()).map(|m| {
                                if m < 250.0 { m * 20.0 + 100.0 } else { m }
                            })
                        })
                        .unwrap_or(0.0);

                    if target_display_mmr >= row_val {
                        computed_rank = (i + 1) as i64;
                        found_slot = true;
                        break;
                    }
                }
                if !found_slot {
                    computed_rank = (rows.len() + 1) as i64;
                }
                computed_rank
            } else if target_display_mmr >= 1900.0 || target_tier >= 22 {
                1
            } else if target_display_mmr >= 1500.0 {
                50
            } else {
                1
            }
        }
    };

    let player_name = cfg.name_spoof.as_ref()
        .map(|n| n.display_name.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| get_learned_real_name())
        .unwrap_or_else(|| "You".to_string());

    let learned_pid = get_learned_player_id();
    let target_pid = learned_pid.as_deref().or_else(|| {
        cfg.name_spoof.as_ref().and_then(|ns| ns.player_id.as_deref())
    });
    let user_pid_str = target_pid.unwrap_or("Epic|local_user|0").to_string();
    let clean_pid = user_pid_str.trim_start_matches("Epic|").trim_start_matches("Steam|").trim_start_matches("Xbox|").trim_start_matches("PS4|").trim_end_matches("|0");

    let mut changed = false;

    // 1. Official Skills/GetSkillLeaderboard v1 & Stats/GetStatLeaderboard v1 schema (Platforms -> Players)
    let platforms_key = if result_obj.get("Platforms").is_some() {
        Some("Platforms")
    } else if result_obj.get("platforms").is_some() {
        Some("platforms")
    } else {
        None
    };

    if let Some(key) = platforms_key {
        if let Some(platforms_arr) = result_obj.get_mut(key).and_then(|p| p.as_array_mut()) {
            for plat in platforms_arr.iter_mut() {
                let players_key = if plat.get("Players").is_some() {
                    Some("Players")
                } else if plat.get("players").is_some() {
                    Some("players")
                } else {
                    None
                };

                if let Some(pkey) = players_key {
                    if let Some(players) = plat.get_mut(pkey).and_then(|pl| pl.as_array_mut()) {
                        players.retain(|p| {
                            let r_pid = p.get("PlayerID").or_else(|| p.get("playerID")).and_then(|v| v.as_str()).unwrap_or("");
                            if clean_pid != "local_user" && !clean_pid.is_empty() && (r_pid.contains(clean_pid) || clean_pid.contains(r_pid)) {
                                return false;
                            }
                            let r_name = p.get("PlayerName").or_else(|| p.get("playerName")).and_then(|v| v.as_str()).unwrap_or("");
                            if !player_name.is_empty() && player_name != "You" && r_name.eq_ignore_ascii_case(&player_name) {
                                return false;
                            }
                            true
                        });

                        let skill_player_obj = serde_json::json!({
                            "PlayerID": user_pid_str,
                            "PlayerName": player_name,
                            "MMR": target_mu,
                            "Value": target_tier
                        });

                        let insert_idx = ((assigned_rank - 1).max(0) as usize).min(players.len());
                        players.insert(insert_idx, skill_player_obj);
                        if players.len() > 100 {
                            players.truncate(100);
                        }
                        changed = true;
                    }
                }
            }
        }
    }

    // 2. Direct Players array (e.g. Stats/GetStatLeaderboardRankForUsers or flat list)
    let players_key = if result_obj.get("Players").is_some() {
        Some("Players")
    } else if result_obj.get("players").is_some() {
        Some("players")
    } else {
        None
    };

    if let Some(pkey) = players_key {
        if let Some(players_arr) = result_obj.get_mut(pkey).and_then(|pl| pl.as_array_mut()) {
            players_arr.retain(|p| {
                let r_pid = p.get("PlayerID").or_else(|| p.get("playerID")).and_then(|v| v.as_str()).unwrap_or("");
                if clean_pid != "local_user" && !clean_pid.is_empty() && (r_pid.contains(clean_pid) || clean_pid.contains(r_pid)) {
                    return false;
                }
                let r_name = p.get("PlayerName").or_else(|| p.get("playerName")).and_then(|v| v.as_str()).unwrap_or("");
                if !player_name.is_empty() && player_name != "You" && r_name.eq_ignore_ascii_case(&player_name) {
                    return false;
                }
                true
            });

            let player_obj = serde_json::json!({
                "PlayerID": user_pid_str,
                "PlayerName": player_name,
                "MMR": target_mu,
                "Value": target_tier
            });

            let insert_idx = ((assigned_rank - 1).max(0) as usize).min(players_arr.len());
            players_arr.insert(insert_idx, player_obj);
            if players_arr.len() > 100 {
                players_arr.truncate(100);
            }
            changed = true;
        }
    }

    let has_platforms = platforms_key.is_some();

    // 3. Official Skills/GetSkillLeaderboardValueForUser v1 & Stats/GetStatLeaderboardValueForUser v1 (single user value queries or legacy format)
    if !has_platforms {
        let is_skill_lb = result_obj.get("LeaderboardID").and_then(|v| v.as_str()).map(|s| s.starts_with("Skill") || s.starts_with("skill")).unwrap_or(true) || result_obj.get("bHasSkill").is_some();
        if is_skill_lb {
            result_obj["bHasSkill"] = serde_json::json!(true);
            result_obj["MMR"] = serde_json::json!(target_mu);
            result_obj["Value"] = serde_json::json!(target_tier);
            result_obj["Tier"] = serde_json::json!(target_tier);
        } else {
            result_obj["bHasValue"] = serde_json::json!(true);
            result_obj["Value"] = serde_json::json!(disp_int);
        }

        result_obj["Rank"] = serde_json::json!(assigned_rank);
        result_obj["UserRank"] = serde_json::json!(assigned_rank);
        result_obj["Position"] = serde_json::json!(assigned_rank);
        result_obj["UserPosition"] = serde_json::json!(assigned_rank);
        changed = true;
    }

    // 4. Rows / Entries array (legacy or custom format)
    for key in &["Rows", "rows", "Entries", "entries", "LeaderboardRows", "leaderboardRows"] {
        if let Some(rows_arr) = result_obj.get_mut(*key).and_then(|r| r.as_array_mut()) {
            if !rows_arr.is_empty() {
                rows_arr.retain(|row| {
                    let r_pid = row.get("PlayerID").or_else(|| row.get("playerID")).and_then(|v| v.as_str()).unwrap_or("");
                    if clean_pid != "local_user" && !clean_pid.is_empty() && (r_pid.contains(clean_pid) || clean_pid.contains(r_pid)) {
                        return false;
                    }
                    let r_name = row.get("PlayerName").or_else(|| row.get("playerName")).or_else(|| row.get("UserName")).or_else(|| row.get("userName")).and_then(|v| v.as_str()).unwrap_or("");
                    if !player_name.is_empty() && player_name != "You" && r_name.eq_ignore_ascii_case(&player_name) {
                        return false;
                    }
                    true
                });

                let user_row_obj = serde_json::json!({
                    "PlayerID": user_pid_str,
                    "playerID": user_pid_str,
                    "PlayerName": player_name,
                    "playerName": player_name,
                    "UserName": player_name,
                    "userName": player_name,
                    "Name": player_name,
                    "name": player_name,
                    "Rank": assigned_rank,
                    "rank": assigned_rank,
                    "UserRank": assigned_rank,
                    "userRank": assigned_rank,
                    "Value": disp_int,
                    "value": disp_int,
                    "Rating": disp_int,
                    "rating": disp_int,
                    "Score": disp_int,
                    "score": disp_int,
                    "Tier": target_tier,
                    "tier": target_tier,
                    "Division": 3,
                    "division": 3,
                    "MMR": target_mu,
                    "mmr": target_mu,
                    "MatchesPlayed": 100,
                    "matchesPlayed": 100,
                    "bHasSkill": true
                });

                let insert_idx = ((assigned_rank - 1).max(0) as usize).min(rows_arr.len());
                rows_arr.insert(insert_idx, user_row_obj);

                if rows_arr.len() > 100 {
                    rows_arr.truncate(100);
                }

                for (idx, row) in rows_arr.iter_mut().enumerate() {
                    let r_num = (idx + 1) as i64;
                    row["Rank"] = serde_json::json!(r_num);
                    if row.get("rank").is_some() {
                        row["rank"] = serde_json::json!(r_num);
                    }
                }
                changed = true;
            }
        }
    }

    // 5. UserRow / userRow (legacy user-specific row object)
    for ukey in &["UserRow", "userRow", "UserEntry", "userEntry"] {
        if let Some(user_row) = result_obj.get_mut(*ukey).and_then(|u| u.as_object_mut()) {
            user_row.insert("PlayerID".to_string(), serde_json::json!(user_pid_str));
            user_row.insert("PlayerName".to_string(), serde_json::json!(player_name));
            user_row.insert("Rank".to_string(), serde_json::json!(assigned_rank));
            user_row.insert("Value".to_string(), serde_json::json!(disp_int));
            user_row.insert("Tier".to_string(), serde_json::json!(target_tier));
            user_row.insert("MMR".to_string(), serde_json::json!(target_mu));
            changed = true;
        }
    }

    if !changed {
        return (body.to_vec(), false);
    }

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn patch_get_player_skill_json(
    body: &[u8],
    fake_ranks: &crate::psynet::FakeRanksPayload,
) -> (Vec<u8>, bool) {
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let result_obj = if let Some(r) = root.get_mut("Result") {
        r
    } else {
        &mut root
    };

    let mut changed = false;

    if let Some(skills_arr) = result_obj.get_mut("Skills").and_then(|s| s.as_array_mut()) {
        for skill_val in skills_arr {
            let pl = get_playlist_id(skill_val);
            if let Some(ov) = get_playlist_override(fake_ranks, pl) {
                if apply_rank_override(skill_val, ov) {
                    changed = true;
                }
            }
        }
    } else if let Some(players_arr) = result_obj.get_mut("Players").and_then(|p| p.as_array_mut()) {
        for player_val in players_arr {
            if let Some(skills_arr) = player_val.get_mut("Skills").and_then(|s| s.as_array_mut()) {
                for skill_val in skills_arr {
                    let pl = get_playlist_id(skill_val);
                    if let Some(ov) = get_playlist_override(fake_ranks, pl) {
                        if apply_rank_override(skill_val, ov) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    if let Some(rl) = &fake_ranks.reward_levels {
        if let Some(reward_obj) = result_obj.get_mut("RewardLevels") {
            if let Some(lvl) = rl.season_level {
                reward_obj["SeasonLevel"] = serde_json::json!(lvl);
                changed = true;
            }
            if let Some(wins) = rl.season_level_wins {
                reward_obj["SeasonLevelWins"] = serde_json::json!(wins);
                changed = true;
            }
        }
    }

    if !changed {
        return (body.to_vec(), false);
    }

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn patch_player_wallet_json(
    body: &[u8],
    credit_spoof: &crate::psynet::CreditSpoofPayload,
) -> (Vec<u8>, bool) {
    if !credit_spoof.enabled {
        return (body.to_vec(), false);
    }
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    fn patch_currency_object(
        curr: &mut serde_json::Value,
        credit_spoof: &crate::psynet::CreditSpoofPayload,
    ) -> (bool, Option<i64>) {
        let mut modified = false;
        let mut identified_id: Option<i64> = None;

        let id_val = curr.get("ID")
            .or_else(|| curr.get("CurrencyID"))
            .or_else(|| curr.get("CurrencyId"))
            .or_else(|| curr.get("Id"))
            .or_else(|| curr.get("id"))
            .or_else(|| curr.get("currencyId"))
            .or_else(|| curr.get("Currency"));

        if let Some(v) = id_val {
            if let Some(num) = v.as_i64() {
                identified_id = Some(num);
            } else if let Some(s) = v.as_str() {
                identified_id = s.parse::<i64>().ok();
            }
        }

        let name_str = curr.get("Name")
            .or_else(|| curr.get("CurrencyType"))
            .or_else(|| curr.get("Type"))
            .or_else(|| curr.get("Slug"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let is_tourney = identified_id.map(|id| (14..=40).contains(&id)).unwrap_or(false)
            || name_str.contains("tournament")
            || name_str.contains("tourney");

        let is_shop_credits = identified_id.map(|id| id == 13).unwrap_or(false)
            || (!is_tourney && (name_str.contains("credit") || name_str.contains("shop")));

        let target_amount = if is_tourney {
            Some(credit_spoof.tournament_amount)
        } else if is_shop_credits {
            Some(credit_spoof.amount)
        } else {
            None
        };

        if let Some(amt) = target_amount {
            let mut amt_set = false;
            for key in &["Amount", "amount", "Count", "count", "Value", "value"] {
                if curr.get(*key).is_some() {
                    curr[*key] = serde_json::json!(amt);
                    modified = true;
                    amt_set = true;
                }
            }
            if !amt_set {
                curr["Amount"] = serde_json::json!(amt);
                modified = true;
            }
        }

        (modified, identified_id)
    }

    fn patch_currency_array(
        arr: &mut Vec<serde_json::Value>,
        credit_spoof: &crate::psynet::CreditSpoofPayload,
    ) -> bool {
        let mut local_changed = false;
        let mut present_ids = std::collections::HashSet::new();

        for curr in arr.iter_mut() {
            let (m, id_opt) = patch_currency_object(curr, credit_spoof);
            if m {
                local_changed = true;
            }
            if let Some(id) = id_opt {
                present_ids.insert(id);
            }
        }

        if !present_ids.contains(&13) {
            arr.push(serde_json::json!({
                "ID": 13,
                "Amount": credit_spoof.amount,
                "ExpirationTime": null,
                "UpdatedTimestamp": 1700000000,
                "IsTradable": true,
                "TradeHold": null
            }));
            local_changed = true;
        }

        for tourney_id in 14..=26 {
            if !present_ids.contains(&tourney_id) {
                arr.push(serde_json::json!({
                    "ID": tourney_id,
                    "Amount": credit_spoof.tournament_amount,
                    "ExpirationTime": null,
                    "UpdatedTimestamp": 1700000000,
                    "IsTradable": false,
                    "TradeHold": null
                }));
                local_changed = true;
            }
        }

        local_changed
    }

    fn traverse_and_patch(
        val: &mut serde_json::Value,
        credit_spoof: &crate::psynet::CreditSpoofPayload,
    ) -> bool {
        let mut modded = false;
        match val {
            serde_json::Value::Object(map) => {
                if let Some(currencies_val) = map.get_mut("Currencies") {
                    if let Some(arr) = currencies_val.as_array_mut() {
                        if patch_currency_array(arr, credit_spoof) {
                            modded = true;
                        }
                    } else if let Some(cur_map) = currencies_val.as_object_mut() {
                        cur_map.insert("13".to_string(), serde_json::json!(credit_spoof.amount));
                        for tid in 14..=26 {
                            cur_map.insert(tid.to_string(), serde_json::json!(credit_spoof.tournament_amount));
                        }
                        modded = true;
                    }
                }

                for (k, v) in map.iter_mut() {
                    let k_lower = k.to_ascii_lowercase();
                    if k_lower == "tournamentcredits"
                        || k_lower == "tournamentpoints"
                        || k_lower == "tournamentcreditsamount"
                        || k_lower == "tournamentcredit"
                    {
                        if v.is_number() || v.is_string() {
                            *v = serde_json::json!(credit_spoof.tournament_amount);
                            modded = true;
                        }
                    } else if k_lower == "credits" || k_lower == "creditsamount" || k_lower == "itemshopcredits" {
                        if v.is_number() {
                            *v = serde_json::json!(credit_spoof.amount);
                            modded = true;
                        }
                    } else if v.is_object() || v.is_array() {
                        if traverse_and_patch(v, credit_spoof) {
                            modded = true;
                        }
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                let looks_like_currencies = arr.iter().any(|item| {
                    item.get("ID").is_some() || item.get("CurrencyID").is_some() || item.get("IsTradable").is_some()
                });
                if looks_like_currencies {
                    if patch_currency_array(arr, credit_spoof) {
                        modded = true;
                    }
                } else {
                    for item in arr.iter_mut() {
                        if traverse_and_patch(item, credit_spoof) {
                            modded = true;
                        }
                    }
                }
            }
            _ => {}
        }
        modded
    }

    let mut changed = traverse_and_patch(&mut root, credit_spoof);
    if !changed {
        if let Some(obj) = root.as_object_mut() {
            let mut list = Vec::new();
            patch_currency_array(&mut list, credit_spoof);
            obj.insert("Currencies".to_string(), serde_json::Value::Array(list));
            changed = true;
        }
    }

    if !changed {
        return (body.to_vec(), false);
    }
    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn get_playlist_id(skill: &serde_json::Value) -> i32 {
    skill.get("Playlist").and_then(|v| {
        v.as_i64().map(|n| n as i32).or_else(|| {
            v.as_str().and_then(|s| s.parse::<i32>().ok())
        })
    }).unwrap_or(0)
}

fn get_playlist_override<'a>(
    fake_ranks: &'a crate::psynet::FakeRanksPayload,
    playlist: i32,
) -> Option<&'a crate::psynet::FakeRankOverridePayload> {
    if let Some(playlists) = &fake_ranks.playlists {
        let pl_str = playlist.to_string();
        if let Some(ov) = playlists.get(&pl_str) {
            return Some(ov);
        }
    }
    fake_ranks.default.as_ref()
}

fn apply_rank_override(
    skill: &mut serde_json::Value,
    ov: &crate::psynet::FakeRankOverridePayload,
) -> bool {
    let mut modified = false;
    if let Some(disp) = ov.display_mmr {
        let mu = mu_from_display(disp);
        skill["Mu"] = serde_json::json!(mu);
        skill["MMR"] = serde_json::json!(mu);
        modified = true;
    } else if let Some(mu) = ov.mu {
        let clamped_mu = mu_from_display(mu * 20.0 + 100.0);
        skill["Mu"] = serde_json::json!(clamped_mu);
        skill["MMR"] = serde_json::json!(clamped_mu);
        modified = true;
    }

    if let Some(sigma) = ov.sigma {
        skill["Sigma"] = serde_json::json!(sigma);
        modified = true;
    }
    if let Some(tier) = ov.tier {
        skill["Tier"] = serde_json::json!(tier);
        if tier > 0 {
            let current_matches = skill.get("MatchesPlayed").and_then(|v| v.as_i64()).unwrap_or(0);
            if current_matches < 10 {
                skill["MatchesPlayed"] = serde_json::json!(100);
            }
            skill["PlacementMatches"] = serde_json::json!(10);
            skill["PlacementMatchesPlayed"] = serde_json::json!(10);
        }
        modified = true;
    }
    if let Some(div) = ov.division {
        skill["Division"] = serde_json::json!(div);
        modified = true;
    }
    if let Some(ws) = ov.win_streak {
        skill["WinStreak"] = serde_json::json!(ws);
        modified = true;
    }

    modified
}

fn mu_from_display(disp: f64) -> f64 {
    (disp.max(0.0) - 100.0) / 20.0
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RealSkillEntry {
    playlist: i32,
    mu: f64,
    sigma: f64,
    display_mmr: i32,
    tier: i32,
    division: i32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RealSkillFile {
    skills: Vec<RealSkillEntry>,
}

fn extract_and_save_real_skills(body: &[u8]) {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return;
    };
    let result_obj = root.get("Result").unwrap_or(&root);
    let skills_opt = result_obj.get("Skills").and_then(|s| s.as_array())
        .or_else(|| {
            result_obj.get("Players")
                .and_then(|p| p.as_array())
                .and_then(|arr| arr.first())
                .and_then(|p0| p0.get("Skills"))
                .and_then(|s| s.as_array())
        });

    let Some(skills_arr) = skills_opt else {
        return;
    };

    let mut entries = Vec::new();
    for s in skills_arr {
        let pl = get_playlist_id(s);
        let mu = s.get("Mu").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sigma = s.get("Sigma").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let disp = (mu * 20.0 + 100.0).max(0.0);
        let tier = s.get("Tier").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let div = s.get("Division").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        entries.push(RealSkillEntry {
            playlist: pl,
            mu: (mu * 10000.0).round() / 10000.0,
            sigma: (sigma * 10000.0).round() / 10000.0,
            display_mmr: disp.round() as i32,
            tier,
            division: div,
        });
    }

    if entries.is_empty() {
        return;
    }

    let file_data = RealSkillFile { skills: entries };
    let Ok(json_str) = serde_json::to_string_pretty(&file_data) else {
        return;
    };

    let dir = crate::psynet::config_dir();
    let path = dir.join("real_skill.json");
    let _ = std::fs::write(&path, json_str);
    crate::applog::event(&format!(
        "proxy: saved {} playlists to real_skill.json",
        file_data.skills.len()
    ));
}

#[derive(Clone)]
struct StaleConfigEntry {
    status: StatusCode,
    headers: hyper::HeaderMap,
    body: Vec<u8>,
}
static LAST_GOOD_CONFIG: std::sync::Mutex<Option<StaleConfigEntry>> = std::sync::Mutex::new(None);

async fn handle_http_config(
    req: Request<Incoming>,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("https://config.psynet.gg{path}{query}");

    if let Some(bid) = parse_build_id(&path) {
        let mut lock = LAST_GAME_BUILD_ID.lock().unwrap();
        if lock.as_deref() != Some(&bid) {
            crate::applog::event(&format!("proxy: detected game build ID: {bid}"));
            *lock = Some(bid);
        }
    }

    let is_battlecars = path.to_ascii_lowercase().contains("/config/battlecars/");
    crate::applog::event(&format!(
        "proxy: >>> {} {} (battlecars={})",
        req.method(),
        path,
        is_battlecars
    ));

    let method = req.method().clone();
    let headers = req.headers().clone();

    let req_body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes().to_vec(),
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full_body(format!("read req body error: {e}")))
                .unwrap());
        }
    };

    let mut up_builder = client.request(method, &upstream_url);
    for (k, v) in headers.iter() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower != "host"
            && k_lower != "content-length"
            && k_lower != "accept-encoding"
            && k_lower != "if-none-match"
            && k_lower != "if-modified-since"
            && k_lower != "if-match"
            && k_lower != "if-unmodified-since"
            && k_lower != "if-range"
        {
            up_builder = up_builder.header(k.as_str(), v.as_bytes());
        }
    }
    if !req_body_bytes.is_empty() {
        up_builder = up_builder.body(req_body_bytes);
    }

    let (status, headers, body_bytes) = match up_builder.send().await {
        Ok(r) => {
            let s = r.status();
            let h = r.headers().clone();
            match r.bytes().await {
                Ok(bytes) => {
                    let b = bytes.to_vec();
                    if s.is_success() && !b.is_empty() {
                        *LAST_GOOD_CONFIG.lock().unwrap() = Some(StaleConfigEntry {
                            status: s,
                            headers: h.clone(),
                            body: b.clone(),
                        });
                    }
                    (s, h, b)
                }
                Err(e) => {
                    crate::applog::event(&format!("proxy: error reading upstream body: {e}"));
                    if let Some(cached) = LAST_GOOD_CONFIG.lock().unwrap().clone() {
                        crate::applog::event("proxy: serving cached last-good config fallback");
                        (cached.status, cached.headers, cached.body)
                    } else {
                        let resp = Response::builder()
                            .status(StatusCode::BAD_GATEWAY)
                            .body(full_body(format!("read body error: {e}")))
                            .unwrap();
                        return Ok(resp);
                    }
                }
            }
        }
        Err(e) => {
            crate::applog::event(&format!("proxy: upstream error: {e}"));
            if let Some(cached) = LAST_GOOD_CONFIG.lock().unwrap().clone() {
                crate::applog::event("proxy: serving cached last-good config fallback");
                (cached.status, cached.headers, cached.body)
            } else {
                let resp = Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(full_body(format!("upstream error: {e}")))
                    .unwrap();
                return Ok(resp);
            }
        }
    };

    let mut out_body = body_bytes;
    let mut patched = false;

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };
    if crate::features::is_build_supported() {
        let default_cfg = crate::psynet::SpoofPayload::default();
        let cfg_ref = cfg_opt.as_ref().unwrap_or(&default_cfg);
        let (next_body, changed) = patch_config(&out_body, cfg_ref);
        if changed {
            out_body = next_body;
            patched = true;
            crate::applog::event(&format!(
                "proxy: patched config ({} bytes)",
                out_body.len()
            ));
        }
    }

    let (next_prod, prod_changed) = patch_products_config(&out_body);
    if prod_changed {
        out_body = next_prod;
        patched = true;
    }

    if let Some(next_psynet) = patch_psynet_url(&out_body) {
        out_body = next_psynet;
        patched = true;
        crate::applog::event(&format!(
            "proxy: patched PsyNetUrl to broker ({} bytes)",
            out_body.len()
        ));
    }

    if !patched {
        crate::applog::event(&format!(
            "proxy: unpatched config ({} bytes, Psysignature generated)",
            out_body.len()
        ));
    }

    let mut resp_builder = Response::builder().status(status.as_u16());
    for (k, v) in headers.iter() {
        let k_lower = k.as_str().to_ascii_lowercase();
        if k_lower != "content-length"
            && k_lower != "transfer-encoding"
            && k_lower != "content-encoding"
            && k_lower != "psysignature"
            && k_lower != "psysig"
            && k_lower != "etag"
            && k_lower != "last-modified"
        {
            resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
        }
    }

    // Prevent client-side caching of config responses
    resp_builder = resp_builder.header("Cache-Control", "no-cache, no-store, must-revalidate");
    resp_builder = resp_builder.header("Pragma", "no-cache");
    resp_builder = resp_builder.header("Expires", "0");

    // Always provide both Psysignature and PsySig (signed with PSY_CDN_KEY) so Rocket League always accepts the config
    let sig = resign_config_cdn(&out_body);
    resp_builder = resp_builder.header("Psysignature", &sig);
    resp_builder = resp_builder.header("PsySig", &sig);

    // Record Config CDN event
    record_network_event(
        "CONFIG_CDN",
        "INBOUND",
        &path,
        &status.to_string(),
        if patched { "BattleCars PsyNet Config CDN (Patched)" } else { "BattleCars PsyNet Config CDN" },
        &String::from_utf8_lossy(&out_body),
        patched,
    );

    resp_builder = resp_builder.header("Content-Length", out_body.len().to_string());
    let resp = resp_builder.body(full_body(out_body)).unwrap();
    Ok(resp)
}

fn resign_config_cdn(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(PSY_CDN_KEY).expect("valid HMAC key");
    mac.update(body);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn resign_rpc_response(psy_time: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(PSY_RESP_KEY).expect("valid HMAC key");
    mac.update(format!("{psy_time}-").as_bytes());
    mac.update(body);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// Rewrite `PsyNetUrl.URL` and `PsyNetUrl.URLv2` in the battlecars config body
/// to route WebSocket connections and RPC requests through our local broker.
#[allow(dead_code)]
fn patch_psynet_url(body: &[u8]) -> Option<Vec<u8>> {
    let http_base = broker_http_base()?;
    let (obj_start, obj_end) = find_named_object(body, "PsyNetUrl")?;
    let obj = body[obj_start..obj_end].to_vec();

    let local_services = format!("{http_base}/Services");
    let local_rpc = format!("{http_base}/rpc");

    let mut patched_obj = obj;
    let mut changed = false;

    if let Some(next) = replace_json_string_field(&patched_obj, "URLv2", &local_rpc) {
        patched_obj = next;
        changed = true;
    }

    if let Some(next) = replace_json_string_field(&patched_obj, "URL", &local_services) {
        patched_obj = next;
        changed = true;
    }

    if !changed {
        return None;
    }

    let mut out = Vec::with_capacity(body.len() + 64);
    out.extend_from_slice(&body[..obj_start]);
    out.extend_from_slice(&patched_obj);
    out.extend_from_slice(&body[obj_end..]);
    Some(out)
}

/// Replace the value of a JSON string field `"key":"<old_value>"` inside `body`.
/// Returns `Some(new_body)` if the field was found and the value differed, `None` otherwise.
fn replace_json_string_field(body: &[u8], key: &str, new_value: &str) -> Option<Vec<u8>> {
    // Encode new_value as a JSON string (without surrounding quotes).
    let encoded_json = serde_json::to_string(new_value).ok()?;
    if encoded_json.len() < 2 {
        return None;
    }
    let encoded = &encoded_json.as_bytes()[1..encoded_json.len() - 1];

    let prefix = format!("\"{key}\":\"");
    let prefix_bytes = prefix.as_bytes();
    let i = find_bytes(body, prefix_bytes)?;
    let val_start = i + prefix_bytes.len();
    let j = json_string_end(body, val_start)?;

    if &body[val_start..j] == encoded {
        return None; // already set to the desired value
    }

    let mut out = Vec::with_capacity(body.len() + encoded.len());
    out.extend_from_slice(&body[..val_start]);
    out.extend_from_slice(encoded);
    out.extend_from_slice(&body[j..]);
    Some(out)
}

fn scan_object_end(body: &[u8], start: usize) -> Option<usize> {
    let mut in_str = false;
    let mut esc = false;
    let mut depth = 0;
    for (i, &c) in body[start..].iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
                continue;
            }
            if c == b'\\' {
                esc = true;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn json_string_end(body: &[u8], val_start: usize) -> Option<usize> {
    let mut esc = false;
    for (j, &c) in body[val_start..].iter().enumerate() {
        if esc {
            esc = false;
            continue;
        }
        if c == b'\\' {
            esc = true;
            continue;
        }
        if c == b'"' {
            return Some(val_start + j);
        }
    }
    None
}

fn find_named_object(body: &[u8], name: &str) -> Option<(usize, usize)> {
    let key = format!("\"{name}\"");
    let key_bytes = key.as_bytes();
    let at = find_bytes(body, key_bytes)?;
    let mut i = at + key_bytes.len();
    while i < body.len() && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= body.len() || body[i] != b':' {
        return None;
    }
    i += 1;
    while i < body.len() && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= body.len() || body[i] != b'{' {
        return None;
    }
    let close = scan_object_end(body, i)?;
    Some((i, close + 1))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn replace_equip_text(body: &[u8], equip_id: &str, new_text: &str) -> Option<Vec<u8>> {
    let encoded_json = serde_json::to_string(new_text).ok()?;
    if encoded_json.len() < 2 {
        return None;
    }
    let encoded = &encoded_json.as_bytes()[1..encoded_json.len() - 1];

    let prefix = format!("\"ID\":\"{equip_id}\",\"Text\":\"");
    let val_start = if let Some(i) = find_bytes(body, prefix.as_bytes()) {
        i + prefix.len()
    } else {
        let id_pat = format!("\"ID\":\"{equip_id}\"");
        let id_at = find_bytes(body, id_pat.as_bytes())?;
        let mut start = id_at;
        while start > 0 && body[start] != b'{' {
            start -= 1;
        }
        let end = scan_object_end(body, start)? + 1;
        let obj = &body[start..end];
        let tkey = b"\"Text\":\"";
        let k = find_bytes(obj, tkey)?;
        start + k + tkey.len()
    };

    let j = json_string_end(body, val_start)?;
    if &body[val_start..j] == encoded {
        return None;
    }

    let mut out = Vec::with_capacity(body.len() + encoded.len());
    out.extend_from_slice(&body[..val_start]);
    out.extend_from_slice(encoded);
    out.extend_from_slice(&body[j..]);
    Some(out)
}

fn replace_equip_category(body: &[u8], equip_id: &str, new_cat: &str) -> Option<Vec<u8>> {
    let id_pat = format!("\"ID\":\"{equip_id}\"");
    let id_at = find_bytes(body, id_pat.as_bytes())?;
    let mut start = id_at;
    while start > 0 && body[start] != b'{' {
        start -= 1;
    }
    let end = scan_object_end(body, start)? + 1;
    let obj = &body[start..end];
    let ckey = b"\"Category\":\"";

    if let Some(k) = find_bytes(obj, ckey) {
        let val_start = start + k + ckey.len();
        let j = json_string_end(body, val_start)?;
        if &body[val_start..j] == new_cat.as_bytes() {
            return None;
        }
        let mut out = Vec::with_capacity(body.len() + new_cat.len());
        out.extend_from_slice(&body[..val_start]);
        out.extend_from_slice(new_cat.as_bytes());
        out.extend_from_slice(&body[j..]);
        Some(out)
    } else {
        let insert_at = start + id_at - start + id_pat.len();
        let frag = format!(",\"Category\":\"{new_cat}\"");
        let mut out = Vec::with_capacity(body.len() + frag.len());
        out.extend_from_slice(&body[..insert_at]);
        out.extend_from_slice(frag.as_bytes());
        out.extend_from_slice(&body[insert_at..]);
        Some(out)
    }
}

pub fn is_hex6(s: &str) -> bool {
    s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn upsert_title_category(body: &[u8], cat_id: &str, color: &str, glow_color: &str) -> (Vec<u8>, bool) {
    let Some((ptc_start, ptc_end)) = find_named_object(body, "PlayerTitleConfig") else {
        return (body.to_vec(), false);
    };

    let def = format!(
        "{{\"ID\":\"{cat_id}\",\"Color\":\"{color}\",\"GlowColor\":\"{glow_color}\"}}"
    );

    let ptc_obj = &body[ptc_start..ptc_end];
    let key = b"\"Categories\"";
    let Some(k) = find_bytes(ptc_obj, key) else {
        return (body.to_vec(), false);
    };

    let mut i = k + key.len();
    while i < ptc_obj.len() && ptc_obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= ptc_obj.len() || ptc_obj[i] != b':' {
        return (body.to_vec(), false);
    }
    i += 1;
    while i < ptc_obj.len() && ptc_obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= ptc_obj.len() || ptc_obj[i] != b'[' {
        return (body.to_vec(), false);
    }

    let arr_start = ptc_start + i;
    let Some(arr_end) = scan_array_end(body, arr_start) else {
        return (body.to_vec(), false);
    };

    // Check if category already exists in Categories array
    let id_needle = format!("\"ID\":\"{cat_id}\"");
    let arr_slice = &body[arr_start..=arr_end];
    if let Some(id_at) = find_bytes(arr_slice, id_needle.as_bytes()) {
        let abs_id = arr_start + id_at;
        let mut obj_s = abs_id;
        while obj_s > arr_start && body[obj_s] != b'{' {
            obj_s -= 1;
        }
        if let Some(obj_e) = scan_object_end(body, obj_s) {
            if &body[obj_s..=obj_e] == def.as_bytes() {
                return (body.to_vec(), false);
            }
            let mut out = Vec::with_capacity(body.len() + def.len());
            out.extend_from_slice(&body[..obj_s]);
            out.extend_from_slice(def.as_bytes());
            out.extend_from_slice(&body[obj_e + 1..]);
            return (out, true);
        }
    }

    // Insert at beginning of array [ {def}, ... ]
    let insert_at = arr_start + 1;
    let inner_is_empty = body[arr_start + 1..arr_end].iter().all(|c| c.is_ascii_whitespace());
    let frag = if inner_is_empty {
        def
    } else {
        format!("{def},")
    };

    let mut out = Vec::with_capacity(body.len() + frag.len());
    out.extend_from_slice(&body[..insert_at]);
    out.extend_from_slice(frag.as_bytes());
    out.extend_from_slice(&body[insert_at..]);
    (out, true)
}

pub fn sanitize_category_part(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "title".to_string()
    } else {
        out
    }
}

fn upsert_title_entry(body: &[u8], title_id: &str, text: &str, category: &str) -> (Vec<u8>, bool) {
    let Some((ptc_start, ptc_end)) = find_named_object(body, "PlayerTitleConfig") else {
        return (body.to_vec(), false);
    };

    let def = format!(
        "{{\"ID\":\"{title_id}\",\"Text\":\"{text}\",\"Category\":\"{category}\"}}"
    );

    let ptc_obj = &body[ptc_start..ptc_end];
    let key = b"\"Titles\"";
    let Some(k) = find_bytes(ptc_obj, key) else {
        return (body.to_vec(), false);
    };

    let mut i = k + key.len();
    while i < ptc_obj.len() && ptc_obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= ptc_obj.len() || ptc_obj[i] != b':' {
        return (body.to_vec(), false);
    }
    i += 1;
    while i < ptc_obj.len() && ptc_obj[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= ptc_obj.len() || ptc_obj[i] != b'[' {
        return (body.to_vec(), false);
    }

    let arr_start = ptc_start + i;
    let Some(arr_end) = scan_array_end(body, arr_start) else {
        return (body.to_vec(), false);
    };

    let id_needle = format!("\"ID\":\"{title_id}\"");
    let arr_slice = &body[arr_start..=arr_end];
    if let Some(id_at) = find_bytes(arr_slice, id_needle.as_bytes()) {
        let abs_id = arr_start + id_at;
        let mut obj_s = abs_id;
        while obj_s > arr_start && body[obj_s] != b'{' {
            obj_s -= 1;
        }
        if let Some(obj_e) = scan_object_end(body, obj_s) {
            if &body[obj_s..=obj_e] == def.as_bytes() {
                return (body.to_vec(), false);
            }
            let mut out = Vec::with_capacity(body.len() + def.len());
            out.extend_from_slice(&body[..obj_s]);
            out.extend_from_slice(def.as_bytes());
            out.extend_from_slice(&body[obj_e + 1..]);
            return (out, true);
        }
    }

    let insert_at = arr_start + 1;
    let inner_is_empty = body[arr_start + 1..arr_end].iter().all(|c| c.is_ascii_whitespace());
    let frag = if inner_is_empty {
        def
    } else {
        format!("{def},")
    };

    let mut out = Vec::with_capacity(body.len() + frag.len());
    out.extend_from_slice(&body[..insert_at]);
    out.extend_from_slice(frag.as_bytes());
    out.extend_from_slice(&body[insert_at..]);
    (out, true)
}

/// The title the player is actually wearing (the swap/equip title). Empty when
/// nothing is equipped — we do NOT substitute a spawned or default title, so a
/// player wearing no title stays bare instead of being forced into one.
fn equipped_title_id() -> String {
    let Some(cfg) = crate::psynet::load_active_spoof_from_disk() else {
        return String::new();
    };
    let id = if !cfg.equip_title_id.trim().is_empty() {
        cfg.equip_title_id.trim()
    } else {
        cfg.display_title_id.trim()
    };
    id.to_string()
}

/// (title_id, title_text) for the equipped title, or empty strings when none is
/// worn. Callers must treat empty as "leave the player's real title alone".
fn configured_title() -> (String, String) {
    let id = equipped_title_id();
    if id.is_empty() {
        return (String::new(), String::new());
    }
    let text = crate::psynet::load_active_spoof_from_disk()
        .map(|c| {
            if !c.custom_text.trim().is_empty() {
                c.custom_text.trim().to_string()
            } else if !c.display_title_id.trim().is_empty() {
                c.display_title_id.trim().to_string()
            } else {
                id.clone()
            }
        })
        .unwrap_or_else(|| id.clone());
    (id, text)
}

const TITLE_PRODUCT_ID: i32 = 3036;

fn spawned_title_ids() -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    let mut ids = get_all_title_ids();
    ids.extend(crate::psynet::load_title_swap_equip_ids());
    crate::psynet::clean_title_ids(&ids)
}

fn title_product_json(title_id: &str, timestamp: i64) -> serde_json::Value {
    serde_json::json!({
        "ProductID": TITLE_PRODUCT_ID,
        "InstanceID": title_instance_id(title_id),
        "SeriesID": lookup_series_id_for_product(TITLE_PRODUCT_ID),
        "Amount": 1,
        "ExpirationTime": -1,
        "IsTradable": false,
        "TradeHold": -1,
        "AddedTimestamp": timestamp,
        "UpdatedTimestamp": timestamp,
        "Attributes": [{ "Key": "TitleId", "Value": title_id }],
    })
}

fn title_instance_id(title_id: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in title_id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}7469746c{:016x}", TITLE_PRODUCT_ID as u32, h)
}

fn item_title_id(item: &serde_json::Value) -> Option<&str> {
    item.get("Attributes")?.as_array()?.iter().find_map(|a| {
        let key = a.get("Key")?.as_str()?;
        if key.eq_ignore_ascii_case("TitleId") { a.get("Value")?.as_str() } else { None }
    })
}

pub fn patch_config(body: &[u8], cfg: &crate::psynet::SpoofPayload) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let mut any_change = false;

    // Ensure Developer category and VelocityRL Developer title are always registered
    let (next_dev_cat, did_dev_cat) = upsert_title_category(&out, "Developer", "00FFAA", "00FFFF");
    if did_dev_cat {
        out = next_dev_cat;
        any_change = true;
    }
    let (next_dev_title, did_dev_title) = upsert_title_entry(&out, "VelocityRL_Developer", "VelocityRL Developer", "Developer");
    if did_dev_title {
        out = next_dev_title;
        any_change = true;
    }

    {
        let equip_id = if !cfg.equip_title_id.trim().is_empty() {
            cfg.equip_title_id.trim()
        } else {
            cfg.display_title_id.trim()
        };
        let display_id = cfg.display_title_id.trim();
        let mut custom_text = cfg.custom_text.trim();
        if custom_text.is_empty() {
            custom_text = cfg.custom_name.trim();
        }

        let cat = cfg.category.trim();
        let clean_cat = if !cat.is_empty() {
            sanitize_category_part(cat)
        } else {
            String::new()
        };

        let mut registered_category_colors: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();

        if !equip_id.is_empty() {
            if !custom_text.is_empty() {
                if let Some(next) = replace_equip_text(&out, equip_id, custom_text) {
                    out = next;
                    any_change = true;
                } else {
                    let cat_to_use = if !clean_cat.is_empty() { &clean_cat } else { "Special Titles" };
                    let (next, did) = upsert_title_entry(&out, equip_id, custom_text, cat_to_use);
                    if did {
                        out = next;
                        any_change = true;
                    }
                }
            }

            if let Some(tc) = &cfg.title_color {
                if is_hex6(&tc.color) {
                    let glow = if is_hex6(&tc.glow_color) { &tc.glow_color } else { &tc.color };
                    let custom_cat = format!("RLItemMod_{}", sanitize_category_part(equip_id));
                    let color_up = tc.color.to_ascii_uppercase();
                    let glow_up = glow.to_ascii_uppercase();

                    let mut can_apply = true;
                    if let Some((existing_c, existing_g)) = registered_category_colors.get(&custom_cat) {
                        if existing_c != &color_up || existing_g != &glow_up {
                            crate::applog::event(&format!(
                                "proxy: WARNING: Category '{}' already has custom color (#{}/#{}); ignoring conflicting color (#{}/#{}) for title '{}'",
                                custom_cat, existing_c, existing_g, color_up, glow_up, equip_id
                            ));
                            can_apply = false;
                        }
                    } else {
                        registered_category_colors.insert(custom_cat.clone(), (color_up, glow_up));
                    }

                    if can_apply {
                        let (next, did) = upsert_title_category(&out, &custom_cat, &tc.color, glow);
                        if did {
                            out = next;
                            any_change = true;
                        }
                        if let Some(next) = replace_equip_category(&out, equip_id, &custom_cat) {
                            out = next;
                            any_change = true;
                        }
                    }
                } else if !clean_cat.is_empty() {
                    if let Some(next) = replace_equip_category(&out, equip_id, &clean_cat) {
                        out = next;
                        any_change = true;
                    }
                }
            } else if !clean_cat.is_empty() {
                if let Some(next) = replace_equip_category(&out, equip_id, &clean_cat) {
                    out = next;
                    any_change = true;
                }
            }
        }

        if let Some(swaps) = &cfg.swaps {
            for sw in swaps {
                let target_id = if !sw.equip_title_id.trim().is_empty() {
                    sw.equip_title_id.trim()
                } else if !sw.display_title_id.trim().is_empty() {
                    sw.display_title_id.trim()
                } else {
                    ""
                };

                if target_id.is_empty() {
                    continue;
                }

                let sw_text = sw.custom_text.trim();
                let effective_cat = if !sw.category.trim().is_empty() { sw.category.trim() } else { "Special Titles" };
                if !sw_text.is_empty() {
                    if let Some(next) = replace_equip_text(&out, target_id, sw_text) {
                        out = next;
                        any_change = true;
                    } else {
                        let (next, did) = upsert_title_entry(&out, target_id, sw_text, effective_cat);
                        if did {
                            out = next;
                            any_change = true;
                        }
                    }
                } else if !sw.display_title_id.trim().is_empty()
                    && sw.display_title_id.trim() != target_id
                    && sw.display_title_id.trim() != "custom"
                {
                    let disp = sw.display_title_id.trim();
                    let src_pat = format!("\"ID\":\"{disp}\"");
                    let mut resolved_text = String::new();
                    if let Some(src_at) = find_bytes(&out, src_pat.as_bytes()) {
                        let mut start = src_at;
                        while start > 0 && out[start] != b'{' {
                            start -= 1;
                        }
                        if let Some(end) = scan_object_end(&out, start) {
                            let src_obj = &out[start..end + 1];
                            let tkey = b"\"Text\":\"";
                            if let Some(k) = find_bytes(src_obj, tkey) {
                                let val_start = start + k + tkey.len();
                                if let Some(j) = json_string_end(&out, val_start) {
                                    resolved_text = String::from_utf8_lossy(&out[val_start..j]).to_string();
                                }
                            }
                        }
                    }
                    let t_to_use = if !resolved_text.is_empty() { &resolved_text } else { target_id };
                    if let Some(next) = replace_equip_text(&out, target_id, t_to_use) {
                        out = next;
                        any_change = true;
                    } else {
                        let (next, did) = upsert_title_entry(&out, target_id, t_to_use, effective_cat);
                        if did {
                            out = next;
                            any_change = true;
                        }
                    }
                } else {
                    let (next, did) = upsert_title_entry(&out, target_id, target_id, effective_cat);
                    if did {
                        out = next;
                        any_change = true;
                    }
                }

                let sw_cat = sw.category.trim();
                let clean = if !sw_cat.is_empty() {
                    sanitize_category_part(sw_cat)
                } else {
                    String::new()
                };

                if let Some(tc) = &sw.title_color {
                    if is_hex6(&tc.color) {
                        let glow = if is_hex6(&tc.glow_color) { &tc.glow_color } else { &tc.color };
                        let custom_cat = format!("RLItemMod_{}", sanitize_category_part(target_id));
                        let color_up = tc.color.to_ascii_uppercase();
                        let glow_up = glow.to_ascii_uppercase();

                        let mut can_apply = true;
                        if let Some((existing_c, existing_g)) = registered_category_colors.get(&custom_cat) {
                            if existing_c != &color_up || existing_g != &glow_up {
                                crate::applog::event(&format!(
                                    "proxy: WARNING: Category '{}' already has custom color (#{}/#{}); ignoring conflicting color (#{}/#{}) for title '{}'",
                                    custom_cat, existing_c, existing_g, color_up, glow_up, target_id
                                ));
                                can_apply = false;
                            }
                        } else {
                            registered_category_colors.insert(custom_cat.clone(), (color_up, glow_up));
                        }

                        if can_apply {
                            let (next, did) = upsert_title_category(&out, &custom_cat, &tc.color, glow);
                            if did {
                                out = next;
                                any_change = true;
                            }
                            if let Some(next) = replace_equip_category(&out, target_id, &custom_cat) {
                                out = next;
                                any_change = true;
                            }
                        }
                    } else if !clean.is_empty() {
                        if let Some(next) = replace_equip_category(&out, target_id, &clean) {
                            out = next;
                            any_change = true;
                        }
                    }
                } else if !clean.is_empty() {
                    if let Some(next) = replace_equip_category(&out, target_id, &clean) {
                        out = next;
                        any_change = true;
                    }
                }
            }
        }

        if custom_text.is_empty() && !display_id.is_empty() && display_id != equip_id && display_id != "custom" {
            let src_pat = format!("\"ID\":\"{display_id}\"");
            if let Some(src_at) = find_bytes(&out, src_pat.as_bytes()) {
                let mut start = src_at;
                while start > 0 && out[start] != b'{' {
                    start -= 1;
                }
                if let Some(end) = scan_object_end(&out, start) {
                    let src_obj = &out[start..end + 1];
                    let tkey = b"\"Text\":\"";
                    if let Some(k) = find_bytes(src_obj, tkey) {
                        let val_start = start + k + tkey.len();
                        if let Some(j) = json_string_end(&out, val_start) {
                            let text_val = String::from_utf8_lossy(&out[val_start..j]).to_string();
                            if !text_val.is_empty() && !equip_id.is_empty() {
                                if let Some(next) = replace_equip_text(&out, equip_id, &text_val) {
                                    out = next;
                                    any_change = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let features = crate::features::get_cached_features();

    if features.flags.camera_spoof {
        if let Some(cam) = &cfg.camera_spoof {
            if cam.enabled {
                let (next, changed) = patch_camera(&out, cam);
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.rich_palette {
        if let Some(palette) = &cfg.palette_spoof {
            if palette.enabled {
                let (next, changed) = patch_palette(&out);
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.dynamic_logos {
        if let Some(logo) = &cfg.logo_spoof {
            if logo.enabled && !logo.logo_url.trim().is_empty() {
                let (next, changed) = patch_logo(&out, logo.logo_url.trim());
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if features.flags.blog_motd {
        if let Some(blog) = &cfg.blog_spoof {
            if blog.enabled && !blog.motd.trim().is_empty() {
                let (next, changed) = patch_blog_motd(&out, blog.motd.trim());
                if changed {
                    out = next;
                    any_change = true;
                }
            }
        }
    }

    if let Some(menu_bg) = &cfg.menu_bg_spoof {
        if menu_bg.enabled && !menu_bg.background.trim().is_empty() {
            let (next, changed) = patch_menu_bg(&out, menu_bg.background.trim());
            if changed {
                out = next;
                any_change = true;
            }
        }
    }

    // Modern Rocket League UPsyNetUrl_X::ValidateURL explicitly rejects non-HTTPS URLs
    // (disallowed https URL: http://...). Never rewrite PsyNetUrl to plain http broker,
    // which caused the engine to disconnect and report OSCS_NotConnected / OSCS_ServiceUnavailable.

    let (next_prod, prod_changed) = patch_products_config(&out);
    if prod_changed {
        out = next_prod;
        any_change = true;
    }

    (out, any_change)
}

fn patch_menu_bg(body: &[u8], bg: &str) -> (Vec<u8>, bool) {
    let effective_bg = if bg.is_empty() || bg.eq_ignore_ascii_case("default") {
        "MMBG_Default".to_string()
    } else if !bg.starts_with("MMBG_") {
        format!("MMBG_{bg}")
    } else {
        bg.to_string()
    };

    let mut out = body.to_vec();
    let mut any_changed = false;

    // 1. Override in ClassPropertyConfig for UIConfig_TA
    let (next1, changed1) = upsert_class_property_override(
        &out,
        "UIConfig_TA",
        "MainMenuBG",
        &effective_bg,
    );
    if changed1 {
        out = next1;
        any_changed = true;
    }

    // 2. Override in ClassPropertyConfig for GFxData_MainMenu_TA
    let (next2, changed2) = upsert_class_property_override(
        &out,
        "GFxData_MainMenu_TA",
        "MainMenuBG",
        &effective_bg,
    );
    if changed2 {
        out = next2;
        any_changed = true;
    }

    // 3. Direct UIConfig_TA JSON object if present
    if let Some((obj_start, obj_end)) = find_named_object(&out, "UIConfig_TA")
        .or_else(|| find_named_object(&out, "UIConfig"))
    {
        let obj = &out[obj_start..obj_end];
        if let Some(patched_obj) = replace_json_string_field(obj, "MainMenuBG", &effective_bg) {
            let mut next = Vec::with_capacity(out.len() + 64);
            next.extend_from_slice(&out[..obj_start]);
            next.extend_from_slice(&patched_obj);
            next.extend_from_slice(&out[obj_end..]);
            out = next;
            any_changed = true;
        } else if let Some(close_idx) = obj.iter().rposition(|&c| c == b'}') {
            let inject_str = format!(",\"MainMenuBG\":\"{effective_bg}\"");
            let mut res = Vec::with_capacity(out.len() + inject_str.len());
            res.extend_from_slice(&out[..obj_start + close_idx]);
            res.extend_from_slice(inject_str.as_bytes());
            res.extend_from_slice(&out[obj_start + close_idx..]);
            out = res;
            any_changed = true;
        }
    } else if let Some(patched) = replace_json_string_field(&out, "MainMenuBG", &effective_bg) {
        out = patched;
        any_changed = true;
    }

    (out, any_changed)
}

fn patch_logo(body: &[u8], url: &str) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let has_escaped_slash = find_bytes(body, b"\\/").is_some();
    let encoded_url = if has_escaped_slash {
        url.replace('/', "\\/")
    } else {
        url.to_string()
    };

    let Some((start, end)) = find_named_object(&out, "DynamicLogosConfig") else {
        // Inject DynamicLogosConfig before the last '}'
        let Some(close_idx) = out.iter().rposition(|&c| c == b'}') else {
            return (out, false);
        };
        let block = format!(
            ",\"DynamicLogosConfig\":{{\"Class\":\"DynamicLogosConfig_X\",\"bUseDynamicLogos\":true,\"LogoURL\":\"{encoded_url}\"}}"
        );
        let mut res = Vec::with_capacity(out.len() + block.len());
        res.extend_from_slice(&out[..close_idx]);
        res.extend_from_slice(block.as_bytes());
        res.extend_from_slice(&out[close_idx..]);
        return (res, true);
    };

    let mut changed = false;

    // Force bUseDynamicLogos: true
    let obj = out[start..end].to_vec();
    if let Some(b_at) = find_bytes(&obj, b"\"bUseDynamicLogos\":") {
        let val_start = start + b_at + b"\"bUseDynamicLogos\":".len();
        let mut val_end = val_start;
        while val_end < out.len() && out[val_end].is_ascii_alphanumeric() {
            val_end += 1;
        }
        if &out[val_start..val_end] != b"true" {
            let mut next = Vec::with_capacity(out.len() + 4);
            next.extend_from_slice(&out[..val_start]);
            next.extend_from_slice(b"true");
            next.extend_from_slice(&out[val_end..]);
            out = next;
            changed = true;
        }
    }

    // Replace LogoURL
    let (cur_start, cur_end) = match find_named_object(&out, "DynamicLogosConfig") {
        Some(b) => b,
        None => return (out, changed),
    };
    let cur_obj = out[cur_start..cur_end].to_vec();
    let mut found_url = false;
    for key in &["LogoURL", "LogoUrl", "SeasonLogo", "SeasonLogoURL", "LogoImageURL", "DynamicLogoURL"] {
        let pat = format!("\"{key}\":\"");
        if let Some(k_at) = find_bytes(&cur_obj, pat.as_bytes()) {
            found_url = true;
            let val_start = cur_start + k_at + pat.len();
            if let Some(val_end) = json_string_end(&out, val_start) {
                if &out[val_start..val_end] != encoded_url.as_bytes() {
                    let mut next = Vec::with_capacity(out.len() + encoded_url.len());
                    next.extend_from_slice(&out[..val_start]);
                    next.extend_from_slice(encoded_url.as_bytes());
                    next.extend_from_slice(&out[val_end..]);
                    out = next;
                    changed = true;
                    break;
                }
            }
        }
    }

    if !found_url {
        let close_obj = cur_end - 1;
        if out[close_obj] == b'}' {
            let snippet = format!(",\"LogoURL\":\"{encoded_url}\"");
            let mut next = Vec::with_capacity(out.len() + snippet.len());
            next.extend_from_slice(&out[..close_obj]);
            next.extend_from_slice(snippet.as_bytes());
            next.extend_from_slice(&out[close_obj..]);
            out = next;
            changed = true;
        }
    }

    (out, changed)
}

fn json_string_contents(s: &str) -> Option<Vec<u8>> {
    let serialized = serde_json::to_string(s).ok()?;
    if serialized.len() >= 2 && serialized.starts_with('"') && serialized.ends_with('"') {
        Some(serialized[1..serialized.len() - 1].as_bytes().to_vec())
    } else {
        None
    }
}

fn patch_blog_motd(body: &[u8], motd: &str) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let Some(encoded_motd) = json_string_contents(motd) else {
        return (out, false);
    };
    let Some((start, end)) = find_named_object(&out, "BlogConfig") else {
        // Inject BlogConfig before the last '}'
        let Some(close_idx) = out.iter().rposition(|&c| c == b'}') else {
            return (out, false);
        };
        let mut block = Vec::new();
        block.extend_from_slice(b",\"BlogConfig\":{\"Class\":\"BlogConfig_X\",\"MotD\":\"");
        block.extend_from_slice(&encoded_motd);
        block.extend_from_slice(b"\"}");
        let mut res = Vec::with_capacity(out.len() + block.len());
        res.extend_from_slice(&out[..close_idx]);
        res.extend_from_slice(&block);
        res.extend_from_slice(&out[close_idx..]);
        return (res, true);
    };

    let obj = out[start..end].to_vec();
    let mut changed = false;

    for key in &["MotD", "Motd", "MOTD", "NewsText"] {
        let pat = format!("\"{key}\":\"");
        if let Some(k_at) = find_bytes(&obj, pat.as_bytes()) {
            let val_start = start + k_at + pat.len();
            if let Some(val_end) = json_string_end(&out, val_start) {
                if &out[val_start..val_end] != encoded_motd.as_slice() {
                    let mut next = Vec::with_capacity(out.len() + encoded_motd.len());
                    next.extend_from_slice(&out[..val_start]);
                    next.extend_from_slice(&encoded_motd);
                    next.extend_from_slice(&out[val_end..]);
                    out = next;
                    changed = true;
                    break;
                }
            }
        }
    }

    (out, changed)
}

fn format_camera_limit(min: f64, max: f64, interval: f64, def_min: f64, def_max: f64, def_interval: f64) -> String {
    let mut actual_min = min;
    let mut actual_max = max;
    let mut actual_interval = interval;
    if actual_max <= 0.0 && actual_min <= 0.0 {
        actual_min = def_min;
        actual_max = def_max;
    }
    if actual_interval <= 0.0 {
        actual_interval = def_interval;
    }
    if actual_max < actual_min {
        actual_max = actual_min;
    }
    format!("(Min={:.6},Max={:.6},interval={:.6})", actual_min, actual_max, actual_interval)
}

fn patch_camera(body: &[u8], cam: &crate::psynet::CameraSpoofPayload) -> (Vec<u8>, bool) {
    let fov_str = format_camera_limit(cam.fov.min, cam.fov.max, cam.fov.interval, 60.0, 1000.0, 1.0);
    let height_str = format_camera_limit(cam.height.min, cam.height.max, cam.height.interval, 40.0, 1000.0, 1.0);
    let dist_str = format_camera_limit(cam.distance.min, cam.distance.max, cam.distance.interval, 100.0, 1000.0, 1.0);

    let targets = [
        ("Camera_TA", "FOVLimits", fov_str),
        ("Camera_TA", "HeightLimits", height_str),
        ("Camera_TA", "DistanceLimits", dist_str),
    ];

    let mut out = body.to_vec();
    let mut changed = false;

    for (class_name, prop_name, val_str) in &targets {
        let (next, did) = upsert_class_property_override(&out, class_name, prop_name, val_str);
        if did {
            out = next;
            changed = true;
        }
    }

    (out, changed)
}

pub fn patch_products_config(body: &[u8]) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let mut any_changed = false;

    // We do NOT disable save syncing (SaveGameManager_TA / OnlineStorageConfig_TA).
    // The user's save files sync normally to cloud/OSS.
    // Instead, we stop item/product synchronization and duplication/stripping:
    // - ProductsConfig_TA: MaxSecondsBetweenFullProductSyncs = 2147483647 (stops periodic full product re-syncs from PsyNet)
    // - ProductsConfig_TA: bUseSaveDataCacheProducts = True (forces local cached products to remain available)
    // - ProductsConfig_TA: bEnableOfflineItemsDuplicationPrevention = False (prevents deleting duplicate/offline products)
    // - SaveData_TA: bClearSyncTimestampOnMissingProduct = False (prevents clearing timestamps or triggering sync on missing items)
    let overrides = [
        ("ProductsConfig_TA", "bUseSaveDataCacheProducts", "True"),
        ("SaveData_TA", "bClearSyncTimestampOnMissingProduct", "False"),
        ("ProductsConfig_TA", "bEnableOfflineItemsDuplicationPrevention", "False"),
        ("ProductsConfig_TA", "MaxSecondsBetweenFullProductSyncs", "2147483647"),
        ("LoadoutValidation_TA", "bValidateOnlineProducts", "False"),
        ("LoadoutValidation_TA", "bValidateDLC", "False"),
        ("LoadoutValidation_TA", "bEnableStrictLoadoutValidation", "False"),
        ("SaveGameManager_TA", "bEnableSaving", "True"),
        ("SaveGameManager_TA", "bDisableSaveOSS", "False"),
        ("UIConfig_TA", "bCrateRouletteEnabled", "False"),
        ("GameplaySettingsSave_TA", "bQuickDropOpening", "True"),
        ("GFxData_Playlist_TA", "MaximumRankDisparity", "100"),
        ("GFxData_Playlist_TA", "bUndersizedPartyWithRankDisparity", "False"),
        ("GFxData_Playlist_TA", "bShowRankDisparity", "False"),
        ("RankedConfig_X", "bUsePsyNetRankedDisparity", "False"),
        ("RankedConfig_X", "MaximumRankDisparity", "100"),
    ];

    for (cls, prop, val) in overrides {
        let (next, ch) = upsert_class_property_override(&out, cls, prop, val);
        if ch {
            out = next;
            any_changed = true;
        }
    }

    if let Some((start, end)) = find_named_object(&out, "ProductsConfig")
        .or_else(|| find_named_object(&out, "ProductsConfig_TA"))
    {
        let obj = &out[start..end];
        let mut obj_str = String::from_utf8_lossy(obj).into_owned();
        let mut obj_changed = false;

        if let Some(pos) = obj_str.find("\"bUseSaveDataCacheProducts\"") {
            if let Some(fpos) = obj_str[pos..].find("false") {
                let abs_fpos = pos + fpos;
                obj_str.replace_range(abs_fpos..abs_fpos + 5, "true");
                obj_changed = true;
            }
        } else if let Some(last_brace) = obj_str.rfind('}') {
            obj_str.insert_str(last_brace, ",\"bUseSaveDataCacheProducts\":true");
            obj_changed = true;
        }

        if let Some(pos) = obj_str.find("\"bClearSyncTimestampOnMissingProduct\"") {
            if let Some(tpos) = obj_str[pos..].find("true") {
                let abs_tpos = pos + tpos;
                obj_str.replace_range(abs_tpos..abs_tpos + 4, "false");
                obj_changed = true;
            }
        } else if let Some(last_brace) = obj_str.rfind('}') {
            obj_str.insert_str(last_brace, ",\"bClearSyncTimestampOnMissingProduct\":false");
            obj_changed = true;
        }

        if let Some(pos) = obj_str.find("\"bEnableOfflineItemsDuplicationPrevention\"") {
            if let Some(tpos) = obj_str[pos..].find("true") {
                let abs_tpos = pos + tpos;
                obj_str.replace_range(abs_tpos..abs_tpos + 4, "false");
                obj_changed = true;
            }
        } else if let Some(last_brace) = obj_str.rfind('}') {
            obj_str.insert_str(last_brace, ",\"bEnableOfflineItemsDuplicationPrevention\":false");
            obj_changed = true;
        }

        if let Some(pos) = obj_str.find("\"MaxSecondsBetweenFullProductSyncs\"") {
            if let Some(col) = obj_str[pos..].find(':') {
                let num_start = pos + col + 1;
                let trimmed = obj_str[num_start..].trim_start();
                let skip = obj_str[num_start..].len() - trimmed.len();
                let actual_num_start = num_start + skip;
                let num_end = actual_num_start + obj_str[actual_num_start..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(obj_str.len() - actual_num_start);
                if &obj_str[actual_num_start..num_end] != "2147483647" {
                    obj_str.replace_range(actual_num_start..num_end, "2147483647");
                    obj_changed = true;
                }
            }
        } else if let Some(last_brace) = obj_str.rfind('}') {
            obj_str.insert_str(last_brace, ",\"MaxSecondsBetweenFullProductSyncs\":2147483647");
            obj_changed = true;
        }

        if obj_changed {
            let mut next = Vec::with_capacity(out.len() + 64);
            next.extend_from_slice(&out[..start]);
            next.extend_from_slice(obj_str.as_bytes());
            next.extend_from_slice(&out[end..]);
            out = next;
            any_changed = true;
        }
    } else {
        if let Some(close_idx) = out.iter().rposition(|&c| c == b'}') {
            let block = ",\"ProductsConfig\":{\"Class\":\"ProductsConfig_TA\",\"bUseSaveDataCacheProducts\":true,\"MaxSecondsBetweenFullProductSyncs\":2147483647,\"bClearSyncTimestampOnMissingProduct\":false,\"bEnableOfflineItemsDuplicationPrevention\":false}";
            let mut res = Vec::with_capacity(out.len() + block.len());
            res.extend_from_slice(&out[..close_idx]);
            res.extend_from_slice(block.as_bytes());
            res.extend_from_slice(&out[close_idx..]);
            out = res;
            any_changed = true;
        }
    }

    if let Some((start, end)) = find_named_object(&out, "UIConfig_TA")
        .or_else(|| find_named_object(&out, "UIConfig"))
    {
        let obj = &out[start..end];
        let mut obj_str = String::from_utf8_lossy(obj).into_owned();
        let mut obj_changed = false;

        if let Some(pos) = obj_str.find("\"bCrateRouletteEnabled\"") {
            if let Some(tpos) = obj_str[pos..].find("true") {
                let abs_tpos = pos + tpos;
                obj_str.replace_range(abs_tpos..abs_tpos + 4, "false");
                obj_changed = true;
            }
        } else if let Some(last_brace) = obj_str.rfind('}') {
            obj_str.insert_str(last_brace, ",\"bCrateRouletteEnabled\":false");
            obj_changed = true;
        }

        if obj_changed {
            let mut next = Vec::with_capacity(out.len() + 64);
            next.extend_from_slice(&out[..start]);
            next.extend_from_slice(obj_str.as_bytes());
            next.extend_from_slice(&out[end..]);
            out = next;
            any_changed = true;
        }
    }

    (out, any_changed)
}

fn patch_palette(body: &[u8]) -> (Vec<u8>, bool) {
    let val_str = "CarColorSet_TA'CarColors.OrangeTeamV2'";
    upsert_class_property_override(body, "Team_Soccar_TA", "CarColorSet", val_str)
}

/// Locate the byte span for the root `"ClassPropertyConfig"` JSON object.
/// Note: We avoid serde_json deserialization across the full 500KB+ CDN config payload
/// to guarantee zero key-reordering (which trips EAC/Psynet packet checksum validation).
fn find_class_property_config(body: &[u8]) -> Option<(usize, usize)> {
    let key = b"\"ClassPropertyConfig\"";
    let pos = find_bytes(body, key)?;
    let mut cursor = pos + key.len();
    while cursor < body.len() && body[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= body.len() || body[cursor] != b':' {
        return None;
    }
    cursor += 1;
    while cursor < body.len() && body[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= body.len() || body[cursor] != b'{' {
        return None;
    }
    let end_idx = scan_object_end(body, cursor)?;
    Some((cursor, end_idx + 1))
}

fn find_overrides_array(body: &[u8], obj_start: usize, obj_end: usize) -> Option<(usize, usize)> {
    let block = &body[obj_start..obj_end];
    let key = b"\"Overrides\"";
    let rel_pos = find_bytes(block, key)?;
    let mut cursor = rel_pos + key.len();
    while cursor < block.len() && block[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= block.len() || block[cursor] != b':' {
        return None;
    }
    cursor += 1;
    while cursor < block.len() && block[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= block.len() || block[cursor] != b'[' {
        return None;
    }
    let arr_open = obj_start + cursor;
    let arr_close = scan_array_end(body, arr_open)?;
    Some((arr_open, arr_close + 1))
}

fn scan_array_end(body: &[u8], start: usize) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    let mut nest_depth = 0;
    for (idx, &byte) in body[start..].iter().enumerate() {
        if in_quote {
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'"' {
                in_quote = false;
            }
            continue;
        }
        match byte {
            b'"' => in_quote = true,
            b'[' | b'{' => nest_depth += 1,
            b']' | b'}' => {
                nest_depth -= 1;
                if nest_depth == 0 {
                    if byte == b']' {
                        return Some(start + idx);
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// Modifies or inserts a single class-property override in the CDN payload.
/// Psynet config CDN responses (`/v2/Config/BattleCars/...`) supply UE3 class properties
/// via an `Overrides` list of `{ "Class": "...", "Property": "...", "Value": "..." }`.
fn upsert_class_property_override(
    body: &[u8],
    class_target: &str,
    prop_target: &str,
    target_value: &str,
) -> (Vec<u8>, bool) {
    let (cfg_start, cfg_end) = match find_class_property_config(body) {
        Some(bounds) => bounds,
        None => {
            // ClassPropertyConfig wasn't shipped in this origin build payload; synthesize it.
            let Some(tail_brace) = body.iter().rposition(|&b| b == b'}') else {
                return (body.to_vec(), false);
            };
            let synth_block = format!(
                ",\"ClassPropertyConfig\":{{\"Class\":\"ClassPropertyConfig_X\",\"Overrides\":[{{\"Class\":\"{class_target}\",\"Property\":\"{prop_target}\",\"Value\":\"{target_value}\"}}]}}"
            );
            let mut patched = Vec::with_capacity(body.len() + synth_block.len());
            patched.extend_from_slice(&body[..tail_brace]);
            patched.extend_from_slice(synth_block.as_bytes());
            patched.extend_from_slice(&body[tail_brace..]);
            return (patched, true);
        }
    };

    let (arr_start, arr_end) = match find_overrides_array(body, cfg_start, cfg_end) {
        Some(bounds) => bounds,
        None => return (body.to_vec(), false),
    };

    let inner_open = arr_start + 1;
    let inner_close = arr_end - 1;
    if inner_open > inner_close {
        return (body.to_vec(), false);
    }

    // Inspect existing array elements for matching Class + Property
    let mut scan_offset = inner_open;
    while scan_offset < inner_close {
        let Some(rel_hit) = find_bytes(&body[scan_offset..inner_close], b"\"Class\"") else {
            break;
        };
        let class_tag_idx = scan_offset + rel_hit;
        let mut entry_open = class_tag_idx;
        while entry_open > arr_start && body[entry_open] != b'{' {
            entry_open -= 1;
        }
        if body[entry_open] != b'{' {
            scan_offset = class_tag_idx + 1;
            continue;
        }
        let Some(entry_close) = scan_object_end(body, entry_open) else {
            scan_offset = class_tag_idx + 1;
            continue;
        };
        let elem_slice = &body[entry_open..=entry_close];

        let class_pattern = format!("\"Class\":\"{class_target}\"");
        let prop_pattern = format!("\"Property\":\"{prop_target}\"");

        if find_bytes(elem_slice, class_pattern.as_bytes()).is_some()
            && find_bytes(elem_slice, prop_pattern.as_bytes()).is_some()
        {
            // Found target override entry. Replace Value string slice in place.
            let val_tag = b"\"Value\":\"";
            let Some(val_tag_offset) = find_bytes(elem_slice, val_tag) else {
                return (body.to_vec(), false);
            };
            let val_start = entry_open + val_tag_offset + val_tag.len();
            let Some(val_end) = json_string_end(body, val_start) else {
                return (body.to_vec(), false);
            };

            if &body[val_start..val_end] == target_value.as_bytes() {
                return (body.to_vec(), false);
            }

            let mut patched = Vec::with_capacity(body.len() + target_value.len());
            patched.extend_from_slice(&body[..val_start]);
            patched.extend_from_slice(target_value.as_bytes());
            patched.extend_from_slice(&body[val_end..]);
            return (patched, true);
        }

        scan_offset = entry_close + 1;
    }

    // Target override not present in existing array — insert new entry object.
    let item_json = format!(
        "{{\"Class\":\"{class_target}\",\"Property\":\"{prop_target}\",\"Value\":\"{target_value}\"}}"
    );
    let current_inner = &body[inner_open..inner_close];
    let is_empty = current_inner.iter().all(|b| b.is_ascii_whitespace());

    let payload_chunk = if is_empty {
        item_json
    } else {
        format!(",{item_json}")
    };

    let mut patched = Vec::with_capacity(body.len() + payload_chunk.len());
    patched.extend_from_slice(&body[..inner_close]);
    patched.extend_from_slice(payload_chunk.as_bytes());
    patched.extend_from_slice(&body[inner_close..]);
    (patched, true)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActiveSwapRule {
    pub owned_id: i32,
    pub wanted_id: i32,
    pub paint_id: i32,
    pub slot: String,
    pub slot_idx: u8,
}

pub fn is_known_body_id(id: i32) -> bool {
    matches!(
        id,
        21 | 22 | 23 | 24 | 25 | 26 | 27 | 28 | 29 | 30
            | 31 | 402 | 403 | 404 | 523 | 597 | 600 | 607 | 625 | 723
            | 803 | 1018 | 1159 | 1171 | 1172 | 1286 | 1295 | 1300 | 1317 | 1412
            | 1416 | 1475 | 1478 | 1533 | 1568 | 1603 | 1623 | 1624 | 1675 | 1689
            | 1691 | 1856 | 1883 | 1894 | 1919 | 1932 | 2070 | 2268 | 2269 | 2298
            | 2313 | 2665 | 2666 | 2853 | 2919 | 2949 | 2950 | 2951 | 3031 | 3138
            | 3155 | 3156 | 3157 | 3265 | 3311 | 3315 | 3316 | 3426 | 3451 | 3582
            | 3594 | 3614 | 3622 | 3702 | 3875 | 3879 | 3880 | 4014 | 4155 | 4268
            | 4284 | 4318 | 4319 | 4320 | 4367 | 4472 | 4473 | 4745 | 4770 | 4780
            | 4781 | 4782 | 4861 | 4864 | 4906 | 5020 | 5039 | 5188 | 5265 | 5361
            | 5364 | 5365 | 5366 | 5367 | 5368 | 5369 | 5470 | 5488 | 5547 | 5709
            | 5713 | 5773 | 5823 | 5837 | 5858 | 5879 | 5951 | 5964 | 5979 | 6122
            | 6243 | 6244 | 6247 | 6260 | 6489 | 6836 | 6939 | 7012 | 7052 | 7211
            | 7336 | 7337 | 7338 | 7341 | 7343 | 7415 | 7477 | 7512 | 7532 | 7593
            | 7651 | 7696 | 7772 | 7815 | 7890 | 7901 | 7947 | 7948 | 7979 | 8006
            | 8360 | 8361 | 8383 | 8454 | 8524 | 8565 | 8566 | 8669 | 8806 | 8807
            | 9053 | 9084 | 9085 | 9088 | 9089 | 9140 | 9357 | 9388 | 9427 | 9894
            | 10044 | 10094 | 10440 | 10441 | 10689 | 10694 | 10695 | 10697 | 10698 | 10805
            | 10817 | 10822 | 10896 | 10897 | 10900 | 10901 | 11016 | 11038 | 11095 | 11098
            | 11138 | 11141 | 11314 | 11315 | 11336 | 11379 | 11394 | 11505 | 11534 | 11603
            | 11677 | 11736 | 11800 | 11905 | 11932 | 11933 | 11941 | 11949 | 11950 | 11996
            | 12104 | 12105 | 12106 | 12142 | 12173 | 12262 | 12286 | 12315 | 12318 | 12324
            | 12325 | 12335 | 12361 | 12382 | 12484 | 12532 | 12563 | 12569 | 12638 | 12652
            | 12657 | 12658 | 12669 | 12779 | 12781 | 12814 | 12842 | 12888 | 12951 | 12999
            | 13000 | 13008 | 13042 | 13071 | 13074 | 13075
    )
}

fn load_swap_entries_from_paths(paths: &[std::path::PathBuf]) -> Vec<serde_json::Value> {
    // A present, valid empty inventory is authoritative. Old files are only
    // fallback migration sources, otherwise restoring everything resurrects them.
    paths.iter().find_map(|path| {
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<Vec<serde_json::Value>>(&raw).ok()
    }).unwrap_or_default()
}

pub fn load_active_swap_rules(cfg: Option<&crate::psynet::SpoofPayload>) -> Vec<ActiveSwapRule> {
    let mut rules = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let candidate_paths = crate::psynet::canonical_swaps_paths();

    for entry in load_swap_entries_from_paths(&candidate_paths) {
        let wid = entry.get("wanted_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let oid = entry.get("owned_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let pid = entry.get("paint_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let slot = entry.get("slot").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let slot_idx = entry.get("slot_index")
            .and_then(|v| v.as_i64())
            .map(|n| n as u8)
            .unwrap_or_else(|| crate::presets::upk_slot_index_from_str(&slot));
        if wid > 0 && seen.insert((oid, wid, slot_idx)) {
            rules.push(ActiveSwapRule {
                owned_id: oid,
                wanted_id: wid,
                paint_id: pid,
                slot,
                slot_idx,
            });
        }
    }

    if let Some(c) = cfg {
        if let Some(inv) = &c.inventory_spoof {
            if inv.enabled {
                for it in &inv.items {
                    if it.product_id > 0 && seen.insert((0, it.product_id, 0)) {
                        let is_body = is_known_body_id(it.product_id);
                        rules.push(ActiveSwapRule {
                            owned_id: 0,
                            wanted_id: it.product_id,
                            paint_id: it.paint_id,
                            slot: if is_body { "Body".to_string() } else { "".to_string() },
                            slot_idx: if is_body { 0 } else { 255 },
                        });
                    }
                }
            }
        }
    }

    ensure_spawned_items_loaded();
    if let Ok(dyn_items) = DYNAMIC_SPAWNED_ITEMS.read() {
        for &(wid, pid) in dyn_items.iter() {
            if wid > 0 && seen.insert((0, wid, 0)) {
                let is_body = is_known_body_id(wid);
                rules.push(ActiveSwapRule {
                    owned_id: 0,
                    wanted_id: wid,
                    paint_id: pid,
                    slot: if is_body { "Body".to_string() } else { "".to_string() },
                    slot_idx: if is_body { 0 } else { 255 },
                });
            }
        }
    }

    rules
}

static DYNAMIC_SPAWNED_ITEMS: std::sync::RwLock<Vec<(i32, i32)>> = std::sync::RwLock::new(Vec::new());
static SPAWNED_ITEMS_INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CONTAINER_COUNTS: std::sync::LazyLock<std::sync::RwLock<std::collections::HashMap<i32, u32>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashMap::new()));

fn get_spawned_items_file_path() -> std::path::PathBuf {
    crate::psynet::config_dir().join("spawned_items.json")
}

fn get_legacy_title_ids_file_path() -> std::path::PathBuf {
    crate::psynet::config_dir().join("title_ids.json")
}

pub fn migrate_legacy_title_ids() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        if cfg!(test) {
            return;
        }
        let path = get_legacy_title_ids_file_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            let legacy: Vec<String> = serde_json::from_str(&content).unwrap_or_default();
            let mut ids = crate::psynet::load_spawned_titles();
            ids.extend(legacy);
            if crate::psynet::write_spawned_titles(&ids).is_ok() {
                let _ = std::fs::remove_file(&path);
                crate::applog::event("proxy: migrated title_ids.json into spoof config spawned_titles");
            }
        }
        match crate::psynet::migrate_spawn_title_swaps() {
            Ok(0) => {}
            Ok(n) => crate::applog::event(&format!("proxy: moved {n} spawned title swap(s) into spawned_titles")),
            Err(e) => crate::applog::event(&format!("proxy: title swap migration failed: {e}")),
        }
    });
}

fn store_spawned_titles(ids: &[String]) -> Result<(), String> {
    let cfg = crate::psynet::write_spawned_titles(ids)?;
    if tokio::runtime::Handle::try_current().is_ok() {
        tauri::async_runtime::spawn(set_spoof_config(cfg));
    } else {
        tauri::async_runtime::block_on(set_spoof_config(cfg));
    }
    notify_inventory_updated();
    Ok(())
}

/// Adds titles to the spawned set and returns the ones that were not already owned.
pub fn register_title_ids(ids: &[String]) -> Result<Vec<String>, String> {
    migrate_legacy_title_ids();
    let mut current = crate::psynet::load_spawned_titles();
    let new_ids: Vec<String> = crate::psynet::clean_title_ids(ids)
        .into_iter()
        .filter(|id| !current.contains(id))
        .collect();
    if new_ids.is_empty() {
        return Ok(new_ids);
    }
    current.extend(new_ids.iter().cloned());
    store_spawned_titles(&current)?;
    let push = new_ids.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = inject_title_reward_drop(&push).await {
            crate::applog::event(&format!("proxy: live title push failed: {e}"));
        }
    });
    Ok(new_ids)
}

pub fn unregister_title_id(id: &str) -> Result<(), String> {
    migrate_legacy_title_ids();
    let mut current = crate::psynet::load_spawned_titles();
    current.retain(|t| t != id.trim());
    store_spawned_titles(&current)
}

pub fn get_all_title_ids() -> Vec<String> {
    migrate_legacy_title_ids();
    crate::psynet::load_spawned_titles()
}

pub fn clear_all_title_ids() -> Result<usize, String> {
    migrate_legacy_title_ids();
    let count = crate::psynet::load_spawned_titles().len();
    store_spawned_titles(&[])?;
    Ok(count)
}

fn get_container_counts_file_path() -> std::path::PathBuf {
    crate::psynet::config_dir().join("container_counts.json")
}

fn load_persisted_container_counts() -> std::collections::HashMap<i32, u32> {
    let path = get_container_counts_file_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(counts) = serde_json::from_str::<std::collections::HashMap<i32, u32>>(&content) {
            return counts;
        }
    }
    std::collections::HashMap::new()
}

fn save_persisted_container_counts(counts: &std::collections::HashMap<i32, u32>) {
    let path = get_container_counts_file_path();
    if let Ok(data) = serde_json::to_string_pretty(counts) {
        let _ = std::fs::write(&path, data);
    }
}

pub fn get_container_quantity(product_id: i32) -> u32 {
    ensure_spawned_items_loaded();
    if let Ok(map) = CONTAINER_COUNTS.read() {
        if let Some(&qty) = map.get(&product_id) {
            if qty > 0 {
                return qty;
            }
        }
    }
    1
}

pub fn set_container_quantities(items: &[(i32, u32)]) {
    ensure_spawned_items_loaded();
    let mut updated = false;
    let mut current_map = std::collections::HashMap::new();
    if let Ok(mut map) = CONTAINER_COUNTS.write() {
        for &(id, qty) in items {
            if id > 0 {
                map.insert(id, qty);
                updated = true;
            }
        }
        current_map = map.clone();
    }
    if updated {
        save_persisted_container_counts(&current_map);
    }
}

pub fn decrement_container_quantity(product_id: i32) -> u32 {
    ensure_spawned_items_loaded();
    let mut remaining = 0;
    let mut remove = false;
    let mut current_map = std::collections::HashMap::new();
    if let Ok(mut map) = CONTAINER_COUNTS.write() {
        if let Some(entry) = map.get_mut(&product_id) {
            if *entry > 1 {
                *entry -= 1;
                remaining = *entry;
            } else {
                map.remove(&product_id);
                remove = true;
            }
        }
        current_map = map.clone();
    }
    save_persisted_container_counts(&current_map);
    if remove {
        unregister_spawned_items(&[product_id]);
    }
    remaining
}

fn load_persisted_spawned_items() -> Vec<(i32, i32)> {
    let path = get_spawned_items_file_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(items) = serde_json::from_str::<Vec<(i32, i32)>>(&content) {
            return items;
        }
    }
    Vec::new()
}

fn save_persisted_spawned_items(items: &[(i32, i32)]) {
    let path = get_spawned_items_file_path();
    if let Ok(data) = serde_json::to_string_pretty(items) {
        let _ = std::fs::write(&path, data);
    }
    // Synchronize into cfg.inventory_spoof in psynet_config.json so the UI and game config stay in lockstep
    if let Ok(dir) = crate::psynet::find_proxy_dir() {
        if let Some(mut cfg) = crate::psynet::load_active_spoof_from_disk() {
            let mut inv = cfg.inventory_spoof.unwrap_or_default();
            inv.enabled = !items.is_empty();
            inv.items.retain(|it| items.iter().any(|&(pid, paint)| pid == it.product_id && paint == it.paint_id));
            for &(pid, paint) in items {
                if !inv.items.iter().any(|it| it.product_id == pid && it.paint_id == paint) {
                    let sid = lookup_series_id_for_product(pid);
                    inv.items.push(crate::psynet::InventorySpoofItemPayload {
                        product_id: pid,
                        paint_id: paint,
                        series_id: sid,
                        slot: String::new(),
                        dlc: false,
                    });
                }
            }
            cfg.inventory_spoof = Some(inv);
            let _ = crate::psynet::write_spoof(&dir, &cfg);
        }
    }
}

static ITEMS_LOOKUP_MAP: std::sync::LazyLock<std::collections::HashMap<i32, (String, String, String)>> = std::sync::LazyLock::new(|| {
    let mut map = std::collections::HashMap::new();
    const BUNDLED: &str = include_str!("../../resources/items.json");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(BUNDLED) {
        let items_arr = v.get("items").or_else(|| v.get("Items")).and_then(|a| a.as_array());
        if let Some(arr) = items_arr {
            for it in arr {
                let id = it.get("ID").or_else(|| it.get("id")).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if id > 0 {
                    let name = it.get("Product").or_else(|| it.get("product")).or_else(|| it.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let slot = it.get("Slot").or_else(|| it.get("slot")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let pkg = it.get("AssetPackage").or_else(|| it.get("asset_package")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    map.insert(id, (name, slot, pkg));
                }
            }
        }
    }
    map
});

pub fn lookup_product_details(pid: i32) -> (String, String, String) {
    if let Some(entry) = ITEMS_LOOKUP_MAP.get(&pid) {
        entry.clone()
    } else {
        (format!("Item #{pid}"), "Cosmetic".to_string(), String::new())
    }
}

pub fn candidate_swaps_paths() -> Vec<std::path::PathBuf> {
    crate::psynet::canonical_swaps_paths()
}

pub fn sync_spawned_items_to_swaps(items: &[(i32, i32)]) {
    if items.is_empty() {
        return;
    }
    let candidate_paths = candidate_swaps_paths();
    let mut existing_swaps: Vec<serde_json::Value> = Vec::new();
    for p in &candidate_paths {
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(p) {
                if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    if !arr.is_empty() {
                        existing_swaps = arr;
                        break;
                    }
                }
            }
        }
    }

    let mut changed = false;
    let now_ts = chrono::Utc::now().to_rfc3339();

    for &(pid, paint) in items {
        if pid <= 0 {
            continue;
        }
        let already_present = existing_swaps.iter().any(|s| {
            let wid = s.get("wanted_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let oid = s.get("owned_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let p = s.get("paint_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            wid == pid && oid == pid && p == paint
        });

        if !already_present {
            let (name, slot, asset_pkg) = lookup_product_details(pid);
            let entry = serde_json::json!({
                "owned_id": pid,
                "wanted_id": pid,
                "owned_name": name,
                "wanted_name": name,
                "owned_paint_id": null,
                "owned_custom_hex": null,
                "paint_id": paint,
                "custom_paint_hex": null,
                "asset_package": asset_pkg,
                "slot": slot,
                "timestamp": now_ts
            });
            existing_swaps.push(entry);
            changed = true;
        }
    }

    if changed {
        if let Ok(json_data) = serde_json::to_vec_pretty(&existing_swaps) {
            for target in &candidate_paths {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(target, &json_data);
            }
        }
    }
}

static GLOBAL_APP_HANDLE: std::sync::RwLock<Option<tauri::AppHandle>> = std::sync::RwLock::new(None);

pub fn set_app_handle(handle: tauri::AppHandle) {
    if let Ok(mut g) = GLOBAL_APP_HANDLE.write() {
        *g = Some(handle);
    }
}

pub fn get_app_handle() -> Option<tauri::AppHandle> {
    GLOBAL_APP_HANDLE.read().ok().and_then(|g| g.clone())
}

pub fn notify_inventory_updated() {
    use tauri::Emitter;
    if let Some(app) = get_app_handle() {
        let _ = app.emit("inventory-updated", ());
    }
}

fn ensure_spawned_items_loaded() {
    if !SPAWNED_ITEMS_INITIALIZED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let persisted = load_persisted_spawned_items();
        if !persisted.is_empty() {
            if let Ok(mut g) = DYNAMIC_SPAWNED_ITEMS.write() {
                for item in persisted {
                    if !g.contains(&item) {
                        g.push(item);
                    }
                }
            }
        }
        let persisted_counts = load_persisted_container_counts();
        if !persisted_counts.is_empty() {
            if let Ok(mut map) = CONTAINER_COUNTS.write() {
                for (id, qty) in persisted_counts {
                    map.insert(id, qty);
                }
            }
        }
    }
}

static CLIENT_WS_SENDER: std::sync::RwLock<Option<tokio::sync::mpsc::UnboundedSender<tokio_tungstenite::tungstenite::Message>>> = std::sync::RwLock::new(None);
static NOTIFIED_SPAWNED_ITEMS: std::sync::LazyLock<std::sync::RwLock<std::collections::HashSet<(i32, i32)>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashSet::new()));

pub fn register_client_ws_sender(sender: tokio::sync::mpsc::UnboundedSender<tokio_tungstenite::tungstenite::Message>) {
    if let Ok(mut g) = CLIENT_WS_SENDER.write() {
        *g = Some(sender);
    }
    // Do NOT clear NOTIFIED_SPAWNED_ITEMS on reconnect: WebSocket reconnects during gameplay
    // must not re-dispatch duplicate RewardResult frames for items the game already possesses.
    ensure_inventory_reconciliation_worker_started();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async {
            reconcile_spawned_inventory().await;
        });
    }
}

pub fn unregister_client_ws_sender() {
    if let Ok(mut g) = CLIENT_WS_SENDER.write() {
        *g = None;
    }
}

pub fn has_active_client_ws() -> bool {
    if let Ok(g) = CLIENT_WS_SENDER.read() {
        g.is_some()
    } else {
        false
    }
}

pub async fn reconcile_spawned_inventory() {
    if !has_active_client_ws() {
        return;
    }
    // Only reconcile items from DYNAMIC_SPAWNED_ITEMS (items spawned or unboxed in current/persisted profile).
    // Swapped donor items (where owned_id != wanted_id) must NEVER be pushed as standalone reward drops!
    let all_spawned: Vec<(i32, i32)> = {
        if let Ok(g) = DYNAMIC_SPAWNED_ITEMS.read() {
            g.clone()
        } else {
            Vec::new()
        }
    };
    if all_spawned.is_empty() {
        return;
    }

    let missing: Vec<(i32, i32)> = {
        let notified = NOTIFIED_SPAWNED_ITEMS.read().unwrap_or_else(|e| e.into_inner());
        all_spawned.into_iter().filter(|item| item.0 > 0 && !notified.contains(item)).collect()
    };

    if !missing.is_empty() {
        crate::applog::event(&format!("proxy: inventory reconciler pushing {} missing spawned items live", missing.len()));
        if let Ok(true) = inject_reward_drop(&missing).await {
            if let Ok(mut notified) = NOTIFIED_SPAWNED_ITEMS.write() {
                for item in missing {
                    notified.insert(item);
                }
            }
        }
    }
}

pub fn ensure_inventory_reconciliation_worker_started() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(2500));
                loop {
                    interval.tick().await;
                    reconcile_spawned_inventory().await;
                }
            });
        }
    }
}

/// Construct and send downstream `PsyService: Reward/RewardResult` frame
/// with authentic series IDs to make items pop up and drop into player inventory live in-game.
pub async fn inject_reward_drop_with_series(items: &[(i32, i32, i32)]) -> Result<bool, String> {
    if items.is_empty() {
        return Ok(false);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut product_data = Vec::new();
    for &(pid, paint, sid) in items {
        if pid <= 0 {
            continue;
        }
        let inst_hex = format!("{:08x}{:08x}0000000000000000", pid as u32, paint.max(0) as u32);
        let series_id = if sid > 0 { sid } else { lookup_series_id_for_product(pid) };
        let mut prod_obj = serde_json::json!({
            "ProductID": pid,
            "InstanceID": inst_hex,
            "SeriesID": series_id,
            "Amount": 1,
            "ExpirationTime": -1,
            "IsTradable": false,
            "TradeHold": -1,
            "AddedTimestamp": now,
            "UpdatedTimestamp": now,
            "bSkipRoulette": true,
            "bUnlocked": true,
        });
        // Always emit both top-level Paint fields AND Attributes array — the game reads both paths
        // depending on context (RewardResult animation uses top-level Paint, inventory reads Attributes).
        let mut attrs = serde_json::json!([
            { "Key": "bSkipRoulette", "Value": "true" },
            { "Key": "bUnlocked",     "Value": "true" }
        ]);
        if paint > 0 {
            prod_obj["Paint"] = serde_json::json!(paint);
            prod_obj["PaintID"] = serde_json::json!(paint);
            if let Some(arr) = attrs.as_array_mut() {
                arr.insert(0, serde_json::json!({ "Key": "Painted", "Value": paint.to_string() }));
                arr.insert(1, serde_json::json!({ "Key": "Paint",   "Value": paint.to_string() }));
            }
        }
        prod_obj["Attributes"] = attrs;
        product_data.push(prod_obj);
    }

    if product_data.is_empty() {
        return Ok(false);
    }

    send_reward_result_frame(product_data).await
}

async fn send_reward_result_frame(product_data: Vec<serde_json::Value>) -> Result<bool, String> {
    let sender_opt = CLIENT_WS_SENDER.read().ok().and_then(|g| g.clone());
    let Some(sender) = sender_opt else {
        crate::applog::event("proxy: no active client WebSocket tunnel to push RewardResult frame");
        return Ok(false);
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let count = product_data.len();

    let body_json = serde_json::json!({
        "Drops":           product_data,
        "ProductData":     product_data,
        "RewardDrops":     product_data,
        "CurrencyDrops":   [],
        "ChallengeRewards": [],
        "RocketPassInfo":  {},
        "bSkipRoulette":   true,
        "bUnlocked":       true,
        "bSuccess":        true
    });

    let body_str = serde_json::to_string(&body_json).map_err(|e| e.to_string())?;
    let time_str = now.to_string();

    let mut mac = Hmac::<Sha256>::new_from_slice(PSY_RESP_KEY).map_err(|e| e.to_string())?;
    mac.update(format!("{time_str}-").as_bytes());
    mac.update(body_str.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let frame = format!(
        "PsyService: Reward/RewardResult\r\nPsyServiceVersion: 2\r\nPsyTime: {time_str}\r\nPsySig: {sig}\r\n\r\n{body_str}"
    );

    crate::applog::event(&format!(
        "proxy: injecting live Reward/RewardResult frame for {count} product(s) downstream over WebSocket"
    ));

    sender
        .send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
        .map_err(|e| format!("Failed to dispatch WebSocket frame to client: {e}"))?;

    Ok(true)
}

pub async fn inject_title_reward_drop(title_ids: &[String]) -> Result<bool, String> {
    if title_ids.is_empty() {
        return Ok(false);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let product_data = title_ids.iter().map(|t| title_product_json(t, now)).collect();
    send_reward_result_frame(product_data).await
}

pub async fn inject_reward_drop(items: &[(i32, i32)]) -> Result<bool, String> {
    let items_with_series: Vec<(i32, i32, i32)> = items
        .iter()
        .map(|&(pid, paint)| (pid, paint, lookup_series_id_for_product(pid)))
        .collect();
    let res = inject_reward_drop_with_series(&items_with_series).await?;
    if res {
        if let Ok(mut notified) = NOTIFIED_SPAWNED_ITEMS.write() {
            for &item in items {
                notified.insert(item);
            }
        }
    }
    Ok(res)
}

/// Register items into the dynamic spawned inventory.
/// Returns the subset of `items` that were **actually new** (not already present).
/// Callers should only push a RewardResult WS frame for the returned new items to avoid
/// duplicate drop animations and phantom "2x" counts in garage.
pub fn register_spawned_items(items: &[(i32, i32)]) -> Vec<(i32, i32)> {
    ensure_spawned_items_loaded();
    let mut newly_added: Vec<(i32, i32)> = Vec::new();
    let mut current_items = Vec::new();
    if let Ok(mut g) = DYNAMIC_SPAWNED_ITEMS.write() {
        for &(id, paint) in items {
            if id > 0 && !g.contains(&(id, paint)) {
                g.push((id, paint));
                newly_added.push((id, paint));
            }
        }
        current_items = g.clone();
    }
    if !newly_added.is_empty() {
        save_persisted_spawned_items(&current_items);
        sync_spawned_items_to_swaps(&newly_added);
        notify_inventory_updated();
        ensure_inventory_reconciliation_worker_started();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                reconcile_spawned_inventory().await;
            });
        }
    }
    newly_added
}

pub fn unregister_spawned_items(product_ids: &[i32]) {
    ensure_spawned_items_loaded();
    let mut current_items = Vec::new();
    if let Ok(mut items) = DYNAMIC_SPAWNED_ITEMS.write() {
        items.retain(|(id, _)| !product_ids.contains(id));
        current_items = items.clone();
    }
    save_persisted_spawned_items(&current_items);
    let mut current_counts = std::collections::HashMap::new();
    if let Ok(mut map) = CONTAINER_COUNTS.write() {
        for id in product_ids {
            map.remove(id);
        }
        current_counts = map.clone();
    }
    save_persisted_container_counts(&current_counts);
    if let Ok(mut notified) = NOTIFIED_SPAWNED_ITEMS.write() {
        notified.retain(|(id, _)| !product_ids.contains(id));
    }
    // Also remove from swaps.json so active swap rules don't resurrect the unregistered item
    for p in &candidate_swaps_paths() {
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(p) {
                if let Ok(mut arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    let prev_len = arr.len();
                    arr.retain(|s| {
                        let wid = s.get("wanted_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let oid = s.get("owned_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        !(product_ids.contains(&wid) || product_ids.contains(&oid))
                    });
                    if arr.len() != prev_len {
                        if let Ok(data) = serde_json::to_vec_pretty(&arr) {
                            let _ = std::fs::write(p, data);
                        }
                    }
                }
            }
        }
    }
    notify_inventory_updated();
}

pub fn clear_all_swap_files() {
    for p in &candidate_swaps_paths() {
        if p.is_file() {
            let _ = std::fs::write(p, b"[]");
        }
    }
    notify_inventory_updated();
}

pub fn clear_dynamic_spawned_items() {
    if let Ok(mut g) = DYNAMIC_SPAWNED_ITEMS.write() {
        g.clear();
    }
    save_persisted_spawned_items(&[]);
    if let Ok(mut map) = CONTAINER_COUNTS.write() {
        map.clear();
    }
    save_persisted_container_counts(&std::collections::HashMap::new());
    if let Ok(mut notified) = NOTIFIED_SPAWNED_ITEMS.write() {
        notified.clear();
    }
    for p in &candidate_swaps_paths() {
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(p) {
                if let Ok(mut arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    let prev_len = arr.len();
                    arr.retain(|s| {
                        let wid = s.get("wanted_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let oid = s.get("owned_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        wid != oid
                    });
                    if arr.len() != prev_len {
                        if let Ok(data) = serde_json::to_vec_pretty(&arr) {
                            let _ = std::fs::write(p, data);
                        }
                    }
                }
            }
        }
    }
    notify_inventory_updated();
}

pub fn load_active_spawned_items(cfg: Option<&crate::psynet::SpoofPayload>) -> Vec<(i32, i32)> {
    ensure_spawned_items_loaded();
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(g) = DYNAMIC_SPAWNED_ITEMS.read() {
        for &(pid, paint) in g.iter() {
            if pid > 0 && seen.insert((pid, paint)) {
                items.push((pid, paint));
            }
        }
    }
    for r in load_active_swap_rules(cfg) {
        if r.wanted_id > 0 && seen.insert((r.wanted_id, r.paint_id)) {
            items.push((r.wanted_id, r.paint_id));
        }
    }
    items
}

fn find_server_address(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::Object(map) => {
            // Match Game Server address (ServerAddress or GameURL).
            // Never fall back to PingURL: PingURL is a QoS ping beacon, not the
            // match server; overwriting target_server with PingURL causes first-queue
            // connection timeouts.
            for key in &["ServerAddress", "GameURL"] {
                if let Some(s) = map.get(*key).and_then(|v| v.as_str()) {
                    if !s.starts_with("127.0.0.1") && s.contains(':') {
                        return Some(s.to_string());
                    }
                }
            }
            for (_k, v) in map {
                if let Some(s) = v.as_str() {
                    let trimmed = s.trim();
                    if trimmed.starts_with('{') && trimmed.ends_with('}') {
                        if let Ok(inner) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if let Some(found) = find_server_address(&inner) {
                                return Some(found);
                            }
                        }
                    }
                } else if let Some(found) = find_server_address(v) {
                    return Some(found);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(found) = find_server_address(v) {
                    return Some(found);
                }
            }
        }
        _ => {}
    }
    None
}

/// Recursively search for modern Rocket League match security keys (`Keys`: `{ Key, IV, HMACKey }`)
/// or flat keys in reservation JSON.
pub fn find_reservation_crypto(val: &serde_json::Value) -> Option<crate::udp_proxy::ReservationCrypto> {
    match val {
        serde_json::Value::Object(map) => {
            // Check direct "Keys" container object
            if let Some(keys_obj) = map.get("Keys").and_then(|v| v.as_object()) {
                if let (Some(k), Some(iv), Some(h)) = (
                    keys_obj.get("Key").and_then(|v| v.as_str()),
                    keys_obj.get("IV").and_then(|v| v.as_str()),
                    keys_obj.get("HMACKey").and_then(|v| v.as_str()),
                ) {
                    if let Some(crypto) = parse_reservation_crypto_keys(k, iv, h) {
                        return Some(crypto);
                    }
                }
            }

            // Check flat keys in map
            if let (Some(k), Some(iv), Some(h)) = (
                map.get("Key").and_then(|v| v.as_str()),
                map.get("IV").and_then(|v| v.as_str()),
                map.get("HMACKey").and_then(|v| v.as_str()),
            ) {
                if let Some(crypto) = parse_reservation_crypto_keys(k, iv, h) {
                    return Some(crypto);
                }
            }

            for (_k, v) in map {
                if let Some(s) = v.as_str() {
                    let trimmed = s.trim();
                    if trimmed.starts_with('{') && trimmed.ends_with('}') {
                        if let Ok(inner) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if let Some(found) = find_reservation_crypto(&inner) {
                                return Some(found);
                            }
                        }
                    }
                } else if let Some(found) = find_reservation_crypto(v) {
                    return Some(found);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(found) = find_reservation_crypto(v) {
                    return Some(found);
                }
            }
        }
        _ => {}
    }
    None
}

/// Parse base64, hex, or raw 32-byte Key, 16-byte IV, and 32-byte HMACKey.
fn parse_reservation_crypto_keys(
    key_str: &str,
    iv_str: &str,
    hmac_key_str: &str,
) -> Option<crate::udp_proxy::ReservationCrypto> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;

    let parse_bytes = |s: &str, expected_len: usize| -> Option<Vec<u8>> {
        let trimmed = s.trim();
        // Try Base64
        if let Ok(b) = b64.decode(trimmed) {
            if b.len() == expected_len {
                return Some(b);
            }
        }
        // Try Hex
        if trimmed.len() == expected_len * 2 {
            if let Ok(b) = (0..trimmed.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&trimmed[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
            {
                if b.len() == expected_len {
                    return Some(b);
                }
            }
        }
        // Try Raw bytes
        if trimmed.as_bytes().len() == expected_len {
            return Some(trimmed.as_bytes().to_vec());
        }
        None
    };

    let key_bytes = parse_bytes(key_str, 32)?;
    let iv_bytes = parse_bytes(iv_str, 16)?;
    let hmac_bytes = parse_bytes(hmac_key_str, 32)?;

    let mut aes_key = [0u8; 32];
    let mut aes_iv = [0u8; 16];
    let mut hmac_key = [0u8; 32];

    aes_key.copy_from_slice(&key_bytes);
    aes_iv.copy_from_slice(&iv_bytes);
    hmac_key.copy_from_slice(&hmac_bytes);

    Some(crate::udp_proxy::ReservationCrypto::new(aes_key, aes_iv, hmac_key))
}

fn patch_product_ids_node(
    node: &mut serde_json::Value,
    rules: &[ActiveSwapRule],
    changed: &mut bool,
) {
    if let Some(arr) = node.as_array_mut() {
        for r in rules {
            if r.wanted_id <= 0 {
                continue;
            }
            if r.owned_id > 0 {
                for v in arr.iter_mut() {
                    if v.as_i64().map(|n| n as i32) == Some(r.owned_id) {
                        if v.as_i64().map(|n| n as i32) != Some(r.wanted_id) {
                            *v = serde_json::json!(r.wanted_id);
                            *changed = true;
                        }
                    }
                }
            }
            let is_body = r.slot_idx == 0 || r.slot.eq_ignore_ascii_case("Body") || is_known_body_id(r.wanted_id);
            if is_body {
                let mut body_replaced = false;
                for v in arr.iter_mut() {
                    let id = v.as_i64().map(|n| n as i32).unwrap_or(0);
                    if (id == 11603 || is_known_body_id(id) || (r.owned_id == 23 && id == 23)) && id != r.wanted_id {
                        *v = serde_json::json!(r.wanted_id);
                        *changed = true;
                        body_replaced = true;
                        break;
                    }
                }
                if body_replaced {
                    continue;
                }
            }
            let already = arr.iter().any(|v| v.as_i64().map(|n| n as i32) == Some(r.wanted_id));
            if !already {
                arr.push(serde_json::json!(r.wanted_id));
                *changed = true;
            }
        }
    }
}

fn patch_loadout_int_array(
    arr: &mut Vec<serde_json::Value>,
    rules: &[ActiveSwapRule],
    changed: &mut bool,
) {
    for r in rules {
        if r.wanted_id <= 0 {
            continue;
        }

        let mut replaced = false;

        // 1. If owned_id is specified (> 0) and present in arr, replace it in place
        if r.owned_id > 0 {
            for val in arr.iter_mut() {
                if val.as_i64().map(|n| n as i32) == Some(r.owned_id) {
                    if val.as_i64().map(|n| n as i32) != Some(r.wanted_id) {
                        *val = serde_json::json!(r.wanted_id);
                        *changed = true;
                    }
                    replaced = true;
                    break;
                }
            }
        }

        // 2. Map by slot index if known (Body = 0, Decal = 1, Wheels = 2, Boost = 3, etc.)
        let is_body = r.slot_idx == 0
            || r.slot.eq_ignore_ascii_case("Body")
            || is_known_body_id(r.wanted_id)
            || (r.owned_id > 0 && is_known_body_id(r.owned_id));

        let slot_idx = if is_body {
            Some(0)
        } else if r.slot_idx != 255 {
            Some(r.slot_idx as usize)
        } else {
            crate::savedata::slot_name_to_index(&r.slot)
        };

        if !replaced {
            if let Some(idx) = slot_idx {
                if idx < arr.len() {
                    let cur_id = arr[idx].as_i64().map(|n| n as i32).unwrap_or(0);
                    // For body slot (idx 0), always replace if rule targets a body
                    if is_body || cur_id != r.wanted_id {
                        if cur_id != r.wanted_id {
                            arr[idx] = serde_json::json!(r.wanted_id);
                            *changed = true;
                        }
                        replaced = true;
                    }
                } else if idx == arr.len() {
                    // Exact next slot index: push to maintain correct slot alignment
                    arr.push(serde_json::json!(r.wanted_id));
                    *changed = true;
                    replaced = true;
                }
            }
        }

        // 3. If not replaced and wanted_id not already in array, append
        if !replaced {
            let already_present = arr.iter().any(|v| v.as_i64().map(|n| n as i32) == Some(r.wanted_id));
            if !already_present {
                arr.push(serde_json::json!(r.wanted_id));
                *changed = true;
            }
        }
    }
}

fn patch_loadout_node(
    loadout: &mut serde_json::Value,
    rules: &[ActiveSwapRule],
    changed: &mut bool,
) {
    if rules.is_empty() {
        return;
    }

    if let Some(map) = loadout.as_object_mut() {
        if let Some(products_val) = map.get_mut("Products") {
            if let Some(arr) = products_val.as_array_mut() {
                let is_int_arr = arr.iter().any(|v| v.is_i64() || v.is_u64());
                if is_int_arr {
                    patch_loadout_int_array(arr, rules, changed);
                } else {
                    for r in rules {
                        if r.wanted_id <= 0 {
                            continue;
                        }
                        let mut replaced = false;
                        for obj in arr.iter_mut() {
                            let pid = obj.get("ProductID")
                                .or_else(|| obj.get("ProductId"))
                                .or_else(|| obj.get("id"))
                                .and_then(|v| v.as_i64())
                                .map(|n| n as i32);
                            let slot_idx = obj.get("SlotIndex").and_then(|v| v.as_i64()).map(|n| n as u8);

                            let is_body_rule = (r.slot_idx == 0 || r.slot.eq_ignore_ascii_case("Body"))
                                && is_known_body_id(r.wanted_id);
                            let is_body_slot = is_body_rule
                                && (slot_idx == Some(0) || is_known_body_id(pid.unwrap_or(0)));
                            let is_matching_owned = r.owned_id > 0 && pid == Some(r.owned_id);
                            let is_matching_slot = r.owned_id > 0 && r.slot_idx < 255 && slot_idx == Some(r.slot_idx);
                            let is_matching_wanted = pid == Some(r.wanted_id);

                            // When replacing a body slot, ensure we only replace if slot_idx == Some(0) or target/source is a body,
                            // never accidentally replace non-body slots unless slot index matches.
                            let should_replace = if is_body_rule {
                                is_body_slot
                            } else {
                                is_matching_owned || is_matching_slot || is_matching_wanted
                            };

                            if should_replace {
                                if pid != Some(r.wanted_id) {
                                    obj["ProductID"] = serde_json::json!(r.wanted_id);
                                    *changed = true;
                                }
                                if r.paint_id > 0 {
                                    if obj.get("PaintID").and_then(|v| v.as_i64()).map(|n| n as i32) != Some(r.paint_id) {
                                        obj["PaintID"] = serde_json::json!(r.paint_id);
                                        obj["Paint"] = serde_json::json!(r.paint_id);
                                        *changed = true;
                                    }
                                    let needs_attr = match obj.get("Attributes").and_then(|v| v.as_array()) {
                                        Some(attrs) => !attrs.iter().any(|a| {
                                            let is_paint_key = a.get("Key").and_then(|k| k.as_str()).map(|k| k.eq_ignore_ascii_case("Painted") || k.eq_ignore_ascii_case("Paint")).unwrap_or(false);
                                            is_paint_key && (a.get("Value").and_then(|v| v.as_str()) == Some(&r.paint_id.to_string())
                                                    || a.get("Value").and_then(|v| v.as_i64()) == Some(r.paint_id as i64))
                                        }),
                                        None => true,
                                    };
                                    if needs_attr {
                                        obj["Attributes"] = serde_json::json!([
                                            { "Key": "Painted", "Value": r.paint_id.to_string() },
                                            { "Key": "Paint", "Value": r.paint_id.to_string() }
                                        ]);
                                        *changed = true;
                                    }
                                }
                                replaced = true;
                                break;
                            }
                        }
                        if !replaced {
                            let mut obj = serde_json::json!({
                                "ProductID": r.wanted_id,
                                "SlotIndex": r.slot_idx,
                                "PaintID": r.paint_id,
                                "Paint": r.paint_id,
                            });
                            if r.paint_id > 0 {
                                obj["Attributes"] = serde_json::json!([
                                    { "Key": "Painted", "Value": r.paint_id.to_string() },
                                    { "Key": "Paint", "Value": r.paint_id.to_string() }
                                ]);
                            }
                            arr.push(obj);
                            *changed = true;
                        }
                    }
                }
            }
        } else {
            let mut arr = Vec::new();
            for r in rules {
                if r.wanted_id > 0 {
                    let mut obj = serde_json::json!({
                        "ProductID": r.wanted_id,
                        "SlotIndex": r.slot_idx,
                        "PaintID": r.paint_id,
                        "Paint": r.paint_id,
                    });
                    if r.paint_id > 0 {
                        obj["Attributes"] = serde_json::json!([
                            { "Key": "Painted", "Value": r.paint_id.to_string() },
                            { "Key": "Paint", "Value": r.paint_id.to_string() }
                        ]);
                    }
                    arr.push(obj);
                }
            }
            if !arr.is_empty() {
                map.insert("Products".to_string(), serde_json::Value::Array(arr));
                *changed = true;
            }
        }

        if let Some(prod_ids_val) = map.get_mut("ProductIDs") {
            patch_product_ids_node(prod_ids_val, rules, changed);
        }
    } else if let Some(arr) = loadout.as_array_mut() {
        let is_int_arr = arr.iter().any(|v| v.is_i64() || v.is_u64());
        if is_int_arr {
            patch_loadout_int_array(arr, rules, changed);
        }
    }
}

fn patch_reservation_value_sync(
    val: &mut serde_json::Value,
    proxy_port: u16,
    rules: &[ActiveSwapRule],
    learned_pid: Option<&str>,
    changed: &mut bool,
) {
    match val {
        serde_json::Value::Object(map) => {
            // Reroute match server addresses to local UDP proxy (127.0.0.1:{proxy_port})
            // Intercepts UDP traffic, decrypts AES-256-CBC with reservation keys,
            // patches bunches, recomputes HMAC-SHA256, and relays upstream.
            if proxy_port > 0 {
                let target_loopback = format!("127.0.0.1:{proxy_port}");
                for key in &["ServerAddress", "GameURL"] {
                    if let Some(s) = map.get(*key).and_then(|v| v.as_str()) {
                        if !s.starts_with("127.0.0.1:") {
                            map.insert((*key).to_string(), serde_json::json!(target_loopback));
                            *changed = true;
                        }
                    }
                }
            }

            if let Some(players_val) = map.get_mut("Players") {
                if let Some(players_arr) = players_val.as_array_mut() {
                    let total_players = players_arr.len();
                    for player in players_arr.iter_mut() {
                        let is_remote = player.get("bRemotePlayer").and_then(|v| v.as_bool()).unwrap_or(false);
                        let is_primary = player.get("bPrimaryPlayer").and_then(|v| v.as_bool()).unwrap_or(false);
                        let mut is_local = is_primary || (!is_remote && total_players <= 1);

                        if !is_remote {
                            is_local = true;
                        }

                        if let Some(pid_val) = player.get("PlayerID") {
                            if let Some(s) = pid_val.as_str() {
                                let norm_pid = normalize_player_id(s);
                                if let Some(learned) = learned_pid {
                                    if norm_pid == learned {
                                        is_local = true;
                                    }
                                }
                            }
                        }

                        if is_local {
                            let (title_id, title_text) = configured_title();
                            let curr_title = player.get("TitleID").and_then(|v| v.as_str()).unwrap_or_default();
                            let curr_pt = player.get("PlayerTitle").and_then(|v| v.as_str()).unwrap_or_default();
                            if !title_id.is_empty() && (curr_title != title_id || curr_pt != title_text) {
                                if let Some(p) = player.as_object_mut() {
                                    p.insert("TitleID".to_string(), serde_json::json!(title_id));
                                    p.insert("TitleId".to_string(), serde_json::json!(title_id));
                                    p.insert("PlayerTitle".to_string(), serde_json::json!(title_text));
                                    p.insert("Title".to_string(), serde_json::json!(title_text));
                                    *changed = true;
                                }
                            }

                            // Inject custom title and name if active spoof config defines one
                            if let Some(spoof) = crate::psynet::load_active_spoof_from_disk() {
                                {
                                    let equip_id = if !spoof.equip_title_id.trim().is_empty() {
                                        spoof.equip_title_id.trim()
                                    } else {
                                        spoof.display_title_id.trim()
                                    };
                                    let mut custom_text = spoof.custom_text.trim();
                                    if custom_text.is_empty() {
                                        custom_text = spoof.custom_name.trim();
                                    }
                                    if !equip_id.is_empty() {
                                        if let Some(p) = player.as_object_mut() {
                                            p.insert("TitleID".to_string(), serde_json::json!(equip_id));
                                            if !custom_text.is_empty() {
                                                p.insert("PlayerTitle".to_string(), serde_json::json!(custom_text));
                                                p.insert("Title".to_string(), serde_json::json!(custom_text));
                                            }
                                            *changed = true;
                                        }
                                    }

                                    // Name spoofing on reservation player object
                                    let spoof_name = spoof.name_spoof.as_ref()
                                        .filter(|ns| ns.enabled && !ns.display_name.trim().is_empty())
                                        .map(|ns| ns.display_name.trim())
                                        .or_else(|| {
                                            if !spoof.custom_name.trim().is_empty() {
                                                Some(spoof.custom_name.trim())
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(target_name) = spoof_name {
                                        if let Some(p) = player.as_object_mut() {
                                            p.insert("PlayerName".to_string(), serde_json::json!(target_name));
                                            p.insert("Name".to_string(), serde_json::json!(target_name));
                                            *changed = true;
                                        }
                                    }
                                }
                            }

                            if let Some(loadout_val) = player.get_mut("Loadout") {
                                patch_loadout_node(loadout_val, rules, changed);
                            }
                            if let Some(loadouts_val) = player.get_mut("Loadouts") {
                                if let Some(arr) = loadouts_val.as_array_mut() {
                                    for l in arr.iter_mut() {
                                        patch_loadout_node(l, rules, changed);
                                    }
                                } else {
                                    patch_loadout_node(loadouts_val, rules, changed);
                                }
                            }
                            if let Some(online_val) = player.get_mut("OnlineLoadout") {
                                patch_loadout_node(online_val, rules, changed);
                            }
                            if let Some(online_arr) = player.get_mut("OnlineLoadouts").and_then(|v| v.as_array_mut()) {
                                for l in online_arr.iter_mut() {
                                    patch_loadout_node(l, rules, changed);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(loadout_val) = map.get_mut("Loadout") {
                patch_loadout_node(loadout_val, rules, changed);
            }

            if let Some(loadouts_val) = map.get_mut("Loadouts") {
                if let Some(arr) = loadouts_val.as_array_mut() {
                    for l in arr.iter_mut() {
                        patch_loadout_node(l, rules, changed);
                    }
                } else {
                    patch_loadout_node(loadouts_val, rules, changed);
                }
            }

            if let Some(online_val) = map.get_mut("OnlineLoadout") {
                patch_loadout_node(online_val, rules, changed);
            }

            if let Some(online_arr) = map.get_mut("OnlineLoadouts").and_then(|v| v.as_array_mut()) {
                for l in online_arr.iter_mut() {
                    patch_loadout_node(l, rules, changed);
                }
            }

            if let Some(prod_ids_val) = map.get_mut("ProductIDs") {
                patch_product_ids_node(prod_ids_val, rules, changed);
            }

            // Products.PlayWithLoadout or PlayWithLoadout event support
            let is_play_with_loadout = map.get("EventName")
                .and_then(|v| v.as_str())
                .map(|s| s.contains("PlayWithLoadout") || s.contains("Products.PlayWithLoadout"))
                .unwrap_or(false);
            if is_play_with_loadout {
                if let Some(event_data) = map.get_mut("EventData").and_then(|v| v.as_object_mut()) {
                    let is_primary = event_data.get("bPrimaryPlayer").and_then(|v| v.as_bool()).unwrap_or(true);
                    if is_primary {
                        if let Some(loadout_val) = event_data.get_mut("Loadout") {
                            patch_loadout_node(loadout_val, rules, changed);
                        }
                        if let Some(prods_val) = event_data.get_mut("Products") {
                            patch_loadout_node(prods_val, rules, changed);
                        }
                        if let Some(prod_ids_val) = event_data.get_mut("ProductIDs") {
                            patch_product_ids_node(prod_ids_val, rules, changed);
                        }

                        let ev_player_id = event_data.get("PlayerID")
                            .or_else(|| event_data.get("PlayerId"))
                            .or_else(|| event_data.get("EpicAccountId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let ev_norm = normalize_player_id(ev_player_id);
                        let ev_is_local = if let Some(learned) = learned_pid {
                            ev_norm == learned
                        } else {
                            true
                        };

                        if ev_is_local {
                            let (title_id, title_text) = configured_title();
                            if !title_id.is_empty() {
                                event_data.insert("TitleID".to_string(), serde_json::json!(title_id));
                                event_data.insert("TitleId".to_string(), serde_json::json!(title_id));
                                event_data.insert("PlayerTitle".to_string(), serde_json::json!(title_text));
                                event_data.insert("Title".to_string(), serde_json::json!(title_text));
                                *changed = true;
                            }
                            if let Some(spoof) = crate::psynet::load_active_spoof_from_disk() {

                                // Also patch PlayerName in PlayWithLoadout event if name spoofing
                                let spoof_name = spoof.name_spoof.as_ref()
                                    .filter(|ns| ns.enabled && !ns.display_name.trim().is_empty())
                                    .map(|ns| ns.display_name.trim())
                                    .or_else(|| {
                                        if !spoof.custom_name.trim().is_empty() {
                                            Some(spoof.custom_name.trim())
                                        } else {
                                            None
                                        }
                                    });
                                if let Some(target_name) = spoof_name {
                                    if event_data.contains_key("PlayerName") || event_data.contains_key("Name") {
                                        event_data.insert("PlayerName".to_string(), serde_json::json!(target_name));
                                        event_data.insert("Name".to_string(), serde_json::json!(target_name));
                                        *changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for (k, v) in map.iter_mut() {
                if k == "ServerAddress" || k == "GameURL" || k == "PingURL" || k == "Players" || k == "Loadout" || k == "Loadouts" || k == "OnlineLoadout" || k == "OnlineLoadouts" || k == "ProductIDs" {
                    continue;
                }
                if let Some(s) = v.as_str() {
                    let trimmed = s.trim();
                    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                    {
                        if let Ok(mut inner_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            let mut inner_changed = false;
                            patch_reservation_value_sync(&mut inner_val, proxy_port, rules, learned_pid, &mut inner_changed);
                            if inner_changed {
                                if let Ok(serialized) = serde_json::to_string(&inner_val) {
                                    *v = serde_json::json!(serialized);
                                    *changed = true;
                                }
                            }
                        }
                    }
                } else {
                    patch_reservation_value_sync(v, proxy_port, rules, learned_pid, changed);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                patch_reservation_value_sync(v, proxy_port, rules, learned_pid, changed);
            }
        }
        _ => {}
    }
}

pub async fn patch_reservation_json(
    body: &[u8],
    items: &[(i32, i32)],
) -> (Vec<u8>, bool) {
    let rules: Vec<ActiveSwapRule> = items
        .iter()
        .map(|&(w, p)| {
            let is_b = is_known_body_id(w);
            ActiveSwapRule {
                owned_id: 0,
                wanted_id: w,
                paint_id: p,
                slot: if is_b { "Body".to_string() } else { "".to_string() },
                slot_idx: if is_b { 0 } else { 255 },
            }
        })
        .collect();
    patch_reservation_json_with_rules(body, &rules).await
}

pub async fn patch_reservation_json_with_rules(
    body: &[u8],
    rules: &[ActiveSwapRule],
) -> (Vec<u8>, bool) {
    let Ok(mut root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (body.to_vec(), false);
    };

    let server_addr_opt = find_server_address(&root);
    let crypto_opt = find_reservation_crypto(&root);

    let proxy_port = if let Some(addr_str) = &server_addr_opt {
        match crate::udp_proxy::set_active_reservation_with_crypto(addr_str, crypto_opt).await {
            Ok(p) => p,
            Err(e) => {
                crate::applog::event(&format!("proxy: error setting UDP reservation target {addr_str}: {e}"));
                crate::udp_proxy::ensure_udp_proxy().await.unwrap_or(0)
            }
        }
    } else {
        crate::udp_proxy::ensure_udp_proxy().await.unwrap_or(0)
    };

    let learned_pid = get_learned_player_id();
    let mut changed = false;
    patch_reservation_value_sync(&mut root, proxy_port, rules, learned_pid.as_deref(), &mut changed);

    if changed {
        if let Ok(out) = serde_json::to_vec(&root) {
            return (out, true);
        }
    }
    (body.to_vec(), false)
}

pub fn patch_player_products_json(
    body: &[u8],
    items: &[(i32, i32)],
) -> (Vec<u8>, bool) {
    patch_player_products_json_with_titles(body, items, &spawned_title_ids())
}

pub fn patch_player_products_json_with_titles(
    body: &[u8],
    items: &[(i32, i32)],
    title_ids: &[String],
) -> (Vec<u8>, bool) {
    if items.is_empty() && title_ids.is_empty() {
        return (body.to_vec(), false);
    }
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let prod_array = if let Some(arr) = root
        .get_mut("Result")
        .and_then(|r| r.get_mut("ProductData"))
        .and_then(|v| v.as_array_mut())
    {
        Some(arr)
    } else if let Some(arr) = root
        .get_mut("Result")
        .and_then(|r| r.get_mut("Products"))
        .and_then(|v| v.as_array_mut())
    {
        Some(arr)
    } else if let Some(arr) = root.get_mut("ProductData").and_then(|v| v.as_array_mut()) {
        Some(arr)
    } else if let Some(arr) = root.get_mut("Products").and_then(|v| v.as_array_mut()) {
        Some(arr)
    } else if let Some(arr) = root.get_mut("LoadoutProducts").and_then(|v| v.as_array_mut()) {
        Some(arr)
    } else if let Some(arr) = root.get_mut("PlayerProducts").and_then(|v| v.as_array_mut()) {
        Some(arr)
    } else if let Some(arr) = root.get_mut("Result").and_then(|r| r.as_array_mut()) {
        Some(arr)
    } else if let Some(arr) = root.as_array_mut() {
        Some(arr)
    } else {
        None
    };

    let Some(arr) = prod_array else {
        return (body.to_vec(), false);
    };

    let mut changed = false;
    for &(wanted_id, paint_id) in items {
        if wanted_id <= 0 {
            continue;
        }

        let is_crate = is_container_or_drop(wanted_id);
        let amount = if is_crate {
            get_container_quantity(wanted_id).max(1)
        } else {
            1
        };

        let mut matched = false;
        for item in arr.iter_mut() {
            let pid = item.get("ProductID")
                .or_else(|| item.get("ProductId"))
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
            if pid == Some(wanted_id as i64) {
                let item_paint = item.get("Attributes").and_then(|v| v.as_array()).and_then(|attrs| {
                    attrs.iter().find(|a| {
                        a.get("Key").and_then(|k| k.as_str()).map(|k| {
                            k.eq_ignore_ascii_case("Painted") || k.eq_ignore_ascii_case("Paint")
                        }).unwrap_or(false)
                    }).and_then(|a| {
                        a.get("Value").and_then(|v| {
                            v.as_str().and_then(|s| s.parse::<i32>().ok())
                                .or_else(|| v.as_i64().map(|n| n as i32))
                        })
                    })
                }).or_else(|| {
                    item.get("Paint")
                        .or_else(|| item.get("paint"))
                        .or_else(|| item.get("PaintID"))
                        .or_else(|| item.get("paint_id"))
                        .or_else(|| item.get("PaintId"))
                        .or_else(|| item.get("paintId"))
                        .and_then(|v| v.as_i64().map(|n| n as i32).or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                }).unwrap_or(0);
                if item_paint == paint_id {
                    if is_crate {
                        item["Amount"] = serde_json::json!(amount);
                        changed = true;
                    }
                    matched = true;
                    break;
                }
            }
        }

        if !matched {
            // Deterministic 128-bit hex instance ID based on wanted_id and paint_id
            // Must be strictly valid 32-character hexadecimal (0-9, a-f) so UnrealScript
            // hex-decodes it into FQWord (UpperBits, LowerBits) without parse error.
            let inst_hex = format!("{:08x}{:08x}0000000000000000", wanted_id as u32, paint_id as u32);
            let static_ts: i64 = 1700000000;
            let series_id = lookup_series_id_for_product(wanted_id);

            let mut prod_obj = serde_json::json!({
                "ProductID": wanted_id,
                "InstanceID": inst_hex,
                "SeriesID": series_id,
                "Amount": amount,
                "ExpirationTime": -1,
                "IsTradable": false,
                "TradeHold": -1,
                "AddedTimestamp": static_ts,
                "UpdatedTimestamp": static_ts,
            });

            let mut attrs = Vec::new();
            if paint_id > 0 {
                prod_obj["Paint"] = serde_json::json!(paint_id);
                prod_obj["PaintID"] = serde_json::json!(paint_id);
                attrs.push(serde_json::json!({
                    "Key": "Painted",
                    "Value": paint_id.to_string()
                }));
                attrs.push(serde_json::json!({
                    "Key": "Paint",
                    "Value": paint_id.to_string()
                }));
            }
            if is_crate {
                prod_obj["bSkipRoulette"] = serde_json::json!(true);
                prod_obj["bUnlocked"] = serde_json::json!(true);
                attrs.push(serde_json::json!({
                    "Key": "bSkipRoulette",
                    "Value": "true"
                }));
                attrs.push(serde_json::json!({
                    "Key": "bUnlocked",
                    "Value": "true"
                }));
            }
            if !attrs.is_empty() {
                prod_obj["Attributes"] = serde_json::json!(attrs);
            }

            arr.push(prod_obj);
            changed = true;
        }
    }

    for tid in title_ids {
        if arr.iter().any(|it| item_title_id(it) == Some(tid.as_str())) {
            continue;
        }
        arr.push(title_product_json(tid, 1700000000));
        changed = true;
    }

    // Pass 1: Ensure series ID and crate skip-roulette attributes on every item in array
    for item in arr.iter_mut() {
        if let Some(obj) = item.as_object_mut() {
            let pid = obj.get("ProductID")
                .or_else(|| obj.get("ProductId"))
                .or_else(|| obj.get("id"))
                .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(0) as i32;

            let has_valid_series = obj.get("SeriesID").map(|v| v.is_i64()).unwrap_or(false);
            if !has_valid_series {
                let sid = lookup_series_id_for_product(pid);
                obj.insert("SeriesID".to_string(), serde_json::json!(sid));
                changed = true;
            }

            if is_container_or_drop(pid) {
                if obj.get("bSkipRoulette").and_then(|v| v.as_bool()) != Some(true) {
                    obj.insert("bSkipRoulette".to_string(), serde_json::json!(true));
                    changed = true;
                }
                if obj.get("bUnlocked").and_then(|v| v.as_bool()) != Some(true) {
                    obj.insert("bUnlocked".to_string(), serde_json::json!(true));
                    changed = true;
                }
                let mut attrs = obj.get("Attributes").and_then(|v| v.as_array().cloned()).unwrap_or_default();
                let mut attr_changed = false;
                if let Some(skip_attr) = attrs.iter_mut().find(|a| a.get("Key").and_then(|k| k.as_str()).map(|k| k.eq_ignore_ascii_case("bSkipRoulette")).unwrap_or(false)) {
                    if skip_attr.get("Value").and_then(|v| v.as_str()) != Some("true") {
                        skip_attr["Value"] = serde_json::json!("true");
                        attr_changed = true;
                    }
                } else {
                    attrs.push(serde_json::json!({ "Key": "bSkipRoulette", "Value": "true" }));
                    attr_changed = true;
                }
                if let Some(unlock_attr) = attrs.iter_mut().find(|a| a.get("Key").and_then(|k| k.as_str()).map(|k| k.eq_ignore_ascii_case("bUnlocked")).unwrap_or(false)) {
                    if unlock_attr.get("Value").and_then(|v| v.as_str()) != Some("true") {
                        unlock_attr["Value"] = serde_json::json!("true");
                        attr_changed = true;
                    }
                } else {
                    attrs.push(serde_json::json!({ "Key": "bUnlocked", "Value": "true" }));
                    attr_changed = true;
                }
                if attr_changed {
                    obj.insert("Attributes".to_string(), serde_json::json!(attrs));
                    changed = true;
                }
            }
        }
    }

    // Pass 2: Deduplicate non-stackable items to eliminate duplicate instances with identical ProductID and PaintID
    let mut seen_items = std::collections::HashSet::new();
    let mut deduped_arr = Vec::with_capacity(arr.len());
    for item in arr.drain(..) {
        let pid = item.get("ProductID")
            .or_else(|| item.get("ProductId"))
            .or_else(|| item.get("id"))
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0) as i32;

        let is_crate = is_container_or_drop(pid);
        if is_crate {
            // Container/crate instances can have multiples
            deduped_arr.push(item);
        } else {
            let paint = item.get("Attributes").and_then(|v| v.as_array()).and_then(|attrs| {
                attrs.iter().find(|a| {
                    a.get("Key").and_then(|k| k.as_str()).map(|k| {
                        k.eq_ignore_ascii_case("Painted") || k.eq_ignore_ascii_case("Paint")
                    }).unwrap_or(false)
                }).and_then(|a| {
                    a.get("Value").and_then(|v| {
                        v.as_str().and_then(|s| s.parse::<i32>().ok())
                            .or_else(|| v.as_i64().map(|n| n as i32))
                    })
                })
            }).or_else(|| {
                item.get("Paint")
                    .or_else(|| item.get("paint"))
                    .or_else(|| item.get("PaintID"))
                    .or_else(|| item.get("paint_id"))
                    .or_else(|| item.get("PaintId"))
                    .or_else(|| item.get("paintId"))
                    .and_then(|v| v.as_i64().map(|n| n as i32).or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            }).unwrap_or(0);

            // Titles share one ProductID; key on InstanceID or full attributes.
            let instance = item.get("InstanceID")
                .or_else(|| item.get("InstanceId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let key = match instance {
                Some(inst) => format!("i:{inst}"),
                None => {
                    let mut attrs: Vec<String> = item.get("Attributes")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().map(|x| x.to_string()).collect())
                        .unwrap_or_default();
                    attrs.sort();
                    format!("a:{pid}:{paint}:{}", attrs.join(","))
                }
            };
            if seen_items.insert(key) {
                deduped_arr.push(item);
            } else {
                changed = true;
            }
        }
    }
    *arr = deduped_arr;

    // Record all requested and possessed items in NOTIFIED_SPAWNED_ITEMS so the
    // background reconciler knows client already has them in inventory.
    if let Ok(mut notified) = NOTIFIED_SPAWNED_ITEMS.write() {
        for &(wid, paint) in items {
            if wid > 0 {
                notified.insert((wid, paint));
            }
        }
    }

    if changed {
        match serde_json::to_vec(&root) {
            Ok(bytes) => (bytes, true),
            Err(_) => (body.to_vec(), false),
        }
    } else {
        (body.to_vec(), false)
    }
}

fn patch_ownership_value_recursive(val: &mut serde_json::Value, changed: &mut bool) {
    match val {
        serde_json::Value::Object(obj) => {
            for key in ["bOwned", "isOwned", "Owned", "bHasProduct", "HasEntitlement", "bSuccess", "Success", "bActive", "IsActive"] {
                if let Some(v) = obj.get_mut(key) {
                    if v.as_bool() == Some(false) {
                        *v = serde_json::Value::Bool(true);
                        *changed = true;
                    }
                }
            }
            for key in ["status", "Status", "ownershipStatus", "OwnershipStatus"] {
                if let Some(v) = obj.get_mut(key) {
                    if let Some(s) = v.as_str() {
                        if s.eq_ignore_ascii_case("NOT_OWNED") || s.eq_ignore_ascii_case("EXPIRED") {
                            *v = serde_json::Value::String("ACTIVE".to_string());
                            *changed = true;
                        } else if s.eq_ignore_ascii_case("InventoryOutOfSync") {
                            *v = serde_json::Value::String("OK".to_string());
                            *changed = true;
                        }
                    }
                }
            }
            for key in ["Error", "error"] {
                if let Some(err_val) = obj.get_mut(key) {
                    if let Some(s) = err_val.as_str() {
                        if s.contains("InventoryOutOfSync") {
                            *err_val = serde_json::Value::Null;
                            *changed = true;
                        }
                    }
                }
            }
            for (_k, v) in obj.iter_mut() {
                patch_ownership_value_recursive(v, changed);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                patch_ownership_value_recursive(item, changed);
            }
        }
        _ => {}
    }
}

pub fn patch_check_ownership_json(
    body: &[u8],
    _items: &[(i32, i32)],
) -> (Vec<u8>, bool) {
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let mut changed = false;
    patch_ownership_value_recursive(&mut root, &mut changed);

    if changed {
        match serde_json::to_vec(&root) {
            Ok(bytes) => (bytes, true),
            Err(_) => (body.to_vec(), false),
        }
    } else {
        (body.to_vec(), false)
    }
}

pub fn patch_cross_entitlement_json(
    body: &[u8],
    spawned_items: &[(i32, i32)],
) -> (Vec<u8>, bool) {
    if spawned_items.is_empty() {
        return (body.to_vec(), false);
    }
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let mut changed = false;
    let target = if let Some(arr) = root
        .get_mut("Result")
        .and_then(|r| r.get_mut("CrossEntitledProductIDs"))
        .and_then(|a| a.as_array_mut())
    {
        Some(arr)
    } else {
        root.get_mut("CrossEntitledProductIDs")
            .and_then(|a| a.as_array_mut())
    };

    if let Some(target) = target {
        for (wid, _) in spawned_items {
            if *wid > 0 && !target.iter().any(|v| v.as_i64() == Some(*wid as i64)) {
                target.push(serde_json::json!(wid));
                changed = true;
            }
        }
    }

    if changed {
        match serde_json::to_vec(&root) {
            Ok(bytes) => (bytes, true),
            Err(_) => (body.to_vec(), false),
        }
    } else {
        (body.to_vec(), false)
    }
}

/// Detect an access-denied / out-of-sync response before it is rewritten.
pub fn response_denies_access(body: &[u8]) -> bool {
    let upper = body.to_ascii_uppercase();
    upper.windows(12).any(|w| w == b"ACCESSDENIED")
        || upper.windows(18).any(|w| w == b"INVENTORYOUTOFSYNC")
        || upper.windows(18).any(|w| w == b"PARTYRANKDISPARITY")
        || upper.windows(17).any(|w| w == b"TOURRANKDISPARITY")
        || upper.windows(19).any(|w| w == b"RANKEDCRITERIAUNMET")
        || upper.windows(13).any(|w| w == b"RANKDISPARITY")
}

/// Neutralize "InventoryOutOfSync" and "AccessDenied" error states from PsyNet JSON responses
/// preventing client popup/notification "Inventory out of sync, resyncing..." or drop opening hangs
pub fn neutralize_inventory_out_of_sync_json(body: &[u8]) -> (Vec<u8>, bool) {
    if !response_denies_access(body) {
        return (body.to_vec(), false);
    }

    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    // A frame whose whole purpose is an InventoryOutOfSync/AccessDenied error
    // (e.g. {"Error":{"Type":"InventoryOutOfSync","Message":""}}) must never
    // reach the client: RL shows the "Inventory out of sync, Syncing now..."
    // popup directly from this. Answer it as an empty successful result so the
    // RPC completes cleanly instead.
    let is_error_frame = root.get("Error").or_else(|| root.get("error")).is_some()
        && root.get("Result").is_none()
        && root.get("result").is_none();
    if is_error_frame {
        if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({ "Result": {} })) {
            return (bytes, true);
        }
    }

    let mut changed = false;
    neutralize_inventory_out_of_sync_value(&mut root, &mut changed);

    if changed {
        match serde_json::to_vec(&root) {
            Ok(bytes) => (bytes, true),
            Err(_) => (body.to_vec(), false),
        }
    } else {
        (body.to_vec(), false)
    }
}

fn neutralize_inventory_out_of_sync_value(val: &mut serde_json::Value, changed: &mut bool) {
    match val {
        serde_json::Value::Object(map) => {
            for key in [
                "status", "Status", "Message", "message", "Error", "error",
                "ErrorCode", "errorCode", "Result", "result", "ResultCode",
                "resultCode", "Code", "code", "Reason", "reason",
                "Type", "type", "Name", "name", "Detail", "detail",
            ] {
                if let Some(v) = map.get_mut(key) {
                    if let Some(s) = v.as_str() {
                        if s.to_ascii_uppercase().contains("INVENTORYOUTOFSYNC")
                            || s.to_ascii_uppercase().contains("ACCESSDENIED")
                            || s.to_ascii_uppercase().contains("PARTYRANKDISPARITY")
                            || s.to_ascii_uppercase().contains("TOURRANKDISPARITY")
                            || s.to_ascii_uppercase().contains("RANKEDCRITERIAUNMET")
                            || s.to_ascii_uppercase().contains("RANKDISPARITY")
                        {
                            *v = if key.eq_ignore_ascii_case("status")
                                || key.eq_ignore_ascii_case("result")
                                || key.eq_ignore_ascii_case("code")
                            {
                                serde_json::Value::String("OK".to_string())
                            } else {
                                serde_json::Value::Null
                            };
                            *changed = true;
                        }
                    }
                }
            }
            for key in ["bSuccess", "Success", "success"] {
                if let Some(v) = map.get_mut(key) {
                    if v.as_bool() == Some(false) {
                        *v = serde_json::Value::Bool(true);
                        *changed = true;
                    }
                }
            }
            for (_k, v) in map.iter_mut() {
                neutralize_inventory_out_of_sync_value(v, changed);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                neutralize_inventory_out_of_sync_value(v, changed);
            }
        }
        _ => {}
    }
}

pub fn patch_users_can_show_avatar_json(body: &[u8]) -> (Vec<u8>, bool) {
    let Ok(mut root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (body.to_vec(), false);
    };

    let result_obj = if let Some(res) = root.get_mut("Result").and_then(|r| r.as_object_mut()) {
        res
    } else if let Some(obj) = root.as_object_mut() {
        obj
    } else {
        return (body.to_vec(), false);
    };

    let mut changed = false;

    if result_obj.contains_key("AllowedPlayerIDs") || result_obj.contains_key("HiddenPlayerIDs") {
        let mut allowed_ids: Vec<String> = Vec::new();

        if let Some(arr) = result_obj.get("AllowedPlayerIDs").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    allowed_ids.push(s.to_string());
                }
            }
        }

        if let Some(hidden) = result_obj.get_mut("HiddenPlayerIDs").and_then(|v| v.as_array_mut()) {
            if !hidden.is_empty() {
                for v in hidden.iter() {
                    if let Some(s) = v.as_str() {
                        if !allowed_ids.contains(&s.to_string()) {
                            allowed_ids.push(s.to_string());
                        }
                    }
                }
                hidden.clear();
                changed = true;
            }
        }

        if let Some(learned) = get_learned_player_id() {
            let learned_norm = normalize_player_id(&learned);
            if !allowed_ids.iter().any(|id| normalize_player_id(id) == learned_norm) {
                allowed_ids.push(learned);
                changed = true;
            }
        }

        if changed {
            result_obj.insert("AllowedPlayerIDs".to_string(), serde_json::json!(allowed_ids));
            result_obj.insert("HiddenPlayerIDs".to_string(), serde_json::json!([]));
        }
    }

    if changed {
        match serde_json::to_vec(&root) {
            Ok(bytes) => (bytes, true),
            Err(_) => (body.to_vec(), false),
        }
    } else {
        (body.to_vec(), false)
    }
}

pub fn inspect_active_upk_payload() -> (String, serde_json::Value) {
    let candidate_paths = [
        std::path::PathBuf::from("/root/TAGame.upk"),
        std::path::PathBuf::from("/root/ProjectX_rlim.upk"),
        std::path::PathBuf::from("/root/velrlapi/games/rocketleague/TAGame/CookedPCConsole/TAGame.upk"),
    ];

    let mut found_path = None;
    for p in &candidate_paths {
        if p.is_file() {
            found_path = Some(p.clone());
            break;
        }
    }

    let Some(upk_path) = found_path else {
        return ("None".to_string(), serde_json::json!({ "error": "No local UPK found to inspect" }));
    };

    let filename = upk_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let Ok(data) = std::fs::read(&upk_path) else {
        return (filename, serde_json::json!({ "error": "Failed to read UPK file" }));
    };

    if data.len() < 64 {
        return (filename, serde_json::json!({ "error": "File too small to be a valid UPK" }));
    }

    let Ok((summary, meta)) = crate::upk::parser::parse_prefix(&data) else {
        return (filename, serde_json::json!({ "error": "Failed to parse UPK header prefix" }));
    };

    let name_offset = summary.name_offset as usize;
    let enc_end = (summary.total_header_size - meta.garbage_size) as usize;
    if name_offset >= enc_end || enc_end > data.len() {
        return (filename, serde_json::json!({
            "error": "Header bounds invalid",
            "name_offset": name_offset,
            "total_header_size": summary.total_header_size,
        }));
    }

    let enc_len = (enc_end - name_offset) & !15;
    let enc_block = &data[name_offset..name_offset + enc_len];
    let plain_header = crate::upk::crypto::decrypt_ecb(&crate::upk::swapper::TAGAME_KEY, enc_block);

    let chunks_rel = meta.compressed_chunks_offset as usize;
    let chunks = if chunks_rel + 4 <= plain_header.len() {
        crate::upk::parser::parse_chunks(&plain_header, meta.compressed_chunks_offset).unwrap_or_default()
    } else {
        Vec::new()
    };

    let names = if summary.name_count > 0 && !plain_header.is_empty() {
        crate::upk::palette::parse_names_in_block(&plain_header, summary.name_count.min(100)).unwrap_or_default()
    } else {
        Vec::new()
    };

    (
        filename,
        serde_json::json!({
            "status": "Decrypted Successfully",
            "file_version": summary.file_version,
            "licensee_version": summary.licensee_version,
            "total_header_size": summary.total_header_size,
            "name_count": summary.name_count,
            "export_count": summary.export_count,
            "import_count": summary.import_count,
            "compressed_chunks_count": chunks.len(),
            "first_chunks": chunks.iter().take(4).map(|c| {
                serde_json::json!({
                    "uncomp_offset": c.uncompressed_offset,
                    "uncomp_size": c.uncompressed_size,
                    "comp_offset": c.compressed_offset,
                    "comp_size": c.compressed_size,
                })
            }).collect::<Vec<_>>(),
            "sample_decrypted_names": names.iter().take(25).collect::<Vec<_>>(),
        })
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_avatar_matches_lobby_urls() {
        let player = "Epic|37674b519c3544beb544f437f539ebf2|0";
        assert!(super::avatar_path_matches_player(
            "/content/images/players/avatars/large/37674b519c3544beb544f437f539ebf2.png", player));
        assert!(super::avatar_path_matches_player(
            "/rlim/avatar?player_id=Epic%7C37674b519c3544beb544f437f539ebf2%7C0", player));
        assert!(!super::avatar_path_matches_player("/avatars/someone-else.png", player));
        assert!(!super::avatar_path_matches_player("/avatars/local.png", ""));
    }

    #[test]
    fn test_access_denied_response_is_neutralized() {
        let body = br#"{"Result":"AccessDenied","bSuccess":false,"Message":"AccessDenied"}"#;
        assert!(response_denies_access(body));
        let (patched, changed) = neutralize_inventory_out_of_sync_json(body);
        assert!(changed);
        let text = String::from_utf8(patched).unwrap();
        assert!(!text.contains("AccessDenied"));
        assert!(text.contains("\"bSuccess\":true"));
        assert!(text.contains("\"Result\":\"OK\""));

        let clean = br#"{"Result":{"Drops":[]},"bSuccess":true}"#;
        assert!(!response_denies_access(clean));
        let (same, changed) = neutralize_inventory_out_of_sync_json(clean);
        assert!(!changed);
        assert_eq!(same, clean);
    }

    #[test]
    fn test_inventory_out_of_sync_error_frame_becomes_success() {
        // Exact S2C frame from the real dump (event 1125):
        // {"Error":{"Type":"InventoryOutOfSync","Message":""}}
        let body = br#"{"Error":{"Type":"InventoryOutOfSync","Message":""}}"#;
        assert!(response_denies_access(body));
        let (patched, changed) = neutralize_inventory_out_of_sync_json(body);
        assert!(changed, "error frame must be neutralized");
        let text = String::from_utf8(patched).unwrap();
        assert!(
            !text.to_ascii_lowercase().contains("outofsync"),
            "client must never see InventoryOutOfSync: {text}"
        );
        assert!(
            text.contains("\"Result\""),
            "frame must be rewritten as a successful result: {text}"
        );
    }

    #[test]
    fn test_patch_palette() {
        let input = br#"{"ClassPropertyConfig":{"Class":"ClassPropertyConfig_X","Overrides":[{"Class":"GFxData_MusicPlayer_TA","Property":"bDebugMusicPlayer","Value":"true"},{"Class":"Camera_TA","Property":"FOVLimits","Value":"(Min=1.000000,Max=1000.000000,interval=1.000000)"}]}}"#;
        let (patched, changed) = patch_palette(input);
        assert!(changed);
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"Class\":\"Team_Soccar_TA\""));
        assert!(s.contains("\"Property\":\"CarColorSet\""));
        assert!(s.contains("\"Value\":\"CarColorSet_TA'CarColors.OrangeTeamV2'\""));
    }

    #[test]
    fn test_parse_build_id() {
        assert_eq!(
            parse_build_id("/v2/Config/BattleCars/-1887694083/Prod/Epic/INT/"),
            Some("-1887694083".to_string())
        );
        assert_eq!(
            parse_build_id("/Config/BattleCars/99999/"),
            Some("99999".to_string())
        );
        assert_eq!(parse_build_id("/favicon.ico"), None);
    }

    #[test]
    fn test_resign_config_cdn() {
        let body = b"test payload";
        let sig = resign_config_cdn(body);
        assert!(!sig.is_empty());
        // Verify deterministic output
        assert_eq!(sig, resign_config_cdn(body));
    }

    #[test]
    fn test_load_certified_key_only_includes_leaf() {
        let key = load_certified_key(LEAF_CONFIG_CERT_PEM, LEAF_CONFIG_KEY_PEM)
            .expect("should load leaf config");
        assert_eq!(
            key.cert.len(),
            1,
            "server certificate chain should only contain the leaf cert, not root CA"
        );
    }


    #[test]
    fn test_patch_psynet_url_rewrites_to_broker() {
        BROKER_PORT.store(15759, Ordering::SeqCst);
        let input = br#"{"PsyNetUrl":{"Class":"PsyNetUrl_X","URL":"https://api.rlpp.psynet.gg/Services","URLv2":"https://api.rlpp.psynet.gg/rpc"}}"#;
        let patched = patch_psynet_url(input).expect("should patch PsyNetUrl");
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"URL\":\"http://127.0.0.1:15759/Services\""));
        assert!(s.contains("\"URLv2\":\"http://127.0.0.1:15759/rpc\""));
    }

    #[test]
    fn test_patch_eos_accounts_json_scoped_to_target_pid() {
        let mut json: serde_json::Value = serde_json::json!([
            {
                "accountId": "37674b519c3544beb544f437f539ebf2",
                "displayName": "RealPlayer",
                "preferredLanguage": "en"
            },
            {
                "accountId": "99999999999944beb544f437f539ebf2",
                "displayName": "Teammate",
                "preferredLanguage": "en"
            }
        ]);

        let (modified, learned_pid, learned_name) = patch_eos_accounts_json(
            &mut json,
            "SpoofedName",
            Some("37674b519c3544beb544f437f539ebf2"),
            None,
        );

        assert!(modified);
        assert_eq!(learned_pid.as_deref(), Some("37674b519c3544beb544f437f539ebf2"));
        assert_eq!(learned_name.as_deref(), Some("RealPlayer"));

        let arr = json.as_array().unwrap();
        assert_eq!(arr[0]["displayName"], "SpoofedName");
        assert_eq!(arr[1]["displayName"], "Teammate");
    }

    #[test]
    fn test_patch_ws_name_fields_only_own_id() {
        let body = br#"{"Result":{"PlayerData":[
            {"PlayerID":"Epic|37674b519c3544beb544f437f539ebf2|0","PlayerName":"RealPlayer"},
            {"PlayerID":"Epic|otherplayer123|0","PlayerName":"Teammate"}
        ]}}"#;

        let (patched, changed) = patch_ws_name_fields(
            body,
            "SpoofedName",
            Some("37674b519c3544beb544f437f539ebf2"),
        );

        assert!(changed);
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"PlayerName\":\"SpoofedName\""));
        assert!(s.contains("\"PlayerName\":\"Teammate\""));
        assert!(!s.contains("\"PlayerName\":\"RealPlayer\""));
    }

    #[test]
    fn test_patch_ws_name_fields_nested_message_payload() {
        let inner = r#"{"Players":[{"PlayerID":"Epic|me123|0","PlayerName":"RealPlayer"}],"Settings":{"MapName":"stadium_p"}}"#;
        let root = serde_json::json!({
            "MessageType": "AddReservationMessagePrivate_X",
            "MessagePayload": inner
        });
        let raw = serde_json::to_vec(&root).unwrap();

        let (patched, changed) = patch_ws_name_fields(
            &raw,
            "SpoofedName",
            Some("me123"),
        );

        assert!(changed);
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("SpoofedName"));
        assert!(!s.contains("RealPlayer"));
        assert!(s.contains("stadium_p"));
    }

    #[test]
    fn test_patch_ws_name_fields_ignores_unmatched_id() {
        let body = br#"{"Result":{"PlayerData":[
            {"PlayerID":"Epic|otherplayer123|0","PlayerName":"Teammate"}
        ]}}"#;
        let (patched, changed) = patch_ws_name_fields(
            body,
            "SpoofedName",
            Some("37674b519c3544beb544f437f539ebf2"),
        );

        assert!(!changed);
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"PlayerName\":\"Teammate\""));
        assert!(!s.contains("SpoofedName"));
    }

    #[test]
    fn test_is_loadout_sensitive_detects_sensitive_frames() {
        assert!(is_loadout_sensitive("products/getloadoutproducts v1", b"{}"));
        assert!(is_loadout_sensitive("genericstorage/getplayergenericstorage v1", b"{}"));
        assert!(is_loadout_sensitive("other", b"{\"SaveData\": \"ProfileLoadoutSave_TA\"}"));
        assert!(!is_loadout_sensitive("skills/getplayerskill v1", b"{\"Skills\":[]}"));
    }

    #[test]
    fn test_learned_identity_ignores_placeholders() {
        set_learned_player_id("Epic|temp|0");
        assert_ne!(get_learned_player_id().as_deref(), Some("Epic|temp|0"));

        set_learned_player_id("Epic|realaccountid|0");
        assert_eq!(get_learned_player_id().as_deref(), Some("Epic|realaccountid|0"));
    }

    #[test]
    fn test_patch_leaderboard_json_single_value_auto_rank_1() {
        let body = br#"{"Result":{"LeaderboardID":"Skill11","bHasSkill":true,"MMR":20.0,"Value":10}}"#;
        let mut cfg = crate::psynet::SpoofPayload::default();
        let mut ov = std::collections::HashMap::new();
        ov.insert("11".to_string(), crate::psynet::FakeRankOverridePayload {
            display_mmr: Some(3000.0),
            tier: Some(22),
            ..Default::default()
        });
        cfg.fake_ranks = Some(crate::psynet::FakeRanksPayload {
            enabled: true,
            playlists: Some(ov),
            ..Default::default()
        });

        let (patched, changed) = patch_leaderboard_json(body, &cfg);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        assert_eq!(val["Result"]["Rank"], 1);
        assert_eq!(val["Result"]["UserRank"], 1);
        assert_eq!(val["Result"]["Value"], 22);
        assert_eq!(val["Result"]["Tier"], 22);
        assert_eq!(val["Result"]["MMR"], 145.0);
    }

    #[test]
    fn test_patch_get_player_skill_json_places_fake_rank() {
        let body = br#"{"Result":{"Skills":[{"Playlist":11,"Tier":0,"Division":0,"MatchesPlayed":0}]}}"#;
        let mut ov = std::collections::HashMap::new();
        ov.insert("11".to_string(), crate::psynet::FakeRankOverridePayload {
            display_mmr: Some(1900.0),
            tier: Some(22),
            division: Some(3),
            ..Default::default()
        });
        let fr = crate::psynet::FakeRanksPayload {
            enabled: true,
            playlists: Some(ov),
            ..Default::default()
        };
        let (patched, changed) = patch_get_player_skill_json(body, &fr);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let s = &val["Result"]["Skills"][0];
        assert_eq!(s["Tier"], 22);
        assert_eq!(s["Division"], 3);
        assert_eq!(s["MatchesPlayed"], 100);
        assert_eq!(s["PlacementMatches"], 10);
        assert_eq!(s["PlacementMatchesPlayed"], 10);
    }

    #[test]
    fn test_patch_leaderboard_json_rows_auto_rank_computed() {
        let body = br#"{"Result":{"LeaderboardID":"Skill11","Rows":[
            {"PlayerID":"top1","Value":2004,"Rank":1},
            {"PlayerID":"top2","Value":1978,"Rank":2}
        ],"UserRow":{"PlayerID":"my_pid","Value":800,"Rank":0}}}"#;

        let mut cfg = crate::psynet::SpoofPayload::default();
        let mut ov = std::collections::HashMap::new();
        ov.insert("11".to_string(), crate::psynet::FakeRankOverridePayload {
            display_mmr: Some(3000.0),
            tier: Some(22),
            ..Default::default()
        });
        cfg.fake_ranks = Some(crate::psynet::FakeRanksPayload {
            enabled: true,
            playlists: Some(ov),
            ..Default::default()
        });

        let (patched, changed) = patch_leaderboard_json(body, &cfg);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        assert_eq!(val["Result"]["Rank"], 1);
        assert_eq!(val["Result"]["UserRow"]["Rank"], 1);
        assert_eq!(val["Result"]["UserRow"]["Value"], 3000);
        assert_eq!(val["Result"]["Rows"][0]["Rank"], 1);
        assert_eq!(val["Result"]["Rows"][0]["Value"], 3000);
        assert_eq!(val["Result"]["Rows"][1]["Rank"], 2);
    }

    #[test]
    fn test_patch_official_skill_leaderboard_platforms() {
        let body = br#"{"Result":{"LeaderboardID":"Skill10","Platforms":[{"Platform":"Epic","Players":[{"PlayerID":"Epic|top1|0","PlayerName":"top1","MMR":81.86,"Value":22}]}]}}"#;
        let mut cfg = crate::psynet::SpoofPayload::default();
        cfg.name_spoof = Some(crate::psynet::NameSpoofPayload {
            display_name: "You".to_string(),
            ..Default::default()
        });
        cfg.leaderboard_spoof = Some(crate::psynet::LeaderboardSpoofPayload {
            enabled: true,
            sync_from_fake_ranks: false,
            custom_rank: Some(1),
            custom_mmr: Some(3000),
        });

        let (patched, changed) = patch_leaderboard_json(body, &cfg);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        assert_eq!(val["Result"]["Platforms"][0]["Players"][0]["PlayerName"], "You");
        assert_eq!(val["Result"]["Platforms"][0]["Players"][0]["Value"], 22);
        assert_eq!(val["Result"]["Platforms"][0]["Players"][0]["MMR"], 145.0);
    }

    #[test]
    fn test_patch_player_wallet_credits_and_tournaments() {
        let body = br#"{"Result":{"Currencies":[{"ID":13,"Amount":250,"IsTradable":true},{"ID":15,"Amount":100,"IsTradable":false}]}}"#;
        let cs = crate::psynet::CreditSpoofPayload {
            enabled: true,
            amount: 999999,
            tournament_amount: 500000,
        };
        let (patched, changed) = patch_player_wallet_json(body, &cs);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let currencies = val["Result"]["Currencies"].as_array().unwrap();
        
        let c13 = currencies.iter().find(|c| c["ID"] == 13).unwrap();
        assert_eq!(c13["Amount"], 999999);

        let c15 = currencies.iter().find(|c| c["ID"] == 15).unwrap();
        assert_eq!(c15["Amount"], 500000);

        // Ensure season variants 14..=26 were injected
        for tid in 14..=26 {
            let c = currencies.iter().find(|c| c["ID"] == tid);
            assert!(c.is_some(), "Currency ID {tid} must be present for season compatibility");
            assert_eq!(c.unwrap()["Amount"], 500000);
        }
    }

    #[test]
    fn test_patch_player_products_json_injection() {
        let body = br#"{"ProductData":[{"ProductID":100,"InstanceID":"00000000000000000000000000000001","SeriesID":1}]}"#;
        let items = vec![(4284, 12), (100, 0)]; // 4284 should be injected, 100 already exists
        let (patched, changed) = patch_player_products_json(body, &items);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let arr = val["ProductData"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let injected = arr.iter().find(|i| i["ProductID"] == 4284).unwrap();
        assert_eq!(injected["Attributes"][0]["Key"], "Painted");
        assert_eq!(injected["Attributes"][0]["Value"], "12");
    }

    #[test]
    fn test_patch_cross_entitlement_json() {
        let body = br#"{"Result":{"CrossEntitledProductIDs":[23,376]}}"#;
        let items = vec![(4284, 12), (23, 0)];
        let (patched, changed) = patch_cross_entitlement_json(body, &items);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let ids = val["Result"]["CrossEntitledProductIDs"].as_array().unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.iter().any(|v| v == 4284));
    }

    #[test]
    fn test_patch_check_ownership_json() {
        let body = br#"{"bOwned":false,"Success":false,"bHasProduct":false}"#;
        let items = vec![(4284, 0)];
        let (patched, changed) = patch_check_ownership_json(body, &items);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        assert_eq!(val["bOwned"], true);
        assert_eq!(val["Success"], true);
        assert_eq!(val["bHasProduct"], true);
    }

    #[tokio::test]
    async fn test_patch_reservation_json_reroutes_and_injects_loadout() {
        let inner = serde_json::json!({
            "ServerAddress": "162.245.204.12:7777",
            "ReservationID": "test-res-uuid",
            "Players": [
                {
                    "PlayerID": "Epic|testuser|0",
                    "bRemotePlayer": false,
                    "bPrimaryPlayer": true,
                    "Loadout": {
                        "Products": [23, 100]
                    }
                }
            ],
            "Settings": { "MapName": "stadium_p" }
        });
        let root = serde_json::json!({
            "MessageType": "AddReservationMessagePrivate_X",
            "MessagePayload": inner.to_string()
        });
        let raw = serde_json::to_vec(&root).unwrap();

        let items = vec![(803, 12)]; // Titanium White Octane
        let (patched, changed) = patch_reservation_json(&raw, &items).await;
        assert!(changed, "Reservation frame must be patched");

        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let payload_str = val["MessagePayload"].as_str().expect("Payload should be stringified");
        let parsed_inner: serde_json::Value = serde_json::from_str(payload_str).unwrap();

        let server_addr = parsed_inner["ServerAddress"].as_str().unwrap();
        assert!(
            server_addr.starts_with("127.0.0.1:"),
            "ServerAddress must be rerouted to 127.0.0.1 UDP proxy, got: {server_addr}"
        );

        let players = parsed_inner["Players"].as_array().unwrap();
        let prods = players[0]["Loadout"]["Products"].as_array().unwrap();
        assert!(
            prods.iter().any(|v| v == 803),
            "Injected item 803 must be present in player's Loadout.Products: {prods:?}"
        );
    }

    #[tokio::test]
    async fn test_patch_reservation_direct_object() {
        let root = serde_json::json!({
            "MessageType": "AddReservationMessagePublic_X",
            "ServerAddress": "198.51.100.50:7777",
            "Players": [
                {
                    "bRemotePlayer": false,
                    "Loadout": {
                        "Products": [
                            { "ProductID": 23, "SlotIndex": 0, "PaintID": 0 }
                        ]
                    }
                }
            ]
        });
        let raw = serde_json::to_vec(&root).unwrap();

        let items = vec![(803, 1)];
        let (patched, changed) = patch_reservation_json(&raw, &items).await;
        assert!(changed);

        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let server_addr = val["ServerAddress"].as_str().unwrap();
        assert!(server_addr.starts_with("127.0.0.1:"));

        let prods = val["Players"][0]["Loadout"]["Products"].as_array().unwrap();
        // Since both 23 and 803 are Body items (Slot 0), 803 replaces 23 in-place
        assert_eq!(prods.len(), 1);
        let injected = prods.iter().find(|p| p["ProductID"] == 803).unwrap();
        assert_eq!(injected["PaintID"], 1);
    }

    #[tokio::test]
    async fn test_patch_reservation_flat_array_c2u() {
        let inner = serde_json::json!({
            "ReservationID": "test-c2u",
            "Players": [
                {
                    "PlayerID": "Steam|76561198643170831|0",
                    "PlayerName": "one",
                    "bRemotePlayer": false,
                    "Loadout": [11603, 376, 63, 1948, 1903]
                }
            ]
        });
        let root = serde_json::json!({
            "MessageType": "AddReservationMessagePublic_X",
            "MessagePayload": inner.to_string()
        });
        let raw = serde_json::to_vec(&root).unwrap();

        let rules = vec![
            ActiveSwapRule {
                owned_id: 4284,
                wanted_id: 4284,
                paint_id: 5,
                slot: "Body".to_string(),
                slot_idx: 0,
            }
        ];
        let (patched, changed) = patch_reservation_json_with_rules(&raw, &rules).await;
        assert!(changed, "Flat array loadout must be patched");

        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let payload_str = val["MessagePayload"].as_str().unwrap();
        let parsed_inner: serde_json::Value = serde_json::from_str(payload_str).unwrap();
        let loadout = parsed_inner["Players"][0]["Loadout"].as_array().unwrap();

        assert_eq!(loadout[0], 4284, "Index 0 must be replaced from fallback 11603 to 4284 (Fennec)");
        assert_eq!(loadout.len(), 5, "Loadout array length should remain intact");
    }

    #[test]
    fn test_patch_products_config_and_overrides() {
        let body = br#"{"Config":{},"ClassPropertyConfig":{"Class":"ClassPropertyConfig_X","Overrides":[]},"OnlineStorageConfig":{"Class":"OnlineStorageConfig_TA","MinSecondsBetweenCloudUploads":30}}"#;
        let (patched, changed) = patch_products_config(body);
        assert!(changed);
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"bUseSaveDataCacheProducts\",\"Value\":\"True\""));
        assert!(s.contains("\"MaxSecondsBetweenFullProductSyncs\",\"Value\":\"2147483647\""));
        assert!(s.contains("\"bClearSyncTimestampOnMissingProduct\",\"Value\":\"False\""));
        assert!(s.contains("\"bEnableOfflineItemsDuplicationPrevention\",\"Value\":\"False\""));
        assert!(s.contains("\"bDisableSaveOSS\",\"Value\":\"False\""));
        assert!(s.contains("\"bCrateRouletteEnabled\",\"Value\":\"False\""));
        assert!(s.contains("\"bQuickDropOpening\",\"Value\":\"True\""));
        assert!(s.contains("\"ProductsConfig\":{"));
    }

    #[tokio::test]
    async fn test_patch_reservation_applies_dev_title_to_bits() {
        let test_pid = "37674b519c3544beb544f437f539ebf2";
        set_learned_player_id(test_pid);
        let inner = serde_json::json!({
            "ReservationID": "test-res",
            "Players": [
                {
                    "PlayerID": "Epic|37674b519c3544beb544f437f539ebf2|0",
                    "PlayerName": "AnySpoofedName",
                    "bRemotePlayer": true,
                    "TitleID": "SomeOldTitle",
                    "Loadout": [11603, 376]
                },
                {
                    "PlayerID": "Steam|76561198643170831|0",
                    "PlayerName": "Teammate",
                    "bRemotePlayer": true,
                    "TitleID": "TeammateTitle",
                    "Loadout": [11603, 376]
                }
            ]
        });
        let root = serde_json::json!({
            "MessageType": "AddReservationMessagePublic_X",
            "MessagePayload": inner.to_string()
        });
        let raw = serde_json::to_vec(&root).unwrap();

        let (patched, changed) = patch_reservation_json_with_rules(&raw, &[]).await;
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let payload_str = val["MessagePayload"].as_str().unwrap();
        let parsed_inner: serde_json::Value = serde_json::from_str(payload_str).unwrap();

        let (expected_title_id, expected_title_text) = configured_title();
        let player = &parsed_inner["Players"][0];
        assert_eq!(player["TitleID"], expected_title_id);
        assert_eq!(player["PlayerTitle"], expected_title_text);
        assert_eq!(player["Title"], expected_title_text);

        let other = &parsed_inner["Players"][1];
        assert_eq!(other["TitleID"], "TeammateTitle");
    }

    #[test]
    fn test_normalize_player_id_handles_platform_prefixes() {
        assert_eq!(normalize_player_id("Epic|37674b519c3544beb544f437f539ebf2|0"), "37674b519c3544beb544f437f539ebf2");
        assert_eq!(normalize_player_id("Steam|37674b519c3544beb544f437f539ebf2|0"), "37674b519c3544beb544f437f539ebf2");
        assert_eq!(normalize_player_id("PS4|37674b519c3544beb544f437f539ebf2|0"), "37674b519c3544beb544f437f539ebf2");
        assert_eq!(normalize_player_id("Xbox|37674b519c3544beb544f437f539ebf2|0"), "37674b519c3544beb544f437f539ebf2");
        assert_eq!(normalize_player_id("37674b519c3544beb544f437f539ebf2"), "37674b519c3544beb544f437f539ebf2");
    }

    #[test]
    fn test_patch_player_products_nested_result() {
        let body = br#"{"Result":{"ProductData":[{"ProductID":100,"InstanceID":"abc123","SeriesID":1}]}}"#;
        let items = vec![(4284, 12)]; // Fennec (Titanium White)
        let (patched, changed) = patch_player_products_json(body, &items);
        assert!(changed, "Nested Result.ProductData must be patched");
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let arr = val["Result"]["ProductData"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let injected = arr.iter().find(|i| i["ProductID"] == 4284).unwrap();
        assert_eq!(injected["PaintID"], 12);
        assert_eq!(injected["Paint"], 12);
        assert_eq!(injected["Attributes"][0]["Key"], "Painted");
        assert_eq!(injected["Attributes"][0]["Value"], "12");
        assert_eq!(injected["Attributes"][1]["Key"], "Paint");
        assert_eq!(injected["Attributes"][1]["Value"], "12");
        assert!(injected.get("AddedTimestamp").is_some());
        assert!(injected.get("UpdatedTimestamp").is_some());
    }

    #[test]
    fn test_patch_users_can_show_avatar() {
        let test_pid = "37674b519c3544beb544f437f539ebf2";
        set_learned_player_id(test_pid);
        let body = br#"{"Result":{"AllowedPlayerIDs":["Epic|someother|0"],"HiddenPlayerIDs":["Epic|37674b519c3544beb544f437f539ebf2|0"]}}"#;
        let (patched, changed) = patch_users_can_show_avatar_json(body);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let allowed = val["Result"]["AllowedPlayerIDs"].as_array().unwrap();
        let hidden = val["Result"]["HiddenPlayerIDs"].as_array().unwrap();
        assert!(hidden.is_empty(), "HiddenPlayerIDs must be cleared");
        assert!(allowed.iter().any(|v| normalize_player_id(v.as_str().unwrap()) == test_pid));
        assert!(allowed.iter().any(|v| v.as_str().unwrap().contains("someother")));
    }

    #[test]
    fn test_patch_eos_accounts_json_matches_real_name() {
        let mut val = serde_json::json!([
            {
                "accountId": "37674b519c3544beb544f437f539ebf2",
                "displayName": "OldName"
            },
            {
                "accountId": "someoneelse1234",
                "displayName": "OtherPlayer"
            }
        ]);
        let (modified, learned_pid, learned_name) = patch_eos_accounts_json(
            &mut val,
            "SpoofedDev",
            None,
            Some("OldName"),
        );
        assert!(modified);
        assert_eq!(val[0]["displayName"], "SpoofedDev");
        assert_eq!(val[1]["displayName"], "OtherPlayer");
        assert_eq!(learned_pid.as_deref(), Some("37674b519c3544beb544f437f539ebf2"));
        assert_eq!(learned_name.as_deref(), Some("OldName"));
    }

    #[test]
    fn test_unlock_container_response_matches_rlapi_shape() {
        let resp = super::generate_simulated_unlock_container_payload(b"{}")
            .expect("crate open must produce a response");
        let drops = resp
            .get("Result")
            .and_then(|r| r.get("Drops"))
            .and_then(|d| d.as_array())
            .expect("Result.Drops array");
        assert_eq!(drops.len(), 1);
        assert_eq!(resp.get("bSuccess").and_then(|v| v.as_bool()), Some(true));
        let drop = &drops[0];
        assert!(drop.get("ProductID").unwrap().is_i64());
        assert!(drop.get("InstanceID").unwrap().is_string(), "drops must have non-empty 32-hex InstanceID");
        assert!(drop.get("SeriesID").unwrap().is_i64());
        assert!(drop.get("AddedTimestamp").unwrap().is_i64());
        assert!(drop.get("UpdatedTimestamp").unwrap().is_i64());
        assert!(drop.get("Attributes").unwrap().is_array());
        assert!(resp.get("Drops").is_some(), "top-level Drops field must be present for UnrealScript RPC");
        assert!(resp.get("ContainerDrops").is_none(), "no ContainerDrops field");
    }

    #[test]
    fn test_avatar_player_id_from_sidecar_path() {
        let id = 76561198000000000u64;
        let path = format!(
            "/content/images/players/avatars/large/{id}/0?buildsecret=abc123&tab=1"
        );
        assert_eq!(super::avatar_player_id_from_path(&path), Some(id));
    }

    #[test]
    fn test_avatar_player_id_from_sidecar_path_png() {
        let id = 76561198012345678u64;
        let path = format!("/content/images/players/avatars/large/{id}.png");
        assert_eq!(super::avatar_player_id_from_path(&path), Some(id));
    }

    #[test]
    fn test_epic_player_id_normalizes_to_account_id() {
        assert_eq!(
            super::normalize_player_id("Epic|37674b519c3544beb544f437f539ebf2|0"),
            "37674b519c3544beb544f437f539ebf2"
        );
        let encoded = crate::avatar::percent_decode("epic%7c37674b519c3544beb544f437f539ebf2%7c0");
        assert_eq!(
            super::normalize_player_id(&encoded),
            "37674b519c3544beb544f437f539ebf2"
        );
    }

    #[test]
    fn test_avatar_player_id_from_local_route() {
        let path = "/rlim/avatar/76561198000000000";
        assert_eq!(
            super::avatar_player_id_from_path(path),
            Some(76561198000000000)
        );
    }

    #[tokio::test]
    async fn test_ws_unlock_container_request_tracking_and_interception() {
        let req_id = "test-unlock-req-999";
        let outbound = format!(
            "PsyService: MicroTransactions/UnlockContainer\r\nPsyRequestID: {req_id}\r\nPsyTime: 1700000000\r\n\r\n{{\"InstanceIDs\":[\"123\"]}}"
        );

        let (_, out_patched) = super::patch_ws_frame_binary(outbound.as_bytes(), false).await;
        assert!(!out_patched, "client request body must not be modified");

        let inbound_err = format!(
            "PsyResponseID: {req_id}\r\nPsyStatus: 400\r\nPsyTime: 1700000001\r\n\r\n{{\"Error\":\"TransactionFailed\"}}"
        );

        let (resp_bytes, in_patched) = super::patch_ws_frame_binary(inbound_err.as_bytes(), true).await;
        assert!(in_patched, "server error response must be intercepted and patched");

        let Some(hdr_end) = super::find_bytes(&resp_bytes, b"\r\n\r\n") else {
            panic!("missing header boundary");
        };
        let hdrs = &resp_bytes[..hdr_end];
        let body = &resp_bytes[hdr_end + 4..];

        assert_eq!(super::get_ws_header_value(hdrs, "PsyStatus"), "200");
        assert_eq!(super::get_ws_header_value(hdrs, "PsyResponseID"), req_id);

        let resp_json: serde_json::Value = serde_json::from_slice(body).expect("valid json");
        assert_eq!(resp_json.get("bSuccess").and_then(|v| v.as_bool()), Some(true));
        assert!(resp_json.get("Drops").is_some(), "top-level Drops must be present for UnrealScript RPC");
        assert!(resp_json.get("Result").and_then(|r| r.get("Drops")).is_some());
    }

    #[test]
    fn test_lobby_avatar_path_matches_local_pid() {
        let test_pid = "37674b519c3544beb544f437f539ebf2";
        let path_str = format!("/content/images/players/avatars/large/epic%7c{test_pid}%7c0/0");
        let query_str = "buildSecret=abc123";
        let test_account_id = super::normalize_player_id(test_pid);

        let target_is_local_pid = path_str.split(&['/', '?', '&'][..])
            .chain(query_str.split('&').filter_map(|q| q.split_once('=').map(|(_, v)| v)))
            .any(|part| {
                let decoded = crate::avatar::percent_decode(part);
                let candidate = super::normalize_player_id(&decoded);
                !candidate.is_empty() && candidate == test_account_id
            });

        assert!(target_is_local_pid, "lobby path for local player ID must match as local");
    }

    #[test]
    fn test_nonlocal_player_request_with_tab_one_is_not_local() {
        let local_pid = "37674b519c3544beb544f437f539ebf2";
        let other_pid = "ffffffffffffffffffffffffffffffff";
        let uri = format!("/content/images/players/avatars/large/epic%7c{other_pid}%7c0/0?tab=1&buildSecret=abc123");

        assert!(!super::avatar_path_matches_player(&uri, local_pid),
            "nonlocal player request with tab=1 must never match as local PID");
    }

    #[test]
    fn test_rlim_pfp_route_recognized_as_local() {
        for path in &["/rlim/pfp", "/rlim/avatar", "/rlim/avatar.png", "/a.png"] {
            assert!(super::is_local_avatar_route(path, None),
                "{path} must be recognized as local route");
        }
        // A route with an explicit player id query param is NOT a purely local route
        assert!(!super::is_local_avatar_route("/rlim/avatar", Some("player_id=123")));
    }

    #[test]
    fn test_container_drop_table_authoritative_lookup() {
        assert_eq!(super::lookup_series_id_for_product(2522), 21, "Secret Santa Crate SeriesID");
        assert_eq!(super::lookup_series_id_for_product(2532), 21, "Wonderment SeriesID");
        assert_eq!(super::lookup_series_id_for_product(1009), 2, "Champion 1 Crate SeriesID");
        assert_eq!(super::lookup_series_id_for_product(1102), 2, "Dominus: Arcana SeriesID");
    }

    #[test]
    fn test_unlock_container_with_secret_santa_crate() {
        // Crate ProductID 2522 is 0x000009da in hex
        let inst_id = "000009da000000000000000000000000";
        let req_body = serde_json::json!({
            "PlayerID": "Epic|test|0",
            "InstanceIDs": [inst_id],
            "KeyInstanceIDs": []
        });
        let req_bytes = serde_json::to_vec(&req_body).unwrap();
        let resp = super::generate_simulated_unlock_container_payload(&req_bytes)
            .expect("must produce unlock response");

        assert_eq!(resp["bSuccess"], true);
        assert_eq!(resp["InstanceIDs"][0], inst_id);
        assert_eq!(resp["Result"]["InstanceIDs"][0], inst_id);

        let drop = &resp["Result"]["Drops"][0];
        let drop_pid = drop["ProductID"].as_i64().unwrap() as i32;
        let drop_sid = drop["SeriesID"].as_i64().unwrap() as i32;

        assert_eq!(drop_sid, 21, "Secret Santa drop must have SeriesID 21");
        let secret_santa_pool = [2489, 2495, 2496, 2500, 2579, 2478, 2442, 2481, 2482, 2532, 2854, 2966, 3239, 3453];
        assert!(secret_santa_pool.contains(&drop_pid), "Dropped item {drop_pid} must be in Secret Santa pool");
    }

    #[test]
    fn test_emulate_unboxing_pipeline_and_garage_series_filter_safety() {
        super::clear_dynamic_spawned_items();
        // 1. Emulate spawning a Secret Santa Crate (2522) into active inventory
        let _ = super::register_spawned_items(&[(2522, 0)]);
        {
            let spawned = super::load_active_spawned_items(None);
            assert!(spawned.iter().any(|&(id, _)| id == 2522), "Crate 2522 must be in spawned items");
        }

        // 2. Emulate client sending UnlockContainer RPC request
        let inst_id = "000009da000000000000000000000000"; // 2522 in hex
        let req_body = serde_json::json!({
            "PlayerID": "Epic|37674b519c3544beb544f437f539ebf2|0",
            "InstanceIDs": [inst_id],
            "KeyInstanceIDs": []
        });
        let req_bytes = serde_json::to_vec(&req_body).unwrap();
        let resp = super::generate_simulated_unlock_container_payload(&req_bytes)
            .expect("must produce unlock response");

        // 3. Strict schema validation of unbox response
        assert_eq!(resp["bSuccess"], true);
        assert_eq!(resp["bSkipRoulette"], true);
        assert_eq!(resp["bUnlocked"], true);
        assert_eq!(resp["InstanceIDs"][0], inst_id);
        assert_eq!(resp["Result"]["InstanceIDs"][0], inst_id);
        assert_eq!(resp["Result"]["bSkipRoulette"], true);
        assert_eq!(resp["Result"]["bUnlocked"], true);

        let drop = &resp["Result"]["Drops"][0];
        let drop_pid = drop["ProductID"].as_i64().expect("ProductID must be integer") as i32;
        let drop_sid = drop["SeriesID"].as_i64().expect("SeriesID must be integer") as i32;

        // SeriesID must be valid and match Secret Santa (21)
        assert_eq!(drop_sid, 21, "Secret Santa drop SeriesID must be 21");
        let secret_santa_pool = [2489, 2495, 2496, 2500, 2579, 2478, 2442, 2481, 2482, 2532, 2854, 2966, 3239, 3453];
        assert!(secret_santa_pool.contains(&drop_pid), "Dropped item {drop_pid} must be in Secret Santa pool");

        // InstanceID must be exactly 32-hex characters and non-null (satisfies native C++ FQWord read at [rax + 0x68])
        let drop_inst = drop["InstanceID"].as_str().expect("InstanceID must be non-null string");
        assert_eq!(drop_inst.len(), 32, "InstanceID must be 32 characters hex");
        assert!(drop_inst.chars().all(|c| c.is_ascii_hexdigit()), "InstanceID must be valid hex");
        u128::from_str_radix(drop_inst, 16).expect("InstanceID must parse as 128-bit hex for Unreal FQWord");

        // AddedTimestamp and UpdatedTimestamp must be positive integers
        let added_ts = drop["AddedTimestamp"].as_i64().expect("AddedTimestamp must be integer");
        let updated_ts = drop["UpdatedTimestamp"].as_i64().expect("UpdatedTimestamp must be integer");
        assert!(added_ts > 1700000000);
        assert!(updated_ts > 1700000000);
        assert_eq!(drop["Amount"].as_i64(), Some(1));

        // 4. Verify opened crate is consumed and dropped item is registered in active items
        {
            let spawned = super::load_active_spawned_items(None);
            assert!(!spawned.iter().any(|&(id, _)| id == 2522), "Opened crate 2522 must be consumed and removed");
            assert!(spawned.iter().any(|&(id, _)| id == drop_pid), "Dropped item must be registered in active items");
        }

        // 4b. Verify dropped item is synchronized to swaps.json for VelocityRL UI display
        {
            super::sync_spawned_items_to_swaps(&[(drop_pid, 0)]);
            let mut found_in_swaps = false;
            for p in &super::candidate_swaps_paths() {
                if let Ok(content) = std::fs::read_to_string(p) {
                    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                        if arr.iter().any(|s| {
                            let wid = s.get("wanted_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let oid = s.get("owned_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            wid == drop_pid && oid == drop_pid
                        }) {
                            found_in_swaps = true;
                            break;
                        }
                    }
                }
            }
            assert!(found_in_swaps, "Dropped item must be synchronized into swaps.json for VelocityRL UI");
        }

        // 5. Emulate PsyNet inventory sync via patch_player_products_json
        let base_inventory = serde_json::json!({
            "Result": {
                "ProductData": [
                    {
                        "ProductID": 23,
                        "InstanceID": "00000017000000000000000000000000",
                        "SeriesID": 0,
                        "Amount": 1
                    }
                ]
            }
        });
        let base_bytes = serde_json::to_vec(&base_inventory).unwrap();
        let mut test_items = super::load_active_spawned_items(None);
        if !test_items.iter().any(|&(id, _)| id == drop_pid) {
            test_items.push((drop_pid, 0));
        }
        let (patched_bytes, changed) = super::patch_player_products_json(&base_bytes, &test_items);
        assert!(changed, "Inventory must be patched with dropped product");

        let patched_val: serde_json::Value = serde_json::from_slice(&patched_bytes).unwrap();
        let prods = patched_val["Result"]["ProductData"].as_array().expect("ProductData array");

        // 6. Emulate UnrealScript SaveData_TA:GetUniqueSeriesIDs & C++ [rax + 0x68] read
        let mut unique_series = std::collections::HashSet::new();
        for p in prods {
            // Emulate pointer check (must not be null / None)
            assert!(!p.is_null(), "Product entry P must not be null");
            let pid = p["ProductID"].as_i64().expect("ProductID must be integer");
            assert!(pid > 0, "ProductID must be positive");

            // Emulate C++ native read of [rax + 0x68] (InstanceID)
            let inst = p["InstanceID"].as_str().expect("InstanceID must be string");
            assert_eq!(inst.len(), 32, "InstanceID must be 32-hex");
            u128::from_str_radix(inst, 16).expect("InstanceID must parse as 128-bit hex");

            // Emulate UnrealScript: if (P.SeriesID > 0) SeriesIDs.AddItem(P.SeriesID)
            let sid = p["SeriesID"].as_i64().expect("SeriesID must be integer");
            assert!(sid >= 0, "SeriesID must be non-negative");
            if sid > 0 {
                unique_series.insert(sid);
            }
        }

        assert!(unique_series.contains(&21), "UniqueSeriesIDs must contain Secret Santa series 21");
        super::clear_dynamic_spawned_items();
    }

    #[test]
    fn test_emulate_tier_drop_unboxing_produces_real_items() {
        // Test unboxing all tier drops: Sport (5302), Special (5301), Deluxe (5303), Import (5300), Exotic (5299), Golden Egg (3049)
        let tier_containers = [5302, 5301, 5303, 5300, 5299, 3049];
        let placeholder_pids = [5364, 5365, 5366, 5367, 5368, 5369];

        for &cid in &tier_containers {
            let inst_id = format!("{:08x}000000000000000000000000", cid as u32);
            let req_body = serde_json::json!({
                "PlayerID": "Epic|test_user|0",
                "InstanceIDs": [inst_id],
                "KeyInstanceIDs": []
            });
            let req_bytes = serde_json::to_vec(&req_body).unwrap();
            let resp = super::generate_simulated_unlock_container_payload(&req_bytes)
                .unwrap_or_else(|| panic!("Container {cid} must produce unlock response"));

            assert_eq!(resp["bSuccess"], true);
            let drop = &resp["Result"]["Drops"][0];
            let drop_pid = drop["ProductID"].as_i64().expect("ProductID must be integer") as i32;

            // Must NEVER be an internal placeholder token (5364..=5369)
            assert!(
                !placeholder_pids.contains(&drop_pid),
                "Container {cid} must drop a real cosmetic item, not placeholder {drop_pid}"
            );

            // Must have valid 32-hex InstanceID
            let inst = drop["InstanceID"].as_str().expect("InstanceID must be string");
            assert_eq!(inst.len(), 32);
            u128::from_str_radix(inst, 16).expect("InstanceID must parse as 128-bit hex");

            // Timestamps must be valid
            assert!(drop["AddedTimestamp"].as_i64().unwrap() > 1700000000);
            assert!(drop["UpdatedTimestamp"].as_i64().unwrap() > 1700000000);
        }
    }

    #[test]
    fn test_get_container_drop_table_response_structure() {
        let resp_bytes = super::get_authoritative_container_drop_table_response_bytes();
        let val: serde_json::Value = serde_json::from_slice(&resp_bytes).expect("valid json");
        assert_eq!(val["bSuccess"], true);
        let list = val["Result"]["ContainerDrops"].as_array().expect("array of container drops");
        assert_eq!(list.len(), 120, "authoritative drop table has 120 containers");
    }

    #[test]
    fn test_container_quantity_stacking_and_decrement() {
        super::set_container_quantities(&[(5301, 5), (1353, 10)]);
        assert_eq!(super::get_container_quantity(5301), 5);
        assert_eq!(super::get_container_quantity(1353), 10);

        let body = br#"{"ProductData":[]}"#;
        let (patched, changed) = super::patch_player_products_json(body, &[(5301, 0), (1353, 0)]);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let arr = val["ProductData"].as_array().unwrap();
        let c5301 = arr.iter().find(|i| i["ProductID"] == 5301).unwrap();
        let c1353 = arr.iter().find(|i| i["ProductID"] == 1353).unwrap();
        assert_eq!(c5301["Amount"], 5);
        assert_eq!(c1353["Amount"], 10);

        let rem = super::decrement_container_quantity(5301);
        assert_eq!(rem, 4);
        assert_eq!(super::get_container_quantity(5301), 4);
    }

    #[test]
    fn test_dedup_keeps_distinct_titles_with_same_product_id() {
        let mut data: Vec<serde_json::Value> = (0..50)
            .map(|n| serde_json::json!({
                "ProductID": 3036,
                "InstanceID": format!("t{n:031}"),
                "SeriesID": 1,
                "Attributes": [{ "Key": "TitleId", "Value": format!("Title_{n}") }]
            }))
            .collect();
        data.push(data[0].clone());
        for t in ["NoInst_A", "NoInst_A", "NoInst_B"] {
            data.push(serde_json::json!({
                "ProductID": 3036, "SeriesID": 1,
                "Attributes": [{ "Key": "TitleId", "Value": t }]
            }));
        }
        let body = serde_json::to_vec(&serde_json::json!({ "ProductData": data })).unwrap();
        let (patched, _) = super::patch_player_products_json(&body, &[(4284, 0)]);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let titles: Vec<_> = val["ProductData"].as_array().unwrap().iter()
            .filter(|i| i["ProductID"] == 3036)
            .collect();
        assert_eq!(titles.len(), 52, "50 instanced + NoInst_A + NoInst_B");
    }

    #[test]
    fn test_spawned_titles_injected_with_title_id_attribute() {
        let body = br#"{"ProductData":[{"ProductID":3036,"InstanceID":"0000000000000000000000000000beef","SeriesID":1,"Attributes":[{"Key":"TitleId","Value":"Owned_Title"}]}]}"#;
        let ids: Vec<String> = ["Owned_Title", "RLCS_X_Champion", "CRL_Analyst"].iter().map(|s| s.to_string()).collect();
        let (patched, changed) = super::patch_player_products_json_with_titles(body, &[], &ids);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let arr = val["ProductData"].as_array().unwrap();
        let mut found: Vec<&str> = arr.iter().filter_map(super::item_title_id).collect();
        found.sort();
        assert_eq!(found, vec!["CRL_Analyst", "Owned_Title", "RLCS_X_Champion"]);
        for it in arr {
            assert_eq!(it["ProductID"], 3036);
            let inst = it["InstanceID"].as_str().unwrap();
            assert_eq!(inst.len(), 32);
        }
        let injected = arr.iter().find(|i| super::item_title_id(i) == Some("CRL_Analyst")).unwrap();
        assert!(injected["InstanceID"].as_str().unwrap().chars().all(|c| c.is_ascii_hexdigit()));
    }
}


#[cfg(test)]
#[path = "runtime_regression_tests.rs"]
mod runtime_regression_tests;
