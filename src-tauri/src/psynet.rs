use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tauri::{Manager, State};

#[derive(Default)]
pub struct PsyNetState {

    pub running: Mutex<bool>,
}

static PROXY_LIFECYCLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

static PROXY_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn load_proxy_dir_override(app: &tauri::AppHandle) {
    let cfg_path = app
        .path()
        .app_config_dir()
        .map(|d| d.join("config.json"))
        .unwrap_or_default();
    let override_dir = fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("proxy_dir").and_then(|p| p.as_str()).map(|s| s.to_string()));
    match override_dir {
        Some(dir) if !dir.trim().is_empty() => {
            crate::applog::event(&format!("psynet: proxy dir override loaded from config: {dir}"));
            *PROXY_DIR_OVERRIDE.lock().unwrap() = Some(PathBuf::from(dir));
        }
        _ => {}
    }
}

fn proxy_dir_override() -> Option<PathBuf> {
    PROXY_DIR_OVERRIDE.lock().ok()?.clone()
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TitleColorPayload {
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub glow_color: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TitleSwapEntry {
    #[serde(default)]
    pub equip_title_id: String,
    #[serde(default)]
    pub display_title_id: String,
    #[serde(default)]
    pub custom_text: String,
    #[serde(default)]
    pub category: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_color: Option<TitleColorPayload>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LogoSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub logo_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BlogSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub motd: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PaletteSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PingSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ms: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FakeRankOverridePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_mmr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mu: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub division: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_streak: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FakeRewardLevelsPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_level: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_level_wins: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FakeRanksPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<FakeRankOverridePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlists: Option<std::collections::HashMap<String, FakeRankOverridePayload>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward_levels: Option<FakeRewardLevelsPayload>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct InventorySpoofItemPayload {
    #[serde(default)]
    pub product_id: i32,
    #[serde(default)]
    pub paint_id: i32,
    #[serde(default)]
    pub series_id: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slot: String,
    #[serde(default)]
    pub dlc: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct InventorySpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub items: Vec<InventorySpoofItemPayload>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CameraLimitPayload {
    #[serde(default)]
    pub min: f64,
    #[serde(default)]
    pub max: f64,
    #[serde(default)]
    pub interval: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CameraSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub fov: CameraLimitPayload,
    #[serde(default)]
    pub height: CameraLimitPayload,
    #[serde(default)]
    pub distance: CameraLimitPayload,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SpoofPayload {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub equip_title_id: String,
    #[serde(default)]
    pub display_title_id: String,
    #[serde(default)]
    pub custom_text: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub custom_name: String,

    #[serde(default)]
    pub logo_spoof: Option<LogoSpoofPayload>,

    #[serde(default)]
    pub blog_spoof: Option<BlogSpoofPayload>,

    #[serde(default)]
    pub camera_spoof: Option<CameraSpoofPayload>,

    #[serde(default, skip_serializing)]
    pub ping_spoof: Option<PingSpoofPayload>,

    #[serde(default)]
    pub fake_ranks: Option<FakeRanksPayload>,

    #[serde(default)]
    pub palette_spoof: Option<PaletteSpoofPayload>,
    #[serde(default, skip_serializing)]
    pub inventory_spoof: Option<InventorySpoofPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_color: Option<TitleColorPayload>,
    #[serde(default)]
    pub swaps: Option<Vec<TitleSwapEntry>>,
    #[serde(default = "default_method")]
    pub method: String,
}

fn default_true() -> bool {
    true
}
fn default_method() -> String {
    "raw".into()
}

#[derive(Serialize)]
pub struct PsyNetStatus {
    pub running: bool,
    pub proxy_dir: Option<String>,
    pub config_path: Option<String>,
    pub warning: String,
    pub hosts_redirected: bool,
    pub port443_ok: bool,
    pub port443_owner: Option<String>,
    pub last_capture_secs_ago: Option<u64>,
    pub viewer_ok: bool,
    pub player_id: Option<String>,
    pub ca_installed: bool,
}

pub const CLOSE_WARNING: &str = "Keep VelocityRL open while playing — closing the app stops the proxy and Rocket League loses config.psynet.gg.";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn normalize_win_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn normalize_win_path(path: PathBuf) -> PathBuf {
    path
}

fn find_proxy_dir() -> Result<PathBuf, String> {
    if let Some(ov) = proxy_dir_override() {
        let canon = normalize_win_path(fs::canonicalize(&ov).unwrap_or_else(|_| ov.clone()));
        if canon.is_dir() {
            crate::applog::event(&format!("psynet: find_proxy_dir used override {}", canon.display()));
            return Ok(canon);
        }
    }

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let p = PathBuf::from(appdata).join("VelocityRL").join("proxy");
        let _ = fs::create_dir_all(&p);
        return Ok(p);
    }

    if let Some(localappdata) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(localappdata).join("VelocityRL").join("proxy");
        let _ = fs::create_dir_all(&p);
        return Ok(p);
    }

    let p = std::env::temp_dir().join("VelocityRL_proxy");
    let _ = fs::create_dir_all(&p);
    Ok(p)
}

pub fn config_dir() -> PathBuf {
    find_proxy_dir().unwrap_or_else(|_| std::env::temp_dir().join("VelocityRL_proxy"))
}

pub fn default_spoof_payload() -> SpoofPayload {
    SpoofPayload {
        enabled: true,
        equip_title_id: "Team_Iraq_World_Cup_2026".into(),
        display_title_id: "RLCS_X_Champion".into(),
        custom_text: "RLCS X Champion".into(),
        category: "RLCS_Champion".into(),
        custom_name: String::new(),
        logo_spoof: None,
        blog_spoof: None,
        camera_spoof: None,
        ping_spoof: None,
        fake_ranks: None,
        palette_spoof: None,
        inventory_spoof: None,
        title_color: None,
        swaps: Some(vec![TitleSwapEntry {
            equip_title_id: "Team_Iraq_World_Cup_2026".into(),
            display_title_id: "RLCS_X_Champion".into(),
            custom_text: "RLCS X Champion".into(),
            category: "RLCS_Champion".into(),
            title_color: None,
        }]),
        method: "raw".into(),
    }
}

pub fn validate_spoof_colors(cfg: &SpoofPayload) {
    if !cfg.enabled {
        return;
    }
    let mut seen_categories: std::collections::HashMap<String, (String, String, String)> =
        std::collections::HashMap::new();

    let mut check = |target_id: &str, cat: &str, tc: &Option<TitleColorPayload>| {
        if let Some(c) = tc {
            if crate::proxy::is_hex6(&c.color) {
                let glow = if crate::proxy::is_hex6(&c.glow_color) {
                    &c.glow_color
                } else {
                    &c.color
                };
                let custom_cat = if !cat.trim().is_empty() && cat.trim().starts_with("RLItemMod_") {
                    cat.trim().to_string()
                } else {
                    format!("RLItemMod_{}", crate::proxy::sanitize_category_part(target_id))
                };
                let col = c.color.to_ascii_uppercase();
                let glw = glow.to_ascii_uppercase();
                if let Some((prev_c, prev_g, prev_id)) = seen_categories.get(&custom_cat) {
                    if prev_c != &col || prev_g != &glw {
                        crate::applog::event(&format!(
                            "psynet: WARNING: Category '{}' has conflicting custom colors between title '{}' (#{}/#{}) and title '{}' (#{}/#{}). Won't show conflicting color.",
                            custom_cat, prev_id, prev_c, prev_g, target_id, col, glw
                        ));
                    }
                } else {
                    seen_categories.insert(custom_cat, (col, glw, target_id.to_string()));
                }
            }
        }
    };

    if !cfg.equip_title_id.trim().is_empty() {
        check(cfg.equip_title_id.trim(), cfg.category.trim(), &cfg.title_color);
    }
    if let Some(swaps) = &cfg.swaps {
        for sw in swaps {
            let target_id = if !sw.equip_title_id.trim().is_empty() {
                sw.equip_title_id.trim()
            } else {
                sw.display_title_id.trim()
            };
            if !target_id.is_empty() {
                check(target_id, sw.category.trim(), &sw.title_color);
            }
        }
    }
}

pub fn load_active_spoof_from_disk() -> Option<SpoofPayload> {
    let dir = config_dir();
    let path = config_path(&dir);
    if !path.is_file() {
        return Some(default_spoof_payload());
    }
    let raw = fs::read_to_string(&path).ok()?;
    let raw = raw.trim_start_matches('\u{feff}');
    let res: Option<SpoofPayload> = serde_json::from_str(raw).ok();
    if let Some(cfg) = &res {
        validate_spoof_colors(cfg);
    }
    res
}

pub fn read_or_default_spoof(dir: &Path) -> Result<SpoofPayload, String> {
    let path = config_path(dir);
    if path.is_file() {
        if let Ok(raw) = fs::read_to_string(&path) {
            let raw = raw.trim_start_matches('\u{feff}');
            if let Ok(p) = serde_json::from_str::<SpoofPayload>(raw) {
                validate_spoof_colors(&p);
                return Ok(p);
            }
        }
    }
    let def = default_spoof_payload();
    let _ = write_spoof(dir, &def);
    Ok(def)
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("psynet_config.json")
}

#[allow(dead_code)]
pub fn get_real_skill_mmr(target_playlist: Option<i32>) -> Option<i32> {
    let dir = find_proxy_dir().ok()?;
    let path = dir.join("real_skill.json");
    let text = fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let skills = v.get("skills")?.as_array()?;
    if skills.is_empty() {
        return None;
    }
    if let Some(target) = target_playlist {
        for s in skills {
            if s.get("playlist").and_then(|p| p.as_i64()) == Some(target as i64) {
                if let Some(mmr) = s.get("display_mmr").and_then(|m| m.as_i64()) {
                    if mmr > 0 {
                        return Some(mmr as i32);
                    }
                }
            }
        }
    }
    for pref_pl in [11, 13, 10, 28, 27, 29, 30] {
        for s in skills {
            if s.get("playlist").and_then(|p| p.as_i64()) == Some(pref_pl) {
                if let Some(mmr) = s.get("display_mmr").and_then(|m| m.as_i64()) {
                    if mmr > 0 {
                        return Some(mmr as i32);
                    }
                }
            }
        }
    }
    for s in skills {
        if let Some(mmr) = s.get("display_mmr").and_then(|m| m.as_i64()) {
            if mmr > 0 {
                return Some(mmr as i32);
            }
        }
    }
    None
}

#[allow(dead_code)]
pub fn get_player_mmr(target_playlist: Option<i32>) -> Option<i32> {
    get_real_skill_mmr(target_playlist)
}

fn read_config_player_id(dir: &Path) -> Option<String> {
    let path = config_path(dir);
    let text = fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid_str = v.get("identity")
        .and_then(|id| id.get("player_id"))
        .or_else(|| v.get("name_spoof").and_then(|ns| ns.get("player_id")))?
        .as_str()?
        .trim();
    if pid_str.is_empty() {
        None
    } else {
        Some(pid_str.to_string())
    }
}

fn status_for(dir: Option<PathBuf>, process_alive: bool) -> PsyNetStatus {
    let hosts_redirected = psynet_hosts_redirected();
    let (port443_ok, port443_owner) = loopback443_status();

    let viewer_ok = process_alive && port443_ok;

    let running = process_alive && port443_ok;
    let last_capture_secs_ago = None;
    let ca_installed = is_ca_installed();
    PsyNetStatus {
        running,
        config_path: dir.as_ref().map(|d| config_path(d).to_string_lossy().into_owned()),
        proxy_dir: dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
        warning: CLOSE_WARNING.into(),
        hosts_redirected,
        port443_ok,
        port443_owner,
        last_capture_secs_ago,
        viewer_ok,
        player_id: dir.as_ref().and_then(|d| read_config_player_id(d)),
        ca_installed,
    }
}

fn camera_limit_json(l: &CameraLimitPayload, def_min: f64, def_max: f64, def_interval: f64) -> serde_json::Value {
    let mut min = l.min;
    let mut max = l.max;
    let mut interval = l.interval;
    if max <= 0.0 && min <= 0.0 {
        min = def_min;
        max = def_max;
    }
    if interval <= 0.0 {
        interval = def_interval;
    }
    if max < min {
        max = min;
    }
    serde_json::json!({
        "min": min,
        "max": max,
        "interval": interval,
    })
}

fn fake_rank_override_json(ov: &FakeRankOverridePayload) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    let mmr = ov.display_mmr.map(|v| v.max(0.0));
    let mu = ov.mu.map(|v| {
        let display = v * 20.0 + 100.0;
        let clamped_display = display.max(0.0);
        (clamped_display - 100.0) / 20.0
    }).or_else(|| mmr.map(|m| (m - 100.0) / 20.0));

    if let Some(v) = mmr {
        m.insert("display_mmr".into(), serde_json::json!(v.round() as i64));
    }
    if let Some(v) = mu {
        m.insert("mu".into(), serde_json::json!((v * 10000.0).round() / 10000.0));
    }
    if let Some(v) = ov.sigma {
        m.insert("sigma".into(), serde_json::json!(v));
    }
    if let Some(v) = ov.tier {
        m.insert("tier".into(), serde_json::json!(v.clamp(0, 22)));
    }
    if let Some(v) = ov.division {
        m.insert("division".into(), serde_json::json!(v.clamp(0, 3)));
    }
    if let Some(v) = ov.win_streak {
        m.insert("win_streak".into(), serde_json::json!(v.clamp(0, 100)));
    }
    serde_json::Value::Object(m)
}

fn resolve_swaps(payload: &SpoofPayload) -> Option<Vec<TitleSwapEntry>> {
    if let Some(swaps) = &payload.swaps {
        let from_array: Vec<TitleSwapEntry> = swaps
            .iter()
            .filter(|s| !s.equip_title_id.is_empty())
            .cloned()
            .collect();
        return Some(from_array);
    }
    if payload.equip_title_id.is_empty() {
        return None;
    }
    Some(vec![TitleSwapEntry {
        equip_title_id: payload.equip_title_id.clone(),
        display_title_id: payload.display_title_id.clone(),
        custom_text: payload.custom_text.clone(),
        category: payload.category.clone(),
        title_color: payload.title_color.clone(),
    }])
}

fn write_spoof(dir: &Path, payload: &SpoofPayload) -> Result<PathBuf, String> {
    let path = config_path(dir);

    let method = "raw";

    let mut body = if path.is_file() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| {
                let trimmed = s.trim_start_matches('\u{feff}');
                serde_json::from_str::<serde_json::Value>(trimmed).ok()
            })
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = body.as_object_mut() {
        obj.insert("method".into(), serde_json::json!(method));

        obj.remove("observe_only");
        obj.remove("inventory_spoof");
        obj.remove("ping_spoof");
        if !payload.custom_name.is_empty() {
            obj.insert("custom_name".into(), serde_json::json!(payload.custom_name));
        }
        if let Some(ls) = &payload.logo_spoof {

            obj.insert(
                "logo_spoof".into(),
                serde_json::json!({
                    "enabled": ls.enabled,
                    "logo_url": ls.logo_url.trim(),
                }),
            );
        }
        if let Some(bs) = &payload.blog_spoof {

            obj.insert(
                "blog_spoof".into(),
                serde_json::json!({
                    "enabled": bs.enabled,
                    "motd": bs.motd.trim(),
                }),
            );
        }
        if let Some(cam) = &payload.camera_spoof {
            obj.insert(
                "camera_spoof".into(),
                serde_json::json!({
                    "enabled": cam.enabled,
                    "fov": camera_limit_json(&cam.fov, 60.0, 1000.0, 1.0),
                    "height": camera_limit_json(&cam.height, 40.0, 1000.0, 1.0),
                    "distance": camera_limit_json(&cam.distance, 100.0, 1000.0, 1.0),
                }),
            );
        }

        if let Some(fr) = &payload.fake_ranks {
            let mut fr_obj = serde_json::json!({
                "enabled": fr.enabled,
            });
            if let Some(m) = fr_obj.as_object_mut() {
                if let Some(def) = &fr.default {
                    m.insert("default".into(), fake_rank_override_json(def));
                }
                if let Some(pls) = &fr.playlists {
                    let mut map = serde_json::Map::new();
                    for (k, ov) in pls {
                        map.insert(k.clone(), fake_rank_override_json(ov));
                    }
                    m.insert("playlists".into(), serde_json::Value::Object(map));
                }
                if let Some(rl) = &fr.reward_levels {
                    let mut rl_obj = serde_json::Map::new();
                    if let Some(v) = rl.season_level {
                        let level = v.clamp(0, 8);
                        rl_obj.insert("season_level".into(), serde_json::json!(level));
                    }
                    if let Some(v) = rl.season_level_wins {
                        let wins = v.clamp(0, 10);
                        rl_obj.insert("season_level_wins".into(), serde_json::json!(wins));
                    }
                    if !rl_obj.is_empty() {
                        m.insert("reward_levels".into(), serde_json::Value::Object(rl_obj));
                    }
                }
            }
            obj.insert("fake_ranks".into(), fr_obj);
        }

        if let Some(swaps) = resolve_swaps(payload) {
            obj.insert("enabled".into(), serde_json::json!(payload.enabled));
            obj.insert("swaps".into(), serde_json::to_value(&swaps).unwrap_or(serde_json::json!([])));
            if let Some(first) = swaps.first() {
                obj.insert("equip_title_id".into(), serde_json::json!(first.equip_title_id));
                obj.insert("display_title_id".into(), serde_json::json!(first.display_title_id));
                obj.insert("custom_text".into(), serde_json::json!(first.custom_text));
                obj.insert("category".into(), serde_json::json!(first.category));
                if let Some(tc) = &first.title_color {
                    obj.insert("title_color".into(), serde_json::to_value(tc).unwrap_or(serde_json::Value::Null));
                } else {
                    obj.remove("title_color");
                }
            } else {
                obj.insert("equip_title_id".into(), serde_json::json!(""));
                obj.insert("display_title_id".into(), serde_json::json!(""));
                obj.insert("custom_text".into(), serde_json::json!(""));
                obj.insert("category".into(), serde_json::json!(""));
                obj.remove("title_color");
            }
        }
    }
    fs::write(&path, serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}


#[cfg(windows)]
const RL_PROCESS_NAMES: [&str; 3] = [
    "rocketleague.exe",
    "rocketleague_eac.exe",
    "rocketleague_eos.exe",
];

#[cfg(windows)]
pub fn rocket_league_process() -> Option<(String, u32)> {
    crate::winprobe::find_process_any(&RL_PROCESS_NAMES).map(|(pid, name)| (name, pid))
}

#[cfg(not(windows))]
pub fn rocket_league_process() -> Option<(String, u32)> {
    None
}

pub fn rocket_league_lock_holder() -> Option<String> {
    rocket_league_process().map(|(name, pid)| format!("{name}, PID {pid}"))
}

fn rocket_league_running() -> bool {
    rocket_league_process().is_some()
}

fn windows_hosts_path() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    PathBuf::from(root)
        .join("System32")
        .join("drivers")
        .join("etc")
        .join("hosts")
}

const CONFIG_HOST_PAIRS: &[(&str, &str)] = &[
    ("127.0.0.1", "config.psynet.gg"),
    ("::1", "config.psynet.gg"),
];

fn hosts_has_pair(text: &str, ip: &str, host: &str) -> bool {
    let ip_l = ip.to_ascii_lowercase();
    let host_l = host.to_ascii_lowercase();
    text.lines().any(|line| {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            return false;
        }
        let lower = t.to_ascii_lowercase();
        let mut parts = lower.split_whitespace();
        let Some(first) = parts.next() else {
            return false;
        };
        first == ip_l && parts.any(|p| p == host_l)
    })
}

