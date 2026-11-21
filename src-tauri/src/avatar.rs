/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProfileConfig {
    pub player_name: String,
    pub title_text: String,
    pub title_color: String,
    pub title_glow: String,
    pub banner_id: String,
    pub banner_url: String,
    pub border_id: String,
    pub border_url: String,
    pub avatar_url: String,
    #[serde(default)]
    pub xp_level: Option<i32>,
}

impl Default for CustomProfileConfig {
    fn default() -> Self {
        Self {
            player_name: "Player".into(),
            title_text: "EST. 2015".into(),
            title_color: "#FFD700".into(),
            title_glow: "#FFB300".into(),
            banner_id: "circuitboard".into(),
            banner_url: "https://api.velocityrl.tech/thumbnails/circuitboard_full.png".into(),
            border_id: "flames".into(),
            border_url: "assets/borders/flames.png".into(),
            avatar_url: "https://rlstats.net/images/epic.png".into(),
            xp_level: None,
        }
    }
}

fn profile_card_config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|p| p.join("profile_card.json"))
}

#[tauri::command]
pub fn get_profile_card_config(app: tauri::AppHandle) -> Result<CustomProfileConfig, String> {
    if let Some(path) = profile_card_config_path(&app) {
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str::<CustomProfileConfig>(&content) {
                    return Ok(cfg);
                }
            }
        }
    }
    Ok(CustomProfileConfig::default())
}

#[tauri::command]
pub fn save_profile_card_config(app: tauri::AppHandle, config: CustomProfileConfig) -> Result<(), String> {
    let path = profile_card_config_path(&app)
        .ok_or_else(|| "Failed to resolve app config directory".to_string())?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    fs::write(&path, data).map_err(|e| format!("Failed to save config: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn get_profile_catalog(_app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    // In dev and release, ui/assets/profile_catalog.json contains the catalog
    let paths_to_try = [
        PathBuf::from("ui/assets/profile_catalog.json"),
        PathBuf::from("../ui/assets/profile_catalog.json"),
        PathBuf::from("/root/VelocityRL/ui/assets/profile_catalog.json"),
    ];
    for p in &paths_to_try {
        if p.is_file() {
            if let Ok(content) = fs::read_to_string(p) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    return Ok(val);
                }
            }
        }
    }
    Err("profile_catalog.json not found".into())
}

static ACTIVE_AVATAR_PNG: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

pub fn set_active_avatar_png(bytes: Vec<u8>) {
    if let Ok(mut lock) = ACTIVE_AVATAR_PNG.lock() {
        *lock = Some(bytes);
    }
}

pub fn clear_active_avatar_png() {
    if let Ok(mut lock) = ACTIVE_AVATAR_PNG.lock() {
        *lock = None;
    }
}

pub fn get_active_avatar_png() -> Option<Vec<u8>> {
    if let Ok(guard) = ACTIVE_AVATAR_PNG.lock() {
        if let Some(ref bytes) = *guard {
            return Some(bytes.clone());
        }
    }
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from("/root/avatar.png"),
        PathBuf::from("custom_avatars/active_avatar.png"),
        PathBuf::from("/root/.config/com.velocityrl.app/custom_avatars/active_avatar.png"),
    ];
    let config_dir = crate::psynet::config_dir();
    candidates.push(config_dir.join("custom_avatars").join("active_avatar.png"));
    for p in &candidates {
        if p.is_file() {
            if let Ok(b) = fs::read(p) {
                return Some(b);
            }
        }
    }
    None
}

pub fn save_cached_avatar(app: &tauri::AppHandle, png_bytes: &[u8]) -> Option<PathBuf> {
    if let Ok(dir) = app.path().app_config_dir() {
        let avatar_dir = dir.join("custom_avatars");
        if fs::create_dir_all(&avatar_dir).is_ok() {
            let file_path = avatar_dir.join("active_avatar.png");
            if fs::write(&file_path, png_bytes).is_ok() {
                return Some(file_path);
            }
        }
    }
    None
}

