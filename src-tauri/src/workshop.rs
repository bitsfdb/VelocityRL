use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

const API_BASE: &str = "https://api.velocityrl.tech";

const AUTH_SECRET: &str = "vrl_wk_9f2c4a1e8b7d3e6f5a0c9b8d7e2f1a4c3b6d9e0f2a5c8b1d4e7f0a3c6b9d2e5f";
const REPLACED_UPK: &str = "Labs_Underpass_P.upk";


#[derive(Serialize, Clone)]
pub struct WorkshopAuth {
    pub ok: bool,
    pub steam_id: String,
    pub persona: String,
    pub avatar: String,
    pub session: String,
}

#[derive(serde::Deserialize, Serialize, Clone)]
pub struct WorkshopInstalled {
    pub map_id: String,
    pub map_name: String,
    pub installed_at: String,

    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

#[derive(serde::Deserialize, Serialize, Clone)]
pub struct WorkshopMap {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub size_bytes: Option<u64>,
    pub preview_url: Option<String>,
    pub download_url: Option<String>,
}

#[tauri::command]
pub fn workshop_get_auth() -> Result<Option<WorkshopAuth>, String> {
    Ok(Some(WorkshopAuth {
        ok: true,
        steam_id: String::new(),
        persona: "VelocityRL".into(),
        avatar: String::new(),
        session: mint_session_token(),
    }))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn mint_session_token() -> String {
    type HmacSha256 = Hmac<Sha256>;
    let payload = serde_json::json!({
        "steam_id": "local",
        "persona": "VelocityRL",
        "avatar": "",
        "exp": now_secs() + 10 * 365 * 24 * 3600,
    });
    let p_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    let mut mac = HmacSha256::new_from_slice(AUTH_SECRET.as_bytes()).expect("key");
    mac.update(format!("v1.{}", p_b64).as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("v1.{}.{}", p_b64, sig)
}

#[cfg(test)]
fn verify_session_token(tok: &str) -> Result<serde_json::Value, String> {
    use hmac::Mac;
    type HmacSha256 = Hmac<Sha256>;
    let parts: Vec<&str> = tok.split('.').collect();
    if parts.len() != 3 || parts[0] != "v1" {
        return Err("invalid token format".into());
    }
    let p_bytes = URL_SAFE_NO_PAD.decode(parts[1]).map_err(|e| e.to_string())?;
    let mut mac = HmacSha256::new_from_slice(AUTH_SECRET.as_bytes()).map_err(|e| e.to_string())?;
    mac.update(format!("v1.{}", parts[1]).as_bytes());
    let sig = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|e| e.to_string())?;
    mac.verify_slice(&sig).map_err(|_| "signature verification failed".to_string())?;
    let val: serde_json::Value = serde_json::from_slice(&p_bytes).map_err(|e| e.to_string())?;
    Ok(val)
}

#[tauri::command]
pub async fn workshop_search_maps(_app: AppHandle, query: String) -> Result<Vec<WorkshopMap>, String> {
    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut url = format!("{API_BASE}/v2/rl/workshop/maps");
    if !query.trim().is_empty() {
        url.push_str(&format!("?q={}", urlenc(&query)));
    }
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("server {}", resp.status().as_u16()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(list) = v.get("maps").and_then(|m| m.as_array()) {
        for m in list {
            out.push(WorkshopMap {
                id: m.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                name: m.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                author: m.get("author").and_then(|x| x.as_str()).map(|s| s.to_string()),
                size_bytes: m.get("size_bytes").and_then(|x| x.as_u64()),
                preview_url: m.get("preview_url").and_then(|x| x.as_str()).map(|s| s.to_string()),
                download_url: m.get("download_url").and_then(|x| x.as_str()).map(|s| s.to_string()),
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn workshop_fetch_bakkes_maps(
    page: Option<u32>,
    query: Option<String>,
) -> Result<serde_json::Value, String> {
    let p = page.unwrap_or(1).max(1);
    let mut url = format!("https://bakkesplugins.com/api/rocket-league-maps?page={p}&pageSize=20");
    if let Some(ref q) = query {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            url.push_str(&format!("&search={}", urlenc(trimmed)));
        }
    }

    crate::applog::event(&format!("workshop: fetching maps from {url}"));

    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| {
            let msg = format!("Network request failed: {e}");
            crate::applog::event(&format!("workshop: maps request error: {msg}"));
            msg
        })?;

    let status = resp.status();
    crate::applog::event(&format!("workshop: maps response status {status}"));

    if !status.is_success() {
        return Err(format!("BakkesPlugins returned HTTP {}", status.as_u16()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| {
            let msg = format!("JSON parsing failed: {e}");
            crate::applog::event(&format!("workshop: maps json parse error: {msg}"));
            msg
        })?;

    Ok(json)
}

#[tauri::command]
pub async fn workshop_fetch_bakkes_versions(
    map_id: u64,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("https://bakkesplugins.com/api/rocket-league-maps/{map_id}/versions");
    crate::applog::event(&format!("workshop: fetching map versions from {url}"));

    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| {
            let msg = format!("Network request failed: {e}");
            crate::applog::event(&format!("workshop: map versions error: {msg}"));
            msg
        })?;

    let status = resp.status();
    crate::applog::event(&format!("workshop: map versions response status {status}"));

    if !status.is_success() {
        return Err(format!("BakkesPlugins returned HTTP {}", status.as_u16()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON parsing failed: {e}"))?;

    Ok(json)
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub fn current_installed_map(app: &AppHandle) -> (Option<String>, Option<String>) {
    match read_installed(app) {
        Some(i) => (Some(i.map_id), Some(i.map_name)),
        None => (None, None),
    }
}

fn workshop_state_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("workshop_state.json"))
}

fn read_installed(app: &AppHandle) -> Option<WorkshopInstalled> {
    let p = workshop_state_path(app)?;
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(p).ok()?).ok()?;
    Some(WorkshopInstalled {
        map_id: v.get("map_id")?.as_str()?.to_string(),
        map_name: v.get("map_name")?.as_str()?.to_string(),
        installed_at: v.get("installed_at")?.as_str()?.to_string(),
        source_path: v.get("source_path").and_then(|x| x.as_str()).map(|s| s.to_string()),
        thumbnail_url: v.get("thumbnail_url").and_then(|x| x.as_str()).map(|s| s.to_string()),
    })
}

#[tauri::command]
pub fn workshop_get_installed(app: AppHandle) -> Result<Option<WorkshopInstalled>, String> {
    Ok(read_installed(&app))
}

pub fn read_installed_pub(app: &AppHandle) -> Option<WorkshopInstalled> {
    read_installed(app)
}

#[tauri::command]
pub async fn workshop_install_map(app: AppHandle, map_id: String, map_name: String) -> Result<WorkshopInstalled, String> {
    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Set your Rocket League folder in Settings first.".into());
    }
    let dir = crate::upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)).map_err(|e| e.to_string())?;
    let target = dir.join(REPLACED_UPK);
    let backup = dir.join(format!("{REPLACED_UPK}.vrl_bak"));

    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{API_BASE}/v2/rl/workshop/maps/{map_id}/download"))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: server {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() < 1 << 16 {
        return Err("downloaded file is too small to be a map package".into());
    }

    if !backup.exists() {
        if target.exists() {
            fs::rename(&target, &backup)
                .map_err(|e| format!("could not back up {REPLACED_UPK}: {e}"))?;
        }
    }
    fs::write(&target, &bytes).map_err(|e| {
        if e.raw_os_error() == Some(32) {
            "Rocket League currently has Underpass loaded in-game. Exit the match or training into the Main Menu and try again.".to_string()
        } else {
            format!("write map package: {e}")
        }
    })?;

    let fp = crate::integrity::upk_fingerprint(&target);
    let mut state = crate::load_integrity(&app);
    crate::integrity::mark_swap_package(&mut state, REPLACED_UPK, fp.as_deref());
    let _ = crate::save_integrity(&app, &state);

    let installed = WorkshopInstalled {
        map_id: map_id.clone(),
        map_name: map_name.clone(),
        installed_at: crate::now_iso8601_utc(),
        source_path: None,
        thumbnail_url: None,
    };
    let sp = workshop_state_path(&app).ok_or("config dir unavailable")?;
    fs::write(&sp, serde_json::to_string(&installed).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    crate::applog::event(&format!("workshop: installed map {map_id} ({map_name}) over {REPLACED_UPK}"));
    crate::presets::append_history(&app, "map_install", &[], &format!("installed map '{map_name}'"));
    Ok(installed)
}

#[tauri::command]
pub async fn workshop_install_custom_map(
    app: AppHandle,
    source_path: String,
    name: Option<String>,
) -> Result<WorkshopInstalled, String> {
    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Set your Rocket League folder in Settings first.".into());
    }
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err("Selected file no longer exists.".into());
    }
    install_map_bytes(&app, &config, &src, name).await
}

#[tauri::command]
pub async fn workshop_install_map_from_url(
    app: AppHandle,
    url: String,
    name: Option<String>,
) -> Result<WorkshopInstalled, String> {
    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Set your Rocket League folder in Settings first.".into());
    }
    let url = url.trim().trim_matches('"').to_string();
    if !url.starts_with("http://") && !url.starts_with("https://")
    {
        return Err("Enter a direct http(s) link to a .upk file.".into());
    }
    if url.contains("steamcommunity.com") || url.contains("/workshop/filedetails") {
        return Err("That's a workshop page, not the file. Use the direct download URL (.upk).".into());
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&url, &mut hasher);
    let cache_name = format!("url_{:016x}.upk", std::hash::Hasher::finish(&hasher));
    let cache_path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("map_cache")
        .join(&cache_name);

    if !cache_path.is_file() {
        let client = reqwest::Client::builder()
            .user_agent(crate::app_user_agent())
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client.get(&url).send().await.map_err(|e| format!("download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download failed: server {}", resp.status().as_u16()));
        }
        let total = resp.content_length().unwrap_or(0);
        if total > (512 << 20) as u64 {
            return Err("File is larger than 512 MB.".into());
        }

        let mut file = {
            if let Some(dir) = cache_path.parent() {
                fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            fs::File::create(&cache_path).map_err(|e| format!("cache write failed: {e}"))?
        };
        use std::io::Write;
        let mut downloaded: u64 = 0;
        let mut last_pct: i32 = -1;
        let mut resp = resp;
        while let Some(chunk) = resp.chunk().await.map_err(|e| {
            let _ = fs::remove_file(&cache_path);
            format!("download failed: {e}")
        })? {
            if downloaded + chunk.len() as u64 > (512 << 20) as u64 {
                let _ = fs::remove_file(&cache_path);
                return Err("File is larger than 512 MB.".into());
            }
            file.write_all(&chunk).map_err(|e| format!("cache write failed: {e}"))?;
            downloaded += chunk.len() as u64;
            let pct = if total > 0 { (downloaded * 100 / total) as i32 } else { -1 };
            if pct != last_pct {
                last_pct = pct;
                let _ = app.emit(
                    "map-download-progress",
                    serde_json::json!({ "downloaded": downloaded, "total": total, "percent": pct }),
                );
            }
        }
        let _ = app.emit(
            "map-download-progress",
            serde_json::json!({ "downloaded": downloaded, "total": total, "percent": 100 }),
        );
    }
    let display = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            url.trim_end_matches('/')
                .rsplit(['/'])
                .next()
                .and_then(|s| s.rsplit_once('.').map(|(stem, _)| stem))
                .unwrap_or("Map from URL")
                .to_string()
        });
    let inst = install_map_bytes(&app, &config, &cache_path, Some(display.clone())).await?;

