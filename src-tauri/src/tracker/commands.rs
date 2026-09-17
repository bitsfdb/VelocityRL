use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use crate::tracker::models::*;
use crate::tracker::session_tracker::SessionTracker;

#[tauri::command]
pub fn get_overlay_state(tracker: State<'_, Arc<Mutex<SessionTracker>>>) -> Result<OverlayStatePayload, String> {
    let t = tracker.lock().map_err(|e| e.to_string())?;
    Ok(t.to_overlay_payload())
}

#[tauri::command]
pub fn get_connection_status(tracker: State<'_, Arc<Mutex<SessionTracker>>>) -> Result<String, String> {
    let t = tracker.lock().map_err(|e| e.to_string())?;
    Ok(t.connection_status.clone())
}

#[tauri::command]
pub fn reset_session(tracker: State<'_, Arc<Mutex<SessionTracker>>>, app: AppHandle) -> Result<OverlayStatePayload, String> {
    use tauri::Emitter;
    let mut t = tracker.lock().map_err(|e| e.to_string())?;
    t.reset_session();
    let payload = t.to_overlay_payload();
    let _ = app.emit("overlay-state", &payload);
    Ok(payload)
}

#[tauri::command]
pub fn set_player_identity(
    primary_id: String,
    name_fallback: String,
    tracker: State<'_, Arc<Mutex<SessionTracker>>>,
) -> Result<(), String> {
    let mut t = tracker.lock().map_err(|e| e.to_string())?;
    t.config.player_primary_id = primary_id.trim().to_string();
    t.config.player_name_fallback = name_fallback.trim().to_string();
    Ok(())
}

#[tauri::command]
pub fn set_overlay_config(
    config: OverlayConfig,
    tracker: State<'_, Arc<Mutex<SessionTracker>>>,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;
    let mut t = tracker.lock().map_err(|e| e.to_string())?;
    t.config = config;
    let payload = t.to_overlay_payload();
    let _ = app.emit("overlay-state", &payload);
    Ok(())
}

#[tauri::command]
pub fn update_player_skill(
    _rating: i32,
    tracker: State<'_, Arc<Mutex<SessionTracker>>>,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;
    let t = tracker.lock().map_err(|e| e.to_string())?;
    let payload = t.to_overlay_payload();
    let _ = app.emit("overlay-state", &payload);
    Ok(())
}

#[tauri::command]
pub fn tracker_set_playlist(
    _playlist: i32,
    tracker: State<'_, Arc<Mutex<SessionTracker>>>,
    app: AppHandle,
) -> Result<OverlayStatePayload, String> {
    use tauri::Emitter;
    let t = tracker.lock().map_err(|e| e.to_string())?;
    let payload = t.to_overlay_payload();
    let _ = app.emit("overlay-state", &payload);
    Ok(payload)
}

#[tauri::command]
pub fn set_click_through(app: AppHandle, enable: bool) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        win.set_ignore_cursor_events(enable).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn test_simulate_match(
    win: bool,
    app: AppHandle,
    tracker: State<'_, Arc<Mutex<SessionTracker>>>,
) -> Result<(), String> {
    crate::tracker::stats_api::simulate_fake_match(&app, &tracker, win);
    Ok(())
}

#[tauri::command]
pub fn tracker_open_overlay_window(app: AppHandle, style: Option<String>) -> Result<(), String> {
    let session = tracker_load_session(app.clone()).unwrap_or_default();
    let s = style.unwrap_or_else(|| session.overlay_style.clone());
    let scale_factor = session.scale as f64 / 100.0;
    let (base_w, base_h) = match s.as_str() {
        "circle" => (280.0, 165.0),
        "minimal" => (210.0, 45.0),
        "redesigned" => (165.0, 100.0),
        _ => (280.0, 165.0),
    };
    let (w, h) = (base_w * scale_factor, base_h * scale_factor);

    if let Some(win) = app.get_webview_window("tracker_overlay") {
        let _ = win.unminimize();
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(w, h)));
        let _ = win.show();
        let _ = win.set_always_on_top(true);
        let _ = win.set_ignore_cursor_events(session.is_locked);
        let _ = tracker_position_overlay(app.clone(), session.position.clone());
    }

    Ok(())
}