#[tauri::command]
pub async fn fetch_image_from_url(app: tauri::AppHandle, url: String) -> Result<String, String> {
    let clean_url = url.trim();
    if clean_url.is_empty() {
        return Err("URL is empty".into());
    }

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())?;

    let mut direct_image_url = clean_url.to_string();

    // Support Steam profile links by querying Steam OpenAPI / miniprofile or fallback
    if clean_url.contains("steamcommunity.com") && !clean_url.ends_with(".png") && !clean_url.ends_with(".jpg") {
        let mut resolved_via_openapi = false;
        for part in clean_url.split('/') {
            if let Some(sid) = parse_steam_id(part) {
                let acc_id = (sid & 0xFFFF_FFFF) as u32;
                let mini_url = format!("https://steamcommunity.com/miniprofile/{acc_id}/json");
                    if let Ok(resp) = client.get(&mini_url).send().await {
                        if resp.status().is_success() {
                            if let Ok(profile) = resp.json::<SteamMiniProfileResponse>().await {
                                if let Some(url) = profile.avatar_url {
                                    if !url.trim().is_empty() {
                                        direct_image_url = url;
                                        resolved_via_openapi = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
            }
        }

        if !resolved_via_openapi {
            let xml_url = if clean_url.ends_with('/') {
                format!("{clean_url}?xml=1")
            } else {
                format!("{clean_url}/?xml=1")
            };
            if let Ok(resp) = client.get(&xml_url).send().await {
                if let Ok(text) = resp.text().await {
                    if let Some(pos) = text.find("<avatarFull><![CDATA[") {
                        let rest = &text[pos + 21..];
                        if let Some(end) = rest.find("]]></avatarFull>") {
                            direct_image_url = rest[..end].to_string();
                        }
                    } else if let Some(pos) = text.find("<avatarFull>") {
                        let rest = &text[pos + 12..];
                        if let Some(end) = rest.find("</avatarFull>") {
                            direct_image_url = rest[..end].to_string();
                        }
                    }
                }
            }
        }
    }

    let resp = client.get(&direct_image_url).send().await
        .map_err(|e| format!("Failed to fetch URL: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP error {}: {}", resp.status(), resp.status().canonical_reason().unwrap_or("Failed")));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("Failed to read image data: {e}"))?;

    // Validate with image crate
    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("Invalid image data: {e}"))?;

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode image to PNG: {e}"))?;

    // Cache downloaded avatar image to local filesystem
    let _ = save_cached_avatar(&app, &png_bytes);

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamAuthenticatedUser {
    pub steam_id: String,
    pub persona_name: String,
    pub avatar_url: String,
    pub avatar_data_url: String,
}

static AUTHENTICATED_STEAM_USER: std::sync::Mutex<Option<SteamAuthenticatedUser>> = std::sync::Mutex::new(None);

fn steam_user_path() -> PathBuf {
    crate::psynet::config_dir().join("steam_user.json")
}

pub fn set_authenticated_steam_user(user: SteamAuthenticatedUser) {
    if let Ok(raw) = serde_json::to_string(&user) {
        let path = steam_user_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, raw);
    }
    if let Ok(mut lock) = AUTHENTICATED_STEAM_USER.lock() {
        *lock = Some(user);
    }
}

/// Restore a linked Steam account from disk so login survives an app restart.
pub fn load_authenticated_steam_user_from_disk() -> Option<SteamAuthenticatedUser> {
    let raw = fs::read_to_string(steam_user_path()).ok()?;
    let user: SteamAuthenticatedUser = serde_json::from_str(&raw).ok()?;
    if let Ok(mut lock) = AUTHENTICATED_STEAM_USER.lock() {
        *lock = Some(user.clone());
    }
    Some(user)
}

pub fn get_authenticated_steam_user_internal() -> Option<SteamAuthenticatedUser> {
    AUTHENTICATED_STEAM_USER.lock().ok().and_then(|g| g.clone())
}

pub fn clear_authenticated_steam_user_internal() {
    let _ = fs::remove_file(steam_user_path());
    if let Ok(mut lock) = AUTHENTICATED_STEAM_USER.lock() {
        *lock = None;
    }
}

#[tauri::command]
pub fn get_authenticated_steam_user() -> Option<SteamAuthenticatedUser> {
    get_authenticated_steam_user_internal()
}

#[tauri::command]
pub fn clear_authenticated_steam_user(app: tauri::AppHandle) -> Result<(), String> {
    clear_authenticated_steam_user_internal();
    let _ = set_load_other_pfps_enabled(app.clone(), false);
    let _ = restore_custom_avatar_upk(app);
    Ok(())
}

fn load_other_pfps_config_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|p| p.join("load_other_pfps.json"))
}

