use crate::SwapEntry;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct SwapsIniConfig {
    pub enabled: bool,
    pub body: i32,
    pub body_paint: i32,
    pub decal: i32,
    pub decal_paint: i32,
    pub wheels: i32,
    pub wheels_paint: i32,
    pub boost: i32,
    pub boost_paint: i32,
    pub topper: i32,
    pub antenna: i32,
    pub paint_finish: i32,
    pub engine_audio: i32,
    pub trail: i32,
    pub goal_explosion: i32,
    pub player_banner: i32,
    pub player_anthem: i32,
    pub avatar_border: i32,
}

/// Resolves the `<GameDir>/TAGame/Config` directory from the `CookedPCConsole` or Game directory.
pub fn resolve_config_dir(game_or_cooked_dir: &Path) -> PathBuf {
    if game_or_cooked_dir.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("CookedPCConsole")).unwrap_or(false) {
        if let Some(parent) = game_or_cooked_dir.parent() {
            let cfg = parent.join("Config");
            let _ = fs::create_dir_all(&cfg);
            return cfg;
        }
    }

    let tagame_cfg = game_or_cooked_dir.join("TAGame").join("Config");
    if tagame_cfg.is_dir() || game_or_cooked_dir.join("TAGame").is_dir() {
        let _ = fs::create_dir_all(&tagame_cfg);
        return tagame_cfg;
    }

    let cfg = game_or_cooked_dir.join("Config");
    let _ = fs::create_dir_all(&cfg);
    cfg
}

#[cfg(windows)]
pub fn get_windows_documents_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. Query Windows Registry for user's configured Documents folder (handles moved/redirected folders)
    if let Ok(hkcu) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders")
    {
        if let Ok(personal) = hkcu.get_value::<String, _>("Personal") {
            let expanded = if personal.contains('%') {
                if let Ok(profile) = std::env::var("USERPROFILE") {
                    personal.replace("%USERPROFILE%", &profile)
                } else {
                    personal
                }
            } else {
                personal
            };
            dirs.push(PathBuf::from(expanded));
        }
    }

    // 2. Standard %USERPROFILE%\Documents and OneDrive\Documents
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(&profile);
        dirs.push(p.join("Documents"));
        dirs.push(p.join("OneDrive").join("Documents"));
    }

    // 3. Fallback to HOMEDRIVE + HOMEPATH
    if let (Ok(drive), Ok(path)) = (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
        let p = PathBuf::from(format!("{drive}{path}"));
        dirs.push(p.join("Documents"));
        dirs.push(p.join("OneDrive").join("Documents"));
    }

    dirs.retain(|d| d.is_dir() || d.parent().map(|p| p.is_dir()).unwrap_or(false));
    dirs.dedup();
    dirs
}

/// Finds all candidate TAGame.ini and TASystemSettings.ini files in the installation and user documents directory.
pub fn find_all_tagame_ini_paths(game_or_cooked_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let config_dir = resolve_config_dir(game_or_cooked_dir);
    paths.push(config_dir.join("TAGame.ini"));
    paths.push(config_dir.join("TASystemSettings.ini"));
    paths.push(config_dir.join("PC").join("Cooked").join("TAGame.ini"));

    if let Some(parent) = config_dir.parent() {
        paths.push(parent.join("Config").join("TAGame.ini"));
        paths.push(parent.join("Config").join("TASystemSettings.ini"));
        paths.push(parent.join("Config").join("PC").join("Cooked").join("TAGame.ini"));
    }

    #[cfg(windows)]
    {
        for doc_dir in get_windows_documents_dirs() {
            let rl_cfg = doc_dir.join("My Games").join("Rocket League").join("TAGame").join("Config");
            paths.push(rl_cfg.join("TASystemSettings.ini"));
            paths.push(rl_cfg.join("TAGame.ini"));
        }
    }

    paths.retain(|p| p.is_file() || p.parent().map(|par| par.is_dir()).unwrap_or(false));
    paths.dedup();
    paths
}

/// Updates or inserts a section into an existing INI file without corrupting other sections.
pub fn update_tagame_ini_section(
    ini_path: &Path,
    section_name: &str,
    key_values: &[(&str, &str)],
) -> Result<(), String> {
    let mut lines: Vec<String> = if ini_path.is_file() {
        fs::read_to_string(ini_path)
            .unwrap_or_default()
            .lines()
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };

    let target_header = format!("[{section_name}]");
    let mut in_section = false;
    let mut section_start = None;
    let mut section_end = None;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case(&target_header) {
            in_section = true;
            section_start = Some(idx);
        } else if in_section && trimmed.starts_with('[') && trimmed.ends_with(']') {
            section_end = Some(idx);
            break;
        }
    }

    if in_section && section_end.is_none() {
        section_end = Some(lines.len());
    }

    let mut section_content = vec![target_header];
    for (k, v) in key_values {
        section_content.push(format!("{k}={v}"));
    }

    if let (Some(start), Some(end)) = (section_start, section_end) {
        lines.splice(start..end, section_content);
    } else {
        if !lines.is_empty() && !lines.last().unwrap().is_empty() {
            lines.push(String::new());
        }
        lines.extend(section_content);
    }

    if let Some(parent) = ini_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(ini_path, lines.join("\r\n") + "\r\n").map_err(|e| e.to_string())?;
    Ok(())
}