    if let Some(mut lib) = read_map_library(&app) {
        lib.push(CustomMapEntry {
            name: display,
            path: cache_path.to_string_lossy().to_string(),
            source_url: Some(url),
            added_at: crate::now_iso8601_utc(),
            thumbnail_url: None,
        });
        save_map_library(&app, &lib);
    }
    Ok(inst)
}

async fn install_map_bytes(
    app: &AppHandle,
    config: &crate::Config,
    src: &Path,
    name: Option<String>,
) -> Result<WorkshopInstalled, String> {
    let display_name = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Custom map".into())
        });

    let mut magic = [0u8; 4];
    {
        use std::io::Read;
        let mut f = fs::File::open(&src).map_err(|e| format!("can't read file: {e}"))?;
        f.read_exact(&mut magic).map_err(|e| format!("can't read file: {e}"))?;
    }
    let is_upk = magic == [0xC1, 0x83, 0x2A, 0x9E]
        || magic == [0xC1, 0x83, 0x2A, 0x9E]
        || u32::from_le_bytes(magic) == 0x9E2A83C1;
    if !is_upk {
        return Err("That doesn't look like an Unreal package (.upk). Pick the map's .upk file.".into());
    }
    let bytes = fs::read(&src).map_err(|e| format!("read failed: {e}"))?;
    if bytes.len() < 1 << 16 {
        return Err("File is too small to be a map package.".into());
    }

    let dir = crate::upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)).map_err(|e| e.to_string())?;
    let target = dir.join(REPLACED_UPK);
    let backup = dir.join(format!("{REPLACED_UPK}.vrl_bak"));
    if !backup.exists() {
        if target.exists() {
            fs::rename(&target, &backup)
                .map_err(|e| format!("could not back up {REPLACED_UPK}: {e}"))?;
        }
    }
    fs::write(&target, &bytes).map_err(|e| {
        if e.raw_os_error() == Some(32) {
            "Rocket League currently has Underpass loaded in-game. Exit the match or training into the Main Menu and try again.".to_string()
        } else {
            format!("write map package: {e}")
        }
    })?;

    let fp = crate::integrity::upk_fingerprint(&target);
    let mut state = crate::load_integrity(app);
    crate::integrity::mark_swap_package(&mut state, REPLACED_UPK, fp.as_deref());
    let _ = crate::save_integrity(app, &state);

    let thumb = read_map_library(app)
        .and_then(|lib| lib.into_iter().find(|e| e.path == src.to_string_lossy()).and_then(|e| e.thumbnail_url));

    let installed = WorkshopInstalled {
        map_id: format!("custom:{}", src.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
        map_name: display_name.clone(),
        installed_at: crate::now_iso8601_utc(),
        source_path: Some(src.to_string_lossy().to_string()),
        thumbnail_url: thumb.clone(),
    };
    let sp = workshop_state_path(&app).ok_or("config dir unavailable")?;
    fs::write(&sp, serde_json::to_string(&installed).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    crate::applog::event(&format!("workshop: installed custom map '{}' from {}", display_name, src.display()));
    crate::presets::append_history(&app, "map_install", &[], &format!("installed custom map '{display_name}'"));

    if let Some(mut lib) = read_map_library(app) {
        let path_str = src.to_string_lossy().to_string();
        if !lib.iter().any(|e| e.path == path_str) {
            lib.push(CustomMapEntry {
                name: display_name,
                path: path_str,
                source_url: None,
                added_at: crate::now_iso8601_utc(),
                thumbnail_url: thumb,
            });
            save_map_library(app, &lib);
        }
    }
    Ok(installed)
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CustomMapEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub source_url: Option<String>,
    pub added_at: String,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

fn map_library_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("map_library.json"))
}

