use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

use crate::{app_config_dir_of, SwapEntry};

type HmacSha256 = Hmac<Sha256>;

const SHARE_KEY: &[u8] = b"VelocityRL::preset-share::v1::A0A523581C0125D1";

pub const MAX_PRESETS: usize = 50;
pub const MAX_HISTORY: usize = 200;
pub const MAX_PRESET_ITEMS: usize = 100;
pub const MAX_PRESET_MAPS: usize = 30;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PresetMapEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub swaps: Vec<SwapEntry>,
    #[serde(default)]
    pub maps: Vec<PresetMapEntry>,
    #[serde(default)]
    pub active_map_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub at: String,
    pub kind: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub swaps: Vec<SwapEntry>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct PresetFile {
    #[serde(default)]
    presets: Vec<Preset>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct HistoryFile {
    #[serde(default)]
    history: Vec<HistoryEntry>,
}

fn presets_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app_config_dir_of(app).map(|d| d.join("presets.json"))
}

fn history_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app_config_dir_of(app).map(|d| d.join("history.json"))
}

fn load_preset_file(app: &tauri::AppHandle) -> PresetFile {
    presets_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_preset_file(app: &tauri::AppHandle, f: &PresetFile) {
    if let Some(path) = presets_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(f) {
            let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = fs::write(path, json);
        }
    }
}

fn load_history_file(app: &tauri::AppHandle) -> HistoryFile {
    history_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history_file(app: &tauri::AppHandle, f: &HistoryFile) {
    if let Some(path) = history_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(f) {
            let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = fs::write(path, json);
        }
    }
}

pub fn utc_filename_stamp() -> String {
    crate::now_iso8601_utc().replace(|c| c == ':' || c == '-', "")
}

pub fn append_history(app: &tauri::AppHandle, kind: &str, swaps: &[SwapEntry], note: &str) {
    let mut f = load_history_file(app);
    f.history.push(HistoryEntry {
        at: crate::now_iso8601_utc(),
        kind: kind.to_string(),
        note: note.to_string(),
        swaps: swaps.to_vec(),
    });
    let len = f.history.len();
    if len > MAX_HISTORY {
        f.history.drain(0..len - MAX_HISTORY);
    }
    save_history_file(app, &f);
}

fn sign_preset_payload(payload: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(SHARE_KEY).expect("hmac key");
    mac.update(payload);
    mac.finalize().into_bytes().to_vec()
}

fn generate_preset_uuid() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..8).map(|_| format!("{:x}", rng.gen_range(0..16))).collect()
}

#[tauri::command]
pub async fn get_presets(app: tauri::AppHandle) -> Result<Vec<Preset>, String> {
    Ok(load_preset_file(&app).presets)
}

#[tauri::command]
pub async fn save_preset(
    app: tauri::AppHandle,
    name: String,
    maps: Option<Vec<PresetMapEntry>>,
    swaps: Option<Vec<SwapEntry>>,
) -> Result<Preset, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err("Preset name must be 1–64 characters.".into());
    }
    let mut current_swaps = swaps.unwrap_or_else(|| crate::load_swaps(&app));

    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();
    for s in &mut current_swaps {
        if s.owned_name.is_empty() {
            if let Some(it) = items.iter().find(|i| i.id == s.owned_id) {
                s.owned_name = it.product.clone();
                if s.asset_package.is_empty() {
                    s.asset_package = it.asset_package.clone();
                }
            }
        }
        if s.wanted_name.is_empty() {
            if let Some(it) = items.iter().find(|i| i.id == s.wanted_id) {
                s.wanted_name = it.product.clone();
            }
        }
    }

    let mut preset_maps = maps.unwrap_or_default();
    let mut active_map_id = None;

    if preset_maps.is_empty() {
        if let Some(inst) = crate::workshop::read_installed(&app) {
            preset_maps.push(PresetMapEntry {
                id: inst.map_id.clone(),
                name: inst.map_name.clone(),
                download_url: None,
                thumbnail_url: inst.thumbnail_url.clone(),
                source_path: inst.source_path.clone(),
            });
            active_map_id = Some(inst.map_id);
        }
    } else {
        active_map_id = preset_maps.first().map(|m| m.id.clone());
    }

    if current_swaps.is_empty() && preset_maps.is_empty() {
        return Err("No active swaps or maps to save as a preset. Set up swaps first.".into());
    }
    if current_swaps.len() > MAX_PRESET_ITEMS {
        return Err(format!("Preset exceeds maximum limit of {MAX_PRESET_ITEMS} items (has {}).", current_swaps.len()));
    }
    if preset_maps.len() > MAX_PRESET_MAPS {
        return Err(format!("Preset exceeds maximum limit of {MAX_PRESET_MAPS} maps (has {}).", preset_maps.len()));
    }

    let mut f = load_preset_file(&app);
    let existing_idx = f.presets.iter().position(|p| p.name.trim().eq_ignore_ascii_case(&name));
    if let Some(idx) = existing_idx {
        let p = &mut f.presets[idx];
        p.name = name.clone();
        p.swaps = current_swaps.clone();
        p.maps = preset_maps;
        p.active_map_id = active_map_id;
        p.created_at = crate::now_iso8601_utc();
        let updated = p.clone();
        save_preset_file(&app, &f);
        append_history(&app, "preset_save", &current_swaps, &format!("updated preset '{name}'"));
        return Ok(updated);
    }

    if f.presets.len() >= MAX_PRESETS {
        return Err(format!("Preset limit reached ({MAX_PRESETS}). Delete one first."));
    }

    let preset = Preset {
        id: generate_preset_uuid(),
        name: name.clone(),
        created_at: crate::now_iso8601_utc(),
        swaps: current_swaps.clone(),
        maps: preset_maps,
        active_map_id,
    };
    f.presets.push(preset.clone());
    save_preset_file(&app, &f);
    append_history(&app, "preset_save", &current_swaps, &format!("saved preset '{name}'"));
    Ok(preset)
}