fn config_hosts_complete() -> bool {
    #[cfg(windows)]
    {
        let Ok(bytes) = fs::read(windows_hosts_path()) else {
            return false;
        };
        let text = String::from_utf8_lossy(&bytes);
        CONFIG_HOST_PAIRS
            .iter()
            .all(|(ip, host)| hosts_has_pair(&text, ip, host))
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn psynet_hosts_redirected() -> bool {
    #[cfg(windows)]
    {
        let Ok(bytes) = fs::read(windows_hosts_path()) else {
            return false;
        };
        let text = String::from_utf8_lossy(&bytes);
        CONFIG_HOST_PAIRS
            .iter()
            .any(|(ip, host)| hosts_has_pair(&text, ip, host))
            || text.contains("api.rlpp.psynet.gg")
            || text.contains("ws.rlpp.psynet.gg")
    }
    #[cfg(not(windows))]
    {
        false
    }
}

static HOSTS_ENSURE: Mutex<()> = Mutex::new(());

#[cfg(windows)]
fn loopback443_status() -> (bool, Option<String>) {
    if crate::proxy::is_proxy_running() {
        return (true, Some("VelocityRL".to_string()));
    }
    if let Some(pid) = crate::winprobe::loopback_443_owner() {
        let name = crate::winprobe::process_name(pid).unwrap_or_else(|| "?".to_string());
        (false, Some(name))
    } else {
        (false, None)
    }
}

#[cfg(not(windows))]
fn loopback443_status() -> (bool, Option<String>) {
    (crate::proxy::is_proxy_running(), Some("VelocityRL".to_string()))
}

pub fn kill_proxy_on_exit() {
    crate::applog::event("psynet: exit cleanup — stopping proxy and reverting hosts");
    crate::proxy::stop_native_proxy(true);
    let _ = revert_config_hosts();
}

#[cfg(windows)]
fn is_process_elevated() -> bool {
    crate::winprobe::is_elevated()
}





#[cfg(windows)]
fn encode_powershell_cmd(ps_script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = ps_script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(&utf16)
}

#[cfg(windows)]
fn run_elevated_script(script_text: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let encoded = encode_powershell_cmd(script_text);

    let status = if is_process_elevated() {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
                &encoded,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("setup failed: {e}"))?
    } else {
        let runner_cmd = format!(
            "$p = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile','-NonInteractive','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-EncodedCommand','{encoded}'); if ($null -eq $p) {{ exit 1223 }}; exit $p.ExitCode"
        );
        let outer_encoded = encode_powershell_cmd(&runner_cmd);
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
                &outer_encoded,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("elevate failed: {e}"))?
    };

    if status.success() {
        return Ok(());
    }

    let code = status.code();
    if code == Some(1223) {
        return Err(
            "UAC was cancelled. Approve the Administrator prompt to install the CA, edit hosts, and bind :443."
                .into(),
        );
    }

    Err(format!(
        "Proxy setup failed (exit {code:?}). Approve UAC when prompted, then retry."
    ))
}

