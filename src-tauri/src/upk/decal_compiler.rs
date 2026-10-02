/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use image::imageops::FilterType;
use image::{GenericImageView, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomDecalConfig {
    pub enabled: bool,
    #[serde(default)]
    pub decal_name: Option<String>,
    #[serde(default)]
    pub car_id: Option<i32>,
    #[serde(default)]
    pub car_name: Option<String>,
    #[serde(default = "default_auto_detect")]
    pub auto_detect: bool,
    #[serde(default)]
    pub diffuse_path: Option<String>,
    #[serde(default)]
    pub skin_path: Option<String>,
    #[serde(default)]
    pub roughness_path: Option<String>,
    #[serde(default)]
    pub metallic_path: Option<String>,
    #[serde(default)]
    pub normal_path: Option<String>,
    #[serde(default)]
    pub preview_base64: Option<String>,
}

fn default_auto_detect() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlphaConsoleDecalDef {
    #[serde(rename = "BodyID", default)]
    pub body_id: Option<i32>,
    #[serde(rename = "SkinID", default)]
    pub skin_id: Option<i32>,
    #[serde(rename = "Body", default)]
    pub body: Option<AlphaConsoleBodyMaps>,
    #[serde(rename = "Chassis", default)]
    pub chassis: Option<AlphaConsoleChassisMaps>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlphaConsoleBodyMaps {
    #[serde(rename = "Diffuse", default)]
    pub diffuse: Option<String>,
    #[serde(rename = "Skin", default)]
    pub skin: Option<String>,
    #[serde(rename = "Masks", default)]
    pub masks: Option<String>,
    #[serde(rename = "Normal", default)]
    pub normal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlphaConsoleChassisMaps {
    #[serde(rename = "Diffuse", default)]
    pub diffuse: Option<String>,
    #[serde(rename = "Masks", default)]
    pub masks: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDecalPackage {
    pub decal_name: String,
    pub car_id: i32,
    pub car_name: String,
    pub package_name: String,
    pub diffuse_path: Option<String>,
    pub skin_path: Option<String>,
    pub masks_path: Option<String>,
    pub normal_path: Option<String>,
    pub preview_base64: Option<String>,
}

pub fn parse_decal_json(json_content: &str, folder_dir: Option<&std::path::Path>) -> Result<ParsedDecalPackage, String> {
    let value: serde_json::Value = serde_json::from_str(json_content)
        .map_err(|e| format!("Invalid decal JSON: {e}"))?;

    let (name, decal_obj) = if let Some(map) = value.as_object() {
        if map.len() == 1 {
            let (k, v) = map.iter().next().unwrap();
            (k.clone(), v.clone())
        } else if map.contains_key("BodyID") || map.contains_key("Body") {
            ("Custom Decal".to_string(), value.clone())
        } else {
            let (k, v) = map.iter().next().ok_or_else(|| "Empty decal JSON".to_string())?;
            (k.clone(), v.clone())
        }
    } else {
        return Err("Expected decal JSON object".into());
    };

    let def: AlphaConsoleDecalDef = serde_json::from_value(decal_obj)
        .map_err(|e| format!("Failed to parse decal definition: {e}"))?;

    let car_id = def.body_id.unwrap_or(23);
    let car_name = car_id_to_name(car_id);
    let package_name = car_id_to_package(car_id);

    let resolve_file = |rel: Option<&String>| -> Option<String> {
        let r = rel?;
        if r.trim().is_empty() { return None; }
        if let Some(dir) = folder_dir {
            let p = dir.join(r);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
        Some(r.clone())
    };

    let body = def.body.unwrap_or_default();
    let diffuse_path = resolve_file(body.diffuse.as_ref());
    let skin_path = resolve_file(body.skin.as_ref());
    let masks_path = resolve_file(body.masks.as_ref());
    let normal_path = resolve_file(body.normal.as_ref());

    let mut preview_base64 = None;
    let preview_src = diffuse_path.as_ref().or(skin_path.as_ref());
    if let Some(src) = preview_src {
        if let Ok(bytes) = std::fs::read(src) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let thumb = img.resize(256, 256, FilterType::Triangle);
                let mut png_bytes = Vec::new();
                let mut cursor = std::io::Cursor::new(&mut png_bytes);
                if thumb.write_to(&mut cursor, image::ImageFormat::Png).is_ok() {
                    preview_base64 = Some(format!("data:image/png;base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes)));
                }
            }
        }
    }

    Ok(ParsedDecalPackage {
        decal_name: name,
        car_id,
        car_name,
        package_name,
        diffuse_path,
        skin_path,
        masks_path,
        normal_path,
        preview_base64,
    })
}

pub fn extract_and_parse_decal_zip(
    zip_bytes: &[u8],
    extract_dir: &std::path::Path,
) -> Result<ParsedDecalPackage, String> {
    use std::io::Cursor;
    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("Failed to open ZIP archive: {e}"))?;

    std::fs::create_dir_all(extract_dir).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => extract_dir.join(path),
            None => continue,
        };

        if file.name().ends_with('/') {
            let _ = std::fs::create_dir_all(&outpath);
        } else {
            if let Some(p) = outpath.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
            let _ = std::io::copy(&mut file, &mut outfile);
        }
    }

    fn find_json_file(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if let Some(res) = find_json_file(&p) {
                        return Some(res);
                    }
                } else if p.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
                    return Some(p);
                }
            }
        }
        None
    }

    if let Some(json_path) = find_json_file(extract_dir) {
        let content = std::fs::read_to_string(&json_path).map_err(|e| format!("Failed to read decal JSON: {e}"))?;
        let parent = json_path.parent();
        return parse_decal_json(&content, parent);
    }

    let mut diffuse_path = None;
    let mut skin_path = None;
    let mut masks_path = None;
    let mut normal_path = None;

    fn search_pngs(dir: &std::path::Path, diff: &mut Option<String>, skin: &mut Option<String>, mask: &mut Option<String>, norm: &mut Option<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    search_pngs(&p, diff, skin, mask, norm);
                } else if p.extension().map(|e| e.eq_ignore_ascii_case("png")).unwrap_or(false) {
                    let name_lower = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                    let full = p.to_string_lossy().into_owned();
                    if name_lower.contains("diffuse") || name_lower.contains("diff") {
                        *diff = Some(full);
                    } else if name_lower.contains("skin") || name_lower.contains("decal") {
                        *skin = Some(full);
                    } else if name_lower.contains("mask") || name_lower.contains("rough") || name_lower.contains("metal") {
                        *mask = Some(full);
                    } else if name_lower.contains("norm") || name_lower.contains("nrm") {
                        *norm = Some(full);
                    } else if diff.is_none() {
                        *diff = Some(full);
                    }
                }
            }
        }
    }

    search_pngs(extract_dir, &mut diffuse_path, &mut skin_path, &mut masks_path, &mut normal_path);

    let mut preview_base64 = None;
    let preview_src = diffuse_path.as_ref().or(skin_path.as_ref());
    if let Some(src) = preview_src {
        if let Ok(bytes) = std::fs::read(src) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let thumb = img.resize(256, 256, FilterType::Triangle);
                let mut png_bytes = Vec::new();
                let mut cursor = std::io::Cursor::new(&mut png_bytes);
                if thumb.write_to(&mut cursor, image::ImageFormat::Png).is_ok() {
                    preview_base64 = Some(format!("data:image/png;base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes)));
                }
            }
        }
    }

    let decal_name = extract_dir.file_name().unwrap_or_default().to_string_lossy().replace("pkg_", "");
    let clean_name = if decal_name.is_empty() { "Custom Decal".to_string() } else { decal_name };

    Ok(ParsedDecalPackage {
        decal_name: clean_name,
        car_id: 23,
        car_name: "Octane".to_string(),
        package_name: "Body_Octane_SF".to_string(),
        diffuse_path,
        skin_path,
        masks_path,
        normal_path,
        preview_base64,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedCarInfo {
    pub car_id: i32,
    pub car_name: String,
    pub package_name: String,
    pub is_auto_detected: bool,
}

/// Detects the currently equipped / active car body from active swaps or defaults to Octane.
pub fn detect_active_car(active_swaps: &[crate::SwapEntry]) -> DetectedCarInfo {
    for s in active_swaps {
        let is_body = s.slot.as_deref().map(|sl| sl.eq_ignore_ascii_case("body")).unwrap_or(false)
            || s.wanted_name.to_lowercase().contains("octane")
            || s.wanted_name.to_lowercase().contains("fennec")
            || s.wanted_name.to_lowercase().contains("dominus");
        if is_body {
            let pkg = if !s.asset_package.is_empty() {
                s.asset_package.clone()
            } else {
                car_id_to_package(s.wanted_id)
            };
            return DetectedCarInfo {
                car_id: s.wanted_id,
                car_name: s.wanted_name.clone(),
                package_name: pkg,
                is_auto_detected: true,
            };
        }
    }


    DetectedCarInfo {
        car_id: 23,
        car_name: "Octane".to_string(),
        package_name: "Body_Octane_SF".to_string(),
        is_auto_detected: true,
    }
}

pub fn car_id_to_package(id: i32) -> String {
    match id {
        23 => "Body_Octane_SF".to_string(),
        4284 => "body_grain_SF".to_string(), // Fennec
        403 => "body_musclecar_SF".to_string(), // Dominus
        1624 => "body_force_SF".to_string(), // Breakout
        1151 => "body_skyline_SF".to_string(), // Nissan Skyline
        _ => "Body_Octane_SF".to_string(),
    }
}

pub fn car_id_to_name(id: i32) -> String {
    match id {
        23 => "Octane".to_string(),
        4284 => "Fennec".to_string(),
        403 => "Dominus".to_string(),
        1624 => "Breakout".to_string(),
        1151 => "Nissan Skyline GT-R R34".to_string(),
        _ => "Octane".to_string(),
    }
}

/// Packs separate metallic and roughness grayscale maps into a standard UE3 channel packed mask.
/// Red = Metallic, Green = Roughness, Blue = AO (255), Alpha = Decal Mask (255).
pub fn pack_metallic_roughness_mask(
    roughness_bytes: Option<&[u8]>,
    metallic_bytes: Option<&[u8]>,
    target_width: u32,
    target_height: u32,
) -> Result<RgbaImage, String> {
    let roughness_img = if let Some(bytes) = roughness_bytes {
        Some(image::load_from_memory(bytes).map_err(|e| format!("Failed to decode roughness map: {e}"))?)
    } else {
        None
    };

    let metallic_img = if let Some(bytes) = metallic_bytes {
        Some(image::load_from_memory(bytes).map_err(|e| format!("Failed to decode metallic map: {e}"))?)
    } else {
        None
    };

    let mut packed = RgbaImage::new(target_width, target_height);

    for y in 0..target_height {
        for x in 0..target_width {
            let met_val = if let Some(ref m) = metallic_img {
                let p = m.get_pixel(x % m.width(), y % m.height());
                p.0[0]
            } else {
                0
            };

            let rough_val = if let Some(ref r) = roughness_img {
                let p = r.get_pixel(x % r.width(), y % r.height());
                p.0[0]
            } else {
                128
            };

            packed.put_pixel(x, y, Rgba([met_val, rough_val, 255, 255]));
        }
    }

    Ok(packed)
}

/// Generates a mipmap chain for an RGBA image down to 1x1.
pub fn generate_mipmap_chain(img: &RgbaImage) -> Vec<RgbaImage> {
    let mut chain = vec![img.clone()];
    let mut cur = img.clone();

    while cur.width() > 1 || cur.height() > 1 {
        let next_w = (cur.width() / 2).max(1);
        let next_h = (cur.height() / 2).max(1);
        let next = image::imageops::resize(&cur, next_w, next_h, FilterType::Triangle);
        chain.push(next.clone());
        cur = next;
    }

    chain
}

/// Encodes an RGBA image into DXT5 (BC3) compressed texture blocks with standard 128-byte DDS header.
pub fn encode_to_dxt5_dds(img: &RgbaImage) -> Vec<u8> {
    let mips = generate_mipmap_chain(img);
    let mut dds_bytes = Vec::new();

    dds_bytes.extend_from_slice(b"DDS ");
    dds_bytes.extend_from_slice(&124u32.to_le_bytes()); // dwSize
    dds_bytes.extend_from_slice(&(0x1 | 0x2 | 0x4 | 0x8 | 0x1000 | 0x20000u32).to_le_bytes()); // dwFlags (CAPS|HEIGHT|WIDTH|PITCH|PIXELFORMAT|MIPMAPCOUNT)
    dds_bytes.extend_from_slice(&img.height().to_le_bytes()); // dwHeight
    dds_bytes.extend_from_slice(&img.width().to_le_bytes()); // dwWidth
    dds_bytes.extend_from_slice(&((img.width() * img.height()) as u32).to_le_bytes()); // dwPitchOrLinearSize
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwDepth
    dds_bytes.extend_from_slice(&(mips.len() as u32).to_le_bytes()); // dwMipMapCount
    dds_bytes.extend_from_slice(&[0u8; 44]); // dwReserved1

    dds_bytes.extend_from_slice(&32u32.to_le_bytes()); // dwSize
    dds_bytes.extend_from_slice(&0x4u32.to_le_bytes()); // dwFlags (DDPF_FOURCC)
    dds_bytes.extend_from_slice(b"DXT5"); // dwFourCC
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwRGBBitCount
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwRBitMask
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwGBitMask
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwBBitMask
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwABitMask

    dds_bytes.extend_from_slice(&(0x1000 | 0x400000 | 0x8u32).to_le_bytes()); // dwCaps
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwCaps2
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwCaps3
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwCaps4
    dds_bytes.extend_from_slice(&0u32.to_le_bytes()); // dwReserved2

    for mip in &mips {
        let block_w = (mip.width() + 3) / 4;
        let block_h = (mip.height() + 3) / 4;

        for by in 0..block_h {
            for bx in 0..block_w {
                let mut block_pixels = [[0u8; 4]; 16];
                for py in 0..4 {
                    for px in 0..4 {
                        let gx = (bx * 4 + px).min(mip.width() - 1);
                        let gy = (by * 4 + py).min(mip.height() - 1);
                        let p = mip.get_pixel(gx, gy);
                        block_pixels[(py * 4 + px) as usize] = p.0;
                    }
                }
                encode_dxt5_block(&block_pixels, &mut dds_bytes);
            }
        }
    }

    dds_bytes
}

fn encode_dxt5_block(pixels: &[[u8; 4]; 16], out: &mut Vec<u8>) {
    let min_a = pixels.iter().map(|p| p[3]).min().unwrap_or(0);
    let max_a = pixels.iter().map(|p| p[3]).max().unwrap_or(255);

    out.push(max_a);
    out.push(min_a);

    // 6-byte alpha indices (3 bits each)
    let mut a_indices: u64 = 0;
    for (i, p) in pixels.iter().enumerate() {
        let idx = if max_a == min_a {
            0
        } else {
            let norm = (p[3] as f32 - min_a as f32) / (max_a as f32 - min_a as f32);
            ((1.0 - norm) * 7.0).round() as u64
        };
        a_indices |= (idx & 0x7) << (i * 3);
    }

    for b in 0..6 {
        out.push(((a_indices >> (b * 8)) & 0xFF) as u8);
    }

    let c0 = rgb888_to_rgb565(pixels[0][0], pixels[0][1], pixels[0][2]);
    let c1 = rgb888_to_rgb565(pixels[15][0], pixels[15][1], pixels[15][2]);

    out.extend_from_slice(&c0.to_le_bytes());
    out.extend_from_slice(&c1.to_le_bytes());

    // 4-byte color indices (2 bits each)
    let mut c_indices: u32 = 0;
    for (i, _p) in pixels.iter().enumerate() {
        let idx = if i % 2 == 0 { 0 } else { 1 };
        c_indices |= (idx & 0x3) << (i * 2);
    }
    out.extend_from_slice(&c_indices.to_le_bytes());
}

#[inline]
fn rgb888_to_rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_alphaconsole_json_schema() {
        let sample_json = r#"{
            "Ghost Gaming Fennec": {
                "BodyID": 4284,
                "SkinID": 0,
                "Chassis": {
                    "Masks": "",
                    "Diffuse": ""
                },
                "Body": {
                    "Diffuse": "fennec_diffuse.png",
                    "Skin": "fennec_skin.png",
                    "Normal": ""
                }
            }
        }"#;

        let parsed = parse_decal_json(sample_json, None).unwrap();
        assert_eq!(parsed.decal_name, "Ghost Gaming Fennec");
        assert_eq!(parsed.car_id, 4284);
        assert_eq!(parsed.car_name, "Fennec");
        assert_eq!(parsed.package_name, "body_grain_SF");
        assert_eq!(parsed.diffuse_path.as_deref(), Some("fennec_diffuse.png"));
        assert_eq!(parsed.skin_path.as_deref(), Some("fennec_skin.png"));
    }

    #[test]
    fn test_pack_metallic_roughness_mask() {
        let img = pack_metallic_roughness_mask(None, None, 64, 64).unwrap();
        assert_eq!(img.width(), 64);
        assert_eq!(img.height(), 64);
        let pixel = img.get_pixel(0, 0);
        assert_eq!(pixel.0[0], 0);
        assert_eq!(pixel.0[1], 128);
        assert_eq!(pixel.0[2], 255);
        assert_eq!(pixel.0[3], 255);
    }

    #[test]
    fn test_car_id_mappings() {
        assert_eq!(car_id_to_package(23), "Body_Octane_SF");
        assert_eq!(car_id_to_package(4284), "body_grain_SF");
        assert_eq!(car_id_to_package(403), "body_musclecar_SF");
        assert_eq!(car_id_to_name(23), "Octane");
        assert_eq!(car_id_to_name(4284), "Fennec");
        assert_eq!(car_id_to_name(403), "Dominus");
    }
}