/// Updates the `[TAGame.GFxData_Garage_TA]` section in all active `TAGame.ini` files.
pub fn write_tagame_ini_native(
    game_or_cooked_dir: &Path,
    swaps_cfg: &SwapsIniConfig,
    avatar_path: Option<&str>,
    decal_diffuse: Option<&str>,
    decal_skin: Option<&str>,
) -> Result<(), String> {
    let ini_paths = find_all_tagame_ini_paths(game_or_cooked_dir);
    let enabled_str = if swaps_cfg.enabled { "True" } else { "False" };
    let body_str = swaps_cfg.body.to_string();
    let body_paint_str = swaps_cfg.body_paint.to_string();
    let decal_str = swaps_cfg.decal.to_string();
    let decal_paint_str = swaps_cfg.decal_paint.to_string();
    let wheels_str = swaps_cfg.wheels.to_string();
    let wheels_paint_str = swaps_cfg.wheels_paint.to_string();
    let boost_str = swaps_cfg.boost.to_string();
    let boost_paint_str = swaps_cfg.boost_paint.to_string();
    let topper_str = swaps_cfg.topper.to_string();
    let antenna_str = swaps_cfg.antenna.to_string();
    let paint_finish_str = swaps_cfg.paint_finish.to_string();
    let engine_audio_str = swaps_cfg.engine_audio.to_string();
    let trail_str = swaps_cfg.trail.to_string();
    let goal_explosion_str = swaps_cfg.goal_explosion.to_string();
    let player_banner_str = swaps_cfg.player_banner.to_string();
    let player_anthem_str = swaps_cfg.player_anthem.to_string();
    let avatar_border_str = swaps_cfg.avatar_border.to_string();

    let avatar_val = avatar_path.unwrap_or("CustomAvatar.png");
    let diffuse_val = decal_diffuse.unwrap_or("");
    let skin_val = decal_skin.unwrap_or("");

    let key_values = [
        ("bVelocityRLEnabled", enabled_str),
        ("VelocityRLBody", &body_str),
        ("VelocityRLBodyPaint", &body_paint_str),
        ("VelocityRLDecal", &decal_str),
        ("VelocityRLDecalPaint", &decal_paint_str),
        ("VelocityRLWheels", &wheels_str),
        ("VelocityRLWheelsPaint", &wheels_paint_str),
        ("VelocityRLBoost", &boost_str),
        ("VelocityRLBoostPaint", &boost_paint_str),
        ("VelocityRLGoalExplosion", &goal_explosion_str),
        ("VelocityRLPlayerBanner", &player_banner_str),
        ("VelocityRLPlayerAnthem", &player_anthem_str),
        ("VelocityRLAvatarBorder", &avatar_border_str),
        ("VelocityRLTopper", &topper_str),
        ("VelocityRLAntenna", &antenna_str),
        ("VelocityRLTrail", &trail_str),
        ("VelocityRLPaintFinish", &paint_finish_str),
        ("VelocityRLEngineAudio", &engine_audio_str),
        ("VelocityRLAvatarPath", avatar_val),
        ("VelocityRLDecalDiffuse", diffuse_val),
        ("VelocityRLDecalSkin", skin_val),
    ];

    for ini_path in &ini_paths {
        let _ = update_tagame_ini_section(ini_path, "TAGame.GFxData_Garage_TA", &key_values);
    }
    Ok(())
}