#[cfg(not(windows))]
fn run_elevated_script(_script_text: &str) -> Result<(), String> {
    Err("PsyNet proxy is Windows-only for now.".into())
}

#[cfg(windows)]
const BUNDLED_CA_THUMBPRINT: &str = "05969B177719D7613DBED10B7FBE4A0DD846EB7A";

#[cfg(windows)]
pub fn is_user_ca_installed() -> bool {
    use std::os::windows::process::CommandExt;
    let thumb_lower = BUNDLED_CA_THUMBPRINT.to_ascii_lowercase();
    Command::new("certutil")
        .args(["-user", "-store", "Root", BUNDLED_CA_THUMBPRINT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| {
            if !o.status.success() {
                return false;
            }
            let text = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            text.contains(&thumb_lower) || text.contains(BUNDLED_CA_THUMBPRINT)
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_ca_installed() -> bool {
    use std::os::windows::process::CommandExt;
    let thumb_lower = BUNDLED_CA_THUMBPRINT.to_ascii_lowercase();
    let in_system = Command::new("certutil")
        .args(["-store", "Root", BUNDLED_CA_THUMBPRINT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| {
            if !o.status.success() {
                return false;
            }
            let text = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            text.contains(&thumb_lower) || text.contains(BUNDLED_CA_THUMBPRINT)
        })
        .unwrap_or(false);
    let in_user = is_user_ca_installed();
    in_system || in_user
}

#[cfg(windows)]
pub fn install_user_ca_direct() {
    use std::os::windows::process::CommandExt;
    cleanup_stale_user_ca();

    if is_user_ca_installed() {
        return;
    }
    let tmp = std::env::temp_dir().join(format!("vrl_ca_{}.crt", std::process::id()));
    let tmp_str = tmp.to_string_lossy().replace('\'', "''");
    if fs::write(&tmp, crate::proxy::ca_cert_bytes()).is_ok() {
        let ps_cmd = format!(
            r#"$cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2('{tmp_str}'); $store = New-Object System.Security.Cryptography.X509Certificates.X509Store('Root', 'CurrentUser'); $store.Open('ReadWrite'); $store.Add($cert); $store.Close()"#
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps_cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        let _ = Command::new("certutil")
            .args(["-user", "-f", "-addstore", "Root", &tmp.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(windows)]
fn cleanup_stale_user_ca() {
    use std::os::windows::process::CommandExt;
    // Remove non-matching VelocityRL certs and erroneously installed leaf certs from CurrentUser\Root
    let cmd = format!(
        r#"$target = "{BUNDLED_CA_THUMBPRINT}"; Get-ChildItem Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object {{ (($_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*") -and $_.Thumbprint -notlike "*$target*") -or ($_.Subject -match 'CN=(config\.psynet\.gg|api\.rlpp\.psynet\.gg|ws\.rlpp\.psynet\.gg)') }} | Remove-Item -Force -ErrorAction SilentlyContinue"#
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn is_ca_installed() -> bool {
    true
}

#[cfg(not(windows))]
pub fn install_user_ca_direct() {}

pub fn revert_config_hosts() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        return Ok(());
    }
    #[cfg(windows)]
    {
        if !psynet_hosts_redirected() {
            return Ok(());
        }
        if is_process_elevated() {
            let p = windows_hosts_path();
            if let Ok(content) = fs::read_to_string(&p) {
                let cleaned: Vec<&str> = content
                    .lines()
                    .filter(|line| !line.contains("config.psynet.gg") && !line.contains("ws.rlpp.psynet.gg") && !line.contains("api.rlpp.psynet.gg"))
                    .collect();
                let mut new_text = cleaned.join("\r\n");
                new_text.push_str("\r\n");
                let _ = fs::write(&p, new_text);
                let _ = crate::winprobe::flush_dns_cache();
            }
            return Ok(());
        }

        let script_text = r#"$ErrorActionPreference = "SilentlyContinue"
$hostsPath = Join-Path $env:SystemRoot "System32\drivers\etc\hosts"
if (Test-Path -LiteralPath $hostsPath) {
    $lines = Get-Content -LiteralPath $hostsPath
    $clean = $lines | Where-Object { $_ -notmatch 'config\.psynet\.gg' -and $_ -notmatch 'ws\.rlpp\.psynet\.gg' -and $_ -notmatch 'api\.rlpp\.psynet\.gg' }
    [System.IO.File]::WriteAllLines($hostsPath, $clean)
    ipconfig /flushdns | Out-Null
}
exit 0
"#;
        let _ = run_elevated_script(script_text);
        let _ = crate::winprobe::flush_dns_cache();
        Ok(())
    }
}

pub fn ensure_config_hosts() -> Result<bool, String> {
    let _guard = HOSTS_ENSURE.lock().map_err(|e| e.to_string())?;
    ensure_config_hosts_inner()
}

fn ensure_config_hosts_inner() -> Result<bool, String> {
    #[cfg(not(windows))]
    {
        return Ok(true);
    }
    #[cfg(windows)]
    {
        use base64::Engine;
        install_user_ca_direct();

        let hosts_ok = config_hosts_complete();
        let ca_ok = is_ca_installed();
        if hosts_ok && ca_ok {
            crate::applog::event("psynet: config hosts & CA already present");
            return Ok(true);
        }

        crate::applog::event(&format!(
            "psynet: hosts_ok={hosts_ok} ca_ok={ca_ok} — elevating to setup"
        ));

        let ca_b64 = base64::engine::general_purpose::STANDARD.encode(crate::proxy::ca_cert_bytes());

        let script_text = format!(
            r#"$ErrorActionPreference = "SilentlyContinue"
$targetThumb = "{BUNDLED_CA_THUMBPRINT}"

# Aggressively delete any old, mismatched, or stale VelocityRL certificates in LocalMachine and CurrentUser
Get-ChildItem Cert:\LocalMachine\Root -ErrorAction SilentlyContinue | Where-Object {{ ($_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*") -and $_.Thumbprint -notlike "*$targetThumb*" }} | Remove-Item -Force -ErrorAction SilentlyContinue
Get-ChildItem Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object {{ ($_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*") -and $_.Thumbprint -notlike "*$targetThumb*" }} | Remove-Item -Force -ErrorAction SilentlyContinue

# Clean any leaf certificates that may have been erroneously installed directly into Root stores
Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object {{
    ($_.Subject -match 'CN=(config\.psynet\.gg|api\.rlpp\.psynet\.gg|ws\.rlpp\.psynet\.gg)') -or
    ($_.Subject -like '*config.psynet.gg*' -or $_.Subject -like '*api.rlpp.psynet.gg*' -or $_.Subject -like '*ws.rlpp.psynet.gg*') -or
    ($_.Subject -like '*mitmproxy*' -or $_.Issuer -like '*mitmproxy*')
}} | Remove-Item -Force -ErrorAction SilentlyContinue

$hasLocal = @(Get-ChildItem Cert:\LocalMachine\Root -ErrorAction SilentlyContinue | Where-Object {{ $_.Thumbprint -like "*$targetThumb*" }})
$hasUser = @(Get-ChildItem Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object {{ $_.Thumbprint -like "*$targetThumb*" }})

$caB64 = "{ca_b64}"
$caBytes = [System.Convert]::FromBase64String($caB64)
$tmpCa = Join-Path $env:TEMP "velocityrl_ca_{pid}.crt"
[System.IO.File]::WriteAllBytes($tmpCa, $caBytes)
try {{
    if ($hasLocal.Count -eq 0) {{
        try {{
            $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($tmpCa)
            $store = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "LocalMachine")
            $store.Open("ReadWrite")
            $store.Add($cert)
            $store.Close()
        }} catch {{}}
        Import-Certificate -FilePath $tmpCa -CertStoreLocation Cert:\LocalMachine\Root -ErrorAction SilentlyContinue | Out-Null
        certutil -f -addstore Root $tmpCa | Out-Null
    }}
    if ($hasUser.Count -eq 0) {{
        try {{
            $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($tmpCa)
            $store = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "CurrentUser")
            $store.Open("ReadWrite")
            $store.Add($cert)
            $store.Close()
        }} catch {{}}
        Import-Certificate -FilePath $tmpCa -CertStoreLocation Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Out-Null
        certutil -user -f -addstore Root $tmpCa | Out-Null
    }}
}} finally {{
    Remove-Item -LiteralPath $tmpCa -Force -ErrorAction SilentlyContinue
}}
$hostsPath = Join-Path $env:SystemRoot "System32\drivers\etc\hosts"
if (-not (Test-Path -LiteralPath $hostsPath)) {{ throw "hosts file not found" }}

# Clean any obsolete or stale redirects for api.rlpp.psynet.gg or ws.rlpp.psynet.gg
$existingLines = @(Get-Content -LiteralPath $hostsPath -ErrorAction SilentlyContinue)
$filteredLines = @($existingLines | Where-Object {{ $_ -notmatch 'api\.rlpp\.psynet\.gg' -and $_ -notmatch 'ws\.rlpp\.psynet\.gg' }})
if ($filteredLines.Count -ne $existingLines.Count) {{
    [System.IO.File]::WriteAllLines($hostsPath, $filteredLines)
}}

function Add-HostsLine([string]$Path, [string]$Line) {{
    for ($attempt = 1; $attempt -le 5; $attempt++) {{
        try {{
            $fs = [System.IO.FileStream]::new(
                $Path,
                [System.IO.FileMode]::Append,
                [System.IO.FileAccess]::Write,
                ([System.IO.FileShare]"ReadWrite, Delete")
            )
            try {{
                $bytes = [System.Text.Encoding]::ASCII.GetBytes("`r`n$Line")
                $fs.Write($bytes, 0, $bytes.Length)
                return
            }} finally {{ $fs.Dispose() }}
        }} catch [System.IO.IOException] {{
            if ($attempt -eq 5) {{ throw }}
            Start-Sleep -Milliseconds 500
        }}
    }}
}}
$raw = [System.IO.File]::ReadAllText($hostsPath)
foreach ($pair in @(
    @{{ Ip = "127.0.0.1"; Host = "config.psynet.gg" }},
    @{{ Ip = "::1"; Host = "config.psynet.gg" }}
)) {{
    $pat = [regex]::Escape($pair.Ip) + "\s+" + [regex]::Escape($pair.Host)
    if ($raw -notmatch $pat) {{
        Add-HostsLine -Path $hostsPath -Line "$($pair.Ip) $($pair.Host)"
        $raw += "`r`n$($pair.Ip) $($pair.Host)"
    }}
}}
ipconfig /flushdns | Out-Null
exit 0
"#,
            pid = std::process::id()
        );

        let result = run_elevated_script(&script_text);
        result?;

        let _ = crate::winprobe::flush_dns_cache();

        let hosts_ok = config_hosts_complete();
        let ca_ok = is_ca_installed();
        if hosts_ok && ca_ok {
            crate::applog::event("psynet: config.psynet.gg hosts & CA setup complete");
            Ok(false)
        } else if !hosts_ok {
            Err(
                "UAC finished but config.psynet.gg was not added to hosts. Approve the prompt and retry."
                    .into(),
            )
        } else {
            Err(
                "UAC finished but VelocityRL CA was not installed into Trusted Root Certification Authorities. Approve the prompt and retry."
                    .into(),
            )
        }
    }
}

#[tauri::command]
pub async fn ensure_psynet_hosts() -> Result<bool, String> {
    crate::applog::event("psynet: boot hosts ensure requested");
    ensure_config_hosts()
}

#[tauri::command]
pub async fn get_psynet_spoof() -> Result<serde_json::Value, String> {
    let dir = config_dir();
    let path = config_path(&dir);
    if !path.is_file() {
        return Ok(serde_json::json!({}));
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let raw = raw.trim_start_matches('\u{feff}');
    serde_json::from_str(raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

#[tauri::command]
pub async fn save_psynet_spoof(
    state: State<'_, PsyNetState>,
    payload: SpoofPayload,
) -> Result<String, String> {
    if !crate::features::is_build_supported() {
        return Err("VelocityRL build is outdated. Please update to the latest version.".into());
    }
    let _guard = PROXY_LIFECYCLE.lock().await;
    let dir = config_dir();
    let path = write_spoof(&dir, &payload)?;
    crate::proxy::set_spoof_config(payload).await;
    let _ = state;

    crate::applog::event(&format!(
        "psynet: wrote spoof config {} (hot-reload)",
        path.display()
    ));
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn get_psynet_status(state: State<'_, PsyNetState>) -> Result<PsyNetStatus, String> {
    let alive = crate::proxy::is_proxy_running();
    {
        let mut g = state.running.lock().map_err(|e| e.to_string())?;
        *g = alive;
    }
    let dir = config_dir();
    Ok(status_for(Some(dir), alive))
}

#[tauri::command]
pub async fn start_psynet_proxy(
    state: State<'_, PsyNetState>,
    payload: Option<SpoofPayload>,
) -> Result<PsyNetStatus, String> {
    if !crate::features::is_build_supported() {
        return Err("VelocityRL build is outdated. Please update to the latest version.".into());
    }
    crate::applog::event("psynet: start requested (native Rust proxy)");
    let _guard = PROXY_LIFECYCLE.lock().await;
    let dir = config_dir();

    let cfg = if let Some(p) = payload {
        write_spoof(&dir, &p).map_err(|e| {
            crate::applog::event(&format!("psynet: write_spoof failed: {e}"));
            e
        })?;
        p
    } else {
        read_or_default_spoof(&dir)?
    };

    crate::proxy::set_spoof_config(cfg).await;

    if crate::proxy::is_proxy_running() {
        crate::applog::event("psynet: native proxy already running — config hot-reloaded");
        *state.running.lock().map_err(|e| e.to_string())? = true;
        return Ok(status_for(Some(dir), true));
    }

    ensure_config_hosts()?;

    #[cfg(windows)]
    if let Some(pid) = crate::winprobe::loopback_443_owner() {
        if pid != std::process::id() {
            let name = crate::winprobe::process_name(pid).unwrap_or_else(|| "unknown".to_string());
            let msg = format!(
                "Another process owns loopback :443 ({name}, PID {pid}). Quit that process, then start the proxy again."
            );
            crate::applog::event(&format!("psynet: {msg}"));
            return Err(msg);
        }
    }

    crate::proxy::start_native_proxy().await.map_err(|e| {
        crate::applog::event(&format!("psynet: start_native_proxy failed: {e}"));
        e
    })?;

    // Start the plain-HTTP WS broker that RL connects to via PsyNetUrl rewrite.
    if let Err(e) = crate::proxy::start_ws_broker().await {
        crate::applog::event(&format!("psynet: start_ws_broker failed (non-fatal): {e}"));
    }

    *state.running.lock().map_err(|e| e.to_string())? = true;
    let flushed = crate::winprobe::flush_dns_cache();
    let _ = clear_rocket_league_cache();
    crate::applog::event(&format!(
        "psynet: native proxy running; hosts_redirected={} port443_ok=true dns_flushed={flushed}",
        psynet_hosts_redirected()
    ));

    Ok(status_for(Some(dir), true))
}

#[tauri::command]
pub fn clear_rocket_league_cache() -> Result<usize, String> {
    #[cfg(not(windows))]
    {
        Ok(0)
    }
    #[cfg(windows)]
    {
        if rocket_league_process().is_some() {
            crate::applog::event("cache: Rocket League is running — skipping cache wipe to avoid file locks");
            return Ok(0);
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let cache_dir = std::path::PathBuf::from(profile)
                .join("Documents")
                .join("My Games")
                .join("Rocket League")
                .join("TAGame")
                .join("Cache");
            if cache_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&cache_dir) {
                    let mut deleted = 0;
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let res = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };
                        if res.is_ok() {
                            deleted += 1;
                        }
                    }
                    crate::applog::event(&format!(
                        "cache: cleared {deleted} item(s) from Rocket League Cache ({})",
                        cache_dir.display()
                    ));
                    return Ok(deleted);
                }
            }
        }
        Ok(0)
    }
}

#[tauri::command]
pub fn is_rocket_league_running() -> bool {
    rocket_league_running()
}

#[tauri::command]
pub async fn stop_psynet_proxy(
    state: State<'_, PsyNetState>,
    revert_hosts: Option<bool>,
) -> Result<PsyNetStatus, String> {
    crate::applog::event("psynet: stop requested (native Rust proxy)");
    let _guard = PROXY_LIFECYCLE.lock().await;
    let do_revert = revert_hosts.unwrap_or(false);

    crate::proxy::stop_native_proxy(do_revert);

    if do_revert {
        let _ = revert_config_hosts();
    }

    *state.running.lock().map_err(|e| e.to_string())? = false;

    let dir = config_dir();
    crate::applog::event(&format!(
        "psynet: stop complete; running=false revert_hosts={do_revert}"
    ));

    Ok(status_for(Some(dir), false))
}

#[tauri::command]
pub async fn restart_psynet_proxy(
    state: State<'_, PsyNetState>,
) -> Result<PsyNetStatus, String> {
    crate::applog::event("psynet: restart requested (native Rust proxy)");
    let _guard = PROXY_LIFECYCLE.lock().await;

    crate::proxy::stop_native_proxy(false);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let dir = config_dir();
    let cfg = read_or_default_spoof(&dir)?;
    crate::proxy::set_spoof_config(cfg).await;

    ensure_config_hosts()?;

    crate::proxy::start_native_proxy().await.map_err(|e| {
        crate::applog::event(&format!("psynet: restart failed: {e}"));
        e
    })?;

    *state.running.lock().map_err(|e| e.to_string())? = true;
    let flushed = crate::winprobe::flush_dns_cache();
    crate::applog::event(&format!("psynet: proxy restart complete; running=true dns_flushed={flushed}"));

    Ok(status_for(Some(dir), true))
}

pub fn resolve_proxy_dir_for_diag() -> Result<PathBuf, String> {
    find_proxy_dir()
}

pub fn merge_palette_spoof(enabled: bool) -> Result<(), String> {
    let dir = match find_proxy_dir() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };
    let path = config_path(&dir);
    let mut v: serde_json::Value = if path.is_file() {
        let raw = fs::read_to_string(&path).unwrap_or_default();
        let raw = raw.trim_start_matches('\u{feff}');
        serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let existing_enabled = v
        .get("palette_spoof")
        .and_then(|p| p.get("enabled"))
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    if path.is_file() && existing_enabled == enabled {
        return Ok(());
    }
    v["palette_spoof"] = serde_json::json!({ "enabled": enabled });
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    crate::applog::event(&format!(
        "psynet: palette_spoof.enabled -> {enabled} (hot-reload)"
    ));
    Ok(())
}

#[tauri::command]
pub async fn get_psynet_config_json() -> Result<String, String> {
    let dir = find_proxy_dir()?;
    let path = config_path(&dir);
    if !path.is_file() {
        return Ok(String::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let raw = raw.trim_start_matches('\u{feff}');

    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    Ok(serde_json::to_string_pretty(&v).unwrap_or_default())
}

#[tauri::command]
pub async fn save_psynet_config_json(raw: String) -> Result<String, String> {
    let dir = find_proxy_dir()?;
    let path = config_path(&dir);
    let v: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    if !v.is_object() {
        return Err("Config must be a JSON object".into());
    }
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    crate::applog::event(&format!(
        "psynet: wrote raw config {} (hot-reload)",
        path.display()
    ));
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn get_proxy_dir_override() -> Result<Option<String>, String> {
    Ok(proxy_dir_override().map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn save_proxy_dir(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let trimmed = path.trim().to_string();
    let override_val = if trimmed.is_empty() {
        None
    } else {
        let canon = normalize_win_path(
            fs::canonicalize(&trimmed).unwrap_or_else(|_| PathBuf::from(&trimmed)),
        );
        if !canon.is_dir() {
            let _ = fs::create_dir_all(&canon);
        }
        Some(canon.to_string_lossy().into_owned())
    };

    let cfg_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let cfg_path = cfg_dir.join("config.json");
    let mut cfg: serde_json::Value = fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !cfg.is_object() {
        cfg = serde_json::json!({});
    }
    match &override_val {
        Some(p) => cfg["proxy_dir"] = serde_json::json!(p),
        None => {
            if let Some(obj) = cfg.as_object_mut() {
                obj.remove("proxy_dir");
            }
        }
    }
    fs::create_dir_all(&cfg_dir).ok();
    fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap_or_default())
        .map_err(|e| format!("write {}: {e}", cfg_path.display()))?;

    *PROXY_DIR_OVERRIDE.lock().map_err(|e| e.to_string())? =
        override_val.clone().map(PathBuf::from);

    crate::applog::event(&format!(
        "psynet: proxy dir override saved: {}",
        override_val.as_deref().unwrap_or("(cleared — auto-detect)")
    ));
    Ok(override_val.unwrap_or_default())
}

#[tauri::command]
pub fn delete_ca_certificates() -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let script_text = r#"$ErrorActionPreference = "SilentlyContinue"
$deletedCount = 0
try {
    $mStore = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "LocalMachine")
    $mStore.Open("ReadWrite")
    foreach ($c in @($mStore.Certificates)) {
        if ($c.Subject -like "*VelocityRL*" -or $c.Issuer -like "*VelocityRL*") {
            $mStore.Remove($c)
            $deletedCount++
        }
    }
    $mStore.Close()
} catch {}
try {
    $uStore = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "CurrentUser")
    $uStore.Open("ReadWrite")
    foreach ($c in @($uStore.Certificates)) {
        if ($c.Subject -like "*VelocityRL*" -or $c.Issuer -like "*VelocityRL*") {
            $uStore.Remove($c)
            $deletedCount++
        }
    }
    $uStore.Close()
} catch {}
$userCerts = @(Get-ChildItem Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object { $_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*" })
foreach ($c in $userCerts) {
    Remove-Item -LiteralPath $c.PSPath -Force -ErrorAction SilentlyContinue
    $deletedCount++
}
$machineCerts = @(Get-ChildItem Cert:\LocalMachine\Root -ErrorAction SilentlyContinue | Where-Object { $_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*" })
foreach ($c in $machineCerts) {
    Remove-Item -LiteralPath $c.PSPath -Force -ErrorAction SilentlyContinue
    $deletedCount++
}
certutil -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A | Out-Null
certutil -user -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A | Out-Null
Write-Output "Deleted $deletedCount certificate(s)"
exit 0
"#;
        let status = run_elevated_script(script_text);

        // Also run direct user cleanup without elevation
        let cmd = r#"Get-ChildItem Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object { $_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*" } | Remove-Item -Force -ErrorAction SilentlyContinue"#;
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        crate::applog::event("psynet: deleted VelocityRL CA certificates from Windows store");
        status.map(|_| "VelocityRL certificates deleted successfully.".into())
    }
    #[cfg(not(windows))]
    {
        Ok("Not applicable on this platform.".into())
    }
}