pub fn is_load_other_pfps_enabled_internal() -> bool {
    let candidates = [
        PathBuf::from("/root/.config/com.velocityrl.app/load_other_pfps.json"),
        PathBuf::from("load_other_pfps.json"),
    ];
    for p in &candidates {
        if p.is_file() {
            if let Ok(c) = fs::read_to_string(p) {
                if let Ok(val) = serde_json::from_str::<bool>(&c) {
                    return val;
                }
            }
        }
    }
    false
}

#[tauri::command]
pub fn get_load_other_pfps(app: tauri::AppHandle) -> bool {
    if get_authenticated_steam_user_internal().is_none() {
        return false;
    }
    if let Some(p) = load_other_pfps_config_path(&app) {
        if p.is_file() {
            if let Ok(c) = fs::read_to_string(&p) {
                if let Ok(val) = serde_json::from_str::<bool>(&c) {
                    return val;
                }
            }
        }
    }
    is_load_other_pfps_enabled_internal()
}

#[tauri::command]
pub fn set_load_other_pfps_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled && get_authenticated_steam_user_internal().is_none() {
        return Err("Steam login required to enable loading other profile pictures".into());
    }
    if let Some(p) = load_other_pfps_config_path(&app) {
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&p, serde_json::to_string(&enabled).unwrap_or_default());
    }
    let global_p = PathBuf::from("/root/.config/com.velocityrl.app/load_other_pfps.json");
    if let Some(parent) = global_p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&global_p, serde_json::to_string(&enabled).unwrap_or_default());
    Ok(())
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

#[tauri::command]
pub fn get_steam_login_url() -> String {
    let port = crate::proxy::broker_port().unwrap_or(15759);
    format!(
        "https://steamcommunity.com/openid/login?\
        openid.ns=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0&\
        openid.mode=checkid_setup&\
        openid.return_to=http%3A%2F%2F127.0.0.1%3A{port}%2Frlim%2Fsteam_callback&\
        openid.realm=http%3A%2F%2F127.0.0.1%3A{port}%2F&\
        openid.identity=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0%2Fidentifier_select&\
        openid.claimed_id=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0%2Fidentifier_select"
    )
}

