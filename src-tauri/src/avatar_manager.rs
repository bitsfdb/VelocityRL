use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use image::{imageops, DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};

pub const DEFAULT_AVATAR_SIZE: u32 = 128;
pub const SLOT_INDEX_PLAYER_AVATAR: u8 = 19;
pub const SLOT_INDEX_AVATAR_BORDER: u8 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAvatarConfig {
    pub enabled: bool,
    #[serde(default = "default_mode")]
    pub mode: String, // "steam", "custom_file", "url", "preset"
    pub steam_input: Option<String>,
    pub image_path: Option<String>,
    pub image_url: Option<String>,
    #[serde(default = "default_true")]
    pub circle_mask: bool,
    pub border_color: Option<String>,
    #[serde(default)]
    pub border_width: u32,
    pub preview_base64: Option<String>,
    pub active_steam_id64: Option<String>,
    pub active_persona_name: Option<String>,
}

fn default_mode() -> String {
    "steam".into()
}

fn default_true() -> bool {
    true
}

impl Default for CustomAvatarConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "steam".into(),
            steam_input: None,
            image_path: None,
            image_url: None,
            circle_mask: true,
            border_color: Some("#38bdf8".into()),
            border_width: 2,
            preview_base64: None,
            active_steam_id64: None,
            active_persona_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamAvatarInfo {
    pub steam_id64: String,
    pub persona_name: String,
    pub avatar_url: String,
    pub preview_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarApplyResult {
    pub success: bool,
    pub message: String,
    pub preview_base64: String,
    pub target_paths: Vec<String>,
}

/// Resolves Steam profile input (vanity name, SteamID64, or profile URL) and fetches avatar image.
pub async fn fetch_steam_avatar(query: &str) -> Result<SteamAvatarInfo, String> {
    let clean = query.trim().trim_end_matches('/');
    if clean.is_empty() {
        return Err("Steam query cannot be empty".into());
    }

    let (target_type, target_id) = if clean.contains("steamcommunity.com/profiles/") {
        let id = clean.split("/profiles/").nth(1).unwrap_or(clean).split(['/', '?', '#']).next().unwrap_or(clean);
        ("profiles", id.to_string())
    } else if clean.contains("steamcommunity.com/id/") {
        let id = clean.split("/id/").nth(1).unwrap_or(clean).split(['/', '?', '#']).next().unwrap_or(clean);
        ("id", id.to_string())
    } else if clean.chars().all(|c| c.is_ascii_digit()) && clean.len() == 17 {
        ("profiles", clean.to_string())
    } else {
        ("id", clean.to_string())
    };

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    // 1. Try XML profile endpoint
    let xml_url = format!("https://steamcommunity.com/{}/{}/?xml=1", target_type, target_id);
    let mut avatar_url_opt: Option<String> = None;
    let mut persona_name_opt: Option<String> = None;
    let mut steam_id64_opt: Option<String> = None;

    if let Ok(resp) = client.get(&xml_url).send().await {
        if let Ok(text) = resp.text().await {
            if !text.contains("<error>") {
                if let Some(av) = extract_xml_tag(&text, "avatarFull")
                    .or_else(|| extract_xml_tag(&text, "avatarMedium"))
                    .or_else(|| extract_xml_tag(&text, "avatarIcon"))
                {
                    avatar_url_opt = Some(av);
                }
                persona_name_opt = extract_xml_tag(&text, "steamID");
                steam_id64_opt = extract_xml_tag(&text, "steamID64");
            }
        }
    }

    // 2. Fallback: Direct HTML profile parse
    if avatar_url_opt.is_none() {
        let html_url = format!("https://steamcommunity.com/{}/{}/", target_type, target_id);
        let resp = client.get(&html_url).send().await.map_err(|e| format!("Failed to reach Steam profile: {e}"))?;
        let text = resp.text().await.map_err(|e| format!("Failed to read Steam profile: {e}"))?;

        if let Some(pos) = text.find("<meta property=\"og:image\" content=\"") {
            let start = pos + 35;
            if let Some(end) = text[start..].find('"') {
                avatar_url_opt = Some(text[start..start + end].to_string());
            }
        }
        if avatar_url_opt.is_none() {
            if let Some(pos) = text.find("class=\"playerAvatarAutoSizeInner\">") {
                let sub = &text[pos..];
                if let Some(src_pos) = sub.find("src=\"") {
                    let start = src_pos + 5;
                    if let Some(end) = sub[start..].find('"') {
                        avatar_url_opt = Some(sub[start..start + end].to_string());
                    }
                }
            }
        }

        if persona_name_opt.is_none() {
            if let Some(pos) = text.find("<span class=\"actual_persona_name\">") {
                let start = pos + 34;
                if let Some(end) = text[start..].find('<') {
                    persona_name_opt = Some(text[start..start + end].trim().to_string());
                }
            }
        }

        if steam_id64_opt.is_none() {
            if let Some(pos) = text.find("\"steamid\":\"") {
                let start = pos + 11;
                if let Some(end) = text[start..].find('"') {
                    steam_id64_opt = Some(text[start..start + end].to_string());
                }
            }
        }
    }

    let avatar_url = avatar_url_opt.ok_or_else(|| format!("Could not find avatar on Steam profile '{clean}'."))?;
    let steam_id64 = steam_id64_opt.unwrap_or_else(|| target_id.clone());
    let persona_name = persona_name_opt.unwrap_or_else(|| target_id);

    // Download avatar image
    let img_resp = client.get(&avatar_url).send().await.map_err(|e| format!("Failed to download avatar image: {e}"))?;
    let img_bytes = img_resp.bytes().await.map_err(|e| format!("Failed to read avatar bytes: {e}"))?;
    let dyn_img = image::load_from_memory(&img_bytes).map_err(|e| format!("Invalid image data: {e}"))?;

    // Create base64 preview
    let mut png_buf = Vec::new();
    dyn_img.write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let preview_base64 = format!("data:image/png;base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_buf));

    Ok(SteamAvatarInfo {
        steam_id64,
        persona_name,
        avatar_url,
        preview_base64,
    })
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{}>", tag);
    let close_tag = format!("</{}>", tag);
    if let Some(start) = xml.find(&open_tag) {
        let val_start = start + open_tag.len();
        if let Some(end) = xml[val_start..].find(&close_tag) {
            let mut val = xml[val_start..val_start + end].trim().to_string();
            if val.starts_with("<![CDATA[") && val.ends_with("]]>") {
                val = val[9..val.len() - 3].trim().to_string();
            }
            return Some(val);
        }
    }
    None
}

/// Processes an image into square / circular format with optional border.
pub fn process_avatar_image(
    dyn_img: &DynamicImage,
    target_size: u32,
    circle_mask: bool,
    border_color_hex: Option<&str>,
    border_width: u32,
) -> RgbaImage {
    let (w, h) = dyn_img.dimensions();
    let min_dim = w.min(h);
    let left = (w - min_dim) / 2;
    let top = (h - min_dim) / 2;

    // Crop square
    let cropped = dyn_img.crop_imm(left, top, min_dim, min_dim);
    let resized = imageops::resize(&cropped, target_size, target_size, imageops::FilterType::Lanczos3);

    let mut out: RgbaImage = ImageBuffer::new(target_size, target_size);

    let center = (target_size as f32) / 2.0;
    let radius = center - 0.5;
    let radius_sq = radius * radius;

    let border_rgba = border_color_hex.and_then(parse_hex_color);

    for (x, y, pixel) in resized.enumerate_pixels() {
        let mut p = *pixel;
        if circle_mask {
            let dx = (x as f32) + 0.5 - center;
            let dy = (y as f32) + 0.5 - center;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq > radius_sq {
                p.0[3] = 0; // transparent
            } else if dist_sq > (radius - 1.0) * (radius - 1.0) {
                // simple antialias edge
                let alpha_ratio = radius - dist_sq.sqrt();
                p.0[3] = ((p.0[3] as f32) * alpha_ratio.clamp(0.0, 1.0)) as u8;
            }
        }
        out.put_pixel(x, y, p);
    }

    // Apply border outline if requested
    if let (Some(b_col), true) = (border_rgba, border_width > 0) {
        if circle_mask {
            let b_width_f = border_width as f32;
            let inner_radius = (radius - b_width_f).max(0.0);
            let inner_radius_sq = inner_radius * inner_radius;

            for x in 0..target_size {
                for y in 0..target_size {
                    let dx = (x as f32) + 0.5 - center;
                    let dy = (y as f32) + 0.5 - center;
                    let dist_sq = dx * dx + dy * dy;
                    if dist_sq <= radius_sq && dist_sq >= inner_radius_sq {
                        out.put_pixel(x, y, b_col);
                    }
                }
            }
        } else {
            for bw in 0..border_width {
                for x in bw..target_size - bw {
                    out.put_pixel(x, bw, b_col);
                    out.put_pixel(x, target_size - 1 - bw, b_col);
                }
                for y in bw..target_size - bw {
                    out.put_pixel(bw, y, b_col);
                    out.put_pixel(target_size - 1 - bw, y, b_col);
                }
            }
        }
    }

    out
}

fn parse_hex_color(hex: &str) -> Option<Rgba<u8>> {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        Some(Rgba([r, g, b, 255]))
    } else if clean.len() == 8 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        let a = u8::from_str_radix(&clean[6..8], 16).ok()?;
        Some(Rgba([r, g, b, a]))
    } else {
        None
    }
}

