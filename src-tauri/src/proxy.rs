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
    for server in ["1.1.1.1:53", "8.8.8.8:53"] {
        if let Some(ip) = dns_query_a(host, server) {
            crate::applog::event(&format!("proxy: public DNS {host} -> {ip} via {server}"));
            let addr = std::net::SocketAddr::from((ip, 443));
            if let Ok(mut guard) = RESOLVED_IPS_CACHE.lock() {
                guard.get_or_insert_with(std::collections::HashMap::new).insert(host.to_string(), addr);
            }
            return Some(addr);
        }
    }
    if host.eq_ignore_ascii_case("api.epicgames.dev") {
        let fallback = std::net::SocketAddr::from(([104, 18, 125, 108], 443));
        crate::applog::event("proxy: using fallback Cloudflare IP for api.epicgames.dev");
        return Some(fallback);
    }
    crate::applog::event(&format!("proxy: public DNS lookup failed for {host}"));
    None
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
    let mut s = lower.as_str();
    if let Some(rest) = s.strip_prefix("epic|") {
        s = rest;
    }
    if let Some((core, _)) = s.split_once('|') {
        s = core;
    }
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
            if let Some(real) = filter_real_name {
                if let Some(ref disp) = current_disp {
                    if disp.eq_ignore_ascii_case(real) {
                        is_match = true;
                    }
                }
            }
            if !is_match {
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
            }

            if is_match {
                if let Some(old_name) = current_disp {
                    if old_name != new_name {
                        learned_name = Some(old_name);
                    }
                }
                user_data.insert("displayName".to_string(), serde_json::json!(new_name));
                user_data.insert("sanitizedDisplayName".to_string(), serde_json::json!(new_name));
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
        if let Some(real) = filter_real_name {
            if let Some(ref disp) = current_disp {
                if disp.eq_ignore_ascii_case(real) {
                    is_match = true;
                }
            }
        }
        if !is_match {
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
        }

        if is_match {
            if let Some(old_name) = current_disp {
                if old_name != new_name {
                    learned_name = Some(old_name);
                }
            }
            user_data.insert("displayName".to_string(), serde_json::json!(new_name));
            user_data.insert("sanitizedDisplayName".to_string(), serde_json::json!(new_name));
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
        .filter(|s| !s.is_empty() && s != "temp");

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
        "epicDisplayName",
        "PlayerNickName",
        "NickName",
        "username",
        "UserName",
        "TargetName",
        "AccountName",
        "PersonaName",
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
        "microtransaction",
        "getcatalog",
        "catalog",
        "itemshop",
        "store",
        // AuthPlayer must never be patched with name spoofing
        "authplayer",
        "tournaments/getbracket",
        "tournaments/getactivebracket",
        "tournaments/gettournament",
        "tournaments/gettournaments",
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
        b"GetCatalog",
        b"Catalog",
        b"Microtransaction",
        b"GetPlayerCatalog",
        // Presence and party frames — these carry friends' online state and must not be modified
        b"PresenceState",
        b"PartyMember",
        b"RichPresence",
        b"SocialBeacon",
        b"FriendStatus",
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
/// a fixed port like 27505 (avoids conflicts / "broker already in use").
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
    let client = pin_eos_hosts_on_builder(
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("config.psynet.gg", "34.160.180.65:443".parse().unwrap())
            .resolve("api.rlpp.psynet.gg", "34.54.194.77:443".parse().unwrap()),
    )
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let connector = create_upstream_tls_connector();
    let addr = format!("{upstream_host}:{upstream_port}");
    let tcp_conn = match tokio::net::TcpStream::connect(&addr).await {
        Ok(t) => t,
        Err(e) => {
            crate::applog::event(&format!("forward proxy ws: connect to {addr} failed: {e}"));
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(full_body(format!("connect error: {e}")))
                .unwrap());
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

    crate::applog::traffic_debug(&format!(
        "[FWD-WS] ⚠ RAW TUNNEL (no frame patching!) to {} | path={} | status={}",
        upstream_host, path_and_query, status_code
    ));
    crate::applog::record_traffic_event(
        "RAW-WS",
        "TUNNEL",
        &format!("https://{upstream_host}{path_and_query}"),
        Some(&status_code.to_string()),
        false,
        None,
        None,
        &[],
        None,
    );

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

    crate::applog::traffic_debug(&format!(
        "[FWD-HTTP] CLIENT->SRV | {method} {url}"
    ));

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

    crate::applog::record_traffic_event(
        "FWD-HTTP",
        "CLIENT->SRV",
        &format!("{method} {url}"),
        None,
        false,
        None,
        None,
        &req_body_bytes,
        None,
    );

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
        let orig_resp_len = resp_bytes.len();

        crate::applog::traffic_debug(&format!(
            "[FWD-HTTP] SRV->CLIENT | {status} https://{upstream_host}{path}{query}"
        ));

        let spoof_cfg = match crate::psynet::load_active_spoof_from_disk() {
            Some(c) => Some(c),
            None => get_spoof_config().await,
        };
        let name_spoof = spoof_cfg
            .as_ref()
            .and_then(|c| c.name_spoof.as_ref())
            .filter(|n| n.enabled && !n.display_name.trim().is_empty());

        let mut final_body = resp_bytes;
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
                        crate::applog::event(&format!(
                            "forward proxy: spoofed displayName -> '{}' in response from {}",
                            ns.display_name.trim(),
                            upstream_host
                        ));
                        final_body = serialized;
                    }
                }

                if !path.to_ascii_lowercase().contains("authplayer") {
                    let (ws_patched, ws_changed) = patch_ws_name_fields(
                        &final_body,
                        ns.display_name.trim(),
                        target_pid,
                    );
                    if ws_changed {
                        crate::applog::event(&format!(
                            "forward proxy: spoofed name fields -> '{}' in response from {}",
                            ns.display_name.trim(),
                            upstream_host
                        ));
                        final_body = ws_patched;
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
                || find_bytes(&final_body, b"\"currencies\"").is_some()
                || find_bytes(&final_body, b"\"Wallet\"").is_some()
                || find_bytes(&final_body, b"\"wallet\"").is_some()
                || find_bytes(&final_body, b"TournamentCredit").is_some()
                || find_bytes(&final_body, b"TournamentPoint").is_some();
            if is_wallet {
                let (new_body, did_patch) = patch_player_wallet_json(&final_body, cs);
                if did_patch {
                    crate::applog::event(&format!(
                        "forward proxy: patched wallet credits -> {} / tourney {} in response from {}",
                        cs.amount, cs.tournament_amount, upstream_host
                    ));
                    final_body = new_body;
                }
            }
        }

        if let Some(inv) = spoof_cfg.as_ref().and_then(|c| c.inventory_spoof.as_ref()).filter(|i| i.enabled) {
            let path_lower = path.to_ascii_lowercase();
            let is_loadout = path_lower.contains("loadout")
                || path_lower.contains("matchmaking")
                || path_lower.contains("reservation")
                || path_lower.contains("party")
                || path_lower.contains("authplayer")
                || path_lower.contains("genericstorage")
                || find_bytes(&final_body, b"PlayerLoadout").is_some()
                || find_bytes(&final_body, b"LoadoutResponse").is_some()
                || find_bytes(&final_body, b"CustomLoadouts").is_some()
                || find_bytes(&final_body, b"ProfileLoadoutSave_TA").is_some();

            if is_loadout && !inv.items.is_empty() {
                let (new_body, did_patch) = patch_loadout_rpc_json(&final_body, inv, path_lower.contains("authplayer"));
                if did_patch {
                    crate::applog::event(&format!(
                        "forward proxy: patched loadout RPC [{path}] -> {} items spoofed in response from {upstream_host}",
                        inv.items.len()
                    ));
                    final_body = new_body;
                }
            }

            let is_inventory = path_lower.contains("getplayerproducts")
                || path_lower.contains("getloadoutproducts")
                || path_lower.contains("getplayerinventory");
            let is_excluded = path_lower.contains("crossentitlement")
                || path_lower.contains("droptable")
                || path_lower.contains("challenge")
                || path_lower.contains("rocketpass")
                || path_lower.contains("tradein")
                || path_lower.contains("entitlement")
                || path_lower.contains("unlockcontainer");

            let has_titles = !inv.titles.is_empty()
                || spoof_cfg.as_ref().map(|c| !c.equip_title_id.trim().is_empty()).unwrap_or(false);

            if (is_inventory || is_loadout) && !is_excluded && (!inv.items.is_empty() || has_titles) {
                let (new_body, did_patch) = patch_player_inventory_json(&final_body, inv);
                if did_patch {
                    crate::applog::event(&format!(
                        "forward proxy: patched inventory products [{path}] -> {} items, {} titles injected in response from {upstream_host}",
                        inv.items.len(),
                        inv.titles.len()
                    ));
                    final_body = new_body;
                }
            }

            if path_lower.contains("crossentitlement") && !inv.items.is_empty() {
                let (new_body, did_patch) = patch_cross_entitlement_json(&final_body, inv);
                if did_patch {
                    crate::applog::event(&format!(
                        "forward proxy: appended {} spawned product IDs to CrossEntitlement response from {upstream_host}",
                        inv.items.len()
                    ));
                    final_body = new_body;
                }
            }
        }

        crate::applog::record_traffic_event(
            "FWD-HTTP",
            "SRV->CLIENT",
            &format!("{path}{query}"),
            Some(&status.to_string()),
            final_body.len() != orig_resp_len,
            None,
            None,
            &final_body,
            None,
        );

        let mut resp_builder = Response::builder().status(status);
        for (k, v) in resp_headers.iter() {
            let k_lower = k.as_str().to_ascii_lowercase();
            if k_lower != "content-length"
                && k_lower != "content-encoding"
                && k_lower != "transfer-encoding"
            {
                resp_builder = resp_builder.header(k.as_str(), v.as_bytes());
            }
        }
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

    let listener = match tokio::net::TcpListener::bind("127.0.0.1:27505").await {
        Ok(l) => l,
        Err(_) => match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("broker: Failed to bind 127.0.0.1: {e}");
                crate::applog::event(&msg);
                return Err(msg);
            }
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

    let client = reqwest::Client::builder()
        .http1_only()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .resolve("api.rlpp.psynet.gg", "34.54.194.77:443".parse().unwrap())
        .build()
        .map_err(|e| format!("failed to create reqwest client for broker: {e}"))?;

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    *BROKER_STOP_TX.lock().unwrap() = Some(stop_tx);

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

            let client = client.clone();
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

    Ok(port)
}

/// Handle incoming requests on the plain-HTTP broker (ephemeral local port).
/// Upgrades WebSocket connections to the game server, and proxies HTTP RPC/Services calls to api.rlpp.psynet.gg.
async fn handle_broker_request(
    req: Request<Incoming>,
    client: reqwest::Client,
) -> Result<Response<ResponseBoxBody>, hyper::Error> {
    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let path_str = req.uri().path().to_ascii_lowercase();
    if path_str == "/crl/velocityrl.crl" || path_str == "/velocityrl.crl" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/pkix-crl")
            .header("Content-Length", CA_CRL_DER.len().to_string())
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(CA_CRL_DER.to_vec()))
            .unwrap());
    }
    if is_upgrade || path_str.starts_with("/ws") {
        return handle_websocket(req).await;
    }

    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_string();
    let upstream_url = format!("https://api.rlpp.psynet.gg{path_and_query}");

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

    let mut up_builder = client.request(method.clone(), &upstream_url);
    crate::applog::record_traffic_event(
        "BROKER-HTTP",
        "CLIENT->SRV",
        &format!("{method} {path_and_query}"),
        None,
        false,
        None,
        None,
        &body_bytes,
        None,
    );
    for (k, v) in req_headers.iter() {
        let k_str = k.as_str().to_ascii_lowercase();
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
    up_builder = up_builder.header("Host", "api.rlpp.psynet.gg");
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

    crate::applog::traffic_debug(&format!(
        "[BROKER-HTTP] SRV->CLIENT | {status} {method} {path_and_query}"
    ));

    let path_lower = path_and_query.to_ascii_lowercase();
    let is_auth_player = path_lower.contains("authplayer");

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };

    if is_auth_player {
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

        if let Some(http_base) = broker_http_base() {
            let local_ws_v2 = format!("{http_base}/ws/gc2");
            let local_ws_v1 = format!("{http_base}/ws/gc?PsyConnectionType=Player");
            if let Some(next) = replace_json_string_field(&out_body, "PerConURLv2", &local_ws_v2) {
                out_body = next;
                patched = true;
            }
            if let Some(next) = replace_json_string_field(&out_body, "PerConURL", &local_ws_v1) {
                out_body = next;
                patched = true;
            }
            if patched {
                crate::applog::event(&format!(
                    "broker: AuthPlayer WS URL rewritten to local broker ({local_ws_v2})"
                ));
            }
        } else {
            crate::applog::event(
                "broker: AuthPlayer WS rewrite skipped — broker port not set",
            );
        }

        if let Some(cfg) = &cfg_opt {
            if let Some(inv) = &cfg.inventory_spoof {
                if inv.enabled && !inv.items.is_empty() {
                    let (new_body, did_patch) = patch_loadout_rpc_json(&out_body, inv, true);
                    if did_patch {
                        out_body = new_body;
                        patched = true;
                        crate::applog::event(&format!(
                            "broker: patched AuthPlayer loadout cosmetics -> {} items spoofed ({} bytes)",
                            inv.items.len(),
                            out_body.len()
                        ));
                    }
                    let (new_body, did_patch_inv) = patch_player_inventory_json(&out_body, inv);
                    if did_patch_inv {
                        out_body = new_body;
                        patched = true;
                        crate::applog::event(&format!(
                            "broker: patched AuthPlayer products inventory -> {} items injected",
                            inv.items.len()
                        ));
                    }
                }
            }

            if let Some(cs) = &cfg.credit_spoof {
                if cs.enabled {
                    let (new_body, did_patch_wallet) = patch_player_wallet_json(&out_body, cs);
                    if did_patch_wallet {
                        out_body = new_body;
                        patched = true;
                        crate::applog::event(&format!(
                            "broker: patched AuthPlayer wallet currencies -> {} credits, {} tourney",
                            cs.amount, cs.tournament_amount
                        ));
                    }
                }
            }
        }
    } else if let Some(cfg) = &cfg_opt {
        let is_inventory_rpc = path_lower.contains("getplayerproducts")
            || path_lower.contains("getloadoutproducts")
            || path_lower.contains("getplayerinventory");
        let is_loadout_rpc = path_lower.contains("loadout")
            || path_lower.contains("genericstorage")
            || find_bytes(&out_body, b"PlayerLoadout").is_some()
            || find_bytes(&out_body, b"playerLoadout").is_some()
            || find_bytes(&out_body, b"LoadoutResponse").is_some()
            || find_bytes(&out_body, b"loadoutResponse").is_some()
            || find_bytes(&out_body, b"CustomLoadouts").is_some()
            || find_bytes(&out_body, b"ProfileLoadoutSave_TA").is_some();
        let is_excluded_rpc = path_lower.contains("crossentitlement")
            || path_lower.contains("droptable")
            || path_lower.contains("challenge")
            || path_lower.contains("rocketpass")
            || path_lower.contains("tradein")
            || path_lower.contains("entitlement")
            || path_lower.contains("unlockcontainer");

        if (is_loadout_rpc || is_inventory_rpc) && !is_excluded_rpc {
            if let Some(inv) = &cfg.inventory_spoof {
                let has_titles = !inv.titles.is_empty()
                    || !cfg.equip_title_id.trim().is_empty()
                    || cfg.swaps.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
                if inv.enabled && (!inv.items.is_empty() || has_titles) {
                    if !inv.items.is_empty() {
                        let (new_body, did_patch) = patch_loadout_rpc_json(&out_body, inv, false);
                        if did_patch {
                            out_body = new_body;
                            patched = true;
                            crate::applog::event(&format!(
                                "broker: patched HTTP loadout RPC [{path_and_query}] -> {} items spoofed",
                                inv.items.len()
                            ));
                        }
                    }
                    let (new_body, did_patch_inv) = patch_player_inventory_json(&out_body, inv);
                    if did_patch_inv {
                        out_body = new_body;
                        patched = true;
                        crate::applog::event(&format!(
                            "broker: patched HTTP products inventory [{path_and_query}] -> {} items, {} titles injected",
                            inv.items.len(),
                            inv.titles.len()
                        ));
                    }
                }
            }
        }

        if path_lower.contains("crossentitlement") {
            if let Some(inv) = cfg.inventory_spoof.as_ref().filter(|i| i.enabled && !i.items.is_empty()) {
                let (new_body, did_patch) = patch_cross_entitlement_json(&out_body, inv);
                if did_patch {
                    out_body = new_body;
                    patched = true;
                    crate::applog::event(&format!(
                        "broker: appended {} spawned product IDs to CrossEntitlement response",
                        inv.items.len()
                    ));
                }
            }
        }

        let is_wallet_rpc = path_lower.contains("getplayerwallet")
            || path_lower.contains("wallet")
            || path_lower.contains("tournament")
            || path_lower.contains("currencies")
            || path_lower.contains("shops")
            || find_bytes(&out_body, b"\"Currencies\"").is_some()
            || find_bytes(&out_body, b"\"currencies\"").is_some()
            || find_bytes(&out_body, b"\"Wallet\"").is_some()
            || find_bytes(&out_body, b"\"wallet\"").is_some()
            || find_bytes(&out_body, b"TournamentCredit").is_some()
            || find_bytes(&out_body, b"TournamentPoint").is_some();
        if is_wallet_rpc {
            if let Some(cs) = &cfg.credit_spoof {
                if cs.enabled {
                    let (new_body, did_patch) = patch_player_wallet_json(&out_body, cs);
                    if did_patch {
                        out_body = new_body;
                        patched = true;
                        crate::applog::event(&format!(
                            "broker: patched wallet credits in RPC response -> credits: {}, tourney: {}",
                            cs.amount, cs.tournament_amount
                        ));
                    }
                }
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
                    crate::applog::event(&format!(
                        "broker: patched name in RPC response -> '{}'",
                        ns.display_name.trim()
                    ));
                }
            }
        }
    }

    crate::applog::record_traffic_event(
        "BROKER-HTTP",
        "SRV->CLIENT",
        &format!("{method} {path_and_query}"),
        Some(&status.to_string()),
        patched,
        None,
        None,
        &out_body,
        None,
    );

    let mut resp_builder = Response::builder().status(status.as_u16());
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

    if host_hdr.contains("ws.rlpp.psynet.gg") || (host_hdr.contains("psynet.gg") && is_upgrade) {
        return handle_websocket(req).await;
    }

    if host_hdr.contains("api.rlpp.psynet.gg") || host_hdr.contains("rlpp.psynet.gg") {
        return handle_broker_request(req, client).await;
    }

    let name_spoof_on = {
        let spoof_cfg = crate::psynet::load_active_spoof_from_disk();
        spoof_cfg
            .as_ref()
            .and_then(|c| c.name_spoof.as_ref())
            .map(|n| n.enabled)
            .unwrap_or(false)
    };

    if (is_eos_account_host(&host_hdr) && name_spoof_on) || host_hdr.contains("psyonix.com") || host_hdr.contains("live.psynet.gg") {
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
    let tcp_conn = match tokio::net::TcpStream::connect("34.149.116.40:443").await {
        Ok(t) => t,
        Err(e) => {
            crate::applog::event(&format!("proxy: direct connect to 34.149.116.40:443 failed ({e}), trying DNS ws.rlpp.psynet.gg:443"));
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

    let c2u = tokio::spawn(async move {
        while let Some(msg_res) = client_rx.next().await {
            match msg_res {
                Ok(msg) => {
                    let out_msg = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => {
                            let (patched_text, _) = patch_ws_frame_text(&t).await;
                            tokio_tungstenite::tungstenite::Message::Text(patched_text.into())
                        }
                        tokio_tungstenite::tungstenite::Message::Binary(b) => {
                            let (patched_b, _) = patch_ws_frame_binary(&b).await;
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
        while let Some(msg_res) = up_rx.next().await {
            match msg_res {
                Ok(msg) => {
                    let out_msg = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => {
                            let (patched_text, _) = patch_ws_frame_text(&t).await;
                            tokio_tungstenite::tungstenite::Message::Text(patched_text.into())
                        }
                        tokio_tungstenite::tungstenite::Message::Binary(b) => {
                            let (patched_b, _) = patch_ws_frame_binary(&b).await;
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
    });

    tokio::select! {
        _ = c2u => {},
        _ = u2c => {},
    }
    crate::applog::event("proxy: WebSocket tunnel closed");
}

static PSY_PENDING_REQUESTS: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> = std::sync::Mutex::new(None);

pub fn remember_psy_request(req_id: &str, service: &str) {
    if req_id.is_empty() || service.is_empty() { return; }
    if let Ok(mut guard) = PSY_PENDING_REQUESTS.lock() {
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        if map.len() > 2000 {
            map.clear();
        }
        map.insert(req_id.to_string(), service.to_ascii_lowercase());
    }
}

pub fn get_psy_service_for_response(resp_id: &str) -> Option<String> {
    if resp_id.is_empty() { return None; }
    if let Ok(guard) = PSY_PENDING_REQUESTS.lock() {
        if let Some(map) = guard.as_ref() {
            return map.get(resp_id).cloned();
        }
    }
    None
}

async fn patch_ws_frame_text(text: &str) -> (String, bool) {
    let (patched_bytes, changed) = patch_ws_frame_binary(text.as_bytes()).await;
    if changed {
        if let Ok(s) = String::from_utf8(patched_bytes) {
            return (s, true);
        }
    }
    (text.to_string(), false)
}

async fn patch_ws_frame_binary(frame: &[u8]) -> (Vec<u8>, bool) {
    let Some(hdr_end) = find_bytes(frame, b"\r\n\r\n") else {
        return (frame.to_vec(), false);
    };

    let headers_part = &frame[..hdr_end];
    let body_part = &frame[hdr_end + 4..];

    let conn_id = get_ws_header_any(headers_part, &["PsyConnectionID", "ConnectionID", "ConnectionId", "ConnID"]);
    if !conn_id.is_empty() {
        set_learned_player_id(&conn_id);
    }

    let req_id = get_ws_header_any(headers_part, &["PsyRequestID", "RequestID", "PsyReqID", "RequestId", "ReqID", "id"]);
    let req_svc = get_ws_header_any(headers_part, &["PsyService", "Service", "psy-service", "service", "RPC", "rpc", "Method"]);
    if !req_id.is_empty() && !req_svc.is_empty() {
        remember_psy_request(&req_id, &req_svc);
    }

    let resp_id = get_ws_header_any(headers_part, &["PsyResponseID", "ResponseID", "PsyRespID", "ResponseId", "RespID"]);
    let correlated_svc = if !resp_id.is_empty() {
        get_psy_service_for_response(&resp_id)
    } else {
        None
    };

    let mut svc = if !req_svc.is_empty() {
        req_svc
    } else if let Some(ref cs) = correlated_svc {
        cs.clone()
    } else {
        get_ws_header_any(headers_part, &["PsyService", "Service", "psy-service", "service", "RPC", "rpc", "Method"])
    };

    // Fallback: If service is still empty, inspect body for known signatures
    if svc.is_empty() && !body_part.is_empty() {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body_part) {
            if let Some(s) = val.get("Service").or_else(|| val.get("RPC")).or_else(|| val.get("Method")).and_then(|v| v.as_str()) {
                svc = s.to_string();
            } else if val.get("Skills").is_some() || val.get("Players").and_then(|p| p.as_array()).and_then(|a| a.first()).and_then(|f| f.get("Skills")).is_some() {
                svc = "skills/getplayerskills".to_string();
            } else if val.get("PlayerLoadout").is_some() || val.get("LoadoutResponse").is_some() || val.get("CustomLoadouts").is_some() {
                svc = "loadout/getplayerloadout".to_string();
            } else if val.get("Currencies").is_some() || val.get("Wallet").is_some() {
                svc = "shops/getplayerwallet".to_string();
            }
        }
    }

    let svc_lower = svc.to_ascii_lowercase();

    {
        let direction = if !req_id.is_empty() { "CLIENT->SRV" } else { "SRV->CLIENT" };
        crate::applog::traffic_debug(&format!(
            "[WS-FRAME] {} | svc={}",
            direction,
            if svc.is_empty() { "(none)" } else { &svc },
        ));
    }

    let is_skill = svc_lower.contains("skills/getplayerskill")
        || svc_lower.contains("skills/getplayersskills")
        || (find_bytes(body_part, b"\"Skills\"").is_some()
            && (find_bytes(body_part, b"\"Mu\"").is_some()
                || find_bytes(body_part, b"\"Tier\"").is_some()
                || find_bytes(body_part, b"\"Playlist\"").is_some()));

    if is_skill {
        extract_and_save_real_skills(body_part);
    }

    let direction = if !req_id.is_empty() { "CLIENT->SRV" } else { "SRV->CLIENT" };

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };

    let Some(cfg) = cfg_opt else {
        crate::applog::record_traffic_event(
            "WS-FRAME",
            direction,
            if svc.is_empty() { "(none)" } else { &svc },
            None,
            false,
            if req_id.is_empty() { None } else { Some(&req_id) },
            if resp_id.is_empty() { None } else { Some(&resp_id) },
            body_part,
            None,
        );
        return (frame.to_vec(), false);
    };

    let mut current_body = body_part.to_vec();
    let mut any_changed = false;

    let is_wallet = svc_lower.contains("shops/getplayerwallet")
        || svc_lower.contains("getplayerwallet")
        || (svc_lower.contains("wallet") && !svc_lower.contains("tournament"))
        || (find_bytes(body_part, b"\"Currencies\"").is_some()
            && find_bytes(body_part, b"\"Amount\"").is_some()
            && !svc_lower.contains("tournament")
            && !svc_lower.contains("schedule")
            && !svc_lower.contains("cycle"));

    let is_response_frame = !resp_id.is_empty();

    if is_wallet && is_response_frame {
        if let Some(cs) = &cfg.credit_spoof {
            if cs.enabled {
                let (new_body, changed) = patch_player_wallet_json(&current_body, cs);
                if changed {
                    current_body = new_body;
                    any_changed = true;
                    crate::applog::event(&format!(
                        "proxy: patched wallet credits -> {} / tourney {} ({} -> {} bytes)",
                        cs.amount,
                        cs.tournament_amount,
                        frame.len(),
                        current_body.len()
                    ));
                }
            }
        }
    }

    // DSR/RelayToServer is handled separately below.
    // Strictly identify loadout RPC frames
    let is_loadout_ws = svc_lower.contains("loadout/getplayerloadout")
        || svc_lower.contains("loadout/getplayerloadouts")
        || svc_lower.contains("loadout/saveloadout")
        || svc_lower.contains("authplayer")
        || svc_lower.contains("genericstorage/getplayergenericstorage")
        || svc_lower.contains("playerhasloadout")
        || find_bytes(body_part, b"PlayerLoadout").is_some()
        || find_bytes(body_part, b"playerLoadout").is_some()
        || find_bytes(body_part, b"LoadoutResponse").is_some()
        || find_bytes(body_part, b"loadoutResponse").is_some()
        || find_bytes(body_part, b"CustomLoadouts").is_some()
        || find_bytes(body_part, b"CustomLoadout").is_some()
        || find_bytes(body_part, b"ProfileLoadoutSave_TA").is_some()
        || find_bytes(body_part, b"ProductsSave_TA").is_some()
        || find_bytes(body_part, b"PartyMessage_Loadout").is_some()
        || find_bytes(body_part, b"ReplicatedLoadout").is_some();

    let is_excluded_ws = svc_lower.contains("crossentitlement")
        || svc_lower.contains("droptable")
        || svc_lower.contains("challenge")
        || svc_lower.contains("rocketpass")
        || svc_lower.contains("tradein")
        || svc_lower.contains("entitlement")
        || svc_lower.contains("unlockcontainer");

    // Strictly whitelist inventory product RPC frames to prevent over-patching drops/entitlements/destruction
    let is_inventory_ws = !is_excluded_ws && (
        svc_lower.contains("products/getplayerproducts")
        || svc_lower.contains("products/getloadoutproducts")
        || svc_lower.contains("products/getplayerinventory")
        || svc_lower.contains("products/getplayerproductstemplated")
        || (svc_lower.is_empty() && find_bytes(body_part, b"\"ProductData\"").is_some() && (find_bytes(body_part, b"\"InstanceID\"").is_some() || find_bytes(body_part, b"\"ProductInstanceID\"").is_some()))
    );

    if svc_lower.contains("playerhasloadout") {
        if let Ok(mut val) = serde_json::from_slice::<serde_json::Value>(&current_body) {
            if let Some(obj) = val.as_object_mut() {
                obj.insert("Result".into(), serde_json::json!(true));
                obj.insert("HasTemplate".into(), serde_json::json!(true));
                if let Ok(new_body) = serde_json::to_vec(&val) {
                    current_body = new_body;
                    any_changed = true;
                }
            }
        }
    }

    // Only patch loadout/inventory in SRV->CLIENT (response) direction, except DSR relay.
    let is_dsr = svc_lower.contains("dsr/") || svc_lower.contains("relaytoserver");
    if (is_loadout_ws || is_inventory_ws) && (is_response_frame || is_dsr) {
        if let Some(inv) = &cfg.inventory_spoof {
            let has_titles = !inv.titles.is_empty()
                || !cfg.equip_title_id.trim().is_empty()
                || cfg.swaps.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
            let has_work = inv.enabled && (!inv.items.is_empty() || has_titles);
            if has_work {
                let mut ws_patched = false;
                if is_loadout_ws && !inv.items.is_empty() {
                    let (new_body, changed) = patch_loadout_rpc_json(&current_body, inv, svc_lower.contains("authplayer"));
                    if changed {
                        current_body = new_body;
                        ws_patched = true;
                    }
                }
                if is_inventory_ws {
                    let (new_body, changed) = patch_player_inventory_json(&current_body, inv);
                    if changed {
                        current_body = new_body;
                        ws_patched = true;
                    }
                }
                if ws_patched {
                    any_changed = true;
                    crate::applog::event(&format!(
                        "proxy: patched loadout/inventory ws frame [{}] -> {} items, {} titles spoofed ({} -> {} bytes)",
                        svc,
                        inv.items.len(),
                        inv.titles.len(),
                        frame.len(),
                        current_body.len()
                    ));
                }
            }
        }
    }

    let is_cross_entitlement_ws = svc_lower.contains("crossentitlement");
    if is_cross_entitlement_ws && (is_response_frame || is_dsr) {
        if let Some(inv) = &cfg.inventory_spoof {
            if inv.enabled && !inv.items.is_empty() {
                let (new_body, changed) = patch_cross_entitlement_json(&current_body, inv);
                if changed {
                    current_body = new_body;
                    any_changed = true;
                    crate::applog::event(&format!(
                        "proxy: appended {} spawned product IDs to WS CrossEntitlement [{}]",
                        inv.items.len(),
                        svc
                    ));
                }
            }
        }
    }

    let is_leaderboard = false;

    if is_leaderboard {
        let (new_body, changed) = patch_leaderboard_json(&current_body, &cfg);
        if changed {
            current_body = new_body;
            any_changed = true;
            crate::applog::event(&format!(
                "proxy: patched leaderboard ws frame ({} -> {} bytes)",
                frame.len(),
                current_body.len()
            ));
        }
    }

    if is_skill {
        if let Some(fr) = cfg.fake_ranks.as_ref() {
            if fr.enabled {
                let (new_body, changed) = patch_get_player_skill_json(&current_body, fr);
                if changed {
                    current_body = new_body;
                    any_changed = true;
                    crate::applog::event(&format!(
                        "proxy: patched fake ranks ws frame ({} -> {} bytes)",
                        frame.len(),
                        current_body.len()
                    ));
                }
            }
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
                    crate::applog::event(&format!(
                        "proxy: patched name in WS frame -> '{}' (target_pid={:?})",
                        ns.display_name.trim(),
                        target_pid
                    ));
                    current_body = new_body;
                    any_changed = true;
                }
            }
        }
    }

    if !any_changed {
        crate::applog::traffic_debug(&format!(
            "[WS-FRAME] ↩ NOT PATCHED | svc={}",
            if svc.is_empty() { "(none)" } else { &svc }
        ));
        crate::applog::record_traffic_event(
            "WS-FRAME",
            direction,
            if svc.is_empty() { "(none)" } else { &svc },
            None,
            false,
            if req_id.is_empty() { None } else { Some(&req_id) },
            if resp_id.is_empty() { None } else { Some(&resp_id) },
            body_part,
            None,
        );
        return (frame.to_vec(), false);
    }

    crate::applog::traffic_debug(&format!(
        "[WS-FRAME] ✅ PATCHED | svc={}",
        if svc.is_empty() { "(none)" } else { &svc },
    ));
    crate::applog::record_traffic_event(
        "WS-FRAME",
        direction,
        if svc.is_empty() { "(none)" } else { &svc },
        None,
        true,
        if req_id.is_empty() { None } else { Some(&req_id) },
        if resp_id.is_empty() { None } else { Some(&resp_id) },
        &current_body,
        None,
    );

    let new_headers = resign_ws_headers(headers_part, &current_body);
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

fn get_ws_header_any(headers: &[u8], keys: &[&str]) -> String {
    for k in keys {
        let val = get_ws_header_value(headers, k);
        if !val.is_empty() {
            return val;
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
    let psy_time = get_ws_header_value(headers, "PsyTime");
    let has_psysig = find_bytes(headers, b"PsySig:").is_some() || find_bytes(headers, b"psysig:").is_some();
    let has_psysignature = find_bytes(headers, b"Psysignature:").is_some() || find_bytes(headers, b"psysignature:").is_some();

    let updated_headers = if find_bytes(headers, b"Content-Length:").is_some()
        || find_bytes(headers, b"content-length:").is_some()
    {
        replace_ws_header_value(headers, "Content-Length", &body.len().to_string())
    } else {
        headers.to_vec()
    };

    if !has_psysig && !has_psysignature {
        return updated_headers;
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

    let key_to_replace = if has_psysig { "PsySig" } else { "Psysignature" };
    replace_ws_header_value(&updated_headers, key_to_replace, &sig)
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

    let assigned_rank = if let Some(cr) = custom_rank {
        cr
    } else {
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
        if let Some(obj) = result_obj.as_object_mut() {
            let reward_obj = obj.entry("RewardLevels").or_insert_with(|| serde_json::json!({}));
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

fn slot_index_for_item(item: &crate::psynet::InventorySpoofItemPayload) -> usize {
    crate::presets::slot_index_from_str(&item.slot)
}

fn slot_aliases(slot_idx: usize) -> &'static [&'static str] {
    match slot_idx {
        0 => &["Body", "body", "Car", "car", "Vehicle", "vehicle", "CarID", "BodyID", "BodyProductID", "VehicleProductID"],
        1 => &["Skin", "skin", "Decal", "decal", "SkinProductID", "DecalProductID"],
        2 => &["Wheel", "wheel", "Wheels", "wheels", "WheelProductID", "WheelsProductID"],
        3 => &["Boost", "boost", "RocketBoost", "rocketboost", "BoostProductID", "RocketBoostProductID"],
        4 => &["Antenna", "antenna", "AntennaProductID"],
        5 => &["Topper", "topper", "Hat", "hat", "TopperProductID"],
        6 => &["PaintFinish", "paintfinish", "Paint", "paint", "PaintFinishProductID"],
        7 => &["PaintFinishAccent", "CustomFinish", "AccentFinish", "PaintFinishSecondary"],
        8 => &["EngineAudio", "engineaudio", "Audio", "audio", "EngineAudioProductID"],
        9 => &["Trail", "trail", "SupersonicTrail", "TrailProductID"],
        10 => &["GoalExplosion", "goalexplosion", "Explosion", "explosion", "GoalExplosionProductID"],
        11 => &["PlayerBanner", "playerbanner", "Banner", "banner", "BannerProductID"],
        12 => &["PlayerAnthem", "playeranthem", "Anthem", "anthem", "Music", "music", "AnthemProductID"],
        13 => &["AvatarBorder", "avatarborder", "Border", "border", "BorderProductID"],
        _ => &["Body", "body"],
    }
}

fn new_slot_entry(slot_idx: usize, item: &crate::psynet::InventorySpoofItemPayload) -> serde_json::Value {
    let mut attributes = Vec::new();
    if item.paint_id > 0 {
        attributes.push(serde_json::json!({
            "Key": "Painted",
            "Value": item.paint_id,
            "TypeName": "ProductAttribute_Painted_TA"
        }));
        attributes.push(serde_json::json!({
            "Key": "Paint",
            "Value": item.paint_id
        }));
    }
    let instance_id_num = 998_000_000i64 + (slot_idx as i64);
    let instance_id_str = instance_id_num.to_string();
    serde_json::json!({
        "Slot": slot_idx,
        "SlotIndex": slot_idx,
        "ProductID": item.product_id,
        "ProductId": item.product_id,
        "SeriesID": item.series_id,
        "Paint": item.paint_id,
        "PaintID": item.paint_id,
        "InstanceID": instance_id_str,
        "ProductInstanceID": instance_id_str,
        "Attributes": attributes,
    })
}

fn update_slot_entry(elem: &mut serde_json::Value, item: &crate::psynet::InventorySpoofItemPayload) {
    let obj = match elem.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let mut updated_pid = false;
    for k in &["ProductID", "ProductId", "product_id", "Value", "ID", "Id"] {
        if obj.contains_key(*k) {
            obj.insert(k.to_string(), serde_json::json!(item.product_id));
            updated_pid = true;
        }
    }
    if !updated_pid {
        obj.insert("ProductID".into(), serde_json::json!(item.product_id));
    }

    let target_slot = slot_index_for_item(item);
    let instance_id_num = 998_000_000i64 + (target_slot as i64);
    let instance_id_str = instance_id_num.to_string();
    obj.insert("InstanceID".into(), serde_json::json!(&instance_id_str));
    obj.insert("ProductInstanceID".into(), serde_json::json!(&instance_id_str));
    if obj.contains_key("InstanceId") {
        obj.insert("InstanceId".into(), serde_json::json!(&instance_id_str));
    }

    if item.paint_id > 0 {
        let mut updated_paint = false;
        for k in &["Paint", "PaintID", "PaintId", "paint_id", "Painted"] {
            if obj.contains_key(*k) {
                obj.insert(k.to_string(), serde_json::json!(item.paint_id));
                updated_paint = true;
            }
        }
        if !updated_paint {
            obj.insert("Paint".into(), serde_json::json!(item.paint_id));
        }

        let paint_attr = serde_json::json!({
            "Key": "Painted",
            "Value": item.paint_id,
            "TypeName": "ProductAttribute_Painted_TA"
        });

        if let Some(attrs) = obj.get_mut("Attributes").and_then(|a| a.as_array_mut()) {
            let mut paint_found = false;
            for attr in attrs.iter_mut() {
                let key_name = attr.get("Key").and_then(|k| k.as_str()).unwrap_or("");
                if key_name.eq_ignore_ascii_case("paint") || key_name.eq_ignore_ascii_case("painted") {
                    *attr = paint_attr.clone();
                    paint_found = true;
                    break;
                }
            }
            if !paint_found {
                attrs.push(paint_attr);
            }
        } else {
            obj.insert("Attributes".into(), serde_json::json!([paint_attr]));
        }
    }
}


fn patch_loadout_container(
    val: &mut serde_json::Value,
    items: &[&crate::psynet::InventorySpoofItemPayload],
) -> bool {
    let mut changed = false;

    if let Some(arr) = val.as_array_mut() {
        if arr.is_empty() {
            return false;
        }

        let has_teams_or_presets = arr.iter().any(|e| {
            e.get("TeamIndex").is_some()
                || e.get("Loadout").is_some()
                || e.get("Products").is_some()
                || e.get("CustomLoadout").is_some()
                || e.get("customLoadout").is_some()
        });
        if has_teams_or_presets {
            for elem in arr.iter_mut() {
                if let Some(loadout_val) = elem.get_mut("Loadout") {
                    changed |= patch_loadout_container(loadout_val, items);
                } else {
                    changed |= patch_loadout_container(elem, items);
                }
            }
            return changed;
        }

        let is_number_array = arr.iter().all(|e| e.is_number());
        if is_number_array {
            for item in items {
                let target_slot = slot_index_for_item(item);
                if target_slot < arr.len() {
                    arr[target_slot] = serde_json::json!(item.product_id);
                    changed = true;
                } else if target_slot < 32 {
                    while arr.len() <= target_slot {
                        arr.push(serde_json::json!(0));
                    }
                    arr[target_slot] = serde_json::json!(item.product_id);
                    changed = true;
                }
            }
            return changed;
        }

        for item in items {
            let target_slot = slot_index_for_item(item);
            let aliases = slot_aliases(target_slot);
            let mut found = false;

            for elem in arr.iter_mut() {
                if let Some(slot_val) = elem.get("Slot")
                    .or_else(|| elem.get("SlotIndex"))
                    .or_else(|| elem.get("slot"))
                    .or_else(|| elem.get("SlotIdx"))
                {
                    let matches_slot = if let Some(n) = slot_val.as_i64() {
                        n == target_slot as i64
                    } else if let Some(s) = slot_val.as_str() {
                        s.parse::<usize>().map(|n| n == target_slot).unwrap_or_else(|_| {
                            aliases.iter().any(|alias| s.eq_ignore_ascii_case(alias))
                        })
                    } else {
                        false
                    };

                    if matches_slot {
                        update_slot_entry(elem, item);
                        found = true;
                        changed = true;
                        break;
                    }
                }
            }

            if !found {
                arr.push(new_slot_entry(target_slot, item));
                changed = true;
            }
        }
        return changed;
    }

    if let Some(obj) = val.as_object_mut() {
        for (_, sub_val) in obj.iter_mut() {
            if sub_val.is_object() || sub_val.is_array() {
                changed |= patch_loadout_container(sub_val, items);
            } else if let Some(s) = sub_val.as_str() {
                let trimmed = s.trim();
                if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                    || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                {
                    if let Ok(mut inner_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        if patch_loadout_container(&mut inner_val, items) {
                            if let Ok(new_str) = serde_json::to_string(&inner_val) {
                                *sub_val = serde_json::json!(new_str);
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        for item in items {
            let slot_idx = slot_index_for_item(item);
            let aliases = slot_aliases(slot_idx);
            let mut key_found = false;

            for alias in aliases {
                if let Some(field) = obj.get_mut(*alias) {
                    key_found = true;
                    if field.is_number() {
                        *field = serde_json::json!(item.product_id);
                        changed = true;
                    } else if let Some(field_obj) = field.as_object_mut() {
                        field_obj.insert("ProductID".into(), serde_json::json!(item.product_id));
                        if item.paint_id > 0 {
                            field_obj.insert("Paint".into(), serde_json::json!(item.paint_id));
                        }
                        changed = true;
                    }
                }
            }

            let idx_str = slot_idx.to_string();
            if let Some(field) = obj.get_mut(&idx_str) {
                key_found = true;
                if field.is_number() {
                    *field = serde_json::json!(item.product_id);
                    changed = true;
                } else if let Some(field_obj) = field.as_object_mut() {
                    field_obj.insert("ProductID".into(), serde_json::json!(item.product_id));
                    if item.paint_id > 0 {
                        field_obj.insert("Paint".into(), serde_json::json!(item.paint_id));
                    }
                    changed = true;
                }
            }

            if !key_found {
                let has_loadout_markers = obj.keys().any(|k| {
                    k == "Body" || k == "Wheels" || k == "Boost" || k == "0" || k == "2" || k == "3" || k.ends_with("ProductID")
                });
                if has_loadout_markers {
                    let canonical_name = aliases[0];
                    obj.insert(canonical_name.to_string(), serde_json::json!(item.product_id));
                    obj.insert(idx_str, serde_json::json!(item.product_id));
                    if item.paint_id > 0 {
                        obj.insert(format!("{canonical_name}Paint"), serde_json::json!(item.paint_id));
                    }
                    changed = true;
                }
            }
        }
    }

    changed
}

fn patch_loadout_rpc_json(
    body: &[u8],
    inventory_spoof: &crate::psynet::InventorySpoofPayload,
    is_auth_player: bool,
) -> (Vec<u8>, bool) {
    if !inventory_spoof.enabled || inventory_spoof.items.is_empty() {
        return (body.to_vec(), false);
    }
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let items_ref: Vec<&crate::psynet::InventorySpoofItemPayload> = inventory_spoof
        .items
        .iter()
        .filter(|it| it.product_id > 0)
        .collect();
    if items_ref.is_empty() {
        return (body.to_vec(), false);
    }

    let mut changed = false;

    changed |= patch_loadout_container(&mut root, &items_ref);
    if let Some(res) = root.get_mut("Result") {
        changed |= patch_loadout_container(res, &items_ref);
    }

    if is_auth_player {
        if let Some(obj) = root.as_object_mut() {
            let loadout_arr: Vec<serde_json::Value> = items_ref.iter().map(|item| {
                let slot_idx = slot_index_for_item(item);
                new_slot_entry(slot_idx, item)
            }).collect();

            if !obj.contains_key("PlayerLoadout") && !obj.contains_key("playerLoadout") {
                obj.insert("PlayerLoadout".into(), serde_json::json!(loadout_arr));
                changed = true;
            }
            if !obj.contains_key("LoadoutResponse") && !obj.contains_key("loadoutResponse") {
                obj.insert("LoadoutResponse".into(), serde_json::json!({
                    "PlayerLoadout": loadout_arr
                }));
                changed = true;
            }

            if let Some(res_obj) = obj.get_mut("Result").and_then(|r| r.as_object_mut()) {
                if !res_obj.contains_key("PlayerLoadout") && !res_obj.contains_key("playerLoadout") {
                    res_obj.insert("PlayerLoadout".into(), serde_json::json!(loadout_arr));
                    changed = true;
                }
                if !res_obj.contains_key("LoadoutResponse") && !res_obj.contains_key("loadoutResponse") {
                    res_obj.insert("LoadoutResponse".into(), serde_json::json!({
                        "PlayerLoadout": loadout_arr
                    }));
                    changed = true;
                }
            }
        }
    }

    if !changed {
        return (body.to_vec(), false);
    }

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn hash_title_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn patch_inventory_array(
    arr: &mut Vec<serde_json::Value>,
    inventory_spoof: &crate::psynet::InventorySpoofPayload,
) -> bool {
    let mut changed = false;
    let use_string_id = arr
        .iter()
        .find_map(|p| p.get("InstanceID").or_else(|| p.get("InstanceId")))
        .map(|v| v.is_string())
        .unwrap_or(true);

    let mut titles_to_spawn: Vec<String> = inventory_spoof.titles.clone();
    if let Some(cfg) = crate::psynet::load_active_spoof_from_disk() {
        let eq = cfg.equip_title_id.trim();
        if !eq.is_empty() && !titles_to_spawn.iter().any(|t| t.eq_ignore_ascii_case(eq)) {
            titles_to_spawn.push(eq.to_string());
        }
        if let Some(ref swaps) = cfg.swaps {
            for sw in swaps {
                let s1 = sw.equip_title_id.trim();
                if !s1.is_empty() && !titles_to_spawn.iter().any(|t| t.eq_ignore_ascii_case(s1)) {
                    titles_to_spawn.push(s1.to_string());
                }
                let s2 = sw.display_title_id.trim();
                if !s2.is_empty() && s2 != "custom" && !titles_to_spawn.iter().any(|t| t.eq_ignore_ascii_case(s2)) {
                    titles_to_spawn.push(s2.to_string());
                }
            }
        }
    }

    for tid in &titles_to_spawn {
        let tid_trim = tid.trim();
        if tid_trim.is_empty() {
            continue;
        }

        let already_has = arr.iter().any(|p| {
            let is_title_prod = p.get("ProductID")
                .or_else(|| p.get("productId"))
                .and_then(|v| v.as_i64())
                == Some(3036);
            if !is_title_prod {
                return false;
            }
            if let Some(attrs) = p.get("Attributes").or_else(|| p.get("attributes")).and_then(|a| a.as_array()) {
                attrs.iter().any(|attr| {
                    let k = attr.get("Key").or_else(|| attr.get("key")).and_then(|v| v.as_str()).unwrap_or("");
                    let v = attr.get("Value").or_else(|| attr.get("value")).and_then(|v| v.as_str()).unwrap_or("");
                    (k.eq_ignore_ascii_case("Title")
                        || k.eq_ignore_ascii_case("TitleId")
                        || k.eq_ignore_ascii_case("TitleID")
                        || k.eq_ignore_ascii_case("PlayerTitle")
                        || k.eq_ignore_ascii_case("PlayerTitleId")
                        || k.eq_ignore_ascii_case("Name"))
                        && v.eq_ignore_ascii_case(tid_trim)
                })
            } else {
                false
            }
        });

        if !already_has {
            let h = hash_title_str(tid_trim);
            let instance_id_num = 998_500_000i64 + ((h % 500_000) as i64);
            let instance_id_str = instance_id_num.to_string();
            let instance_val = if use_string_id {
                serde_json::json!(instance_id_str)
            } else {
                serde_json::json!(instance_id_num)
            };

            arr.push(serde_json::json!({
                "ProductID": 3036,
                "productId": 3036,
                "InstanceID": instance_val.clone(),
                "ProductInstanceID": instance_val,
                "SeriesID": 0,
                "Attributes": [
                    { "Key": "Title", "Value": tid_trim },
                    { "Key": "TitleId", "Value": tid_trim },
                    { "Key": "TitleID", "Value": tid_trim },
                    { "Key": "PlayerTitle", "Value": tid_trim },
                    { "Key": "PlayerTitleId", "Value": tid_trim },
                    { "Key": "Name", "Value": tid_trim }
                ],
                "AddedTimestamp": 1755399374i64,
                "UpdatedTimestamp": 1755399374i64,
                "TradeHold": -2,
            }));
            changed = true;
        }
    }

    for item in &inventory_spoof.items {
        if item.product_id <= 0 {
            continue;
        }

        let hash_seed = ((item.product_id as u64) << 16) | ((item.paint_id as u64) & 0xFFFF);
        let instance_id_num = 998_000_000i64 + ((hash_seed % 1_000_000) as i64);
        let instance_val = if use_string_id {
            serde_json::json!(instance_id_num.to_string())
        } else {
            serde_json::json!(instance_id_num)
        };

        let mut attributes = Vec::new();
        if item.paint_id > 0 {
            attributes.push(serde_json::json!({
                "Key": "Painted",
                "Value": item.paint_id,
                "TypeName": "ProductAttribute_Painted_TA"
            }));
            attributes.push(serde_json::json!({
                "Key": "Paint",
                "Value": item.paint_id
            }));
        }

        let existing_entry = arr.iter_mut().find(|p| {
            let pid_match = p.get("ProductID")
                .or_else(|| p.get("productId"))
                .or_else(|| p.get("product_id"))
                .and_then(|id| {
                    if let Some(n) = id.as_i64() {
                        Some(n)
                    } else if let Some(s) = id.as_str() {
                        s.parse::<i64>().ok()
                    } else {
                        None
                    }
                })
                == Some(item.product_id as i64);

            if !pid_match {
                return false;
            }

            let p_paint = p.get("Paint")
                .or_else(|| p.get("paint"))
                .or_else(|| p.get("PaintID"))
                .or_else(|| p.get("paint_id"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| {
                    if let Some(attrs) = p.get("Attributes").or_else(|| p.get("attributes")).and_then(|a| a.as_array()) {
                        attrs.iter().find_map(|attr| {
                            let k = attr.get("Key").or_else(|| attr.get("key")).and_then(|v| v.as_str()).unwrap_or("");
                            if k.eq_ignore_ascii_case("painted") || k.eq_ignore_ascii_case("paint") {
                                attr.get("Value").or_else(|| attr.get("value")).and_then(|v| v.as_i64())
                            } else {
                                None
                            }
                        }).unwrap_or(0)
                    } else {
                        0
                    }
                });

            p_paint == (item.paint_id as i64)
        });

        if let Some(existing) = existing_entry {
            if let Some(obj) = existing.as_object_mut() {
                obj.insert("InstanceID".into(), instance_val.clone());
                obj.insert("ProductInstanceID".into(), instance_val);
                if item.paint_id > 0 {
                    obj.insert("Attributes".into(), serde_json::json!(attributes));
                    obj.insert("Paint".into(), serde_json::json!(item.paint_id));
                }
                changed = true;
            }
        } else {
            let now_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(1755399374i64);

            let mut new_prod = serde_json::json!({
                "ProductID": item.product_id,
                "productId": item.product_id,
                "InstanceID": instance_val.clone(),
                "ProductInstanceID": instance_val,
                "SeriesID": item.series_id,
                "Attributes": attributes,
                "AddedTimestamp": now_ts,
                "UpdatedTimestamp": now_ts,
                "TradeHold": -2,
            });
            if item.paint_id > 0 {
                if let Some(obj) = new_prod.as_object_mut() {
                    obj.insert("Paint".into(), serde_json::json!(item.paint_id));
                }
            }

            arr.push(new_prod);
            changed = true;
        }
    }

    // Always ensure base stock/legacy IDs exist so client never detects missing IDs or resets sync timestamp
    for &base_pid in &[1i64, 11560, 6215, 6219, 6222, 6232] {
        let has_base = arr.iter().any(|p| {
            p.get("ProductID")
                .or_else(|| p.get("productId"))
                .or_else(|| p.get("product_id"))
                .and_then(|id| id.as_i64().or_else(|| id.as_str().and_then(|s| s.parse::<i64>().ok())))
                == Some(base_pid)
        });
        if !has_base {
            let instance_id_num = 998_900_000i64 + base_pid;
            let instance_val = if use_string_id {
                serde_json::json!(instance_id_num.to_string())
            } else {
                serde_json::json!(instance_id_num)
            };
            arr.push(serde_json::json!({
                "ProductID": base_pid,
                "productId": base_pid,
                "InstanceID": instance_val.clone(),
                "ProductInstanceID": instance_val,
                "SeriesID": 0,
                "Attributes": [],
                "AddedTimestamp": 1755399374i64,
                "UpdatedTimestamp": 1755399374i64,
                "TradeHold": -2,
            }));
            changed = true;
        }
    }

    changed
}

fn patch_inventory_containers(
    val: &mut serde_json::Value,
    inventory_spoof: &crate::psynet::InventorySpoofPayload,
) -> bool {
    let mut changed = false;

    if let Some(obj) = val.as_object_mut() {
        let keys = [
            "ProductData", "productData", "Products", "products",
            "ProductList", "Inventory",
        ];

        let mut found_any_key = false;
        for k in &keys {
            if let Some(sub_val) = obj.get_mut(*k) {
                if let Some(arr) = sub_val.as_array_mut() {
                    found_any_key = true;
                    changed |= patch_inventory_array(arr, inventory_spoof);
                }
            }
        }

        for rk in &["Result", "result"] {
            if let Some(res_val) = obj.get_mut(*rk) {
                if res_val.is_object() || res_val.is_array() {
                    found_any_key = true;
                    changed |= patch_inventory_containers(res_val, inventory_spoof);
                }
            }
        }

        if !found_any_key {
            let is_inventory_response = obj.contains_key("PlayerID")
                || obj.contains_key("PlayerId")
                || obj.contains_key("AccountID");

            if is_inventory_response {
                let mut new_arr = Vec::new();
                patch_inventory_array(&mut new_arr, inventory_spoof);
                obj.insert("ProductData".into(), serde_json::json!(new_arr));
                changed = true;
            }
        }
    } else if let Some(arr) = val.as_array_mut() {
        let is_product_array = arr.iter().any(|elem| {
            elem.get("ProductID").is_some() || elem.get("InstanceID").is_some() || elem.get("productId").is_some()
        });

        if is_product_array {
            changed |= patch_inventory_array(arr, inventory_spoof);
        } else {
            for elem in arr.iter_mut() {
                changed |= patch_inventory_containers(elem, inventory_spoof);
            }
        }
    }

    changed
}

fn patch_cross_entitlement_json(
    body: &[u8],
    inventory_spoof: &crate::psynet::InventorySpoofPayload,
) -> (Vec<u8>, bool) {
    if !inventory_spoof.enabled && inventory_spoof.items.is_empty() {
        return (body.to_vec(), false);
    }
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let mut spawned_pids: Vec<i64> = vec![1i64, 11560, 6215, 6219, 6222, 6232];
    for it in &inventory_spoof.items {
        if it.product_id > 0 && !spawned_pids.contains(&(it.product_id as i64)) {
            spawned_pids.push(it.product_id as i64);
        }
    }

    fn append_pids_to_array(arr: &mut Vec<serde_json::Value>, pids: &[i64]) -> bool {
        let mut modified = false;
        let is_object_array = arr.iter().any(|v| v.is_object());
        if is_object_array {
            for &pid in pids {
                let exists = arr.iter().any(|elem| {
                    elem.get("ProductID")
                        .or_else(|| elem.get("productId"))
                        .or_else(|| elem.get("ID"))
                        .or_else(|| elem.get("id"))
                        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
                        == Some(pid)
                });
                if !exists {
                    arr.push(serde_json::json!({
                        "ProductID": pid,
                        "productId": pid,
                        "Entitled": true,
                    }));
                    modified = true;
                }
            }
        } else {
            let has_string_ids = arr.iter().any(|v| v.is_string());
            for &pid in pids {
                let exists = arr.iter().any(|elem| {
                    if let Some(n) = elem.as_i64() {
                        n == pid
                    } else if let Some(s) = elem.as_str() {
                        s.parse::<i64>().ok() == Some(pid)
                    } else {
                        false
                    }
                });
                if !exists {
                    if has_string_ids {
                        arr.push(serde_json::json!(pid.to_string()));
                    } else {
                        arr.push(serde_json::json!(pid));
                    }
                    modified = true;
                }
            }
        }
        modified
    }

    fn search_and_append_cross(val: &mut serde_json::Value, pids: &[i64]) -> bool {
        let mut modded = false;
        if let Some(arr) = val.as_array_mut() {
            modded |= append_pids_to_array(arr, pids);
        } else if let Some(obj) = val.as_object_mut() {
            let target_keys = [
                "ProductIDs", "productIds", "ProductIds", "productIDs",
                "Entitlements", "entitlements", "EntitledProducts", "entitledProducts",
                "Products", "products", "CrossEntitlements", "crossEntitlements",
                "Result", "result",
            ];
            let mut found = false;
            for k in &target_keys {
                if let Some(sub) = obj.get_mut(*k) {
                    if sub.is_array() {
                        found = true;
                        modded |= search_and_append_cross(sub, pids);
                    } else if sub.is_object() {
                        modded |= search_and_append_cross(sub, pids);
                    }
                }
            }
            if !found && !obj.is_empty() {
                let mut new_arr = Vec::new();
                append_pids_to_array(&mut new_arr, pids);
                obj.insert("ProductIDs".into(), serde_json::json!(new_arr));
                modded = true;
            }
        }
        modded
    }

    let changed = search_and_append_cross(&mut root, &spawned_pids);

    if !changed {
        return (body.to_vec(), false);
    }

    let out = serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec());
    (out, true)
}

fn patch_player_inventory_json(
    body: &[u8],
    inventory_spoof: &crate::psynet::InventorySpoofPayload,
) -> (Vec<u8>, bool) {
    let has_titles = !inventory_spoof.titles.is_empty()
        || crate::psynet::load_active_spoof_from_disk().map(|c| !c.equip_title_id.trim().is_empty()).unwrap_or(false);
    if !inventory_spoof.enabled || (inventory_spoof.items.is_empty() && !has_titles) {
        return (body.to_vec(), false);
    }
    let mut root: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (body.to_vec(), false),
    };

    let changed = patch_inventory_containers(&mut root, inventory_spoof);

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
            if let Some(res_obj) = obj.get_mut("Result").and_then(|r| r.as_object_mut()) {
                res_obj.insert("Currencies".to_string(), serde_json::Value::Array(list.clone()));
            }
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

    crate::applog::record_traffic_event(
        "CONFIG-HTTP",
        "CLIENT->SRV",
        &format!("{method} {upstream_url}"),
        None,
        false,
        None,
        None,
        &req_body_bytes,
        None,
    );

    let mut up_builder = client.request(method.clone(), &upstream_url);
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
    if let Some(cfg) = &cfg_opt {
        if crate::features::is_build_supported() {
            let (next_body, changed) = patch_config(&out_body, cfg);
            if changed {
                out_body = next_body;
                patched = true;
                crate::applog::event(&format!(
                    "proxy: patched config ({} bytes)",
                    out_body.len()
                ));
            }
        } else if let Some(next) = patch_psynet_url(&out_body) {
            out_body = next;
            patched = true;
            crate::applog::event("proxy: patched PsyNetUrl to broker (unsupported build fallback)");
        }
    } else {
        if let Some(next) = patch_psynet_url(&out_body) {
            out_body = next;
            patched = true;
            crate::applog::event("proxy: patched PsyNetUrl to broker (default config)");
        }
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

    resp_builder = resp_builder.header("Cache-Control", "no-cache, no-store, must-revalidate");
    resp_builder = resp_builder.header("Pragma", "no-cache");
    resp_builder = resp_builder.header("Expires", "0");

    let sig = resign_config_cdn(&out_body);
    resp_builder = resp_builder.header("Psysignature", &sig);
    resp_builder = resp_builder.header("PsySig", &sig);

    resp_builder = resp_builder.header("Content-Length", out_body.len().to_string());
    crate::applog::record_traffic_event(
        "CONFIG-HTTP",
        "SRV->CLIENT",
        &format!("{method} {path}{query}"),
        Some(&status.to_string()),
        patched,
        None,
        None,
        &out_body,
        None,
    );

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

pub fn patch_config(body: &[u8], cfg: &crate::psynet::SpoofPayload) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let mut any_change = false;

    if cfg.enabled {
        let equip_id = if !cfg.equip_title_id.trim().is_empty() {
            cfg.equip_title_id.trim()
        } else {
            "Team_Iraq_World_Cup_2026"
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
                if !sw_text.is_empty() {
                    if let Some(next) = replace_equip_text(&out, target_id, sw_text) {
                        out = next;
                        any_change = true;
                    }
                } else if !sw.display_title_id.trim().is_empty()
                    && sw.display_title_id.trim() != target_id
                    && sw.display_title_id.trim() != "custom"
                {
                    let disp = sw.display_title_id.trim();
                    let src_pat = format!("\"ID\":\"{disp}\"");
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
                                    if !text_val.is_empty() {
                                        if let Some(next) = replace_equip_text(&out, target_id, &text_val) {
                                            out = next;
                                            any_change = true;
                                        }
                                    }
                                }
                            }
                        }
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

    let (next, changed) = upsert_class_property_override(
        &out,
        "FirstTimeExperienceManager_TA",
        "bEnabled",
        "false",
    );
    if changed {
        out = next;
        any_change = true;
    }

    // AuthPlayer and game RPC flow through 127.0.0.1 broker, eliminating external TLS/pinning issues)
    if let Some(next) = patch_psynet_url(&out) {
        out = next;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        BROKER_PORT.store(27505, Ordering::SeqCst);
        let input = br#"{"PsyNetUrl":{"Class":"PsyNetUrl_X","URL":"https://api.rlpp.psynet.gg/Services","URLv2":"https://api.rlpp.psynet.gg/rpc"}}"#;
        let patched = patch_psynet_url(input).expect("should patch PsyNetUrl");
        let s = String::from_utf8(patched).unwrap();
        assert!(s.contains("\"URL\":\"http://127.0.0.1:27505/Services\""));
        assert!(s.contains("\"URLv2\":\"http://127.0.0.1:27505/rpc\""));
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

        for tid in 14..=26 {
            let c = currencies.iter().find(|c| c["ID"] == tid);
            assert!(c.is_some(), "Currency ID {tid} must be present for season compatibility");
            assert_eq!(c.unwrap()["Amount"], 500000);
        }
    }

    #[test]
    fn test_patch_loadout_rpc_array() {
        let body = br#"{"Result":{"PlayerLoadout":[{"Slot":0,"ProductID":23},{"Slot":2,"ProductID":1565}]}}"#;
        let inv = crate::psynet::InventorySpoofPayload {
            enabled: true,
            items: vec![
                crate::psynet::InventorySpoofItemPayload {
                    product_id: 4284,
                    paint_id: 3,
                    series_id: 0,
                    slot: "Body".to_string(),
                    product_name: "Fennec".to_string(),
                    dlc: false,
                    ..Default::default()
                },
                crate::psynet::InventorySpoofItemPayload {
                    product_id: 45,
                    paint_id: 0,
                    series_id: 0,
                    slot: "Boost".to_string(),
                    product_name: "Gold Rush (Alpha Boost)".to_string(),
                    dlc: false,
                    ..Default::default()
                },
            ],
            titles: vec![],
        };

        let (patched, changed) = patch_loadout_rpc_json(body, &inv, false);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let loadout = val["Result"]["PlayerLoadout"].as_array().unwrap();

        let body_slot = loadout.iter().find(|e| e["Slot"] == 0).unwrap();
        assert_eq!(body_slot["ProductID"], 4284);
        assert_eq!(body_slot["Paint"], 3);

        let wheel_slot = loadout.iter().find(|e| e["Slot"] == 2).unwrap();
        assert_eq!(wheel_slot["ProductID"], 1565);

        let boost_slot = loadout.iter().find(|e| e["Slot"] == 3).unwrap();
        assert_eq!(boost_slot["ProductID"], 45);
    }

    #[test]
    fn test_patch_loadout_rpc_object() {
        let body = br#"{"PlayerLoadout":{"Body":23,"Wheels":1565}}"#;
        let inv = crate::psynet::InventorySpoofPayload {
            enabled: true,
            items: vec![crate::psynet::InventorySpoofItemPayload {
                product_id: 4284,
                paint_id: 1,
                series_id: 0,
                slot: "Body".to_string(),
                product_name: "Fennec".to_string(),
                dlc: false,
                ..Default::default()
            }],
            titles: vec![],
        };

        let (patched, changed) = patch_loadout_rpc_json(body, &inv, false);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        assert_eq!(val["PlayerLoadout"]["Body"], 4284);
        assert_eq!(val["PlayerLoadout"]["Wheels"], 1565);
    }

    #[test]
    fn test_patch_loadout_rpc_authplayer() {
        let body = br#"{"SessionID":"7196cb8e","PsyToken":"token123"}"#;
        let inv = crate::psynet::InventorySpoofPayload {
            enabled: true,
            items: vec![crate::psynet::InventorySpoofItemPayload {
                product_id: 4284,
                paint_id: 3,
                series_id: 0,
                slot: "Body".to_string(),
                product_name: "Fennec".to_string(),
                dlc: false,
                ..Default::default()
            }],
            titles: vec![],
        };

        let (patched, changed) = patch_loadout_rpc_json(body, &inv, true);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        assert!(val.get("PlayerLoadout").is_some());
        assert!(val.get("LoadoutResponse").is_some());
        let pl = val["PlayerLoadout"].as_array().unwrap();
        let body_slot = pl.iter().find(|e| e["Slot"] == 0).unwrap();
        assert_eq!(body_slot["ProductID"], 4284);
        assert_eq!(body_slot["Paint"], 3);
    }

    #[test]
    fn test_patch_player_inventory_product_data() {
        let body = br#"{"Result":{"ProductData":[{"ProductID":2363,"InstanceID":"123456","Attributes":[],"SeriesID":19}]}}"#;
        let inv = crate::psynet::InventorySpoofPayload {
            enabled: true,
            items: vec![crate::psynet::InventorySpoofItemPayload {
                product_id: 4284,
                paint_id: 12,
                series_id: 0,
                slot: "Body".to_string(),
                product_name: "Fennec".to_string(),
                dlc: false,
                ..Default::default()
            }],
            titles: vec![],
        };

        let (patched, changed) = patch_player_inventory_json(body, &inv);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let prod_arr = val["Result"]["ProductData"].as_array().unwrap();
        assert!(prod_arr.len() >= 2);
        let fennec = prod_arr.iter().find(|p| p["ProductID"] == 4284).unwrap();
        assert!(fennec["InstanceID"].is_string());
        let attrs = fennec["Attributes"].as_array().unwrap();
        assert!(attrs.iter().any(|a| a["Key"] == "Painted" && a["Value"] == 12));
    }

    #[test]
    fn test_patch_player_inventory_empty_result() {
        let body = br#"{"Result":{"ProductData":[]}}"#;
        let inv = crate::psynet::InventorySpoofPayload {
            enabled: true,
            items: vec![crate::psynet::InventorySpoofItemPayload {
                product_id: 4284,
                paint_id: 0,
                series_id: 0,
                slot: "Body".to_string(),
                product_name: "Fennec".to_string(),
                dlc: false,
                ..Default::default()
            }],
            titles: vec![],
        };

        let (patched, changed) = patch_player_inventory_json(body, &inv);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let prod_arr = val["Result"]["ProductData"].as_array().unwrap();
        assert!(prod_arr.len() >= 1);
        assert!(prod_arr.iter().any(|p| p["ProductID"] == 4284));
    }

    #[test]
    fn test_patch_cross_entitlement_appends_without_wiping() {
        let body = br#"{"Result":{"ProductIDs":[1, 11560, 6219]}}"#;
        let inv = crate::psynet::InventorySpoofPayload {
            enabled: true,
            items: vec![crate::psynet::InventorySpoofItemPayload {
                product_id: 4284,
                paint_id: 0,
                series_id: 0,
                slot: "Body".to_string(),
                product_name: "Fennec".to_string(),
                dlc: false,
                ..Default::default()
            }],
            titles: vec![],
        };

        let (patched, changed) = patch_cross_entitlement_json(body, &inv);
        assert!(changed);
        let val: serde_json::Value = serde_json::from_slice(&patched).unwrap();
        let ids: Vec<i64> = val["Result"]["ProductIDs"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        // Original IDs must be 100% preserved
        assert!(ids.contains(&1));
        assert!(ids.contains(&11560));
        assert!(ids.contains(&6219));
        // Spawned ID must be appended
        assert!(ids.contains(&4284));
    }
}