pub async fn verify_steam_openid_assertion(query: &str) -> Result<u64, String> {
    let mut params: Vec<(String, String)> = Vec::new();
    let mut claimed_id: Option<String> = None;

    for part in query.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            let key = percent_decode(k);
            let val = percent_decode(v);
            if key == "openid.claimed_id" {
                claimed_id = Some(val.clone());
            }
            if key == "openid.mode" {
                params.push(("openid.mode".into(), "check_authentication".into()));
            } else {
                params.push((key, val));
            }
        }
    }

    let claimed = claimed_id.ok_or_else(|| "Missing openid.claimed_id in callback".to_string())?;
    let raw_steam_id = claimed
        .strip_prefix("https://steamcommunity.com/openid/id/")
        .or_else(|| claimed.strip_prefix("http://steamcommunity.com/openid/id/"))
        .ok_or_else(|| format!("Invalid claimed_id format: {claimed}"))?;

    let steam_id = raw_steam_id
        .trim()
        .parse::<u64>()
        .map_err(|e| format!("Invalid steam_id integer: {e}"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let form_body = params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let resp = client
        .post("https://steamcommunity.com/openid/login")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .map_err(|e| format!("Failed to verify assertion with Steam: {e}"))?;

    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !text.contains("is_valid:true") {
        return Err("Steam OpenID assertion verification failed: response was not is_valid:true".into());
    }

    Ok(steam_id)
}

#[derive(Debug, Deserialize)]
struct VelocitySteamProfile {
    #[serde(alias = "personaName", alias = "personaname")]
    persona_name: Option<String>,
    #[serde(alias = "avatarUrl", alias = "avatar_url", alias = "avatarfull")]
    avatar_url: Option<String>,
}

pub async fn fetch_verified_steam_profile(steam_id: u64) -> Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    // 1. Primary: VelocityRL domain API (centralized server-side Steam key and cache)
    let backend_url = format!("https://api.velocityrl.tech/v1/steam/user/{steam_id}");
    if let Ok(resp) = client.get(&backend_url).send().await {
        if resp.status().is_success() {
            if let Ok(prof) = resp.json::<VelocitySteamProfile>().await {
                if let (Some(p), Some(a)) = (prof.persona_name, prof.avatar_url) {
                    if !p.trim().is_empty() && !a.trim().is_empty() {
                        return Ok((p, a));
                    }
                }
            }
        }
    }

    // 2. Backup: Steam Community XML
    let xml_url = format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1");
    if let Ok(resp) = client.get(&xml_url).send().await {
        if let Ok(text) = resp.text().await {
            let mut persona: Option<String> = None;
            let mut avatar: Option<String> = None;

            if let Some(pos) = text.find("<steamID><![CDATA[") {
                let rest = &text[pos + 18..];
                if let Some(end) = rest.find("]]></steamID>") {
                    persona = Some(rest[..end].to_string());
                }
            } else if let Some(pos) = text.find("<steamID>") {
                let rest = &text[pos + 9..];
                if let Some(end) = rest.find("</steamID>") {
                    persona = Some(rest[..end].to_string());
                }
            }

            if let Some(pos) = text.find("<avatarFull><![CDATA[") {
                let rest = &text[pos + 21..];
                if let Some(end) = rest.find("]]></avatarFull>") {
                    avatar = Some(rest[..end].to_string());
                }
            } else if let Some(pos) = text.find("<avatarFull>") {
                let rest = &text[pos + 12..];
                if let Some(end) = rest.find("</avatarFull>") {
                    avatar = Some(rest[..end].to_string());
                }
            }

            if let (Some(p), Some(a)) = (persona, avatar) {
                if !p.is_empty() && !a.is_empty() {
                    return Ok((p, a));
                }
            }
        }
    }

    // 3. Fallback: Steam miniprofile JSON
    let account_id = (steam_id & 0xFFFF_FFFF) as u32;
    if account_id > 0 {
        let mini_url = format!("https://steamcommunity.com/miniprofile/{account_id}/json");
        if let Ok(resp) = client.get(&mini_url).send().await {
            if resp.status().is_success() {
                if let Ok(profile) = resp.json::<SteamMiniProfileResponse>().await {
                    let persona = profile.persona_name.unwrap_or_else(|| steam_id.to_string());
                    if let Some(av) = profile.avatar_url {
                        if !av.trim().is_empty() {
                            return Ok((persona, av));
                        }
                    }
                }
            }
        }
    }

    Err(format!("Could not resolve Steam profile for ID {steam_id}"))
}

static STEAM_AVATAR_CACHE: std::sync::Mutex<Option<std::collections::HashMap<u64, Vec<u8>>>> =
    std::sync::Mutex::new(None);

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SteamMiniProfileResponse {
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub persona_name: Option<String>,
    #[serde(default)]
    pub level: Option<u32>,
}

pub const STEAMID64_INDIVIDUAL_BASE: u64 = 76561197960265728;