/// Serializes and writes `swaps.ini` directly to `<GameDir>/TAGame/Config/swaps.ini`.
pub fn write_swaps_ini(
    game_or_cooked_dir: &Path,
    swaps: &[SwapEntry],
    enabled: bool,
) -> Result<PathBuf, String> {
    let config_dir = resolve_config_dir(game_or_cooked_dir);
    let ini_path = config_dir.join("swaps.ini");

    let mut cfg = SwapsIniConfig {
        enabled,
        ..Default::default()
    };

    for s in swaps {
        let slot = s.slot.as_deref().unwrap_or("").to_lowercase().replace([' ', '_', '-'], "");
        let wanted_id = s.wanted_id;
        let paint_id = s.paint_id as i32;

        if slot.contains("body") {
            cfg.body = wanted_id;
            cfg.body_paint = paint_id;
        } else if slot.contains("decal") {
            cfg.decal = wanted_id;
            cfg.decal_paint = paint_id;
        } else if slot.contains("wheel") {
            cfg.wheels = wanted_id;
            cfg.wheels_paint = paint_id;
        } else if slot.contains("boost") {
            cfg.boost = wanted_id;
            cfg.boost_paint = paint_id;
        } else if slot.contains("topper") {
            cfg.topper = wanted_id;
        } else if slot.contains("antenna") {
            cfg.antenna = wanted_id;
        } else if slot.contains("paintfinish") || slot.contains("finish") {
            cfg.paint_finish = wanted_id;
        } else if slot.contains("engineaudio") || slot.contains("audio") {
            cfg.engine_audio = wanted_id;
        } else if slot.contains("trail") {
            cfg.trail = wanted_id;
        } else if slot.contains("explosion") || slot.contains("goalexplosion") {
            cfg.goal_explosion = wanted_id;
        } else if slot.contains("banner") || slot.contains("playerbanner") {
            cfg.player_banner = wanted_id;
        } else if slot.contains("anthem") || slot.contains("playeranthem") {
            cfg.player_anthem = wanted_id;
        } else if slot.contains("border") || slot.contains("avatarborder") {
            cfg.avatar_border = wanted_id;
        }
    }

    let content = format!(
        "[VelocityRL.Swaps]\r\n\
        Enabled={enabled}\r\n\
        Body={body}\r\n\
        BodyPaint={body_paint}\r\n\
        Decal={decal}\r\n\
        DecalPaint={decal_paint}\r\n\
        Wheels={wheels}\r\n\
        WheelsPaint={wheels_paint}\r\n\
        Boost={boost}\r\n\
        BoostPaint={boost_paint}\r\n\
        Topper={topper}\r\n\
        Antenna={antenna}\r\n\
        PaintFinish={paint_finish}\r\n\
        EngineAudio={engine_audio}\r\n\
        Trail={trail}\r\n\
        GoalExplosion={goal_explosion}\r\n\
        PlayerBanner={player_banner}\r\n\
        PlayerAnthem={player_anthem}\r\n\
        AvatarBorder={avatar_border}\r\n",
        enabled = if cfg.enabled { "true" } else { "false" },
        body = cfg.body,
        body_paint = cfg.body_paint,
        decal = cfg.decal,
        decal_paint = cfg.decal_paint,
        wheels = cfg.wheels,
        wheels_paint = cfg.wheels_paint,
        boost = cfg.boost,
        boost_paint = cfg.boost_paint,
        topper = cfg.topper,
        antenna = cfg.antenna,
        paint_finish = cfg.paint_finish,
        engine_audio = cfg.engine_audio,
        trail = cfg.trail,
        goal_explosion = cfg.goal_explosion,
        player_banner = cfg.player_banner,
        player_anthem = cfg.player_anthem,
        avatar_border = cfg.avatar_border,
    );

    fs::write(&ini_path, content).map_err(|e| format!("Failed to write swaps.ini: {e}"))?;
    let _ = write_tagame_ini_native(game_or_cooked_dir, &cfg, None, None, None);
    Ok(ini_path)
}

/// Reads `swaps.ini` from `<GameDir>/TAGame/Config/swaps.ini`.
pub fn read_swaps_ini(game_or_cooked_dir: &Path) -> Option<SwapsIniConfig> {
    let config_dir = resolve_config_dir(game_or_cooked_dir);
    let ini_path = config_dir.join("swaps.ini");
    let text = fs::read_to_string(&ini_path).ok()?;

    let mut cfg = SwapsIniConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "Enabled" => cfg.enabled = val.eq_ignore_ascii_case("true") || val == "1",
                "Body" => cfg.body = val.parse().unwrap_or(0),
                "BodyPaint" => cfg.body_paint = val.parse().unwrap_or(0),
                "Decal" => cfg.decal = val.parse().unwrap_or(0),
                "DecalPaint" => cfg.decal_paint = val.parse().unwrap_or(0),
                "Wheels" => cfg.wheels = val.parse().unwrap_or(0),
                "WheelsPaint" => cfg.wheels_paint = val.parse().unwrap_or(0),
                "Boost" => cfg.boost = val.parse().unwrap_or(0),
                "BoostPaint" => cfg.boost_paint = val.parse().unwrap_or(0),
                "Topper" => cfg.topper = val.parse().unwrap_or(0),
                "Antenna" => cfg.antenna = val.parse().unwrap_or(0),
                "PaintFinish" => cfg.paint_finish = val.parse().unwrap_or(0),
                "EngineAudio" => cfg.engine_audio = val.parse().unwrap_or(0),
                "Trail" => cfg.trail = val.parse().unwrap_or(0),
                "GoalExplosion" => cfg.goal_explosion = val.parse().unwrap_or(0),
                "PlayerBanner" => cfg.player_banner = val.parse().unwrap_or(0),
                "PlayerAnthem" => cfg.player_anthem = val.parse().unwrap_or(0),
                "AvatarBorder" => cfg.avatar_border = val.parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    Some(cfg)
}

