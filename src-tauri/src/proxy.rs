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

pub fn is_intercept_target(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h.contains("epicgames.dev") || h.contains("psyonix.com") || h.contains("live.psynet.gg")
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
        // Social/presence/party services — contain friends' data, must never be patched
        "social/",
        "presence/",
        "party/",
        "friends/",
        "richpresence",
        "beacon",
        "roster",
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
            "function FindProxyForURL(url, host) {{\n    \
             if (shExpMatch(host, \"*.epicgames.dev\") || host == \"api.epicgames.dev\" || shExpMatch(host, \"*account-public-service*\")) {{\n        \
                 return \"PROXY 127.0.0.1:{SYSTEM_PROXY_PORT}; DIRECT\";\n    \
             }}\n    \
             return \"DIRECT\";\n\
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
        let forward_client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
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

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .redirect(reqwest::redirect::Policy::none())
        .resolve("config.psynet.gg", "34.160.180.65:443".parse().unwrap())
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
                    async move {
                        handle_request(req, client).await
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
    let method = req.method().clone();
    let uri = req.uri();
    let path = uri.path();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
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
            }
        }

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

    // Try standard port 27505 first (matches Go proxy architecture), fallback to ephemeral if occupied.
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

    let path_lower = path_and_query.to_ascii_lowercase();
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
    } else {
        let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
            Some(c) => Some(c),
            None => get_spoof_config().await,
        };
        if let Some(cfg) = &cfg_opt {
            if path_lower.contains("getplayerwallet") {
                if let Some(cs) = &cfg.credit_spoof {
                    if cs.enabled {
                        let (new_body, did_patch) = patch_player_wallet_json(&out_body, cs);
                        if did_patch {
                            out_body = new_body;
                            patched = true;
                            crate::applog::event(&format!(
                                "broker: patched wallet credits in RPC response -> {}",
                                cs.amount
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
    }

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
        .to_ascii_lowercase();

    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    if host_hdr.contains("ws.rlpp.psynet.gg") || is_upgrade {
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
                    if up_tx.send(msg).await.is_err() {
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

    let conn_id = get_ws_header_value(headers_part, "PsyConnectionID");
    if !conn_id.is_empty() {
        set_learned_player_id(&conn_id);
    }

    let svc = get_ws_header_value(headers_part, "PsyService");
    let is_skill = svc.contains("skills/getplayerskill")
        || svc.contains("skills/getplayersskills")
        || (find_bytes(body_part, b"\"Skills\"").is_some()
            && (find_bytes(body_part, b"\"Mu\"").is_some()
                || find_bytes(body_part, b"\"Tier\"").is_some()
                || find_bytes(body_part, b"\"Playlist\"").is_some()));

    if is_skill {
        extract_and_save_real_skills(body_part);
    }

    let cfg_opt = match crate::psynet::load_active_spoof_from_disk() {
        Some(c) => Some(c),
        None => get_spoof_config().await,
    };

    let Some(cfg) = cfg_opt else {
        return (frame.to_vec(), false);
    };

    let mut current_body = body_part.to_vec();
    let mut any_changed = false;

    let is_wallet = svc.contains("shops/getplayerwallet")
        || svc.contains("getplayerwallet")
        || (find_bytes(body_part, b"\"Currencies\"").is_some() && find_bytes(body_part, b"\"IsTradable\"").is_some());

    if is_wallet {
        if let Some(cs) = &cfg.credit_spoof {
            if cs.enabled {
                let (new_body, changed) = patch_player_wallet_json(&current_body, cs);
                if changed {
                    current_body = new_body;
                    any_changed = true;
                    crate::applog::event(&format!(
                        "proxy: patched wallet credits -> {} ({} -> {} bytes)",
                        cs.amount,
                        frame.len(),
                        current_body.len()
                    ));
                }
            }
        }
    }

    let is_leaderboard = svc.contains("getleaderboard")
        || svc.contains("leaderboard")
        || is_leaderboard_body(body_part);

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
        if ns.enabled && !cfg.is_steam && !ns.display_name.trim().is_empty() {
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
        return (frame.to_vec(), false);
    }

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

    if !has_psysig && !has_psysignature {
        return headers.to_vec();
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
    replace_ws_header_value(headers, key_to_replace, &sig)
}

fn is_leaderboard_body(body: &[u8]) -> bool {
    let trim = body.trim_ascii();
    find_bytes(trim, b"\"LeaderboardID\"").is_some()
        || find_bytes(trim, b"\"LeaderboardRows\"").is_some()
        || (find_bytes(trim, b"\"Rows\"").is_some() && (find_bytes(trim, b"\"Rank\"").is_some() || find_bytes(trim, b"\"Value\"").is_some()))
        || (find_bytes(trim, b"\"Entries\"").is_some() && (find_bytes(trim, b"\"Rank\"").is_some() || find_bytes(trim, b"\"Value\"").is_some()))
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
    } else {
        &mut root
    };

    let mut pl = 0;
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

    let mut target_display_mmr = 3000.0;
    let mut target_tier = 22; // Supersonic Legend default
    let mut custom_rank: Option<i64> = None;
    let mut found_override = false;

    if let Some(lb) = lb_spoof_opt {
        if let Some(cr) = lb.custom_rank {
            custom_rank = Some(cr as i64);
            found_override = true;
        }
        if !lb.sync_from_fake_ranks {
            if let Some(cm) = lb.custom_mmr {
                target_display_mmr = cm as f64;
                found_override = true;
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

    // Calculate auto rank if custom rank is not explicitly set
    let assigned_rank = if let Some(cr) = custom_rank {
        cr
    } else {
        // Inspect Rows/Entries in leaderboard to find where target_display_mmr ranks
        let rows_opt = result_obj.get("Rows").and_then(|r| r.as_array())
            .or_else(|| result_obj.get("Entries").and_then(|r| r.as_array()))
            .or_else(|| result_obj.get("LeaderboardRows").and_then(|r| r.as_array()));

        if let Some(rows) = rows_opt {
            let mut computed_rank = 1;
            let mut found_slot = false;
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
        } else {
            // GetLeaderboardValue single-player response
            if target_display_mmr >= 1900.0 || target_tier >= 22 {
                1
            } else if target_display_mmr >= 1500.0 {
                50
            } else {
                1
            }
        }
    };

    // Patch top-level / Result fields
    result_obj["MMR"] = serde_json::json!(target_mu);
    result_obj["Value"] = serde_json::json!(target_tier);
    result_obj["bHasSkill"] = serde_json::json!(true);
    result_obj["Rank"] = serde_json::json!(assigned_rank);
    result_obj["UserRank"] = serde_json::json!(assigned_rank);
    result_obj["RankValue"] = serde_json::json!(assigned_rank);
    result_obj["Position"] = serde_json::json!(assigned_rank);
    result_obj["UserPosition"] = serde_json::json!(assigned_rank);
    let mut changed = true;

    // Patch user row if present (UserRow, PlayerRow, SelfRow, UserEntry)
    for key in &["UserRow", "PlayerRow", "SelfRow", "UserEntry"] {
        if let Some(user_row) = result_obj.get_mut(*key).and_then(|v| v.as_object_mut()) {
            user_row.insert("Rank".into(), serde_json::json!(assigned_rank));
            user_row.insert("UserRank".into(), serde_json::json!(assigned_rank));
            user_row.insert("Value".into(), serde_json::json!(target_display_mmr.round() as i64));
            user_row.insert("Tier".into(), serde_json::json!(target_tier));
            user_row.insert("MMR".into(), serde_json::json!(target_mu));
            user_row.insert("bHasSkill".into(), serde_json::json!(true));
            changed = true;
        }
    }

    // Build user row object
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

    let user_row_obj = serde_json::json!({
        "PlayerID": user_pid_str,
        "PlayerName": player_name,
        "Rank": assigned_rank,
        "UserRank": assigned_rank,
        "Value": target_display_mmr.round() as i64,
        "Tier": target_tier,
        "MMR": target_mu,
        "bHasSkill": true
    });

    // Check if user is in Rows / Entries array and update/insert their entry
    let clean_pid = user_pid_str.trim_start_matches("Epic|").trim_start_matches("Steam|").trim_start_matches("Xbox|").trim_start_matches("PS4|").trim_end_matches("|0");
    for key in &["Rows", "Entries", "LeaderboardRows"] {
        if let Some(rows_arr) = result_obj.get_mut(*key).and_then(|r| r.as_array_mut()) {
            if !rows_arr.is_empty() {
                // Remove existing user entry if already in the list
                rows_arr.retain(|row| {
                    let r_pid = row.get("PlayerID").and_then(|v| v.as_str()).unwrap_or("");
                    !(r_pid.contains(clean_pid) || clean_pid.contains(r_pid))
                });

                // Insert user at computed rank position (e.g. index 0 for Rank 1)
                let insert_idx = ((assigned_rank - 1).max(0) as usize).min(rows_arr.len());
                rows_arr.insert(insert_idx, user_row_obj.clone());

                // Re-index ranks for the rows so they are monotonically ordered 1, 2, 3...
                for (idx, row) in rows_arr.iter_mut().enumerate() {
                    row["Rank"] = serde_json::json!((idx + 1) as i64);
                }
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

    let result_obj = if let Some(r) = root.get_mut("Result") {
        r
    } else {
        &mut root
    };

    let mut changed = false;
    if let Some(currencies) = result_obj.get_mut("Currencies").and_then(|c| c.as_array_mut()) {
        let mut found_credits = false;
        let mut found_tournament = false;
        for curr in currencies.iter_mut() {
            if let Some(id) = curr.get("ID").and_then(|v| v.as_i64()) {
                if id == 13 {
                    curr["Amount"] = serde_json::json!(credit_spoof.amount);
                    found_credits = true;
                    changed = true;
                } else if id == 15 || id == 14 {
                    curr["Amount"] = serde_json::json!(credit_spoof.tournament_amount);
                    found_tournament = true;
                    changed = true;
                }
            }
        }
        if !found_credits {
            currencies.push(serde_json::json!({
                "ID": 13,
                "Amount": credit_spoof.amount,
                "ExpirationTime": null,
                "UpdatedTimestamp": 1700000000,
                "IsTradable": true,
                "TradeHold": null
            }));
            changed = true;
        }
        if !found_tournament {
            currencies.push(serde_json::json!({
                "ID": 15,
                "Amount": credit_spoof.tournament_amount,
                "ExpirationTime": null,
                "UpdatedTimestamp": 1700000000,
                "IsTradable": false,
                "TradeHold": null
            }));
            changed = true;
        }
    } else if result_obj.is_object() {
        result_obj["Currencies"] = serde_json::json!([
            {
                "ID": 13,
                "Amount": credit_spoof.amount,
                "ExpirationTime": null,
                "UpdatedTimestamp": 1700000000,
                "IsTradable": true,
                "TradeHold": null
            },
            {
                "ID": 15,
                "Amount": credit_spoof.tournament_amount,
                "ExpirationTime": null,
                "UpdatedTimestamp": 1700000000,
                "IsTradable": false,
                "TradeHold": null
            }
        ]);
        changed = true;
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
    let uri = req.uri();
    let path = uri.path();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("https://config.psynet.gg{path}{query}");

    if let Some(bid) = parse_build_id(path) {
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

    // Prevent client-side caching of config responses
    resp_builder = resp_builder.header("Cache-Control", "no-cache, no-store, must-revalidate");
    resp_builder = resp_builder.header("Pragma", "no-cache");
    resp_builder = resp_builder.header("Expires", "0");

    // Always provide both Psysignature and PsySig (signed with PSY_CDN_KEY) so Rocket League always accepts the config
    let sig = resign_config_cdn(&out_body);
    resp_builder = resp_builder.header("Psysignature", &sig);
    resp_builder = resp_builder.header("PsySig", &sig);

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

fn expand_rank_placeholders(text: &str) -> String {
    text.replace("{Legend}", "Supersonic Legend")
        .replace("{legend}", "Supersonic Legend")
        .replace("{GrandChampion}", "Grand Champion")
        .replace("{grandchampion}", "Grand Champion")
        .replace("{Champion}", "Champion")
        .replace("{champion}", "Champion")
        .replace("{Diamond}", "Diamond")
        .replace("{diamond}", "Diamond")
        .replace("{Platinum}", "Platinum")
        .replace("{platinum}", "Platinum")
        .replace("{Gold}", "Gold")
        .replace("{gold}", "Gold")
        .replace("{Silver}", "Silver")
        .replace("{silver}", "Silver")
        .replace("{Bronze}", "Bronze")
        .replace("{bronze}", "Bronze")
}

fn replace_equip_text(body: &[u8], equip_id: &str, new_text: &str) -> Option<Vec<u8>> {
    let expanded = expand_rank_placeholders(new_text);
    let encoded_json = serde_json::to_string(&expanded).ok()?;
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

pub fn patch_config(body: &[u8], cfg: &crate::psynet::SpoofPayload) -> (Vec<u8>, bool) {
    let mut out = body.to_vec();
    let mut any_change = false;

    if cfg.enabled {
        let equip_id = cfg.equip_title_id.trim();
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

    // Always rewrite PsyNetUrl to local broker (matches Go proxy architecture:
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
}