#[tauri::command]
pub async fn delete_preset(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let mut f = load_preset_file(&app);
    f.presets.retain(|p| p.id != id);
    save_preset_file(&app, &f);
    Ok(())
}

#[tauri::command]
pub async fn apply_preset(app: tauri::AppHandle, id: String) -> Result<Vec<String>, String> {
    let f = load_preset_file(&app);
    let preset = f
        .presets
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "Preset not found.".to_string())?;
    if preset.swaps.is_empty() && preset.maps.is_empty() {
        return Err("Preset has no swaps or maps.".into());
    }
    if crate::psynet::is_rocket_league_running() {
        return Err("Rocket League is running — close it before applying a preset.".into());
    }

    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }

    let mut results = Vec::new();
    let mut applied: Vec<SwapEntry> = Vec::new();

    if !preset.swaps.is_empty() {
        let _ = crate::get_items(app.clone(), None).await;
        let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let items_json = fs::read_to_string(config_dir.join("items.json"))
            .map_err(|_| "Items database missing — check your internet connection and try again.".to_string())?;
        let game_dir = crate::upk::palette::resolve_cooked_dir(std::path::Path::new(&config.game_dir))
            .unwrap_or_else(|_| config.game_dir.clone().into());
        let opts = crate::build_swap_opts(game_dir.clone(), items_json);
        let items = crate::get_items(app.clone(), None).await.unwrap_or_default();

        for s in &preset.swaps {
            let pkg = if !s.asset_package.is_empty() {
                s.asset_package.clone()
            } else {
                items
                    .iter()
                    .find(|i| i.id == s.owned_id)
                    .map(|i| i.asset_package.clone())
                    .unwrap_or_default()
            };
            if !pkg.is_empty() {
                let bak = crate::integrity::bak_path_for(&game_dir.join(&pkg));
                if bak.exists() {
                    if let Some(bak_s) = bak.to_str() {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            crate::upk::restore_single(bak_s)
                        }));
                    }
                }
            }
            let paint = if (0..=12).contains(&s.paint_id) { s.paint_id } else { 0 };
            let label = format!(
                "{} → {}{}",
                s.owned_name,
                s.wanted_name,
                if paint > 0 { format!(" ({paint})") } else { String::new() }
            );
            match crate::run_swap_caught(&s.owned_id.to_string(), &s.wanted_id.to_string(), paint, &opts) {
                Ok(_) => {
                    results.push(format!("OK  {label}"));
                    applied.push(s.clone());
                }
                Err(e) => results.push(format!("FAIL  {label}  ({e})")),
            }
        }

        let mut swaps = crate::load_swaps(&app);
        for s in applied.iter() {
            swaps.retain(|x| x.owned_id != s.owned_id);
            swaps.push(s.clone());
        }
        crate::save_swaps(&app, &swaps);
    }

    let target_map_id = preset.active_map_id.as_ref().or_else(|| preset.maps.first().map(|m| &m.id));
    if let Some(id) = target_map_id {
        if let Ok(lib) = crate::workshop::workshop_get_map_library(app.clone()) {
            if let Some(entry) = lib.iter().find(|e| &e.name == id || e.path.contains(id) || preset.maps.iter().any(|m| &m.id == id && m.name == e.name)) {
                if std::path::Path::new(&entry.path).is_file() {
                    match crate::workshop::workshop_install_from_library(app.clone(), entry.path.clone()).await {
                        Ok(inst) => results.push(format!("OK  Loaded map: {}", inst.map_name)),
                        Err(e) => results.push(format!("FAIL  Map: {e}")),
                    }
                }
            }
        }
    }

    append_history(
        &app,
        "preset_apply",
        &applied,
        &format!("applied preset '{}'", preset.name),
    );

    let fails = results.iter().filter(|r| r.starts_with("FAIL")).count();
    if fails > 0 && applied.is_empty() && results.len() == fails {
        return Err(format!("All {} actions failed:\n{}", results.len(), results.join("\n")));
    }
    Ok(results)
}