#[tauri::command]
pub fn tracker_set_overlay_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)));
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_close_overlay_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        let _ = win.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_start_dragging(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        win.start_dragging().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_move_overlay_by(app: AppHandle, dx: i32, dy: i32) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        if let Ok(pos) = win.outer_position() {
            let new_x = (pos.x + dx).max(0);
            let new_y = (pos.y + dy).max(0);
            let _ = win.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(new_x, new_y)));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_center_overlay(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        let _ = win.center();
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_position_overlay(app: AppHandle, pos: String) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        if pos == "center" {
            let _ = win.center();
            return Ok(());
        }
        if pos.starts_with("custom:") {
            let parts: Vec<&str> = pos.trim_start_matches("custom:").split(',').collect();
            if parts.len() == 2 {
                if let (Ok(x), Ok(y)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                    if x > 10 && y > 10 {
                        let _ = win.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)));
                        return Ok(());
                    }
                }
            }
        }
        let monitor = win.current_monitor().ok().flatten().or_else(|| win.primary_monitor().ok().flatten());
        if let Some(mon) = monitor {
            let origin = mon.position();
            let size = mon.size();
            let scale = mon.scale_factor().max(1.0);
            let (win_w, win_h) = if let Ok(sz) = win.outer_size() {
                (sz.width as i32, sz.height as i32)
            } else {
                ((330.0 * scale) as i32, (235.0 * scale) as i32)
            };
            let pad = (20.0 * scale) as i32;

            let (phys_x, phys_y) = match pos.as_str() {
                "top-left" => (origin.x + pad, origin.y + pad),
                "top-right" => (origin.x + size.width as i32 - win_w - pad, origin.y + pad),
                "bottom-left" => (origin.x + pad, origin.y + size.height as i32 - win_h - pad),
                "center" => {
                    let _ = win.center();
                    return Ok(());
                },
                _ => (origin.x + size.width as i32 - win_w - pad, origin.y + size.height as i32 - win_h - pad),
            };

            let _ = win.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(phys_x, phys_y)));
        } else {
            let _ = win.center();
        }
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_apply_scale_opacity(app: AppHandle, scale: i32, opacity: i32) -> Result<(), String> {
    use tauri::Emitter;
    let _ = app.emit("tracker-scale-changed", scale);
    let _ = app.emit("tracker-opacity-changed", opacity);
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        let js = format!(
            "if(typeof applyScale==='function')applyScale({});if(typeof applyOpacity==='function')applyOpacity({});",
            scale, opacity
        );
        let _ = win.eval(&js);
    }
    Ok(())
}

#[tauri::command]
pub fn tracker_set_overlay_locked(app: AppHandle, locked: bool) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tracker_overlay") {
        let _ = win.set_ignore_cursor_events(locked);
        if locked {
            if let Ok(pos) = win.outer_position() {
                if let Ok(mut session) = tracker_load_session(app.clone()) {
                    session.position = format!("custom:{},{}", pos.x, pos.y);
                    session.is_locked = true;
                    let _ = tracker_save_session(session, app.clone(), app.state());
                }
            }
        }
    }
    use tauri::Emitter;
    let _ = app.emit("tracker-overlay-locked", locked);
    Ok(())
}

#[tauri::command]
pub fn tracker_load_session(app: AppHandle) -> Result<TrackerUiSession, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let path = dir.join("tracker_ui_session.json");
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(parsed) = serde_json::from_str::<TrackerUiSession>(&data) {
            return Ok(parsed);
        }
    }
    Ok(TrackerUiSession::default())
}