fn read_map_library(app: &AppHandle) -> Option<Vec<CustomMapEntry>> {
    let p = map_library_path(app)?;
    let raw = fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}

pub(crate) fn save_map_library(app: &AppHandle, lib: &[CustomMapEntry]) {
    if let Some(p) = map_library_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(lib) {
            let _ = fs::create_dir_all(p.parent().unwrap_or(&p));
            let _ = fs::write(p, json);
        }
    }
}

#[tauri::command]
pub fn workshop_get_map_library(app: AppHandle) -> Result<Vec<CustomMapEntry>, String> {
    Ok(read_map_library(&app).unwrap_or_default())
}

#[tauri::command]
pub async fn workshop_install_from_library(app: AppHandle, path: String) -> Result<WorkshopInstalled, String> {
    let entry = read_map_library(&app)
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.path == path)
        .ok_or("That map is no longer in your library.")?;
    if !Path::new(&path).is_file() {
        return Err("The map file is gone (moved or deleted) — pick it again with 'Upload custom .upk'.".into());
    }
    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Set your Rocket League folder in Settings first.".into());
    }
    install_map_bytes(&app, &config, Path::new(&path), Some(entry.name)).await
}

#[tauri::command]
pub fn workshop_remove_from_library(app: AppHandle, path: String) -> Result<(), String> {
    if let Some(mut lib) = read_map_library(&app) {
        lib.retain(|e| e.path != path);
        save_map_library(&app, &lib);
    }
    Ok(())
}