fn code_for_preset(p: &Preset) -> Result<String, String> {
    let payload = serde_json::json!({
        "v": 1,
        "name": p.name,
        "swaps": p.swaps,
        "maps": p.maps,
        "active_map_id": p.active_map_id,
    })
    .to_string();
    let sig = sign_preset_payload(payload.as_bytes());
    Ok(format!(
        "1.{}.{}",
        B64.encode(payload.as_bytes()),
        B64.encode(sig)
    ))
}

#[tauri::command]
pub async fn peek_preset_code(code: String) -> Result<Preset, String> {
    let (name, swaps, maps, active_map_id) = parse_code(&code)?;
    Ok(Preset {
        id: String::new(),
        name,
        created_at: String::new(),
        swaps,
        maps,
        active_map_id,
    })
}

fn parse_code(code: &str) -> Result<(String, Vec<SwapEntry>, Vec<PresetMapEntry>, Option<String>), String> {
    let code = code.trim();
    let rest = code
        .strip_prefix("1.")
        .ok_or("Not a valid preset code.")?;
    let (payload_b64, sig_b64) = rest
        .split_once('.')
        .ok_or("Malformed preset code: missing signature.")?;
    let payload = B64
        .decode(payload_b64)
        .map_err(|_| "Malformed preset code: bad payload.")?;
    let sig = B64
        .decode(sig_b64)
        .map_err(|_| "Malformed preset code: bad signature.")?;
    if sig != sign_preset_payload(&payload) {
        return Err("Preset code failed verification — it was modified or corrupted.".into());
    }
    let val: serde_json::Value =
        serde_json::from_slice(&payload).map_err(|_| "Malformed preset code payload.")?;
    if val.get("v").and_then(|v| v.as_i64()) != Some(1) {
        return Err("Unsupported preset code version.".into());
    }
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Shared preset")
        .to_string();
    let swaps: Vec<SwapEntry> = serde_json::from_value(val.get("swaps").cloned().unwrap_or_default())
        .map_err(|_| "Preset code contains invalid swaps.")?;
    let maps: Vec<PresetMapEntry> = serde_json::from_value(val.get("maps").cloned().unwrap_or_default())
        .unwrap_or_default();
    let active_map_id: Option<String> = val.get("active_map_id").and_then(|v| v.as_str()).map(|s| s.to_string());

    if swaps.len() > MAX_PRESET_ITEMS {
        return Err(format!("Preset exceeds maximum limit of {MAX_PRESET_ITEMS} items (has {}).", swaps.len()));
    }
    if maps.len() > MAX_PRESET_MAPS {
        return Err(format!("Preset exceeds maximum limit of {MAX_PRESET_MAPS} maps (has {}).", maps.len()));
    }

    Ok((name, swaps, maps, active_map_id))
}

#[tauri::command]
pub async fn export_preset_code(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let f = load_preset_file(&app);
    let preset = f
        .presets
        .iter()
        .find(|p| p.id == id)
        .ok_or("Preset not found.")?;
    code_for_preset(preset)
}

#[tauri::command]
pub async fn import_preset_code(
    app: tauri::AppHandle,
    code: String,
) -> Result<Preset, String> {
    let (name, swaps, maps, active_map_id) = parse_code(&code)?;
    if swaps.is_empty() && maps.is_empty() {
        return Err("Preset code contains no swaps or maps.".into());
    }
    let mut f = load_preset_file(&app);
    if f.presets.len() >= MAX_PRESETS {
        return Err(format!("Preset limit reached ({MAX_PRESETS}). Delete one first."));
    }
    let existing = f.presets.iter_mut().find(|p| p.name == name).map(|p| {
        p.swaps = swaps.clone();
        p.maps = maps.clone();
        p.active_map_id = active_map_id.clone();
        p.created_at = crate::now_iso8601_utc();
        p.clone()
    });
    if let Some(updated) = existing {
        save_preset_file(&app, &f);
        append_history(&app, "preset_save", &swaps, &format!("imported preset '{name}' (overwrote)"));
        return Ok(updated);
    }
    let preset = Preset {
        id: generate_preset_uuid(),
        name,
        created_at: crate::now_iso8601_utc(),
        swaps: swaps.clone(),
        maps,
        active_map_id,
    };
    f.presets.push(preset.clone());
    save_preset_file(&app, &f);
    append_history(&app, "preset_save", &swaps, "imported preset from code");
    Ok(preset)
}

