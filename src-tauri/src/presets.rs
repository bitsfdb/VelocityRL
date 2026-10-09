/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
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
pub const MAX_PRESET_ITEMS: usize = 50;
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

pub fn sanitize_swaps(swaps: &mut [SwapEntry], items: &[crate::Item]) -> bool {
    let mut changed = false;
    for s in swaps {
        let owned = items.iter().find(|i| i.id == s.owned_id);
        let wanted = items.iter().find(|i| i.id == s.wanted_id);

        if s.owned_name.trim().is_empty() {
            if let Some(it) = owned {
                s.owned_name = it.product.clone();
                changed = true;
            }
        }
        if s.wanted_name.trim().is_empty() {
            if let Some(it) = wanted {
                s.wanted_name = it.product.clone();
                changed = true;
            }
        }
        if s.asset_package.trim().is_empty() {
            if let Some(it) = wanted.or(owned) {
                if !it.asset_package.is_empty() && it.asset_package != "None" {
                    s.asset_package = it.asset_package.clone();
                    changed = true;
                }
            }
        }
        let real_slot = wanted.map(|i| &i.slot).or_else(|| owned.map(|i| &i.slot));
        if let Some(rs) = real_slot {
            let rs_trimmed = rs.trim();
            if !rs_trimmed.is_empty() {
                let current_slot_empty = match &s.slot {
                    None => true,
                    Some(sl) => sl.trim().is_empty() || sl.trim().eq_ignore_ascii_case("item"),
                };
                if current_slot_empty {
                    s.slot = Some(rs_trimmed.to_string());
                    changed = true;
                } else if let Some(cur_sl) = &s.slot {
                    if slot_index_from_str(cur_sl) != slot_index_from_str(rs_trimmed) {
                        s.slot = Some(rs_trimmed.to_string());
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

#[tauri::command]
pub async fn get_presets(app: tauri::AppHandle) -> Result<Vec<Preset>, String> {
    let mut f = load_preset_file(&app);
    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();
    let mut dirty = false;
    for p in &mut f.presets {
        if sanitize_swaps(&mut p.swaps, &items) {
            dirty = true;
        }
    }
    if dirty {
        save_preset_file(&app, &f);
    }
    Ok(f.presets)
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
    sanitize_swaps(&mut current_swaps, &items);

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

    let config = crate::get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }

    let cooked = crate::upk::palette::resolve_cooked_dir(std::path::Path::new(&config.game_dir))
        .unwrap_or_else(|_| std::path::PathBuf::from(&config.game_dir));

    let mut results = Vec::new();
    let mut applied: Vec<SwapEntry> = Vec::new();

    let mut current_swaps = preset.swaps.clone();
    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();
    sanitize_swaps(&mut current_swaps, &items);

    for s in &current_swaps {
        let paint_str = if s.paint_id > 0 {
            format!(" ({})", crate::upk::swapper::paint_label(s.paint_id))
        } else {
            String::new()
        };
        results.push(format!("OK  {} → {}{}", s.owned_name, s.wanted_name, paint_str));
        applied.push(s.clone());
    }

    crate::save_swaps(&app, &current_swaps);
    if let Err(e) = crate::sync_all_swaps_to_tagame(&app, &cooked, &current_swaps).await {
        return Err(format!("Failed to apply preset swaps: {e}"));
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

#[allow(dead_code)]
pub const COSMETIC_SLOTS_COUNT: usize = 14;

pub fn slot_index_from_str(slot: &str) -> usize {
    let clean = slot.to_lowercase().replace([' ', '_', '-'], "");
    match clean.as_str() {
        "body" | "bodies" => 0,
        "skin" | "decal" | "decals" => 1,
        "wheel" | "wheels" => 2,
        "boost" | "boosts" | "rocketboost" | "rocketboosts" => 3,
        "antenna" | "antennas" => 4,
        "topper" | "toppers" | "hat" | "hats" => 5,
        "paintfinish" | "paintfinishes" | "paint" | "paints" | "finish" | "finishes" => 6,
        "paintfinishsecondary" | "paintfinishaccent" | "accentpaint" | "paintaccent" | "accent" | "accents" => 7,
        "engineaudio" | "audio" | "audios" | "engine" | "engines" => 8,
        "trail" | "trails" | "friction" => 9,
        "goalexplosion" | "goalexplosions" | "explosion" | "explosions" | "ge" => 10,
        "playerbanner" | "playerbanners" | "banner" | "banners" => 11,
        "playeranthem" | "playeranthems" | "anthem" | "anthems" | "music" | "track" => 12,
        "avatarborder" | "avatarborders" | "border" | "borders" => 13,
        _ => {
            if clean.contains("goal") || clean.contains("explosion") {
                10
            } else if clean.contains("trail") {
                9
            } else if clean.contains("audio") || clean.contains("engine") {
                8
            } else if clean.contains("accent") {
                7
            } else if clean.contains("paint") || clean.contains("finish") {
                6
            } else if clean.contains("topper") || clean.contains("hat") {
                5
            } else if clean.contains("antenna") {
                4
            } else if clean.contains("boost") {
                3
            } else if clean.contains("wheel") {
                2
            } else if clean.contains("decal") || clean.contains("skin") {
                1
            } else if clean.contains("banner") {
                11
            } else if clean.contains("anthem") || clean.contains("music") {
                12
            } else if clean.contains("border") {
                13
            } else if clean.contains("body") {
                0
            } else {
                0
            }
        }
    }
}

pub fn slot_name_from_index(idx: usize) -> &'static str {
    match idx {
        0 => "Body",
        1 => "Decal",
        2 => "Wheels",
        3 => "Boost",
        4 => "Antenna",
        5 => "Topper",
        6 => "Paint Finish",
        7 => "Paint Finish (Accent)",
        8 => "Engine Audio",
        9 => "Trail",
        10 => "Goal Explosion",
        11 => "Player Banner",
        12 => "Player Anthem",
        13 => "Avatar Border",
        _ => "Item",
    }
}

pub fn default_donor_for_slot(idx: usize) -> (i32, &'static str) {
    match idx {
        0 => (23, "Octane"),
        1 => (0, "Decal"),
        2 => (376, "OEM"),
        3 => (63, "Standard"),
        4 => (16, "Antenna"),
        5 => (232, "Halo"),
        6 => (0, "Paint Finish"),
        7 => (0, "Paint Finish"),
        8 => (0, "Engine Audio"),
        9 => (1948, "Classic"),
        10 => (1903, "Standard"),
        11 => (0, "Player Banner"),
        12 => (0, "Player Anthem"),
        13 => (0, "Avatar Border"),
        _ => (0, "Item"),
    }
}

pub fn encode_14slot_binary(swaps: &[SwapEntry]) -> String {
    let mut payload = [0u8; 42];
    for s in swaps {
        let slot_str = s.slot.as_deref().unwrap_or("").trim();
        if slot_str.is_empty() || slot_str.eq_ignore_ascii_case("item") {
            continue;
        }
        let idx = slot_index_from_str(slot_str);
        if idx < 14 {
            let item_id = if s.wanted_id > 0 { s.wanted_id as u16 } else { s.owned_id as u16 };
            let pid = (s.paint_id.max(0).min(255)) as u8;
            payload[idx * 3] = (item_id & 0xFF) as u8;
            payload[idx * 3 + 1] = ((item_id >> 8) & 0xFF) as u8;
            payload[idx * 3 + 2] = pid;
        }
    }
    let hmac_full = sign_preset_payload(&payload);
    let mut combined = [0u8; 58];
    combined[..42].copy_from_slice(&payload);
    combined[42..58].copy_from_slice(&hmac_full[..16]);
    B64.encode(combined)
}

fn parse_legacy_v1_code(rest: &str) -> Result<(String, Vec<SwapEntry>, Vec<PresetMapEntry>, Option<String>), String> {
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
        return Err(format!("Invalid preset code: contains more than {MAX_PRESET_ITEMS} swaps."));
    }
    if maps.len() > MAX_PRESET_MAPS {
        return Err(format!("Invalid preset code: contains more than {MAX_PRESET_MAPS} maps."));
    }
    Ok((name, swaps, maps, active_map_id))
}

fn parse_zlib_code(rest: &str) -> Result<(String, Vec<SwapEntry>, Vec<PresetMapEntry>, Option<String>), String> {
    let (payload_b64, sig_b64) = rest
        .split_once('.')
        .ok_or("Malformed zlib preset code: missing signature.")?;
    let comp_bytes = B64.decode(payload_b64).map_err(|_| "Invalid base64 zlib payload")?;
    let sig = B64.decode(sig_b64).map_err(|_| "Invalid base64 signature")?;
    if sig != sign_preset_payload(&comp_bytes) {
        return Err("Preset code signature verification failed.".into());
    }
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut dec = ZlibDecoder::new(&comp_bytes[..]);
    let mut decomp = Vec::new();
    dec.read_to_end(&mut decomp).map_err(|e| format!("Failed to decompress zlib preset: {e}"))?;
    let val: serde_json::Value = serde_json::from_slice(&decomp).map_err(|e| format!("Invalid JSON: {e}"))?;
    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("Shared preset").to_string();
    let swaps: Vec<SwapEntry> = serde_json::from_value(val.get("swaps").cloned().unwrap_or_default()).unwrap_or_default();
    let maps: Vec<PresetMapEntry> = serde_json::from_value(val.get("maps").cloned().unwrap_or_default()).unwrap_or_default();
    let active_map_id = val.get("active_map_id").and_then(|v| v.as_str()).map(|s| s.to_string());
    if swaps.len() > MAX_PRESET_ITEMS {
        return Err(format!("Invalid preset code: contains more than {MAX_PRESET_ITEMS} swaps."));
    }
    if maps.len() > MAX_PRESET_MAPS {
        return Err(format!("Invalid preset code: contains more than {MAX_PRESET_MAPS} maps."));
    }
    Ok((name, swaps, maps, active_map_id))
}

fn code_for_preset(p: &Preset) -> Result<String, String> {
    if p.maps.is_empty() {
        Ok(encode_14slot_binary(&p.swaps))
    } else {
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
}

#[tauri::command]
pub async fn peek_preset_code(app: tauri::AppHandle, code: String) -> Result<Preset, String> {
    let (name, mut swaps, maps, active_map_id) = parse_code(&code)?;
    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();
    sanitize_swaps(&mut swaps, &items);
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

    if let Some(rest) = code.strip_prefix("1.") {
        return parse_legacy_v1_code(rest);
    }

    if let Some(rest) = code.strip_prefix("zlib.").or_else(|| code.strip_prefix("z.")) {
        return parse_zlib_code(rest);
    }

    let clean = code.strip_prefix("2.").or_else(|| code.strip_prefix("v2.")).unwrap_or(code);
    if let Ok(raw) = B64.decode(clean) {
        if raw.len() == 58 {
            let payload = &raw[..42];
            let sig = &raw[42..58];
            let expected = sign_preset_payload(payload);
            if &expected[..16] != sig {
                return Err("Preset code failed verification — invalid signature.".into());
            }

            let mut swaps = Vec::new();
            for i in 0..14 {
                let item_id = u16::from_le_bytes([payload[i * 3], payload[i * 3 + 1]]) as i32;
                let paint_id = payload[i * 3 + 2] as i32;
                if item_id > 0 {
                    let slot_name = slot_name_from_index(i).to_string();
                    let (donor_id, donor_name) = default_donor_for_slot(i);
                    let actual_donor_id = if donor_id > 0 { donor_id } else { item_id };
                    swaps.push(SwapEntry {
                        owned_id: actual_donor_id,
                        wanted_id: item_id,
                        owned_name: donor_name.to_string(),
                        wanted_name: String::new(),
                        owned_paint_id: None,
                        owned_custom_hex: None,
                        paint_id,
                        custom_paint_hex: None,
                        asset_package: String::new(),
                        slot: Some(slot_name),
                        timestamp: None,
                    });
                }
            }

            return Ok(("Shared Preset".to_string(), swaps, Vec::new(), None));
        }
    }

    Err("Not a valid preset code.".into())
}

#[tauri::command]
pub async fn export_preset_code(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let mut f = load_preset_file(&app);
    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();
    let preset = f
        .presets
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("Preset not found.")?;
    sanitize_swaps(&mut preset.swaps, &items);
    code_for_preset(preset)
}

#[tauri::command]
pub async fn import_preset_code(
    app: tauri::AppHandle,
    code: String,
) -> Result<Preset, String> {
    let (name, mut swaps, maps, active_map_id) = parse_code(&code)?;
    if swaps.is_empty() && maps.is_empty() {
        return Err("Preset code contains no swaps or maps.".into());
    }
    let items = crate::get_items(app.clone(), None).await.unwrap_or_default();
    sanitize_swaps(&mut swaps, &items);
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
            owned_paint_id: None,
            owned_custom_hex: None,
            paint_id: 0,
            custom_paint_hex: None,
            asset_package: pick.asset_package.clone(),
            slot: Some(donor.slot.clone()),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
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
    let cooked = crate::upk::palette::resolve_cooked_dir(std::path::Path::new(&config.game_dir))
        .unwrap_or_else(|_| std::path::PathBuf::from(&config.game_dir));

    let mut results = Vec::new();
    let mut swaps = crate::load_swaps(&app);

    for s in &plan {
        swaps.retain(|x| x.owned_id != s.owned_id);
        swaps.push(s.clone());
        let paint_str = if s.paint_id > 0 {
            format!(" ({})", crate::upk::swapper::paint_label(s.paint_id))
        } else {
            String::new()
        };
        results.push(format!("OK  {} → {}{}", s.owned_name, s.wanted_name, paint_str));
    }

    crate::save_swaps(&app, &swaps);
    if let Err(e) = crate::sync_all_swaps_to_tagame(&app, &cooked, &swaps).await {
        return Err(format!("Failed to apply random loadout: {e}"));
    }

    append_history(&app, "random", &plan, "random loadout applied");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_14slot_binary_encode_length_is_exact_78_chars() {
        let swaps = vec![
            SwapEntry {
                owned_id: 23,
                wanted_id: 4284,
                owned_name: "Octane".into(),
                wanted_name: "Fennec".into(),
                owned_paint_id: None,
                owned_custom_hex: None,
                paint_id: 12,
                custom_paint_hex: None,
                asset_package: "body_grain".into(),
                slot: Some("Body".into()),
                timestamp: None,
            },
            SwapEntry {
                owned_id: 376,
                wanted_id: 1565,
                owned_name: "OEM".into(),
                wanted_name: "Cristiano".into(),
                owned_paint_id: None,
                owned_custom_hex: None,
                paint_id: 3,
                custom_paint_hex: None,
                asset_package: "wheel_cristiano".into(),
                slot: Some("Wheels".into()),
                timestamp: None,
            },
        ];

        let code = encode_14slot_binary(&swaps);
        assert_eq!(code.len(), 78, "14-slot binary base64 code must be exactly 78 characters");

        let (name, decoded_swaps, maps, active_map) = parse_code(&code).expect("must parse code");
        assert_eq!(name, "Shared Preset");
        assert!(maps.is_empty());
        assert!(active_map.is_none());
        assert_eq!(decoded_swaps.len(), 2);

        let body_swap = decoded_swaps.iter().find(|s| s.slot.as_deref() == Some("Body")).expect("body slot");
        assert_eq!(body_swap.wanted_id, 4284);
        assert_eq!(body_swap.paint_id, 12);

        let wheel_swap = decoded_swaps.iter().find(|s| s.slot.as_deref() == Some("Wheels")).expect("wheels slot");
        assert_eq!(wheel_swap.wanted_id, 1565);
        assert_eq!(wheel_swap.paint_id, 3);
    }

    #[test]
    fn test_parse_legacy_v1_json_code() {
        let preset = Preset {
            id: "test-id".into(),
            name: "My Legacy Preset".into(),
            created_at: "2026-09-28T00:00:00Z".into(),
            swaps: vec![
                SwapEntry {
                    owned_id: 23,
                    wanted_id: 4284,
                    owned_name: "Octane".into(),
                    wanted_name: "Fennec".into(),
                    owned_paint_id: None,
                    owned_custom_hex: None,
                    paint_id: 3,
                    custom_paint_hex: None,
                    asset_package: "body_grain".into(),
                    slot: Some("Body".into()),
                    timestamp: None,
                },
            ],
            maps: vec![],
            active_map_id: None,
        };

        let payload = serde_json::json!({
            "v": 1,
            "name": preset.name,
            "swaps": preset.swaps,
            "maps": preset.maps,
            "active_map_id": preset.active_map_id,
        }).to_string();
        let sig = sign_preset_payload(payload.as_bytes());
        let legacy_code = format!("1.{}.{}", B64.encode(payload.as_bytes()), B64.encode(sig));

        let (name, swaps, _, _) = parse_code(&legacy_code).expect("must parse legacy code");
        assert_eq!(name, "My Legacy Preset");
        assert_eq!(swaps.len(), 1);
        assert_eq!(swaps[0].wanted_id, 4284);
    }

    #[test]
    fn test_parse_code_exceeding_50_swaps_rejected() {
        let mut swaps = Vec::new();
        for _ in 0..51 {
            swaps.push(SwapEntry {
                owned_id: 23,
                wanted_id: 4284,
                owned_name: "Octane".into(),
                wanted_name: "Fennec".into(),
                owned_paint_id: None,
                owned_custom_hex: None,
                paint_id: 0,
                custom_paint_hex: None,
                asset_package: "body_grain".into(),
                slot: Some("Body".into()),
                timestamp: None,
            });
        }
        let payload = serde_json::json!({
            "v": 1,
            "name": "Too Many Items",
            "swaps": swaps,
            "maps": [],
            "active_map_id": null,
        }).to_string();
        let sig = sign_preset_payload(payload.as_bytes());
        let code = format!("1.{}.{}", B64.encode(payload.as_bytes()), B64.encode(sig));

        let res = parse_code(&code);
        assert!(res.is_err(), "Must reject preset with >50 swaps");
        assert!(res.unwrap_err().contains("contains more than 50 swaps"));
    }

    #[test]
    fn test_goal_explosion_14slot_binary_encode_decode() {
        let swaps = vec![
            SwapEntry {
                owned_id: 1903,
                wanted_id: 4284,
                owned_name: "Standard".into(),
                wanted_name: "Gravity Bomb".into(),
                owned_paint_id: None,
                owned_custom_hex: None,
                paint_id: 0,
                custom_paint_hex: None,
                asset_package: "Explosion_GravityBomb_SF".into(),
                slot: Some("Goal Explosion".into()),
                timestamp: None,
            },
        ];

        let code = encode_14slot_binary(&swaps);
        assert_eq!(code.len(), 78);

        let (name, decoded_swaps, _, _) = parse_code(&code).expect("must parse code");
        assert_eq!(name, "Shared Preset");
        assert_eq!(decoded_swaps.len(), 1);

        let ge_swap = &decoded_swaps[0];
        assert_eq!(ge_swap.slot.as_deref(), Some("Goal Explosion"));
        assert_eq!(ge_swap.wanted_id, 4284);
        assert_eq!(ge_swap.owned_id, 1903);
    }

    #[test]
    fn test_slot_index_from_str_covers_all_cosmetic_types() {
        assert_eq!(slot_index_from_str("Body"), 0);
        assert_eq!(slot_index_from_str("Bodies"), 0);
        assert_eq!(slot_index_from_str("Decal"), 1);
        assert_eq!(slot_index_from_str("Decals"), 1);
        assert_eq!(slot_index_from_str("Wheels"), 2);
        assert_eq!(slot_index_from_str("Wheel"), 2);
        assert_eq!(slot_index_from_str("Rocket Boost"), 3);
        assert_eq!(slot_index_from_str("Boost"), 3);
        assert_eq!(slot_index_from_str("Antenna"), 4);
        assert_eq!(slot_index_from_str("Antennas"), 4);
        assert_eq!(slot_index_from_str("Topper"), 5);
        assert_eq!(slot_index_from_str("Toppers"), 5);
        assert_eq!(slot_index_from_str("Paint Finish"), 6);
        assert_eq!(slot_index_from_str("Paint Finish (Accent)"), 7);
        assert_eq!(slot_index_from_str("Engine Audio"), 8);
        assert_eq!(slot_index_from_str("Trail"), 9);
        assert_eq!(slot_index_from_str("Goal Explosion"), 10);
        assert_eq!(slot_index_from_str("Goal Explosions"), 10);
        assert_eq!(slot_index_from_str("Player Banner"), 11);
        assert_eq!(slot_index_from_str("Player Anthem"), 12);
        assert_eq!(slot_index_from_str("Avatar Border"), 13);
    }
}