pub fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(h) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(h as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Parses raw numeric 64-bit SteamIDs, 32-bit Account IDs, Steam3 format ([U:1:Z]),
/// or legacy Steam2 format (STEAM_0:X:Y / STEAM_1:X:Y) into a validated 64-bit Steam ID.
pub fn parse_steam_id(raw: &str) -> Option<u64> {
    let decoded = percent_decode(raw.trim());
    let mut s = decoded.trim();
    // Rocket League PlayerIDString values are sometimes prefixed with the
    // platform name (e.g. "steam:7656...", "steam|7656...").
    let lower = s.to_ascii_lowercase();
    for prefix in ["steam:", "steam|", "steamid:", "steamid64:"] {
        if lower.starts_with(prefix) {
            s = s[prefix.len()..].trim();
            break;
        }
    }
    // RL appends "|0" to Steam/Epic IDs (platform|account|suffix).
    if let Some(idx) = s.find('|') {
        s = &s[..idx];
    }
    if s.is_empty() {
        return None;
    }

    // 1. Raw numeric: 64-bit SteamID or 32-bit Account ID
    if let Ok(num) = s.parse::<u64>() {
        if num > STEAMID64_INDIVIDUAL_BASE && num <= STEAMID64_INDIVIDUAL_BASE + (u32::MAX as u64) {
            return Some(num);
        } else if num > 0 && num <= (u32::MAX as u64) {
            return Some(STEAMID64_INDIVIDUAL_BASE + num);
        }
    }

    // 2. Steam3 format: "[U:1:Z]" or "U:1:Z"
    let trimmed_steam3 = s.strip_prefix('[').and_then(|t| t.strip_suffix(']')).unwrap_or(s);
    if let Some(rest) = trimmed_steam3.strip_prefix("U:1:") {
        if let Ok(acc_id) = rest.parse::<u64>() {
            if acc_id > 0 && acc_id <= (u32::MAX as u64) {
                return Some(STEAMID64_INDIVIDUAL_BASE + acc_id);
            }
        }
    }

    // 3. Legacy Steam2 format: "STEAM_0:B:C" or "STEAM_1:B:C"
    if let Some(rest) = s.strip_prefix("STEAM_") {
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() == 3 {
            if let (Ok(b), Ok(c)) = (parts[1].parse::<u64>(), parts[2].parse::<u64>()) {
                if b <= 1 {
                    let acc_id = c.checked_mul(2)?.checked_add(b)?;
                    if acc_id > 0 && acc_id <= (u32::MAX as u64) {
                        return Some(STEAMID64_INDIVIDUAL_BASE + acc_id);
                    }
                }
            }
        }
    }

    None
}

pub async fn fetch_steam_avatar_png(steam_id: u64) -> Option<Vec<u8>> {
    // 1. In-memory cache check
    if let Ok(guard) = STEAM_AVATAR_CACHE.lock() {
        if let Some(map) = guard.as_ref() {
            if let Some(cached) = map.get(&steam_id) {
                return Some(cached.clone());
            }
        }
    }

    // 2. Disk cache check
    let cache_name = format!("steam_{steam_id}.png");
    let candidate_dirs = [
        PathBuf::from("/root/.config/com.velocityrl.app/custom_avatars"),
        PathBuf::from("custom_avatars"),
    ];
    for dir in &candidate_dirs {
        let f = dir.join(&cache_name);
        if f.is_file() {
            if let Ok(b) = fs::read(&f) {
                if b.len() > 32 {
                    if let Ok(mut guard) = STEAM_AVATAR_CACHE.lock() {
                        guard
                            .get_or_insert_with(std::collections::HashMap::new)
                            .insert(steam_id, b.clone());
                    }
                    return Some(b);
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .ok()?;

    let mut direct_avatar_url: Option<String> = None;

    // 3. Primary: VelocityRL domain API proxy (centralized server-side resolution)
    let backend_url = format!("https://api.velocityrl.tech/v1/steam/user/{steam_id}");
    if let Ok(resp) = client.get(&backend_url).send().await {
        if resp.status().is_success() {
            if let Ok(prof) = resp.json::<VelocitySteamProfile>().await {
                if let Some(url) = prof.avatar_url {
                    if !url.trim().is_empty() {
                        direct_avatar_url = Some(url);
                    }
                }
            }
        }
    }

    // 4. Backup: Steam Community XML scrape
    if direct_avatar_url.is_none() {
        let xml_url = format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1");
        if let Ok(resp) = client.get(&xml_url).send().await {
            if let Ok(text) = resp.text().await {
                if let Some(pos) = text.find("<avatarFull><![CDATA[") {
                    let rest = &text[pos + 21..];
                    if let Some(end) = rest.find("]]></avatarFull>") {
                        direct_avatar_url = Some(rest[..end].to_string());
                    }
                } else if let Some(pos) = text.find("<avatarFull>") {
                    let rest = &text[pos + 12..];
                    if let Some(end) = rest.find("</avatarFull>") {
                        direct_avatar_url = Some(rest[..end].to_string());
                    }
                }
            }
        }
    }

    // 5. Fallback: Steam miniprofile JSON
    if direct_avatar_url.is_none() {
        let account_id = (steam_id & 0xFFFF_FFFF) as u32;
        if account_id > 0 {
            let miniprofile_url = format!("https://steamcommunity.com/miniprofile/{account_id}/json");
            if let Ok(resp) = client.get(&miniprofile_url).send().await {
                if resp.status().is_success() {
                    if let Ok(profile) = resp.json::<SteamMiniProfileResponse>().await {
                        if let Some(url) = profile.avatar_url {
                            if !url.trim().is_empty() {
                                direct_avatar_url = Some(url);
                            }
                        }
                    }
                }
            }
        }
    }

    let img_url = direct_avatar_url?;

    let img_resp = client.get(&img_url).send().await.ok()?;
    let bytes = img_resp.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;

    // Cache to disk
    let primary_cache_dir = PathBuf::from("/root/.config/com.velocityrl.app/custom_avatars");
    let _ = fs::create_dir_all(&primary_cache_dir);
    let _ = fs::write(primary_cache_dir.join(&cache_name), &png_bytes);

    // Cache in memory
    if let Ok(mut guard) = STEAM_AVATAR_CACHE.lock() {
        guard
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(steam_id, png_bytes.clone());
    }

    Some(png_bytes)
}

pub fn process_avatar_image(raw_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let img = image::load_from_memory(raw_bytes)
        .map_err(|e| format!("Failed to decode image: {e}"))?;

    // Square crop center
    let (width, height) = (img.width(), img.height());
    let min_dim = width.min(height);
    let x_offset = (width - min_dim) / 2;
    let y_offset = (height - min_dim) / 2;
    let cropped = img.crop_imm(x_offset, y_offset, min_dim, min_dim);

    // Resize to 256x256
    let resized = cropped.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
    let rgba = resized.to_rgba8();

    // Convert to BGRA 32-bit (256 * 256 * 4 = 262,144 bytes)
    let mut bgra = Vec::with_capacity(256 * 256 * 4);
    for pixel in rgba.pixels() {
        bgra.push(pixel[2]); // B
        bgra.push(pixel[1]); // G
        bgra.push(pixel[0]); // R
        bgra.push(pixel[3]); // A
    }

    if bgra.len() != 262_144 {
        return Err(format!("Unexpected raw BGRA buffer size: {} (expected 262144)", bgra.len()));
    }

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    resized.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode processed avatar to PNG: {e}"))?;

    Ok((bgra, png_bytes))
}

#[allow(dead_code)]
fn get_game_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = app.path().app_config_dir() {
        let config_file = dir.join("config.json");
        if config_file.is_file() {
            if let Ok(content) = fs::read_to_string(config_file) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(gd) = val.get("game_dir").and_then(|v| v.as_str()) {
                        let trimmed = gd.trim();
                        if !trimmed.is_empty() {
                            return Some(PathBuf::from(trimmed));
                        }
                    }
                }
            }
        }
    }
    None
}

#[tauri::command]
pub async fn apply_custom_avatar_upk(app: tauri::AppHandle, avatar_source: String) -> Result<String, String> {
    let clean_src = avatar_source.trim().trim_matches('"').trim_matches('\'').trim();
    if clean_src.is_empty() {
        return Err("Avatar source is empty".into());
    }

    // Decode image bytes from base64, URL, or local file
    let image_bytes = if clean_src.starts_with("data:image/") {
        if let Some(comma_pos) = clean_src.find(',') {
            let b64_part = &clean_src[comma_pos + 1..];
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64_part)
                .map_err(|e| format!("Base64 decode failed: {e}"))?
        } else {
            return Err("Invalid data URL".into());
        }
    } else if clean_src.starts_with("http://") || clean_src.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client.get(clean_src).send().await
            .map_err(|e| format!("Failed to download avatar: {e}"))?;
        resp.bytes().await
            .map_err(|e| format!("Failed to read avatar bytes: {e}"))?
            .to_vec()
    } else {
        let path = Path::new(clean_src);
        if !path.is_file() {
            return Err(format!("Avatar file not found: {}", path.display()));
        }
        fs::read(path).map_err(|e| format!("Failed to read avatar file: {e}"))?
    };

    let (raw_bgra, processed_png) = process_avatar_image(&image_bytes)?;
    // Persist downloaded/processed avatar to local disk cache and /root/avatar.png
    let _ = save_cached_avatar(&app, &processed_png);
    let _ = fs::write("/root/avatar.png", &processed_png);
    set_active_avatar_png(processed_png.clone());

    let _ = raw_bgra;
    crate::applog::event("avatar: custom avatar loaded into memory broker; game packages unchanged");
    Ok("Custom avatar saved to the image broker. Lobby display requires the matching API package's lobby avatar hook. Restart Rocket League to refresh cached images.".into())
}

