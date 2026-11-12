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
use crate::upk::crypto;
use crate::upk::swapper::{TAGAME_KEY, TAGAME_BACKUP_NAME};

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
    let candidates = [
        "/root/avatar.png",
        "custom_avatars/active_avatar.png",
        "/root/.config/com.velocityrl.app/custom_avatars/active_avatar.png",
    ];
    for c in &candidates {
        let p = Path::new(c);
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

pub fn set_authenticated_steam_user(user: SteamAuthenticatedUser) {
    if let Ok(mut lock) = AUTHENTICATED_STEAM_USER.lock() {
        *lock = Some(user);
    }
}

pub fn get_authenticated_steam_user_internal() -> Option<SteamAuthenticatedUser> {
    AUTHENTICATED_STEAM_USER.lock().ok().and_then(|g| g.clone())
}

pub fn clear_authenticated_steam_user_internal() {
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
    let _ = restore_custom_avatar_upk(app);
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

fn percent_decode(s: &str) -> String {
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
    let s = decoded.trim();
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

pub fn process_avatar_to_bgra256(raw_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (bgra, _) = process_avatar_image(raw_bytes)?;
    Ok(bgra)
}

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
    let clean_src = avatar_source.trim();
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

    let (_, processed_png) = process_avatar_image(&image_bytes)?;
    // Persist downloaded/processed avatar to local disk cache and /root/avatar.png
    let _ = save_cached_avatar(&app, &processed_png);
    let _ = fs::write("/root/avatar.png", &processed_png);
    set_active_avatar_png(processed_png.clone());

    // If a modified TAGame.upk exists from old direct UPK stomping, restore it to pristine backup
    if let Some(game_dir) = get_game_dir(&app) {
        if let Ok(cooked_dir) = crate::upk::swapper::resolve_cooked_dir(&game_dir) {
            let tagame_path = cooked_dir.join("TAGame.upk");
            let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);
            if backup_path.is_file() {
                let _ = fs::copy(&backup_path, &tagame_path);
            }
        }
    }

    crate::applog::event("avatar: custom avatar activated in broker pipeline (served via HTTP on 15759/7546/49320)");
    Ok("Custom avatar active via broker HTTP pipeline".into())
}

#[tauri::command]
pub fn restore_custom_avatar_upk(app: tauri::AppHandle) -> Result<String, String> {
    clear_active_avatar_png();
    let _ = fs::remove_file("/root/avatar.png");
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = fs::remove_file(dir.join("custom_avatars").join("active_avatar.png"));
    }

    // Ensure TAGame.upk is restored if a backup was previously created
    if let Some(game_dir) = get_game_dir(&app) {
        if let Ok(cooked_dir) = crate::upk::swapper::resolve_cooked_dir(&game_dir) {
            let tagame_path = cooked_dir.join("TAGame.upk");
            let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);
            if backup_path.is_file() {
                let _ = fs::copy(&backup_path, &tagame_path);
            }
        }
    }

    crate::applog::event("avatar: custom avatar cleared from broker pipeline");
    Ok("Vanilla avatar restored".into())
}