#[derive(Debug, Clone, Default)]
pub struct DecalsIniConfig {
    pub enabled: bool,
    pub decal_name: String,
    pub body_id: i32,
    pub skin_id: i32,
    pub body_diffuse: String,
    pub body_skin: String,
    pub chassis_diffuse: String,
    pub chassis_masks: String,
}

/// Serializes and writes `decals.ini` directly to `<GameDir>/TAGame/Config/decals.ini`.
pub fn write_decals_ini(
    game_or_cooked_dir: &Path,
    cfg: &DecalsIniConfig,
) -> Result<PathBuf, String> {
    let config_dir = resolve_config_dir(game_or_cooked_dir);
    let ini_path = config_dir.join("decals.ini");

    let content = format!(
        "[VelocityRL.Decals]\r\n\
        Enabled={enabled}\r\n\
        DecalName={decal_name}\r\n\
        BodyID={body_id}\r\n\
        SkinID={skin_id}\r\n\
        BodyDiffuse={body_diffuse}\r\n\
        BodySkin={body_skin}\r\n\
        ChassisDiffuse={chassis_diffuse}\r\n\
        ChassisMasks={chassis_masks}\r\n",
        enabled = if cfg.enabled { "true" } else { "false" },
        decal_name = cfg.decal_name,
        body_id = cfg.body_id,
        skin_id = cfg.skin_id,
        body_diffuse = cfg.body_diffuse,
        body_skin = cfg.body_skin,
        chassis_diffuse = cfg.chassis_diffuse,
        chassis_masks = cfg.chassis_masks,
    );

    fs::write(&ini_path, content).map_err(|e| format!("Failed to write decals.ini: {e}"))?;
    let swaps = read_swaps_ini(game_or_cooked_dir).unwrap_or_default();
    let _ = write_tagame_ini_native(game_or_cooked_dir, &swaps, None, Some(&cfg.body_diffuse), Some(&cfg.body_skin));
    Ok(ini_path)
}

/// Reads `decals.ini` from `<GameDir>/TAGame/Config/decals.ini`.
pub fn read_decals_ini(game_or_cooked_dir: &Path) -> Option<DecalsIniConfig> {
    let config_dir = resolve_config_dir(game_or_cooked_dir);
    let ini_path = config_dir.join("decals.ini");
    let text = fs::read_to_string(&ini_path).ok()?;

    let mut cfg = DecalsIniConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "Enabled" => cfg.enabled = val.eq_ignore_ascii_case("true") || val == "1",
                "DecalName" => cfg.decal_name = val.to_string(),
                "BodyID" => cfg.body_id = val.parse().unwrap_or(0),
                "SkinID" => cfg.skin_id = val.parse().unwrap_or(0),
                "BodyDiffuse" => cfg.body_diffuse = val.to_string(),
                "BodySkin" => cfg.body_skin = val.to_string(),
                "ChassisDiffuse" => cfg.chassis_diffuse = val.to_string(),
                "ChassisMasks" => cfg.chassis_masks = val.to_string(),
                _ => {}
            }
        }
    }
    Some(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_and_read_swaps_ini() {
        let temp_dir = std::env::temp_dir().join("test_vel_swaps_ini");
        let cooked = temp_dir.join("CookedPCConsole");
        let _ = fs::create_dir_all(&cooked);

        let swaps = vec![
            SwapEntry {
                owned_id: 23,
                wanted_id: 4284,
                owned_name: "Octane".into(),
                wanted_name: "Fennec".into(),
                paint_id: 3,
                asset_package: "Body_Grain".into(),
                slot: Some("Body".into()),
            },
            SwapEntry {
                owned_id: 100,
                wanted_id: 1560,
                owned_name: "OEM".into(),
                wanted_name: "Cristiano".into(),
                paint_id: 12,
                asset_package: "W_Cristiano".into(),
                slot: Some("Wheels".into()),
            },
        ];

        let res = write_swaps_ini(&cooked, &swaps, true);
        assert!(res.is_ok());

        let read = read_swaps_ini(&cooked).expect("Must read written ini");
        assert!(read.enabled);
        assert_eq!(read.body, 4284);
        assert_eq!(read.body_paint, 3);
        assert_eq!(read.wheels, 1560);
        assert_eq!(read.wheels_paint, 12);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