#[tauri::command]
pub async fn workshop_restore(app: AppHandle) -> Result<(), String> {
    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Set your Rocket League folder in Settings first.".into());
    }
    let dir = crate::upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)).map_err(|e| e.to_string())?;
    let target = dir.join(REPLACED_UPK);
    let backup = dir.join(format!("{REPLACED_UPK}.vrl_bak"));
    if backup.exists() {
        let _ = fs::remove_file(&target);
        fs::rename(&backup, &target).map_err(|e| {
            if e.raw_os_error() == Some(32) {
                "Rocket League currently has Underpass loaded in-game. Exit the match or training into the Main Menu and try again.".to_string()
            } else {
                format!("restore failed: {e}")
            }
        })?;
    }
    if let Some(sp) = workshop_state_path(&app) {
        let _ = fs::remove_file(sp);
    }

    let mut state = crate::load_integrity(&app);
    crate::integrity::clear_swap_package(&mut state, REPLACED_UPK);
    let _ = crate::save_integrity(&app, &state);

    crate::applog::event("workshop: restored stock Labs_Underpass_P");
    crate::presets::append_history(&app, "map_restore", &[], "restored original Underpass");
    Ok(())
}

const MAX_MAP_PRESETS: usize = 50;

fn map_presets_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("map_presets.json"))
}