#[tauri::command]
pub async fn preset_download_missing_maps(
    app: tauri::AppHandle,
    maps: Vec<PresetMapEntry>,
) -> Result<Vec<String>, String> {
    use tauri::Emitter;
    let mut results = Vec::new();
    let library = crate::workshop::workshop_get_map_library(app.clone()).unwrap_or_default();

    for (i, m) in maps.iter().enumerate() {
        let already = library.iter().any(|entry| {
            entry.name.eq_ignore_ascii_case(&m.name)
                || (m.download_url.is_some() && entry.source_url == m.download_url)
                || (!m.id.is_empty() && entry.path.contains(&m.id) && std::path::Path::new(&entry.path).is_file())
        });
        if already {
            results.push(format!("ALREADY  {}", m.name));
            continue;
        }

        let _ = app.emit("preset-map-download-progress", serde_json::json!({
            "map_name": &m.name,
            "index": i + 1,
            "total": maps.len(),
            "status": "downloading"
        }));

        if let Some(ref url) = m.download_url {
            let u_lower = url.to_lowercase();
            if u_lower.contains(".zip") || u_lower.contains("bakkesplugins.com") {
                match crate::workshop::workshop_import_bakkes_zip(app.clone(), url.clone()).await {
                    Ok(_) => {
                        results.push(format!("OK  {}", m.name));
                        continue;
                    }
                    Err(e) => {
                        crate::applog::event(&format!("preset: import bakkes zip failed for {}: {e}", m.name));
                    }
                }
            } else if u_lower.ends_with(".upk") || u_lower.ends_with(".udk") {
                match download_and_save_map_package(&app, url, &m.name).await {
                    Ok(_) => {
                        results.push(format!("OK  {}", m.name));
                        continue;
                    }
                    Err(e) => {
                        crate::applog::event(&format!("preset: download upk failed for {}: {e}", m.name));
                    }
                }
            }
        }

        if !m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_digit()) {
            let api_url = format!("https://api.velocityrl.tech/v2/rl/workshop/maps/{}/download", m.id);
            match download_and_save_map_package(&app, &api_url, &m.name).await {
                Ok(_) => {
                    results.push(format!("OK  {}", m.name));
                    continue;
                }
                Err(e) => {
                    crate::applog::event(&format!("preset: download api map failed for {}: {e}", m.name));
                }
            }
        }

        results.push(format!("SKIP  {} (no download source)", m.name));
    }

    Ok(results)
}

async fn download_and_save_map_package(
    app: &tauri::AppHandle,
    url: &str,
    map_name: &str,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| format!("network: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() < 1 << 16 {
        return Err("downloaded package too small".into());
    }
    let cache_dir = app.path().app_config_dir().map_err(|e| e.to_string())?.join("map_cache").join("maps");
    let _ = fs::create_dir_all(&cache_dir);
    let safe_name = crate::workshop::sanitize_map_name(&format!("{map_name}.upk"));
    let dest = cache_dir.join(&safe_name);
    fs::write(&dest, &bytes).map_err(|e| format!("write: {e}"))?;

    let mut lib = crate::workshop::workshop_get_map_library(app.clone()).unwrap_or_default();
    let path_str = dest.to_string_lossy().to_string();
    if !lib.iter().any(|e| e.path == path_str) {
        lib.push(crate::workshop::CustomMapEntry {
            name: map_name.to_string(),
            path: path_str,
            source_url: Some(url.to_string()),
            added_at: crate::now_iso8601_utc(),
            thumbnail_url: None,
        });
        crate::workshop::save_map_library(app, &lib);
    }
    Ok(())
}