/// Exports the processed avatar image into the game / BakkesMod avatar directories.
pub fn save_avatar_to_game_dirs(
    img: &RgbaImage,
    target_names: &[&str],
    game_dir: Option<&Path>,
    app_data_dir: &Path,
) -> Result<Vec<String>, String> {
    let mut out_paths = Vec::new();

    // 1. App data cache directory
    let cache_dir = app_data_dir.join("cache").join("avatars");
    fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;

    // 2. Search for BakkesMod data/avatars directory
    let mut search_dirs = Vec::new();
    search_dirs.push(cache_dir.clone());

    if let Some(gd) = game_dir {
        search_dirs.push(gd.join("bakkesmod").join("data").join("avatars"));
        search_dirs.push(gd.parent().unwrap_or(gd).join("bakkesmod").join("data").join("avatars"));
        search_dirs.push(gd.join("TAGame").join("CookedPCConsole").join("custom_avatars"));
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        search_dirs.push(PathBuf::from(appdata).join("bakkesmod").join("bakkesmod").join("data").join("avatars"));
    }

    let mut encoded_png = Vec::new();
    DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut Cursor::new(&mut encoded_png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;

    for dir in search_dirs {
        let _ = fs::create_dir_all(&dir);
        if dir.is_dir() {
            for name in target_names {
                let safe_name = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
                let file_path = dir.join(format!("{safe_name}.png"));
                if fs::write(&file_path, &encoded_png).is_ok() {
                    out_paths.push(file_path.to_string_lossy().into_owned());
                }
            }
        }
    }

    Ok(out_paths)
}