fn load_map_presets(app: &AppHandle) -> Vec<MapPreset> {
    map_presets_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<MapPresetFile>(&s).ok())
        .map(|f| f.presets)
        .unwrap_or_default()
}

fn save_map_presets(app: &AppHandle, presets: &[MapPreset]) {
    if let Some(p) = map_presets_path(app) {
        let f = MapPresetFile { presets: presets.to_vec() };
        if let Ok(json) = serde_json::to_string_pretty(&f) {
            let _ = fs::create_dir_all(p.parent().unwrap_or(&p));
            let _ = fs::write(p, json);
        }
    }
}

#[tauri::command]
pub fn workshop_get_map_presets(app: AppHandle) -> Result<Vec<MapPreset>, String> {
    Ok(load_map_presets(&app))
}

#[tauri::command]
pub fn workshop_save_map_preset(app: AppHandle, name: String) -> Result<MapPreset, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err("Preset name must be 1–64 characters.".into());
    }
    let installed = read_installed(&app)
        .ok_or("No map is loaded right now — install a map first, then save it as a preset.")?;
    let mut presets = load_map_presets(&app);

    if let Some(existing) = presets.iter_mut().find(|p| p.name == name) {
        let updated = {
            existing.map_id = installed.map_id.clone();
            existing.map_name = installed.map_name.clone();
            existing.source_path = installed.source_path.clone();
            existing.created_at = crate::now_iso8601_utc();
            existing.clone()
        };
        save_map_presets(&app, &presets);
        return Ok(updated);
    }
    if presets.len() >= MAX_MAP_PRESETS {
        return Err(format!("Map preset limit reached ({MAX_MAP_PRESETS}). Delete one first."));
    }
    let preset = MapPreset {
        id: make_preset_id(),
        name: name.clone(),
        map_id: installed.map_id,
        map_name: installed.map_name,
        source_path: installed.source_path.clone(),
        created_at: crate::now_iso8601_utc(),
    };
    presets.push(preset.clone());
    save_map_presets(&app, &presets);
    crate::applog::event(&format!("workshop: saved map preset '{name}'"));
    Ok(preset)
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct MapPreset {
    pub id: String,
    pub name: String,
    pub map_id: String,
    pub map_name: String,

    #[serde(default)]
    pub source_path: Option<String>,
    pub created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct MapPresetFile {
    #[serde(default)]
    presets: Vec<MapPreset>,
}

fn make_preset_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..8).map(|_| format!("{:x}", rng.gen_range(0..16))).collect()
}

#[tauri::command]
pub async fn workshop_apply_map_preset(app: AppHandle, id: String) -> Result<String, String> {
    let preset = load_map_presets(&app)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("Map preset not found.")?;
    if let Some(inst) = read_installed(&app) {
        if inst.map_id == preset.map_id {
            return Ok(format!("{} is already loaded.", preset.map_name));
        }
    }
    let installed = if preset.map_id.starts_with("custom:") {
        let path = preset
            .source_path
            .clone()
            .ok_or("This preset's map file is missing — re-add the map and save the preset again.")?;
        if !Path::new(&path).is_file() {
            return Err("The preset's map file is gone (moved or deleted) — re-add the map and save the preset again.".into());
        }
        let config = crate::get_config(app.clone()).await?;
        if config.game_dir.is_empty() {
            return Err("Set your Rocket League folder in Settings first.".into());
        }
        install_map_bytes(&app, &config, Path::new(&path), Some(preset.map_name.clone())).await?
    } else {
        workshop_install_map(app.clone(), preset.map_id.clone(), preset.map_name.clone()).await?
    };
    crate::presets::append_history(&app, "preset_apply", &[], &format!("loaded map preset '{}' ({})", preset.name, installed.map_name));
    Ok(format!("{} loaded.", installed.map_name))
}