/// Hydrate the broker image on startup. Package writes belong to idle maintenance.
pub fn ensure_cached_avatar_applied(app: &tauri::AppHandle) -> bool {
    let mut cached: Option<Vec<u8>> = None;
    let mut source = String::from("<none>");
    if let Ok(dir) = app.path().app_config_dir() {
        let path = dir.join("custom_avatars").join("active_avatar.png");
        match fs::read(&path) {
            Ok(bytes) => {
                cached = Some(bytes);
                source = path.display().to_string();
            }
            Err(e) => crate::applog::event(&format!(
                "avatar startup: no cached avatar at {} ({e})",
                path.display()
            )),
        }
    }
    if cached.is_none() {
        if let Ok(bytes) = fs::read("/root/avatar.png") {
            cached = Some(bytes);
            source = "/root/avatar.png".into();
        }
    }

    let Some(png_bytes) = cached else {
        crate::applog::event("avatar startup: no cached avatar found, UPK not patched");
        return false;
    };
    let Ok((raw_bgra, processed_png)) = process_avatar_image(&png_bytes) else {
        crate::applog::event("avatar startup: cached avatar could not be decoded");
        return false;
    };
    set_active_avatar_png(processed_png);
    crate::applog::event(&format!(
        "avatar startup: restoring cached avatar from {source}"
    ));
    let _ = raw_bgra;
    true
}