pub fn apply_custom_avatar_to_tagame(tagame_path: &Path, raw_bgra: &[u8]) -> Result<(), String> {
    if raw_bgra.len() != 262_144 {
        return Err("Raw BGRA buffer must be exactly 262,144 bytes".into());
    }
    if !tagame_path.is_file() {
        return Err(format!("TAGame.upk not found at {}", tagame_path.display()));
    }

    let backup_path = tagame_path.with_file_name(TAGAME_BACKUP_NAME);
    if !backup_path.is_file() {
        fs::copy(tagame_path, &backup_path)
            .map_err(|e| format!("Failed to create backup: {e}"))?;
    }

    let mut file_bytes = fs::read(&backup_path)
        .map_err(|e| format!("Failed to read backup: {e}"))?;

    if file_bytes.len() < 32 {
        return Err("TAGame.upk too small".into());
    }

    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
    p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
    p += 4;
    let name_count = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
    p += 4;
    let name_offset = u32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap()) as usize;

    let enc_size = (total_header_size - name_offset + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end > file_bytes.len() {
        return Err("Encrypted header bounds invalid".into());
    }

    let mut plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
    let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count)
        .map_err(|e| e.to_string())?;

    let p_sum = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize }) + 4 + 8;
    let export_count = i32::from_le_bytes(file_bytes[p_sum..p_sum + 4].try_into().unwrap()) as usize;
    let export_offset = i32::from_le_bytes(file_bytes[p_sum + 4..p_sum + 8].try_into().unwrap()) as usize;
    let depends_offset = i32::from_le_bytes(file_bytes[p_sum + 16..p_sum + 20].try_into().unwrap()) as usize;

    let export_rel = export_offset - name_offset;
    let depends_rel = depends_offset - name_offset;

    #[allow(dead_code)]
    struct ExportEntry {
        idx: usize,
        name: String,
        outer_idx: i32,
        outer_name: String,
        serial_size: i32,
        serial_offset: i64,
    }

    let mut exports: Vec<ExportEntry> = Vec::new();
    let mut pos = export_rel;
    while pos + 72 <= depends_rel && pos + 72 <= plain_header.len() && exports.len() < export_count {
        let i32_at = |a: usize| i32::from_le_bytes(plain_header[pos + a..pos + a + 4].try_into().unwrap());
        let outer_idx = i32_at(8);
        let name_idx = i32_at(12);
        let serial_size = i32_at(32);
        let serial_offset = i64::from_le_bytes(plain_header[pos + 36..pos + 44].try_into().unwrap());
        let noc = i32_at(48);

        let nm = if name_idx >= 0 && (name_idx as usize) < names.len() {
            names[name_idx as usize].clone()
        } else {
            String::new()
        };

        exports.push(ExportEntry {
            idx: exports.len() + 1,
            name: nm,
            outer_idx,
            outer_name: String::new(),
            serial_size,
            serial_offset,
        });
        pos += 72 + (noc.max(0) as usize) * 4;
    }

    for i in 0..exports.len() {
        let out_idx = exports[i].outer_idx;
        if out_idx > 0 && (out_idx as usize) <= exports.len() {
            exports[i].outer_name = exports[out_idx as usize - 1].name.clone();
        }
    }

    #[allow(dead_code)]
    struct ChunkEntry {
        pos: usize,
        uncomp_offset: i64,
        uncomp_size: i32,
        comp_offset: i64,
        comp_size: i32,
    }

    let mut c_pos = depends_rel;
    if c_pos + 4 > plain_header.len() {
        return Err("Chunk table offset out of bounds".into());
    }
    let chunk_count = i32::from_le_bytes(plain_header[c_pos..c_pos + 4].try_into().unwrap()) as usize;
    c_pos += 4;

    let mut chunks = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        if c_pos + 36 > plain_header.len() {
            break;
        }
        let uncomp_offset = i64::from_le_bytes(plain_header[c_pos..c_pos + 8].try_into().unwrap());
        let uncomp_size = i32::from_le_bytes(plain_header[c_pos + 8..c_pos + 12].try_into().unwrap());
        let comp_offset = i64::from_le_bytes(plain_header[c_pos + 12..c_pos + 20].try_into().unwrap());
        let comp_size = i32::from_le_bytes(plain_header[c_pos + 20..c_pos + 24].try_into().unwrap());
        chunks.push(ChunkEntry {
            pos: c_pos,
            uncomp_offset,
            uncomp_size,
            comp_offset,
            comp_size,
        });
        c_pos += 36;
    }

    if chunks.len() < 7 {
        return Err("TAGame.upk has insufficient chunks".into());
    }

    // 1. Patch Chunk 1:
    //    a) GFxData_LocalPlayer_TA::HandleGetAvatar (Main menu & local banner)
    //       Original bytecode (14 bytes): 0f 01 d1 85 00 00 2b 32 86 00 00 04 0b 4c
    //       Patched bytecode (14 bytes):  0f 01 d1 85 00 00 20 3a 6b 01 00 04 0b 4c (EX_OBJECT_CONST PlaceholderAnthem #92986)
    //    b) GFxData_Persona_TA::HandleGetAvatar (Scoreboard, party bar, & persona cards)
    //       Original bytecode (14 bytes): 0f 01 db 91 00 00 2b e2 91 00 00 04 0b 4c
    //       Patched bytecode (14 bytes):  0f 01 db 91 00 00 20 3a 6b 01 00 04 0b 4c (EX_OBJECT_CONST PlaceholderAnthem #92986)
    let c1_start = chunks[1].comp_offset as usize;
    let c1_end = c1_start + chunks[1].comp_size as usize;
    if c1_end > file_bytes.len() {
        return Err("Chunk 1 out of bounds".into());
    }

    let mut decomp1 = crate::upk::compression::decompress_chunk(&file_bytes[c1_start..c1_end])
        .map_err(|e| format!("Decompress Chunk 1 failed: {e}"))?;

    // a) Patch GFxData_LocalPlayer_TA::HandleGetAvatar
    let get_avatar_exp = exports.iter().find(|e| e.name == "HandleGetAvatar" && e.outer_name == "GFxData_LocalPlayer_TA")
        .ok_or_else(|| "HandleGetAvatar not found in GFxData_LocalPlayer_TA".to_string())?;

    let fn_rel_off = (get_avatar_exp.serial_offset - chunks[1].uncomp_offset) as usize;
    if fn_rel_off + 48 + 14 <= decomp1.len() {
        let script_disk_sz = u32::from_le_bytes(decomp1[fn_rel_off + 44..fn_rel_off + 48].try_into().unwrap()) as usize;
        if script_disk_sz == 14 {
            let patched_bc = [
                0x0f, 0x01, 0xd1, 0x85, 0x00, 0x00, // EX_LET PlayerAvatarTexture
                0x20, 0x3a, 0x6b, 0x01, 0x00,       // EX_OBJECT_CONST PlaceholderAnthem (#92986)
                0x04, 0x0b, 0x4c                    // EX_RETURN EX_NOTHING EX_END_OF_SCRIPT
            ];
            decomp1[fn_rel_off + 48..fn_rel_off + 48 + 14].copy_from_slice(&patched_bc);
        }
    }

    // b) Patch GFxData_Persona_TA::HandleGetAvatar
    if let Some(persona_avatar_exp) = exports.iter().find(|e| e.name == "HandleGetAvatar" && e.outer_name == "GFxData_Persona_TA") {
        let p_rel_off = (persona_avatar_exp.serial_offset - chunks[1].uncomp_offset) as usize;
        if p_rel_off + 48 + 14 <= decomp1.len() {
            let script_disk_sz = u32::from_le_bytes(decomp1[p_rel_off + 44..p_rel_off + 48].try_into().unwrap()) as usize;
            if script_disk_sz == 14 {
                let patched_persona_bc = [
                    0x0f, 0x01, 0xdb, 0x91, 0x00, 0x00, // EX_LET PlayerAvatar (Export #37339)
                    0x20, 0x3a, 0x6b, 0x01, 0x00,       // EX_OBJECT_CONST PlaceholderAnthem (#92986)
                    0x04, 0x0b, 0x4c                    // EX_RETURN EX_NOTHING EX_END_OF_SCRIPT
                ];
                decomp1[p_rel_off + 48..p_rel_off + 48 + 14].copy_from_slice(&patched_persona_bc);
            }
        }
    }

    let recomp1 = crate::upk::compression::compress_chunk(&decomp1)
        .map_err(|e| format!("Recompress Chunk 1 failed: {e}"))?;
    let orig_c1_sz = chunks[1].comp_size as usize;
    if recomp1.len() <= orig_c1_sz {
        let mut padded1 = recomp1;
        padded1.resize(orig_c1_sz, 0);
        file_bytes[c1_start..c1_start + orig_c1_sz].copy_from_slice(&padded1);
    } else {
        return Err("Recompressed Chunk 1 exceeds allocation".into());
    }

    // 2. Patch Chunk 6: album_anthem_placeholderanthem.PlaceholderAnthem bulk data
    let c6_start = chunks[6].comp_offset as usize;
    let c6_end = c6_start + chunks[6].comp_size as usize;
    if c6_end > file_bytes.len() {
        return Err("Chunk 6 out of bounds".into());
    }

    let mut decomp6 = crate::upk::compression::decompress_chunk(&file_bytes[c6_start..c6_end])
        .map_err(|e| format!("Decompress Chunk 6 failed: {e}"))?;

    let tex_exp = exports.iter().find(|e| e.name == "PlaceholderAnthem")
        .ok_or_else(|| "PlaceholderAnthem not found".to_string())?;

    let tex_rel_off = (tex_exp.serial_offset - chunks[6].uncomp_offset) as usize;
    let pixel_start = tex_rel_off + 360;
    if pixel_start + 262_144 <= decomp6.len() {
        decomp6[pixel_start..pixel_start + 262_144].copy_from_slice(raw_bgra);
    } else {
        return Err("PlaceholderAnthem pixel offset out of bounds".into());
    }

    let recomp6 = crate::upk::compression::compress_chunk(&decomp6)
        .map_err(|e| format!("Recompress Chunk 6 failed: {e}"))?;
    let orig_c6_sz = chunks[6].comp_size as usize;

    if recomp6.len() <= orig_c6_sz {
        // Fits in-place
        let mut padded6 = recomp6;
        padded6.resize(orig_c6_sz, 0);
        file_bytes[c6_start..c6_start + orig_c6_sz].copy_from_slice(&padded6);
    } else {
        // Shift subsequent chunks cleanly
        let delta = recomp6.len() as i64 - orig_c6_sz as i64;
        let tail_start = c6_start + orig_c6_sz;
        let tail = file_bytes[tail_start..].to_vec();

        file_bytes.truncate(c6_start);
        file_bytes.extend_from_slice(&recomp6);
        file_bytes.extend_from_slice(&tail);

        // Update Chunk 6 comp_size in plain_header
        plain_header[chunks[6].pos + 20..chunks[6].pos + 24]
            .copy_from_slice(&(recomp6.len() as i32).to_le_bytes());

        // Shift subsequent chunks comp_offset
        for ch in &chunks[7..] {
            let new_co = ch.comp_offset + delta;
            plain_header[ch.pos + 12..ch.pos + 20]
                .copy_from_slice(&new_co.to_le_bytes());
        }

        // Re-encrypt header with updated chunk table
        let re_enc = crypto::encrypt_ecb(&TAGAME_KEY, &plain_header);
        file_bytes[name_offset..enc_end].copy_from_slice(&re_enc);
    }

    // Atomic write to TAGame.upk
    let tmp_path = tagame_path.with_extension("tmp_avatar");
    fs::write(&tmp_path, &file_bytes)
        .map_err(|e| format!("Failed to write patched UPK: {e}"))?;
    fs::rename(&tmp_path, tagame_path)
        .map_err(|e| format!("Failed to atomically replace TAGame.upk: {e}"))?;

    Ok(())
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
    fn test_process_avatar_to_bgra256() {
        // Create an in-memory 64x64 PNG
        let img = image::RgbaImage::new(64, 64);
        let mut png_bytes = Vec::new();
        let mut cursor = Cursor::new(&mut png_bytes);
        img.write_to(&mut cursor, image::ImageFormat::Png).expect("Encode PNG");

        let bgra = process_avatar_to_bgra256(&png_bytes).expect("Process avatar to BGRA");
        assert_eq!(bgra.len(), 262_144);
    }

    #[test]
    fn test_apply_custom_avatar_to_tagame_real() {
        let tagame_path = Path::new("/root/TAGame_clean.upk");
        if !tagame_path.exists() {
            return;
        }

        let temp_dir = std::env::temp_dir().join(format!("vrl_test_avatar_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let test_upk = temp_dir.join("TAGame.upk");
        fs::copy(tagame_path, &test_upk).expect("copy tagame upk");

        let raw_bgra = vec![0x7fu8; 262_144];
        let res = apply_custom_avatar_to_tagame(&test_upk, &raw_bgra);
        assert!(res.is_ok(), "apply_custom_avatar_to_tagame failed: {:?}", res.err());

        let _ = fs::remove_dir_all(&temp_dir);
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