#[tauri::command]
pub fn workshop_delete_map_preset(app: AppHandle, id: String) -> Result<(), String> {
    let mut presets = load_map_presets(&app);
    presets.retain(|p| p.id != id);
    save_map_presets(&app, &presets);
    Ok(())
}

pub async fn import_local_zip(app: AppHandle, zip_path: &Path) -> Result<CustomMapEntry, String> {
    let zipfile = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(zipfile).map_err(|e| format!("read zip: {e}"))?;
    let mut picked: Option<(String, Vec<u8>)> = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if !(lower.ends_with(".udk") || lower.ends_with(".upk")) {
            continue;
        }
        if entry.size() < 1 << 16 || entry.size() > (512 << 20) {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        use std::io::Read;
        entry.read_to_end(&mut bytes).map_err(|e| format!("read {name}: {e}"))?;
        if bytes.len() >= 4 && u32::from_le_bytes(bytes[0..4].try_into().unwrap()) == 0x9E2A83C1 {
            picked = Some((name.rsplit('/').next().unwrap_or(&name).to_string(), bytes));
            break;
        }
    }
    let (map_file, bytes) =
        picked.ok_or("No usable .udk/.upk map file found inside the zip (must be a cooked UE3 package).")?;
    let cache_dir = app.path().app_config_dir().map_err(|e| e.to_string())?.join("map_cache");
    let maps_dir = cache_dir.join("maps");
    fs::create_dir_all(&maps_dir).map_err(|e| e.to_string())?;
    let safe_name = sanitize_map_name(&map_file);
    let dest = maps_dir.join(&safe_name);
    fs::write(&dest, &bytes).map_err(|e| format!("write map: {e}"))?;
    let display = safe_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| safe_name.clone());

    let mut lib = read_map_library(&app).unwrap_or_default();
    let path_str = dest.to_string_lossy().to_string();
    let existing = lib.iter_mut().find(|e| e.path == path_str);
    let entry = match existing {
        Some(e) => {
            e.name = display.clone();
            e.added_at = crate::now_iso8601_utc();
            e.clone()
        }
        None => {
            let e = CustomMapEntry {
                name: display.clone(),
                path: path_str.clone(),
                source_url: None,
                added_at: crate::now_iso8601_utc(),
                thumbnail_url: None,
            };
            lib.push(e.clone());
            e
        }
    };
    save_map_library(&app, &lib);
    let _ = fs::remove_file(zip_path);
    crate::applog::event(&format!("workshop: imported downloaded map '{display}'"));
    Ok(entry)
}