#[tauri::command]
pub fn tracker_save_session(
    session: TrackerUiSession,
    app: AppHandle,
    tracker: State<'_, Arc<Mutex<SessionTracker>>>,
) -> Result<(), String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("tracker_ui_session.json");
    if let Ok(json) = serde_json::to_string_pretty(&session) {
        let _ = std::fs::write(&path, json);
    }
    if let Ok(mut t) = tracker.lock() {
        t.config.is_locked = session.is_locked;
        t.config.position = session.position.clone();
        if session.win_delta > 0 {
            t.config.win_delta = session.win_delta;
        }
        if session.loss_delta > 0 {
            t.config.loss_delta = session.loss_delta;
        }
        if !session.display_name.is_empty() {
            t.config.player_name_fallback = session.display_name.clone();
        }
    }
    let _ = app.emit("tracker-style-changed", &session.overlay_style);
    let _ = app.emit("tracker-overlay-locked", session.is_locked);

    let target = tauri::EventTarget::webview_window("tracker_overlay");
    let _ = app.emit_to(target.clone(), "tracker-scale-changed", session.scale);
    let _ = app.emit_to(target, "tracker-opacity-changed", session.opacity);
    Ok(())
}

#[tauri::command]
pub fn tracker_ensure_stats_api(game_dir: String) -> Result<bool, String> {
    Ok(ensure_stats_api_files(&game_dir))
}

pub fn stats_api_game_dir(app: &AppHandle) -> String {
    let cfg_path = app
        .path()
        .app_config_dir()
        .map(|d| d.join("config.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("config.json"));
    if let Ok(content) = std::fs::read_to_string(&cfg_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(dir) = v["game_dir"].as_str() {
                return dir.to_string();
            }
        }
    }
    String::new()
}

pub fn find_tagame_dir(game_dir: &str) -> Option<std::path::PathBuf> {
    let mut path = std::path::PathBuf::from(game_dir);
    for _ in 0..4 {
        if path.join("TAGame").is_dir() {
            return Some(path.join("TAGame"));
        }
        if path
            .file_name()
            .map(|n| n.to_string_lossy().eq_ignore_ascii_case("TAGame"))
            .unwrap_or(false)
        {
            return Some(path.clone());
        }
        if !path.pop() {
            break;
        }
    }
    None
}

pub struct StatsApiBinding {
    pub ok: bool,
    pub game_running: bool,
    pub needs_fix: bool,
    pub detail: String,
}

fn user_rl_config_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let profile = std::env::var("USERPROFILE").ok()?;
        let p = std::path::PathBuf::from(profile);
        let candidates = [
            p.join("Documents").join("My Games").join("Rocket League").join("TAGame").join("Config"),
            p.join("OneDrive").join("Documents").join("My Games").join("Rocket League").join("TAGame").join("Config"),
            p.join("OneDrive").join("Documentos").join("My Games").join("Rocket League").join("TAGame").join("Config"),
            p.join("Documentos").join("My Games").join("Rocket League").join("TAGame").join("Config"),
        ];
        for c in &candidates {
            if c.exists() {
                return Some(c.clone());
            }
        }
        Some(candidates[0].clone())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn parse_binding_ini(path: &std::path::Path) -> Option<(f64, i32)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut rate: Option<f64> = None;
    let mut web_port: Option<i32> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("PacketSendRate=") {
            rate = v.trim().parse::<f64>().ok();
        } else if let Some(v) = line.strip_prefix("WebPort=") {
            web_port = v.trim().parse::<i32>().ok();
        }
    }
    Some((rate.unwrap_or(0.0), web_port.unwrap_or(49124)))
}

