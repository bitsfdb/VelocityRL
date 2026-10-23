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
use crate::upk::tagame_swapper::{TAGAME_KEY, TAGAME_BACKUP_NAME};

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

    // Support Steam profile links by scraping ?xml=1 if needed
    if clean_url.contains("steamcommunity.com") && !clean_url.ends_with(".png") && !clean_url.ends_with(".jpg") {
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

    let (bgra_bytes, processed_png) = process_avatar_image(&image_bytes)?;
    // Persist downloaded/processed avatar to local disk cache
    let _ = save_cached_avatar(&app, &processed_png);

    // Locate TAGame.upk
    let tagame_path = if let Some(game_dir) = get_game_dir(&app) {
        crate::upk::tagame_swapper::resolve_cooked_dir(&game_dir)
            .map(|p| p.join("TAGame.upk"))
            .map_err(|e| e.to_string())?
    } else {
        // Fallback detect or check current directory
        let default_tagame = Path::new("TAGame.upk");
        if default_tagame.is_file() {
            default_tagame.to_path_buf()
        } else {
            return Err("Rocket League game directory not configured in Settings".into());
        }
    };

    apply_custom_avatar_to_tagame(&tagame_path, &bgra_bytes)?;
    Ok(format!("Custom avatar successfully applied in-engine to {}", tagame_path.display()))
}

#[tauri::command]
pub fn restore_custom_avatar_upk(app: tauri::AppHandle) -> Result<String, String> {
    let tagame_path = if let Some(game_dir) = get_game_dir(&app) {
        crate::upk::tagame_swapper::resolve_cooked_dir(&game_dir)
            .map(|p| p.join("TAGame.upk"))
            .map_err(|e| e.to_string())?
    } else {
        return Err("Rocket League game directory not configured in Settings".into());
    };

    let backup_path = tagame_path.with_file_name(TAGAME_BACKUP_NAME);
    if !backup_path.is_file() {
        return Err("No TAGame.upk.bak backup found to restore".into());
    }

    fs::copy(&backup_path, &tagame_path)
        .map_err(|e| format!("Failed to restore TAGame.upk from backup: {e}"))?;

    Ok("Vanilla avatar restored from pristine backup".into())
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
}