#[tauri::command]
pub async fn workshop_import_bakkes_zip(app: AppHandle, zip_url: String) -> Result<CustomMapEntry, String> {
    let url = zip_url.trim().to_string();
    let parsed_url = tauri::Url::parse(&url).map_err(|_| "Invalid download URL format.".to_string())?;
    if !is_bakkes_host(&parsed_url) {
        return Err("Not a valid bakkesplugins zip download URL.".into());
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&url, &mut hasher);
    let cache_name = format!("bp_{:016x}.zip", std::hash::Hasher::finish(&hasher));
    let cache_dir = app.path().app_config_dir().map_err(|e| e.to_string())?.join("map_cache");
    let zip_path = cache_dir.join(&cache_name);

    if !zip_path.is_file() {
        let client = reqwest::Client::builder()
            .user_agent(crate::app_user_agent())
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download failed: server {}", resp.status().as_u16()));
        }
        let total = resp.content_length().unwrap_or(0);
        if total > (512 << 20) as u64 {
            return Err("Zip is larger than 512 MB.".into());
        }
        fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
        let mut file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
        use std::io::Write;
        let mut downloaded: u64 = 0;
        let mut resp = resp;
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("download failed: {e}"))? {
            downloaded += chunk.len() as u64;
            if downloaded > (512 << 20) {
                let _ = fs::remove_file(&zip_path);
                return Err("Zip is larger than 512 MB.".into());
            }
            file.write_all(&chunk).map_err(|e| format!("cache write failed: {e}"))?;
            let pct = if total > 0 { (downloaded * 100 / total) as i32 } else { -1 };
            let _ = app.emit(
                "map-download-progress",
                serde_json::json!({ "downloaded": downloaded, "total": total, "percent": pct }),
            );
        }
        drop(file);
    }

    let fname = zip_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let zipfile = std::fs::File::open(&zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(zipfile).map_err(|e| format!("read zip: {e}"))?;
    let mut picked: Option<(String, Vec<u8>)> = None;
    let mut picked_img: Option<(String, Vec<u8>)> = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if picked_img.is_none() && (lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".webp")) && entry.size() < 10 << 20 {
            let mut img_bytes = Vec::with_capacity(entry.size() as usize);
            use std::io::Read;
            if entry.read_to_end(&mut img_bytes).is_ok() {
                picked_img = Some((name.clone(), img_bytes));
            }
        }
        if !(lower.ends_with(".udk") || lower.ends_with(".upk")) {
            continue;
        }
        if entry.size() < 1 << 16 || entry.size() > (512 << 20) {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        use std::io::Read;
        entry.read_to_end(&mut bytes).map_err(|e| format!("read {}: {e}", name))?;

        if bytes.len() >= 4 && u32::from_le_bytes(bytes[0..4].try_into().unwrap()) == 0x9E2A83C1 {
            picked = Some((name.rsplit('/').next().unwrap_or(&name).to_string(), bytes));
            if picked_img.is_some() {
                break;
            }
        }
    }
    let (map_file, bytes) =
        picked.ok_or("No usable .udk/.upk map file found inside the zip (must be a cooked UE3 package).")?;

    let maps_dir = cache_dir.join("maps");
    fs::create_dir_all(&maps_dir).map_err(|e| e.to_string())?;
    let safe_name = sanitize_map_name(&map_file);
    let dest = maps_dir.join(&safe_name);
    fs::write(&dest, &bytes).map_err(|e| format!("write map: {e}"))?;
    let display = safe_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or(safe_name.clone());

    let mut thumb_path: Option<String> = None;
    if let Some((img_name, img_bytes)) = picked_img {
        let ext = img_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("png");
        let stem = safe_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&safe_name);
        let thumb_filename = format!("{stem}_thumb.{ext}");
        let thumb_dest = maps_dir.join(&thumb_filename);
        if fs::write(&thumb_dest, &img_bytes).is_ok() {
            thumb_path = Some(thumb_dest.to_string_lossy().to_string());
        }
    }

    let mut lib = read_map_library(&app).unwrap_or_default();
    let path_str = dest.to_string_lossy().to_string();
    let existing = lib.iter_mut().find(|e| e.path == path_str);
    let entry = match existing {
        Some(e) => {
            e.name = display.clone();
            e.added_at = crate::now_iso8601_utc();
            if thumb_path.is_some() {
                e.thumbnail_url = thumb_path;
            }
            e.clone()
        }
        None => {
            let e = CustomMapEntry {
                name: display.clone(),
                path: path_str.clone(),
                source_url: Some(url.clone()),
                added_at: crate::now_iso8601_utc(),
                thumbnail_url: thumb_path,
            };
            lib.push(e.clone());
            e
        }
    };
    save_map_library(&app, &lib);
    crate::applog::event(&format!(
        "workshop: imported bakkesplugins map '{display}' from {fname}"
    ));
    Ok(entry)
}

#[tauri::command]
pub fn workshop_set_map_thumbnail(app: AppHandle, path_or_name: String, thumbnail_url: String) -> Result<(), String> {
    if let Some(mut lib) = read_map_library(&app) {
        let mut changed = false;
        for entry in lib.iter_mut() {
            if entry.path == path_or_name || entry.name == path_or_name {
                entry.thumbnail_url = Some(thumbnail_url.clone());
                changed = true;
            }
        }
        if changed {
            save_map_library(&app, &lib);
        }
    }
    if let Some(mut inst) = read_installed(&app) {
        if inst.map_name == path_or_name || inst.source_path.as_deref() == Some(&path_or_name) {
            inst.thumbnail_url = Some(thumbnail_url);
            if let Some(sp) = workshop_state_path(&app) {
                let _ = fs::write(&sp, serde_json::to_string(&inst).unwrap_or_default());
            }
        }
    }
    Ok(())
}

pub(crate) fn sanitize_map_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' })
        .collect()
}