pub fn check_stats_api_binding(_game_dir: &str) -> StatsApiBinding {
    let game_running = crate::psynet::rocket_league_lock_holder().is_some();
    if !game_running {
        return StatsApiBinding {
            ok: false,
            game_running: false,
            needs_fix: false,
            detail: "Waiting for Rocket League".into(),
        };
    }

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(user_cfg) = user_rl_config_dir() {
        candidates.push(user_cfg.join("TAStatsAPI.ini"));
        candidates.push(user_cfg.join("DefaultStatsAPI.ini"));
    }

    let mut found = false;
    let mut rate = 0.0f64;
    let mut web_port = 49124i32;
    for c in &candidates {
        if let Some((r, w)) = parse_binding_ini(c) {
            found = true;
            rate = r;
            web_port = w;
            break;
        }
    }

    if !found {
        return StatsApiBinding {
            ok: false,
            game_running: true,
            needs_fix: true,
            detail: "Stats API ini missing — restart Rocket League after VelocityRL writes it".into(),
        };
    }
    if rate <= 0.0 {
        return StatsApiBinding {
            ok: false,
            game_running: true,
            needs_fix: true,
            detail: "Stats API disabled (PacketSendRate=0) — restart Rocket League to pick up the fix".into(),
        };
    }
    if web_port != 49124 {
        return StatsApiBinding {
            ok: false,
            game_running: true,
            needs_fix: true,
            detail: format!("Stats API WebPort={} but tracker expects 49124 — restart Rocket League", web_port),
        };
    }

    StatsApiBinding {
        ok: true,
        game_running: true,
        needs_fix: false,
        detail: "Rocket League detected — starting Stats API...".into(),
    }
}

pub fn ensure_stats_api_files(game_dir: &str) -> bool {
    crate::applog::event(&format!("tracker: checking Stats API configs (game_dir: {})", game_dir));

    let content = "[TAGame.MatchStatsExporter_TA]\nPacketSendRate=30\nPort=49123\nWebPort=49124\n";
    let mut modified = false;

    let is_correct = |s: &str| -> bool {
        s.contains("MatchStatsExporter_TA")
            && s.contains("WebPort=49124")
            && s.contains("PacketSendRate=")
            && !s.contains("PacketSendRate=0\n")
            && !s.contains("PacketSendRate=0.0\n")
    };

    let mut write_if_needed = |dir: &std::path::Path, filename: &str| {
        let _ = std::fs::create_dir_all(dir);
        let file_path = dir.join(filename);
        let should_write = match std::fs::read_to_string(&file_path) {
            Ok(existing) => !is_correct(&existing),
            Err(_) => true,
        };
        if should_write {
            match std::fs::write(&file_path, content) {
                Ok(_) => {
                    crate::applog::event(&format!("tracker: Stats API wrote {} -> {}", filename, file_path.display()));
                    modified = true;
                }
                Err(e) => {
                    crate::applog::event(&format!("tracker: Stats API FAILED to write {} -> {} : {}", filename, file_path.display(), e));
                }
            }
        } else {
            crate::applog::event(&format!("tracker: Stats API verified {} is up to date", filename));
        }
    };

    // Clean up any alien TAStatsAPI.ini from the game installation directory.
    // Easy Anti-Cheat (EAC) hashes and verifies all files in <install>/TAGame/Config/
    // against the store manifest. Any unexpected or modified files in that directory
    // cause EAC to abort the EOS authentication ticket and lock out online play.
    if let Some(tagame) = find_tagame_dir(game_dir) {
        let alien_stats = tagame.join("Config").join("TAStatsAPI.ini");
        if alien_stats.exists() {
            if let Err(e) = std::fs::remove_file(&alien_stats) {
                crate::applog::event(&format!("tracker: could not remove alien TAStatsAPI.ini from game install: {e}"));
            } else {
                crate::applog::event("tracker: removed alien TAStatsAPI.ini from game install Config (prevents EAC tamper detection)");
            }
        }
    }

    // Stats API files belong exclusively in the user Documents directory:
    // Documents\My Games\Rocket League\TAGame\Config\
    // EAC does not monitor user Documents, and UE3 prioritizes Documents config overrides.
    if let Some(user_config) = user_rl_config_dir() {
        crate::applog::event(&format!("tracker: checking Documents user config -> {}", user_config.display()));
        write_if_needed(&user_config, "TAStatsAPI.ini");
        write_if_needed(&user_config, "DefaultStatsAPI.ini");
    }

    crate::applog::event(&format!("tracker: Stats API check complete (modified={})", modified));
    modified
}