#[tauri::command]
pub async fn random_swap_plan(
    app: tauri::AppHandle,
    owned_ids: Vec<i32>,
) -> Result<Vec<SwapEntry>, String> {
    use rand::seq::SliceRandom;
    let items = crate::get_items(app.clone(), None).await?;
    let swappable: Vec<&crate::Item> = items.iter().filter(|i| !crate::is_non_swappable(i)).collect();
    let mut rng = rand::thread_rng();

    let categories: [(&str, i32); 7] = [
        ("body", 23),
        ("wheels", 376),
        ("rocketboost", 63),
        ("goalexplosion", 1903),
        ("trail", 1948),
        ("topper", 232),
        ("antenna", 16),
    ];

    let mut plan = Vec::new();

    for &(slot, default_donor_id) in &categories {

        let donor = owned_ids
            .iter()
            .find_map(|oid| {
                items.iter().find(|i| i.id == *oid && crate::norm_item_slot(&i.slot) == slot)
            })
            .or_else(|| {

                items.iter().find(|i| i.id == default_donor_id)
            })
            .or_else(|| {

                swappable.iter().find(|i| crate::norm_item_slot(&i.slot) == slot && i.quality.eq_ignore_ascii_case("Common")).copied()
            })
            .or_else(|| {

                swappable.iter().find(|i| crate::norm_item_slot(&i.slot) == slot).copied()
            });

        let Some(donor) = donor else { continue };

        let candidates: Vec<&&crate::Item> = swappable
            .iter()
            .filter(|c| crate::norm_item_slot(&c.slot) == slot && c.id != donor.id)
            .collect();

        if candidates.is_empty() {
            continue;
        }

        let premium_candidates: Vec<&&&crate::Item> = candidates
            .iter()
            .filter(|c| !c.quality.eq_ignore_ascii_case("Common"))
            .collect();

        let pick = if !premium_candidates.is_empty() {
            ***premium_candidates.choose(&mut rng).unwrap()
        } else {
            **candidates.choose(&mut rng).unwrap()
        };

        plan.push(SwapEntry {
            owned_id: donor.id,
            wanted_id: pick.id,
            owned_name: donor.product.clone(),
            wanted_name: pick.product.clone(),
            paint_id: 0,
            asset_package: donor.asset_package.clone(),
        });
    }

    if plan.is_empty() {
        return Err("Could not generate a random car loadout. Check your items database.".into());
    }

    Ok(plan)
}

#[tauri::command]
pub async fn apply_swap_plan(
    app: tauri::AppHandle,
    plan: Vec<SwapEntry>,
) -> Result<Vec<String>, String> {
    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    if crate::psynet::is_rocket_league_running() {
        return Err("Rocket League is running — close it first.".into());
    }
    let _ = crate::get_items(app.clone(), None).await;
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let items_json = fs::read_to_string(config_dir.join("items.json"))
        .map_err(|_| "Items database missing — check your internet connection and try again.".to_string())?;
    let game_dir = crate::upk::palette::resolve_cooked_dir(std::path::Path::new(&config.game_dir))
        .unwrap_or_else(|_| config.game_dir.clone().into());
    let opts = crate::build_swap_opts(game_dir.clone(), items_json);
    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();

    let mut results = Vec::new();
    let mut applied: Vec<SwapEntry> = Vec::new();
    for s in &plan {
        let pkg = if !s.asset_package.is_empty() {
            s.asset_package.clone()
        } else {
            items
                .iter()
                .find(|i| i.id == s.owned_id)
                .map(|i| i.asset_package.clone())
                .unwrap_or_default()
        };
        if !pkg.is_empty() {
            let bak = crate::integrity::bak_path_for(&game_dir.join(&pkg));
            if bak.exists() {
                if let Some(bak_s) = bak.to_str() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::upk::restore_single(bak_s)
                    }));
                }
            }
        }
        let label = format!("{} → {}", s.owned_name, s.wanted_name);
        match crate::run_swap_caught(&s.owned_id.to_string(), &s.wanted_id.to_string(), 0, &opts) {
            Ok(_) => {
                results.push(format!("OK  {label}"));
                applied.push(s.clone());
            }
            Err(e) => results.push(format!("FAIL  {label}  ({e})")),
        }
    }

    let mut swaps = crate::load_swaps(&app);
    for s in applied.iter() {
        swaps.retain(|x| x.owned_id != s.owned_id);
        swaps.push(s.clone());
    }
    crate::save_swaps(&app, &swaps);
    append_history(&app, "random", &applied, "random loadout applied");

    let fails = results.iter().filter(|r| r.starts_with("FAIL")).count();
    if fails > 0 && applied.is_empty() {
        return Err(format!("All {} swaps failed:\n{}", results.len(), results.join("\n")));
    }
    Ok(results)
}

#[tauri::command]
pub async fn get_swap_history(app: tauri::AppHandle) -> Result<Vec<HistoryEntry>, String> {
    Ok(load_history_file(&app).history)
}

#[tauri::command]
pub async fn clear_swap_history(app: tauri::AppHandle) -> Result<(), String> {
    save_history_file(&app, &HistoryFile::default());
    Ok(())
}