pub fn is_bakkes_host(url: &tauri::Url) -> bool {
    let u = url.as_str().to_lowercase();
    if u.ends_with(".zip") || u.ends_with(".upk") || u.ends_with(".udk") || u.contains("/site/download") || u.contains("/download") {
        return true;
    }
    matches!(
        url.domain(),
        Some(d) if d == "bakkesplugins.com"
            || d.ends_with(".bakkesplugins.com")
            || d == "bakkesplugin.com"
            || d.ends_with(".bakkesplugin.com")
            || d == "rocket-league.com"
            || d.ends_with(".rocket-league.com")
            || d.ends_with(".r2.cloudflarestorage.com")
            || d.ends_with(".r2.dev")
            || d.ends_with(".cloudfront.net")
            || d.ends_with(".amazonaws.com")
    )
}

fn spawn_download_progress_watcher(app: tauri::AppHandle, path: std::path::PathBuf) {
    std::thread::spawn(move || {
        let mut last_len: u64 = 0;
        let mut idle_ticks: u32 = 0;
        let started = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let _ = app.emit(
                "map-download-progress",
                serde_json::json!({ "downloaded": len, "total": 0, "percent": -1 }),
            );
            if len > last_len {
                last_len = len;
                idle_ticks = 0;
            } else if len > 0 {
                idle_ticks += 1;
                if idle_ticks >= 8 {
                    break;
                }
            } else if started.elapsed() > std::time::Duration::from_secs(20) {
                break;
            }
        }
    });
}

pub fn bakkes_download_interceptor()
-> impl Fn(tauri::Webview<tauri::Wry>, tauri::webview::DownloadEvent<'_>) -> bool + Send + Sync + 'static {
    move |webview, event| {
        use tauri::webview::DownloadEvent;
        match event {
            DownloadEvent::Requested { url, destination } => {
                let u = url.as_str();
                let host_ok = is_bakkes_host(&url);
                if host_ok
                    && (u.to_lowercase().contains(".zip")
                        || u.to_lowercase().contains(".upk")
                        || u.to_lowercase().contains(".udk")
                        || u.contains("/download")
                        || url.domain().map_or(false, |d| d.starts_with("cdn.")))
                {
                    if let Ok(dir) = webview.app_handle().path().app_config_dir() {
                        let stamp = crate::presets::utc_filename_stamp();
                        *destination = dir.join("map_cache").join(format!("bp_dl_{stamp}.zip"));
                        let _ = std::fs::create_dir_all(destination.parent().unwrap_or(&dir));
                        spawn_download_progress_watcher(
                            webview.app_handle().clone(),
                            destination.clone(),
                        );
                    }
                }
                true
            }
            DownloadEvent::Finished { url, path, success } => {
                if success {
                    let u = url.as_str();
                    if is_bakkes_host(&url)
                        && (u.to_lowercase().contains(".zip")
                            || u.to_lowercase().contains(".upk")
                            || u.to_lowercase().contains(".udk")
                            || u.contains("/download")
                            || url.domain().map_or(false, |d| d.starts_with("cdn.")))
                    {
                        let app_handle = webview.app_handle().clone();
                        let _ = app_handle.emit(
                            "map-download-progress",
                            serde_json::json!({ "downloaded": 0, "total": 0, "percent": 100, "phase": "install" }),
                        );
                        if let Some(p) = path {
                            let app2 = webview.app_handle().clone();
                            tauri::async_runtime::spawn(async move {
                                let payload = match import_local_zip(app2.clone(), &p).await {
                                    Ok(entry) => serde_json::json!({
                                        "name": entry.name,
                                        "path": entry.path
                                    }),
                                    Err(e) => serde_json::json!({ "error": e }),
                                };
                                let _ = app2.emit("bakkes-map-imported", payload);
                            });
                        }
                    }
                }
                true
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_session_verifies() {
        let tok = mint_session_token();
        let payload = verify_session_token(&tok).expect("minted token must verify");
        assert_eq!(payload.get("steam_id").and_then(|v: &serde_json::Value| v.as_str()), Some("local"));
    }

    #[test]
    fn forged_session_rejected() {
        type HmacSha256 = Hmac<Sha256>;
        let payload = serde_json::json!({ "steam_id": "x", "exp": now_secs() + 60 });
        let p_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let mut mac = HmacSha256::new_from_slice(b"wrong-key").unwrap();
        mac.update(format!("v1.{}", p_b64).as_bytes());
        let bad = format!("v1.{}.{}", p_b64, URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()));
        assert!(verify_session_token(&bad).is_err());
    }
}