#[tauri::command]
pub fn restore_custom_avatar_upk(app: tauri::AppHandle) -> Result<String, String> {
    clear_active_avatar_png();
    let _ = fs::remove_file("/root/avatar.png");
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = fs::remove_file(dir.join("custom_avatars").join("active_avatar.png"));
    }

    crate::applog::event("avatar: custom avatar cleared from image broker");
    Ok("Custom avatar cleared. Restart Rocket League to refresh cached images.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_profile_config_defaults() {
        let cfg = CustomProfileConfig::default();
        assert_eq!(cfg.player_name, "Player");
        assert_eq!(cfg.title_text, "EST. 2015");
        assert_eq!(cfg.banner_id, "circuitboard");
        assert_eq!(cfg.border_id, "flames");
        assert_eq!(cfg.xp_level, None);
    }

    #[test]
    fn test_active_avatar_in_memory_state() {
        clear_active_avatar_png();
        let fake_png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        set_active_avatar_png(fake_png.clone());
        assert_eq!(get_active_avatar_png(), Some(fake_png));
        clear_active_avatar_png();
    }

    #[tokio::test]
    async fn test_fetch_steam_avatar_cached_or_live() {
        // Steam ID 76561198000000000 has a public profile
        let res = fetch_steam_avatar_png(76561198000000000).await;
        if let Some(png) = res {
            assert!(png.len() > 100);
            assert_eq!(&png[1..4], b"PNG");
        }
    }

    #[tokio::test]
    async fn test_steam_openapi_avatar_resolution() {
        // Steam ID 76561197960287930 (Gabe Newell)
        let res = fetch_steam_avatar_png(76561197960287930).await;
        assert!(res.is_some(), "Steam OpenAPI avatar should resolve");
        let png = res.unwrap();
        assert!(png.len() > 100);
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn test_parse_steam_id_formats_and_bounds() {
        // Standard 64-bit Steam ID
        assert_eq!(parse_steam_id("76561198000000000"), Some(76561198000000000));

        // High-bound 64-bit Steam ID (7656120... range when account id is near u32::MAX)
        let max_id64 = STEAMID64_INDIVIDUAL_BASE + (u32::MAX as u64);
        assert_eq!(parse_steam_id(&max_id64.to_string()), Some(max_id64));

        // Exceeds max 64-bit individual Steam ID range
        assert_eq!(parse_steam_id(&(max_id64 + 1).to_string()), None);

        // 32-bit Account ID
        assert_eq!(parse_steam_id("39734272"), Some(76561198000000000));
        assert_eq!(parse_steam_id("4294967295"), Some(max_id64));

        // 32-bit Account ID out of range (> u32::MAX but < base)
        assert_eq!(parse_steam_id("4294967296"), None);
        assert_eq!(parse_steam_id("12345678901"), None);

        // RL platform-prefixed IDs with trailing |0
        assert_eq!(parse_steam_id("steam|76561198183943334|0"), Some(76561198183943334));
        assert_eq!(parse_steam_id("Steam|76561198183943334|0"), Some(76561198183943334));

        // Steam3 format
        assert_eq!(parse_steam_id("[U:1:39734272]"), Some(76561198000000000));
        assert_eq!(parse_steam_id("U:1:39734272"), Some(76561198000000000));

        // Steam2 legacy format
        assert_eq!(parse_steam_id("STEAM_0:0:19867136"), Some(76561198000000000));
        assert_eq!(parse_steam_id("STEAM_1:0:19867136"), Some(76561198000000000));

        // Percent-encoded query parameters
        assert_eq!(parse_steam_id("%5BU%3A1%3A39734272%5D"), Some(76561198000000000));
        assert_eq!(parse_steam_id("STEAM_0%3A0%3A19867136"), Some(76561198000000000));

        // Invalid inputs
        assert_eq!(parse_steam_id(""), None);
        assert_eq!(parse_steam_id("avatar"), None);
        assert_eq!(parse_steam_id("rlim"), None);
    }

    #[test]
    fn test_get_steam_login_url_structure() {
        let url = get_steam_login_url();
        assert!(url.starts_with("https://steamcommunity.com/openid/login"));
        assert!(url.contains("openid.mode=checkid_setup"));
        assert!(url.contains("rlim%2Fsteam_callback"));
    }

    #[test]
    fn test_authenticated_steam_user_state() {
        clear_authenticated_steam_user_internal();
        assert!(get_authenticated_steam_user_internal().is_none());

        let u = SteamAuthenticatedUser {
            steam_id: "76561198000000000".into(),
            persona_name: "!".into(),
            avatar_url: "https://example.com/avatar.jpg".into(),
            avatar_data_url: "data:image/png;base64,abc".into(),
        };
        set_authenticated_steam_user(u.clone());
        let fetched = get_authenticated_steam_user_internal();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().persona_name, "!");

        clear_authenticated_steam_user_internal();
        assert!(get_authenticated_steam_user_internal().is_none());
    }

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a=1&b=2"), "a%3D1%26b%3D2");
    }
}
