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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NameSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub real_name: Option<String>,
    #[serde(default)]
    pub player_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreditSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_credit_amount", alias = "item_shop_amount")]
    pub amount: i64,
    #[serde(default = "default_tournament_amount", alias = "tournament_credits")]
    pub tournament_amount: i64,
}

fn default_credit_amount() -> i64 {
    100000
}

fn default_tournament_amount() -> i64 {
    100000
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MenuBgSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_menu_bg")]
    pub background: String,
}

fn default_menu_bg() -> String {
    "MMBG_Default".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LeaderboardSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub sync_from_fake_ranks: bool,
    #[serde(default)]
    pub custom_mmr: Option<i32>,
    #[serde(default)]
    pub custom_rank: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoostMeterSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_boost_style")]
    pub style: String,
}

fn default_boost_style() -> String {
    "default".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SfxSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_flip_reset_sfx")]
    pub flip_reset_sfx: String,
    #[serde(default = "default_crossbar_sfx")]
    pub crossbar_sfx: String,
    #[serde(default = "default_volume")]
    pub volume: f64,
}

fn default_flip_reset_sfx() -> String {
    "mario_coin".into()
}

fn default_crossbar_sfx() -> String {
    "loud_ping".into()
}

fn default_volume() -> f64 {
    0.85
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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
    pub is_steam: bool,

    #[serde(default)]
    pub name_spoof: Option<NameSpoofPayload>,

    #[serde(default)]
    pub credit_spoof: Option<CreditSpoofPayload>,

    #[serde(default)]
    pub menu_bg_spoof: Option<MenuBgSpoofPayload>,

    #[serde(default)]
    pub leaderboard_spoof: Option<LeaderboardSpoofPayload>,

    #[serde(default)]
    pub boost_meter_spoof: Option<BoostMeterSpoofPayload>,

    #[serde(default)]
    pub sfx_spoof: Option<SfxSpoofPayload>,

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigPsynetHealth {
    pub ok: bool,
    pub dns_resolved_to_loopback: bool,
    pub tls_cert_trusted: bool,
    pub proxy_responding: bool,
    pub upstream_psynet_reachable: bool,
    pub details: String,
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
    pub config_health: Option<ConfigPsynetHealth>,
}

pub const CLOSE_WARNING: &str = "Keep VelocityRL open while playing — closing the app stops the proxy and Rocket League loses config.psynet.gg.";

pub const ANTI_VIRUS_EXCLUSION_MSG: &str = "Anti-virus is blocking VelocityRL. Add exclusions in Windows Defender to these paths:\nC:\\Windows\\System32\\drivers\\etc\\hosts\n%APPDATA%\\VelocityRL\n%LOCALAPPDATA%\\com.velocityrl.app\n%LOCALAPPDATA%\\Programs\\velocityrl\nTutorial: https://www.youtube.com/watch?v=nRaGvYL2lwk";

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
        is_steam: false,
        name_spoof: None,
        credit_spoof: None,
        menu_bg_spoof: None,
        leaderboard_spoof: None,
        boost_meter_spoof: None,
        sfx_spoof: None,
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
        config_health: None,
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
        if let Some(ns) = &payload.name_spoof {
            let mut ns_map = serde_json::Map::new();
            let effective_enabled = ns.enabled && !payload.is_steam;
            ns_map.insert("enabled".into(), serde_json::json!(effective_enabled));
            ns_map.insert("display_name".into(), serde_json::json!(ns.display_name.trim()));

            let prev_real = obj.get("name_spoof")
                .and_then(|v| v.get("real_name"))
                .and_then(|v| v.as_str())
                .filter(|rn| !rn.trim().is_empty())
                .map(|rn| rn.trim().to_string());

            let prev_pid = obj.get("name_spoof")
                .and_then(|v| v.get("player_id"))
                .and_then(|v| v.as_str())
                .filter(|pid| !pid.trim().is_empty() && !pid.contains("|temp|"))
                .map(|pid| pid.trim().to_string());

            let real_name_to_use = ns.real_name.as_deref()
                .filter(|rn| !rn.trim().is_empty())
                .map(|rn| rn.trim().to_string())
                .or_else(|| crate::proxy::get_learned_real_name())
                .or(prev_real);
            if let Some(rn) = real_name_to_use {
                ns_map.insert("real_name".into(), serde_json::json!(rn));
            }

            let pid_to_use = ns.player_id.as_deref()
                .filter(|pid| !pid.trim().is_empty() && !pid.contains("|temp|"))
                .map(|pid| pid.trim().to_string())
                .or_else(|| crate::proxy::get_learned_player_id())
                .or(prev_pid);
            if let Some(pid) = pid_to_use {
                ns_map.insert("player_id".into(), serde_json::json!(pid));
            }
            obj.insert("name_spoof".into(), serde_json::Value::Object(ns_map));
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

        if let Some(cs) = &payload.credit_spoof {
            obj.insert(
                "credit_spoof".into(),
                serde_json::json!({
                    "enabled": cs.enabled,
                    "amount": cs.amount,
                }),
            );
        }

        if let Some(bg) = &payload.menu_bg_spoof {
            obj.insert(
                "menu_bg_spoof".into(),
                serde_json::json!({
                    "enabled": bg.enabled,
                    "background": bg.background.trim(),
                }),
            );
        }

        if let Some(lb) = &payload.leaderboard_spoof {
            obj.insert(
                "leaderboard_spoof".into(),
                serde_json::json!({
                    "enabled": lb.enabled,
                    "sync_from_fake_ranks": lb.sync_from_fake_ranks,
                    "custom_mmr": lb.custom_mmr,
                    "custom_rank": lb.custom_rank,
                }),
            );
        }

        if let Some(bm) = &payload.boost_meter_spoof {
            obj.insert(
                "boost_meter_spoof".into(),
                serde_json::json!({
                    "enabled": bm.enabled,
                    "style": bm.style.trim(),
                }),
            );
        }

        if let Some(sfx) = &payload.sfx_spoof {
            obj.insert(
                "sfx_spoof".into(),
                serde_json::json!({
                    "enabled": sfx.enabled,
                    "flip_reset_sfx": sfx.flip_reset_sfx.trim(),
                    "crossbar_sfx": sfx.crossbar_sfx.trim(),
                    "volume": sfx.volume,
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

#[cfg(windows)]
extern "system" {
    fn LoadLibraryA(lpLibFileName: *const u8) -> *mut std::ffi::c_void;
    fn GetProcAddress(hModule: *mut std::ffi::c_void, lpProcName: *const u8) -> *mut std::ffi::c_void;
    fn FreeLibrary(hModule: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
pub fn notify_system_proxy_changed() {
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
    const INTERNET_OPTION_REFRESH: u32 = 37;

    unsafe {
        let lib = LoadLibraryA(b"wininet.dll\0".as_ptr());
        if !lib.is_null() {
            let proc = GetProcAddress(lib, b"InternetSetOptionW\0".as_ptr());
            if !proc.is_null() {
                let internet_set_option: unsafe extern "system" fn(
                    *mut std::ffi::c_void,
                    u32,
                    *mut std::ffi::c_void,
                    u32,
                ) -> i32 = std::mem::transmute(proc);

                internet_set_option(std::ptr::null_mut(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null_mut(), 0);
                internet_set_option(std::ptr::null_mut(), INTERNET_OPTION_REFRESH, std::ptr::null_mut(), 0);
            }
            FreeLibrary(lib);
        }
    }
}

#[cfg(not(windows))]
pub fn notify_system_proxy_changed() {}

#[cfg(windows)]
pub fn set_system_proxy_enabled(enabled: bool) {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings") {
        if enabled {
            let _ = key.set_value("ProxyEnable", &1u32);
            let proxy_addr = format!("127.0.0.1:{}", crate::proxy::SYSTEM_PROXY_PORT);
            let _ = key.set_value("ProxyServer", &proxy_addr);
            let _ = key.set_value(
                "ProxyOverride",
                &"<local>;*epicgames.com;*.epicgames.com;*ol.epicgames.com;*.ol.epicgames.com;*unrealengine.com;*.unrealengine.com;*hcaptcha.com;*arkoselabs.com;*epicgames.org",
            );
            let _ = key.delete_value("AutoConfigURL");
            crate::applog::event(&format!(
                "psynet: system proxy enabled -> {proxy_addr} (override=<local>;*epicgames.com;*.epicgames.com;*ol.epicgames.com;*.ol.epicgames.com;*unrealengine.com;*.unrealengine.com;*hcaptcha.com;*arkoselabs.com;*epicgames.org)"
            ));
        } else {
            let _ = key.set_value("ProxyEnable", &0u32);
            let _ = key.delete_value("ProxyServer");
            let _ = key.delete_value("ProxyOverride");
            let _ = key.delete_value("AutoConfigURL");
            crate::applog::event("psynet: proxy disabled (ProxyEnable, ProxyServer, ProxyOverride, AutoConfigURL cleared)");
        }
    }
    notify_system_proxy_changed();
}

#[cfg(windows)]
pub fn clean_system_proxy() {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings") {
        let _ = key.set_value("ProxyEnable", &0u32);
        let _ = key.delete_value("ProxyServer");
        let _ = key.delete_value("ProxyOverride");
        let _ = key.delete_value("AutoConfigURL");
        crate::applog::event("psynet: system proxy cleared (ProxyEnable=0, ProxyServer, ProxyOverride, AutoConfigURL cleared)");
    }
    notify_system_proxy_changed();
}

#[cfg(not(windows))]
pub fn set_system_proxy_enabled(_enabled: bool) {}

#[cfg(not(windows))]
pub fn clean_system_proxy() {}

pub fn kill_proxy_on_exit() {
    crate::applog::event("psynet: exit cleanup — stopping proxy and reverting hosts");
    set_system_proxy_enabled(false);
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

    let output = if is_process_elevated() {
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
            .output()
            .map_err(|e| format!("setup failed: {e}"))?
    } else {
        let runner_cmd = format!(
            "try {{ $p = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile','-NonInteractive','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-EncodedCommand','{encoded}'); if ($null -eq $p) {{ exit 1223 }}; exit $p.ExitCode }} catch {{ exit 1223 }}"
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
            .output()
            .map_err(|e| format!("setup failed: {e}"))?
    };

    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    crate::applog::event(&format!(
        "psynet: setup script exited with code {code:?}; stderr='{stderr}'; stdout='{stdout}'"
    ));
    if code == Some(1223) {
        return Err("Proxy setup was cancelled.".into());
    }

    Err(ANTI_VIRUS_EXCLUSION_MSG.into())
}

#[cfg(not(windows))]
fn run_elevated_script(_script_text: &str) -> Result<(), String> {
    Err("PsyNet proxy is Windows-only for now.".into())
}

#[cfg(windows)]
fn bundled_ca_thumbprint() -> String {
    use sha1::{Digest, Sha1};
    let mut reader = std::io::Cursor::new(crate::proxy::ca_cert_bytes());
    let certs = match rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let Some(der) = certs.first() else {
        return String::new();
    };
    Sha1::digest(der.as_ref())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

#[cfg(windows)]
#[allow(dead_code)]
pub fn is_user_ca_installed() -> bool {
    use std::os::windows::process::CommandExt;
    let thumb = bundled_ca_thumbprint();
    if thumb.is_empty() {
        return false;
    }
    let thumb_lower = thumb.to_ascii_lowercase();
    Command::new("certutil")
        .args(["-user", "-store", "Root", &thumb])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| {
            if !o.status.success() {
                return false;
            }
            let text = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            text.contains(&thumb_lower) || text.contains(&thumb)
        })
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn is_system_ca_installed() -> bool {
    use std::os::windows::process::CommandExt;
    let thumb = bundled_ca_thumbprint();
    if thumb.is_empty() {
        return false;
    }
    let thumb_lower = thumb.to_ascii_lowercase();
    Command::new("certutil")
        .args(["-store", "Root", &thumb])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| {
            if !o.status.success() {
                return false;
            }
            let text = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            text.contains(&thumb_lower) || text.contains(&thumb)
        })
        .unwrap_or(false)
}

#[cfg(windows)]
const STALE_THUMBPRINTS: &[&str] = &[
    "38A28A81A89A71CA078369073BD2F0597422983C",
];

#[cfg(windows)]
pub fn cleanup_known_stale_roots() {
    use std::os::windows::process::CommandExt;
    let current_thumb = bundled_ca_thumbprint();
    for thumb in STALE_THUMBPRINTS {
        if !current_thumb.is_empty() && thumb.eq_ignore_ascii_case(&current_thumb) {
            continue;
        }
        for store in &["Root", "CA"] {
            let _ = Command::new("certutil")
                .args(["-f", "-delstore", store, thumb])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
            let _ = Command::new("certutil")
                .args(["-user", "-f", "-delstore", store, thumb])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }
}

#[cfg(windows)]
pub fn is_ca_installed() -> bool {
    is_system_ca_installed()
}

#[cfg(not(windows))]
pub fn is_ca_installed() -> bool {
    true
}

/// Public wrapper so lib.rs startup check can log hosts state without re-exporting internals.
pub fn config_hosts_complete_pub() -> bool {
    #[cfg(windows)]
    {
        config_hosts_complete()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Install the CA cert + CRL + config leaf to LocalMachine and CurrentUser stores via an
/// elevated PowerShell script (UAC prompt). Does NOT touch the hosts file. Called at startup
/// if the system CA is found to be missing before the proxy auto-start sequence runs.
#[cfg(windows)]
pub fn install_ca_and_crl_elevated() {
    use base64::Engine;
    let ca_b64 = base64::engine::general_purpose::STANDARD.encode(crate::proxy::ca_cert_bytes());
    let leaf_cfg_b64 = base64::engine::general_purpose::STANDARD
        .encode(crate::proxy::leaf_config_cert_bytes());
    let leaf_epic_b64 = base64::engine::general_purpose::STANDARD
        .encode(crate::proxy::leaf_epic_cert_bytes());
    let crl_b64 =
        base64::engine::general_purpose::STANDARD.encode(crate::proxy::ca_crl_bytes());
    let pid = std::process::id();
    let script = format!(
        r##"$ErrorActionPreference = "SilentlyContinue"
$tmpCa   = Join-Path $env:TEMP "velocityrl_ca_{pid}.crt"
$tmpCfg  = Join-Path $env:TEMP "velocityrl_cfg_{pid}.crt"
$tmpEpic = Join-Path $env:TEMP "velocityrl_epic_{pid}.crt"
$tmpCrl  = Join-Path $env:TEMP "velocityrl_{pid}.crl"
[System.IO.File]::WriteAllBytes($tmpCa,   [System.Convert]::FromBase64String("{ca_b64}"))
[System.IO.File]::WriteAllBytes($tmpCfg,  [System.Convert]::FromBase64String("{leaf_cfg_b64}"))
[System.IO.File]::WriteAllBytes($tmpEpic, [System.Convert]::FromBase64String("{leaf_epic_b64}"))
[System.IO.File]::WriteAllBytes($tmpCrl,  [System.Convert]::FromBase64String("{crl_b64}"))
try {{
    foreach ($f in @($tmpCa, $tmpCfg, $tmpEpic, $tmpCrl)) {{
        certutil -f -addstore Root $f | Out-Null
        certutil -user -f -addstore Root $f | Out-Null
        certutil -f -addstore CA $f | Out-Null
        certutil -user -f -addstore CA $f | Out-Null
    }}
}} finally {{
    Remove-Item -LiteralPath $tmpCa, $tmpCfg, $tmpEpic, $tmpCrl -Force -ErrorAction SilentlyContinue
}}
"##
    );
    match run_elevated_script(&script) {
        Ok(()) => crate::applog::event("startup: CA/CRL elevated install succeeded"),
        Err(e) => crate::applog::event(&format!("startup: CA/CRL elevated install failed: {e}")),
    }
}

#[cfg(not(windows))]
pub fn install_ca_and_crl_elevated() {}

#[cfg(windows)]
pub fn ensure_wininet_revocation_disabled() {
    use std::os::windows::process::CommandExt;
    for hive in &[
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        r"HKCU\SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\Internet Settings",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\Internet Settings",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Internet Settings",
    ] {
        let _ = std::process::Command::new("reg")
            .args([
                "add",
                hive,
                "/v",
                "CertificateRevocation",
                "/t",
                "REG_DWORD",
                "/d",
                "0",
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    let _ = std::process::Command::new("reg")
        .args([
            "add",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "Security_HKLM_only",
            "/t",
            "REG_DWORD",
            "/d",
            "1",
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    if let Ok(output) = std::process::Command::new("reg")
        .args(["query", "HKU"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if (line.contains("S-1-5-21-") && !line.ends_with("_Classes")) || line.ends_with(".DEFAULT") {
                let target = format!(r"{line}\Software\Microsoft\Windows\CurrentVersion\Internet Settings");
                let _ = std::process::Command::new("reg")
                    .args([
                        "add",
                        &target,
                        "/v",
                        "CertificateRevocation",
                        "/t",
                        "REG_DWORD",
                        "/d",
                        "0",
                        "/f",
                    ])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
        }
    }

    crate::winprobe::refresh_wininet_settings();
}

#[cfg(windows)]
pub fn ensure_hklm_revocation_disabled() {
    ensure_wininet_revocation_disabled();
}

#[cfg(not(windows))]
pub fn ensure_wininet_revocation_disabled() {}

#[cfg(not(windows))]
pub fn ensure_hklm_revocation_disabled() {}


#[cfg(windows)]
fn append_ca_to_pem_bundles() {
    let ca_bytes = crate::proxy::ca_cert_bytes();
    let Ok(ca_str) = std::str::from_utf8(ca_bytes) else {
        return;
    };
    let appendix = format!("\r\n# VelocityRL CA\r\n{}\r\n", ca_str.trim());

    let mut paths = vec![PathBuf::from(r"C:\Windows\cert.pem")];
    if let Ok(prog_files) = std::env::var("ProgramFiles") {
        paths.push(PathBuf::from(prog_files).join("Common Files").join("SSL").join("cert.pem"));
    }

    for pem in paths {
        if pem.is_file() {
            if let Ok(content) = fs::read_to_string(&pem) {
                if content.len() >= 50000 && !content.contains("# VelocityRL CA") {
                    let mut updated = content.trim_end().to_string();
                    updated.push_str(&appendix);
                    let _ = fs::write(&pem, updated);
                    crate::applog::event(&format!("psynet: appended VelocityRL CA to {}", pem.display()));
                }
            }
        }
    }
}

#[cfg(windows)]
fn clean_ca_from_pem_bundles() {
    let mut paths = vec![PathBuf::from(r"C:\Windows\cert.pem")];
    if let Ok(prog_files) = std::env::var("ProgramFiles") {
        paths.push(PathBuf::from(prog_files).join("Common Files").join("SSL").join("cert.pem"));
    }
    for pem in paths {
        if pem.is_file() {
            if let Ok(raw) = fs::read_to_string(&pem) {
                if let Some(idx) = raw.find("# VelocityRL CA") {
                    let cleaned = raw[..idx].trim_end().to_string();
                    let _ = fs::write(&pem, format!("{cleaned}\r\n"));
                    crate::applog::event(&format!("psynet: reverted VelocityRL CA from {}", pem.display()));
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn install_ca_direct(target_thumb: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let pid = std::process::id();
    let certs_to_install = [
        ("ca", crate::proxy::ca_cert_bytes(), "crt"),
        ("crl", crate::proxy::ca_crl_bytes(), "crl"),
        ("leaf_config", crate::proxy::leaf_config_cert_bytes(), "crt"),
        ("leaf_epic", crate::proxy::leaf_epic_cert_bytes(), "crt"),
        // ws.rlpp.psynet.gg is cert-pinned and never hosts-redirected - do not install its leaf.
    ];

    for (name, bytes, ext) in &certs_to_install {
        let tmp_cert = std::env::temp_dir().join(format!("velocityrl_{name}_{pid}.{ext}"));
        if let Err(e) = fs::write(&tmp_cert, bytes) {
            crate::applog::event(&format!("psynet: failed to write temp {name} {ext}: {e}"));
            continue;
        }
        let tmp_str = tmp_cert.to_string_lossy();
        for store in &["Root", "CA"] {
            let _ = Command::new("certutil")
                .args(["-f", "-addstore", store, &tmp_str])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
            let _ = Command::new("certutil")
                .args(["-user", "-f", "-addstore", store, &tmp_str])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        let _ = fs::remove_file(&tmp_cert);
    }

    append_ca_to_pem_bundles();

    if is_ca_installed() {
        Ok(())
    } else {
        Err(format!(
            "VelocityRL root CA was not installed (need thumb {target_thumb}). Rocket League cannot trust the PsyNet proxy."
        ))
    }
}

#[cfg(windows)]
pub fn write_config_hosts_direct() -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let p = windows_hosts_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(mut perms) = fs::metadata(&p).map(|m| m.permissions()) {
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(&p, perms);
        }
    }
    let _ = Command::new("attrib")
        .args(["-r", "-h", "-s", &p.to_string_lossy()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    let content = match fs::read_to_string(&p) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            crate::applog::event(&format!("psynet: failed to read hosts file {}: {e}", p.display()));
            return Err(ANTI_VIRUS_EXCLUSION_MSG.into());
        }
    };

    let lines: Vec<&str> = content
        .lines()
        .filter(|line| {
            !line.contains("api.rlpp.psynet.gg")
                && !line.contains("ws.rlpp.psynet.gg")
                && !line.contains("::1")
        })
        .collect();

    let mut new_text = lines.join("\r\n");
    if !new_text.is_empty() && !new_text.ends_with("\r\n") {
        new_text.push_str("\r\n");
    }

    let has_ipv4 = hosts_has_pair(&new_text, "127.0.0.1", "config.psynet.gg");
    if !has_ipv4 {
        new_text.push_str("127.0.0.1 config.psynet.gg\r\n");
    }

    let mut last_err = None;
    for attempt in 1..=4 {
        match fs::write(&p, new_text.as_bytes()) {
            Ok(_) => {
                crate::applog::event("psynet: hosts file written successfully via native I/O");
                let _ = crate::winprobe::flush_dns_cache();
                return Ok(());
            }
            Err(e) => {
                crate::applog::event(&format!(
                    "psynet: hosts write attempt {attempt} failed: {e}"
                ));
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
    }
    if let Some(e) = last_err {
        crate::applog::event(&format!("psynet: all hosts write attempts failed: {e}"));
    }
    Err(ANTI_VIRUS_EXCLUSION_MSG.into())
}

#[cfg(windows)]
fn direct_elevated_hosts_and_ca_setup(
    target_thumb: &str,
    hosts_ok: bool,
    ca_ok: bool,
) -> Result<bool, String> {
    crate::applog::event("psynet: direct elevated setup started (no PowerShell)");

    ensure_wininet_revocation_disabled();
    ensure_hklm_revocation_disabled();

    if !ca_ok {
        cleanup_known_stale_roots();
        install_ca_direct(target_thumb)?;
    }

    if !hosts_ok {
        write_config_hosts_direct()?;
    }

    let final_hosts_ok = config_hosts_complete();
    let final_ca_ok = is_ca_installed();

    if final_hosts_ok && final_ca_ok {
        crate::applog::event(&format!(
            "psynet: config.psynet.gg hosts & CA setup complete (thumb={target_thumb})"
        ));
        return Ok(false);
    }

    let _ = revert_config_hosts();
    if !final_ca_ok {
        return Err(format!(
            "VelocityRL root CA was not installed (need thumb {target_thumb}). Rocket League cannot trust the PsyNet proxy."
        ));
    }

    Err(ANTI_VIRUS_EXCLUSION_MSG.into())
}

#[cfg(windows)]
pub fn install_user_ca_direct() {
    use std::os::windows::process::CommandExt;

    ensure_wininet_revocation_disabled();
    ensure_hklm_revocation_disabled();
    cleanup_known_stale_roots();

    let pid = std::process::id();
    let certs_to_install = [
        ("ca", crate::proxy::ca_cert_bytes(), "crt"),
        ("crl", crate::proxy::ca_crl_bytes(), "crl"),
        ("leaf_config", crate::proxy::leaf_config_cert_bytes(), "crt"),
        ("leaf_epic", crate::proxy::leaf_epic_cert_bytes(), "crt"),
    ];

    for (name, bytes, ext) in &certs_to_install {
        let tmp_cert = std::env::temp_dir().join(format!("velocityrl_user_{name}_{pid}.{ext}"));
        if let Ok(()) = fs::write(&tmp_cert, bytes) {
            let tmp_str = tmp_cert.to_string_lossy();
            for store in &["Root", "CA"] {
                let _ = Command::new("certutil")
                    .args(["-user", "-f", "-addstore", store, &tmp_str])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
            if is_process_elevated() {
                for store in &["Root", "CA"] {
                    let _ = Command::new("certutil")
                        .args(["-f", "-addstore", store, &tmp_str])
                        .creation_flags(CREATE_NO_WINDOW)
                        .status();
                }
            }
            let _ = fs::remove_file(&tmp_cert);
        }
    }

    if is_ca_installed() {
        return;
    }

    if is_process_elevated() {
        let _ = install_ca_direct(&bundled_ca_thumbprint());
    }
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
        if is_process_elevated() {
            clean_ca_from_pem_bundles();
            if psynet_hosts_redirected() {
                let p = windows_hosts_path();
                if let Ok(mut perms) = fs::metadata(&p).map(|m| m.permissions()) {
                    if perms.readonly() {
                        perms.set_readonly(false);
                        let _ = fs::set_permissions(&p, perms);
                    }
                }
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
            }
            return Ok(());
        }

        if !psynet_hosts_redirected() {
            return Ok(());
        }

        let script_text = r##"$ErrorActionPreference = "SilentlyContinue"
$hostsPath = Join-Path $env:SystemRoot "System32\drivers\etc\hosts"
if (Test-Path -LiteralPath $hostsPath) {
    try { (Get-Item -LiteralPath $hostsPath).IsReadOnly = $false } catch {}
    $lines = Get-Content -LiteralPath $hostsPath
    $clean = $lines | Where-Object { $_ -notmatch 'config\.psynet\.gg' -and $_ -notmatch 'ws\.rlpp\.psynet\.gg' -and $_ -notmatch 'api\.rlpp\.psynet\.gg' }
    [System.IO.File]::WriteAllLines($hostsPath, $clean)
    ipconfig /flushdns | Out-Null
}
foreach ($pem in @(
    "${env:ProgramFiles}\Common Files\SSL\cert.pem",
    "C:\Windows\cert.pem"
)) {
    if (Test-Path -LiteralPath $pem) {
        $raw = Get-Content -LiteralPath $pem -Raw -ErrorAction SilentlyContinue
        if ($raw -and $raw -match "# VelocityRL CA") {
            $cleaned = [regex]::Replace($raw, '(?s)\r?\n?# VelocityRL CA[\s\S]*\z', "")
            $utf8 = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText($pem, $cleaned.TrimEnd() + "`n", $utf8)
        }
    }
}
exit 0
"##;
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
            "psynet: hosts_ok={hosts_ok} ca_ok={ca_ok} — setting up hosts and CA"
        ));

        let target_thumb = bundled_ca_thumbprint();
        if target_thumb.is_empty() {
            return Err("Bundled VelocityRL CA is invalid — rebuild with resources/certs.".into());
        }

        if is_process_elevated() {
            return direct_elevated_hosts_and_ca_setup(&target_thumb, hosts_ok, ca_ok);
        }

        let ca_b64 = base64::engine::general_purpose::STANDARD.encode(crate::proxy::ca_cert_bytes());
        let leaf_cfg_b64 = base64::engine::general_purpose::STANDARD.encode(crate::proxy::leaf_config_cert_bytes());
        let leaf_epic_b64 = base64::engine::general_purpose::STANDARD.encode(crate::proxy::leaf_epic_cert_bytes());
        let crl_b64 = base64::engine::general_purpose::STANDARD.encode(crate::proxy::ca_crl_bytes());
        let pid = std::process::id();

        let script_text = format!(
            r##"$ErrorActionPreference = "SilentlyContinue"
$targetThumb = "{target_thumb}"

# Wipe stale VelocityRL roots and leaves (do not delete matching targetThumb).
Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root, Cert:\LocalMachine\CA, Cert:\CurrentUser\CA -ErrorAction SilentlyContinue | Where-Object {{
    (($_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*") -and $_.Thumbprint -ne $targetThumb) -or
    ($_.Subject -match 'CN=(config\.psynet\.gg|api\.rlpp\.psynet\.gg|ws\.rlpp\.psynet\.gg)') -or
    ($_.Subject -like '*config.psynet.gg*' -or $_.Subject -like '*api.rlpp.psynet.gg*' -or $_.Subject -like '*ws.rlpp.psynet.gg*') -or
    ($_.Subject -like '*mitmproxy*' -or $_.Issuer -like '*mitmproxy*')
}} | Remove-Item -Force -ErrorAction SilentlyContinue

$tmpCa = Join-Path $env:TEMP "velocityrl_ca_{pid}.crt"
$tmpLeafCfg = Join-Path $env:TEMP "velocityrl_leaf_cfg_{pid}.crt"
$tmpLeafEpic = Join-Path $env:TEMP "velocityrl_leaf_epic_{pid}.crt"
$tmpCrl = Join-Path $env:TEMP "velocityrl_{pid}.crl"
[System.IO.File]::WriteAllBytes($tmpCa, [System.Convert]::FromBase64String("{ca_b64}"))
[System.IO.File]::WriteAllBytes($tmpLeafCfg, [System.Convert]::FromBase64String("{leaf_cfg_b64}"))
[System.IO.File]::WriteAllBytes($tmpLeafEpic, [System.Convert]::FromBase64String("{leaf_epic_b64}"))
[System.IO.File]::WriteAllBytes($tmpCrl, [System.Convert]::FromBase64String("{crl_b64}"))
try {{
    # Install CA, CRL, and leaf certs to both Root and CA stores (system + user).
    foreach ($f in @($tmpCa, $tmpLeafCfg, $tmpLeafEpic, $tmpCrl)) {{
        certutil -f -addstore Root $f | Out-Null
        certutil -user -f -addstore Root $f | Out-Null
        certutil -f -addstore CA $f | Out-Null
        certutil -user -f -addstore CA $f | Out-Null
    }}

    # Append VelocityRL CA to Mozilla OpenSSL bundles if present (C:\Windows\cert.pem, Program Files\Common Files\SSL\cert.pem)
    try {{
        $caText = [System.Text.Encoding]::ASCII.GetString([System.Convert]::FromBase64String("{ca_b64}"))
        foreach ($pem in @(
            "${{env:ProgramFiles}}\Common Files\SSL\cert.pem",
            "C:\Windows\cert.pem"
        )) {{
            if (Test-Path -LiteralPath $pem) {{
                $raw = Get-Content -LiteralPath $pem -Raw -ErrorAction SilentlyContinue
                if ($raw -and $raw.Length -ge 50000 -and $raw -notmatch "# VelocityRL CA") {{
                    $appendix = "`r`n# VelocityRL CA`r`n" + $caText.Trim() + "`r`n"
                    $utf8 = New-Object System.Text.UTF8Encoding $false
                    [System.IO.File]::WriteAllText($pem, $raw.TrimEnd() + $appendix, $utf8)
                }}
            }}
        }}
    }} catch {{}}

    try {{
        $hostsPath = Join-Path $env:SystemRoot "System32\drivers\etc\hosts"
        if (-not (Test-Path -LiteralPath $hostsPath)) {{ exit 5 }}
        try {{ (Get-Item -LiteralPath $hostsPath).IsReadOnly = $false }} catch {{}}
        try {{ attrib -r "$hostsPath" }} catch {{}}

        # Never hosts-redirect api/ws (cert pinning) — config only.
        $existingLines = @(Get-Content -LiteralPath $hostsPath -ErrorAction SilentlyContinue)
        $filteredLines = @($existingLines | Where-Object {{ $_ -notmatch 'api\.rlpp\.psynet\.gg' -and $_ -notmatch 'ws\.rlpp\.psynet\.gg' }})
        if ($filteredLines.Count -ne $existingLines.Count) {{
            [System.IO.File]::WriteAllLines($hostsPath, $filteredLines)
        }}

        function Add-HostsLine([string]$Path, [string]$Line) {{
            for ($attempt = 1; $attempt -le 5; $attempt++) {{
                try {{
                    [System.IO.File]::AppendAllText($Path, "`r`n$Line")
                    return
                }} catch {{
                    if ($attempt -eq 5) {{ exit 5 }}
                    Start-Sleep -Milliseconds 500
                }}
            }}
        }}
        $raw = [System.IO.File]::ReadAllText($hostsPath)
        $raw = ($raw -split "`r?`n" | Where-Object {{ $_ -notmatch '::1\s+config\.psynet\.gg' }}) -join "`r`n"
        [System.IO.File]::WriteAllText($hostsPath, $raw)
        foreach ($pair in @(
            @{{ Ip = "127.0.0.1"; Host = "config.psynet.gg" }}
        )) {{
            $pat = [regex]::Escape($pair.Ip) + "\s+" + [regex]::Escape($pair.Host)
            if ($raw -notmatch $pat) {{
                Add-HostsLine -Path $hostsPath -Line "$($pair.Ip) $($pair.Host)"
                $raw += "`r`n$($pair.Ip) $($pair.Host)"
            }}
        }}
    }} catch {{
        exit 5
    }}
    # Disable server certificate revocation check in WinINet so Rocket League's WebRequest_X does not fail
    # on local MITM certificates lacking public CRL/OCSP responders.
    Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings" -Name "CertificateRevocation" -Value 0 -ErrorAction SilentlyContinue
    Set-ItemProperty -Path "HKLM:\Software\Microsoft\Windows\CurrentVersion\Internet Settings" -Name "CertificateRevocation" -Value 0 -ErrorAction SilentlyContinue
    Set-ItemProperty -Path "HKLM:\SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\Internet Settings" -Name "CertificateRevocation" -Value 0 -ErrorAction SilentlyContinue
    Set-ItemProperty -Path "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Internet Settings" -Name "CertificateRevocation" -Value 0 -ErrorAction SilentlyContinue

    ipconfig /flushdns | Out-Null
    $stillHas = @(Get-ChildItem Cert:\LocalMachine\Root -ErrorAction SilentlyContinue | Where-Object {{ $_.Thumbprint -like "*$targetThumb*" }})
    if ($stillHas.Count -eq 0) {{
        certutil -f -addstore Root $tmpCa | Out-Null
        $stillHas = @(Get-ChildItem Cert:\LocalMachine\Root -ErrorAction SilentlyContinue | Where-Object {{ $_.Thumbprint -like "*$targetThumb*" }})
    }}
    if ($stillHas.Count -eq 0) {{ exit 5 }}
    exit 0
}} finally {{
    Remove-Item -LiteralPath $tmpCa, $tmpLeafCfg, $tmpLeafEpic, $tmpCrl -Force -ErrorAction SilentlyContinue
}}
"##
        );

        if let Err(e) = run_elevated_script(&script_text) {
            let _ = revert_config_hosts();
            return Err(e);
        }

        let _ = crate::winprobe::flush_dns_cache();

        let hosts_ok = config_hosts_complete();
        let ca_ok = is_ca_installed();
        if hosts_ok && ca_ok {
            crate::applog::event(&format!(
                "psynet: config.psynet.gg hosts & CA setup complete (thumb={target_thumb})"
            ));
            return Ok(false);
        }
        // Hosts without the matching CA → RL TLS fails → "Epic Online Services" dialog.
        let _ = revert_config_hosts();
        if !ca_ok {
            return Err(format!(
                "VelocityRL root CA was not installed (need thumb {target_thumb}). Rocket League cannot trust the PsyNet proxy."
            ));
        }
        Err(ANTI_VIRUS_EXCLUSION_MSG.into())
    }
}

#[tauri::command]
pub async fn ensure_psynet_hosts() -> Result<bool, String> {
    crate::applog::event("psynet: boot hosts ensure requested");
    if !crate::proxy::is_proxy_running() {
        crate::applog::event("psynet: starting native proxy before ensuring hosts");
        if let Err(e) = crate::proxy::start_native_proxy().await {
            crate::applog::event(&format!("psynet: failed to start proxy for hosts setup: {e}"));
            return Err(e);
        }
        let _ = crate::proxy::start_ws_broker().await;
    }
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
    if let Some(ns) = &payload.name_spoof {
        set_system_proxy_enabled(ns.enabled);
    }
    crate::proxy::set_spoof_config(payload).await;
    let _ = state;

    crate::applog::event(&format!(
        "psynet: wrote spoof config {}",
        path.display()
    ));
    Ok(path.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LearnedIdentity {
    pub player_id: Option<String>,
    pub real_name: Option<String>,
}

#[tauri::command]
pub async fn get_learned_identity() -> Result<LearnedIdentity, String> {
    Ok(LearnedIdentity {
        player_id: crate::proxy::get_learned_player_id(),
        real_name: crate::proxy::get_learned_real_name(),
    })
}

pub async fn verify_config_psynet_live() -> ConfigPsynetHealth {
    #[cfg(not(windows))]
    {
        ConfigPsynetHealth {
            ok: true,
            dns_resolved_to_loopback: true,
            tls_cert_trusted: true,
            proxy_responding: true,
            upstream_psynet_reachable: true,
            details: "Non-windows platform".to_string(),
        }
    }
    #[cfg(windows)]
    {
        use std::net::ToSocketAddrs;

        // 1. Check DNS resolution of config.psynet.gg
        let dns_resolved_to_loopback = match ("config.psynet.gg", 443).to_socket_addrs() {
            Ok(addrs) => addrs.into_iter().any(|a| a.ip().is_loopback()),
            Err(_) => false,
        };

        // 2. Check if proxy is listening on loopback 443 with TLS
        let insecure_client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .resolve("config.psynet.gg", "127.0.0.1:443".parse().unwrap())
            .timeout(std::time::Duration::from_millis(1500))
            .build()
            .ok();

        let mut proxy_responding = false;
        if let Some(client) = insecure_client {
            if let Ok(resp) = client.get("https://config.psynet.gg/health").send().await {
                if resp.status().is_success() {
                    proxy_responding = true;
                }
            }
        }

        // 3. Check Windows OS native TLS trust (without danger_accept_invalid_certs)
        let mut tls_cert_trusted = false;
        if proxy_responding {
            let native_client = reqwest::Client::builder()
                .no_proxy()
                .resolve("config.psynet.gg", "127.0.0.1:443".parse().unwrap())
                .timeout(std::time::Duration::from_millis(2000))
                .build()
                .ok();

            if let Some(client) = native_client {
                if let Ok(resp) = client.get("https://config.psynet.gg/health").send().await {
                    if resp.status().is_success() {
                        tls_cert_trusted = true;
                    }
                }
            }
        }

        // 4. Check upstream PsyNet connectivity
        let upstream_client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .resolve("config.psynet.gg", "34.160.180.65:443".parse().unwrap())
            .timeout(std::time::Duration::from_millis(2500))
            .build()
            .ok();

        let mut upstream_psynet_reachable = false;
        if let Some(client) = upstream_client {
            if let Ok(resp) = client.get("https://config.psynet.gg/").send().await {
                upstream_psynet_reachable = resp.status().as_u16() < 500;
            }
        }

        // 5. Build summary
        let ok = dns_resolved_to_loopback && proxy_responding && tls_cert_trusted;
        let details = if ok {
            if upstream_psynet_reachable {
                "config.psynet.gg OK".to_string()
            } else {
                "config.psynet.gg working locally, but upstream PsyNet is unreachable (check internet connection)".to_string()
            }
        } else if !proxy_responding {
            "Proxy is not responding on 127.0.0.1:443".to_string()
        } else if !dns_resolved_to_loopback {
            ANTI_VIRUS_EXCLUSION_MSG.to_string()
        } else if !tls_cert_trusted {
            "VelocityRL root CA is not trusted by Windows. Rocket League will reject connection.".to_string()
        } else {
            "config.psynet.gg check failed".to_string()
        };

        crate::applog::event(&format!(
            "psynet: config.psynet.gg check: ok={ok} dns={dns_resolved_to_loopback} tls={tls_cert_trusted} proxy={proxy_responding} upstream={upstream_psynet_reachable} details='{details}'"
        ));

        ConfigPsynetHealth {
            ok,
            dns_resolved_to_loopback,
            tls_cert_trusted,
            proxy_responding,
            upstream_psynet_reachable,
            details,
        }
    }
}

#[tauri::command]
pub async fn check_config_psynet() -> Result<ConfigPsynetHealth, String> {
    Ok(verify_config_psynet_live().await)
}

#[tauri::command]
pub async fn get_psynet_status(state: State<'_, PsyNetState>) -> Result<PsyNetStatus, String> {
    let alive = crate::proxy::is_proxy_running();
    {
        let mut g = state.running.lock().map_err(|e| e.to_string())?;
        *g = alive;
    }
    let dir = config_dir();
    let status = status_for(Some(dir), alive);
    Ok(status)
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
        crate::applog::event("psynet: native proxy already running — config reloaded");
        *state.running.lock().map_err(|e| e.to_string())? = true;
        return Ok(status_for(Some(dir), true));
    }

    // Bind :443 BEFORE rewriting hosts. If listen fails, never leave
    // config.psynet.gg → loopback (that presents as RL/EOS online failure).
    #[cfg(windows)]
    if let Some(pid) = crate::winprobe::loopback_443_owner() {
        if pid != std::process::id() {
            let name = crate::winprobe::process_name(pid).unwrap_or_else(|| "unknown".to_string());
            let is_stale_self = name.eq_ignore_ascii_case("velocity-rl.exe")
                || name.eq_ignore_ascii_case("velocityrl.exe")
                || name.eq_ignore_ascii_case("psynet_proxy.exe")
                || name.eq_ignore_ascii_case("mitmproxy.exe");

            if is_stale_self {
                crate::applog::event(&format!(
                    "psynet: terminating stale VelocityRL/proxy process ({name}, PID {pid}) on :443"
                ));
                crate::winprobe::terminate_process(pid);
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            } else {
                let msg = format!(
                    "Another process owns loopback :443 ({name}, PID {pid}). Quit that process, then start the proxy again."
                );
                crate::applog::event(&format!("psynet: {msg}"));
                let _ = revert_config_hosts();
                return Err(msg);
            }
        }
    }

    if let Err(e) = crate::proxy::start_native_proxy().await {
        crate::applog::event(&format!("psynet: start_native_proxy failed: {e}"));
        let _ = revert_config_hosts();
        return Err(e);
    }

    // Broker is required: config MITM rewrites PsyNetUrl → 127.0.0.1:<ephemeral>.
    // If broker is down, browser/config still "works" but in-game Auth/WS die
    // (looks like EOS/online failure). Never leave hosts pointing at loopback.
    match crate::proxy::start_ws_broker().await {
        Ok(port) => {
            crate::applog::event(&format!("psynet: WS broker on 127.0.0.1:{port}"));
        }
        Err(e) => {
            crate::applog::event(&format!("psynet: start_ws_broker failed: {e}"));
            crate::proxy::stop_native_proxy(true);
            let _ = revert_config_hosts();
            return Err(e);
        }
    }

    // Health probe: verify that port 443 communicates via TLS and processes requests before modifying hosts
    if let Err(e) = crate::proxy::verify_proxy_loopback_health().await {
        crate::applog::event(&format!("psynet: loopback health check failed: {e}"));
        crate::proxy::stop_native_proxy(true);
        let _ = revert_config_hosts();
        return Err(e);
    }

    if let Err(e) = ensure_config_hosts() {
        crate::applog::event(&format!(
            "psynet: hosts/CA setup failed after listen — stopping proxy: {e}"
        ));
        crate::proxy::stop_native_proxy(true);
        let _ = revert_config_hosts();
        return Err(e);
    }

    *state.running.lock().map_err(|e| e.to_string())? = true;
    let flushed = crate::winprobe::flush_dns_cache();
    let _ = clear_rocket_league_cache();
    let name_spoof_active = read_or_default_spoof(&dir)
        .ok()
        .and_then(|p| p.name_spoof)
        .map(|n| n.enabled)
        .unwrap_or(false);
    set_system_proxy_enabled(name_spoof_active);
    crate::applog::event(&format!(
        "psynet: native proxy running; hosts_redirected={} port443_ok=true dns_flushed={flushed}",
        psynet_hosts_redirected()
    ));

    let health = verify_config_psynet_live().await;
    let mut status = status_for(Some(dir), true);
    status.config_health = Some(health);
    Ok(status)
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

    set_system_proxy_enabled(false);
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

    set_system_proxy_enabled(false);
    crate::proxy::stop_native_proxy(false);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let dir = config_dir();
    let cfg = read_or_default_spoof(&dir)?;
    crate::proxy::set_spoof_config(cfg.clone()).await;

    if let Err(e) = crate::proxy::start_native_proxy().await {
        crate::applog::event(&format!("psynet: restart listen failed: {e}"));
        let _ = revert_config_hosts();
        return Err(e);
    }

    if let Err(e) = crate::proxy::start_ws_broker().await {
        crate::applog::event(&format!("psynet: restart broker failed: {e}"));
        crate::proxy::stop_native_proxy(true);
        let _ = revert_config_hosts();
        return Err(e);
    }

    if let Err(e) = ensure_config_hosts() {
        crate::applog::event(&format!("psynet: restart hosts failed: {e}"));
        crate::proxy::stop_native_proxy(true);
        let _ = revert_config_hosts();
        return Err(e);
    }

    *state.running.lock().map_err(|e| e.to_string())? = true;
    let flushed = crate::winprobe::flush_dns_cache();
    let name_spoof_active = cfg.name_spoof.as_ref().map(|n| n.enabled).unwrap_or(false);
    set_system_proxy_enabled(name_spoof_active);
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
        "psynet: palette_spoof.enabled -> {enabled}"
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
        "psynet: wrote raw config {}",
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

        let current_thumb = bundled_ca_thumbprint();
        let all_thumbs = [
            current_thumb.as_str(),
            "0DBB5FBF9A1E635A2414AE14BAEF375D25755BFB",
            "05969B177719D7613DBED10B7FBE4A0DD846EB7A",
            "38A28A81A89A71CA078369073BD2F0597422983C",
            "3AF665291A560DFE85D68950AF29FA588B567ACE",
            "E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76",
            "CFFF312D754F62344E30E11D128CDB1F35CF8FC8",
            "290193877074751336AECEE8554F0D065F8F11CE",
            "9DB9369DF51127837DC086DBA047B8DBB4A626D3",
            "A3B9C9546F22BC05C21BBF427ED966EF2FE0F211",
            "1D4DA3995F3CF0905932A3678C4029E610784EF8",
            "11B5D05A6588541C1E0A61604A9B47FFDEA48BB9",
        ];
        for thumb in &all_thumbs {
            if thumb.is_empty() {
                continue;
            }
            for store in &["Root", "CA"] {
                let _ = Command::new("certutil")
                    .args(["-f", "-delstore", store, thumb])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                let _ = Command::new("certutil")
                    .args(["-user", "-f", "-delstore", store, thumb])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
        }
        cleanup_known_stale_roots();
        clean_ca_from_pem_bundles();
        set_system_proxy_enabled(false);

        let script_text = r#"$ErrorActionPreference = "SilentlyContinue"
$deletedCount = 0
try {
    foreach ($s in @("Root", "CA")) {
        foreach ($loc in @("LocalMachine", "CurrentUser")) {
            $mStore = New-Object System.Security.Cryptography.X509Certificates.X509Store($s, $loc)
            $mStore.Open("ReadWrite")
            foreach ($c in @($mStore.Certificates)) {
                if ($c.Subject -like "*VelocityRL*" -or $c.Issuer -like "*VelocityRL*" -or $c.Subject -like "*config.psynet.gg*" -or $c.Subject -like "*ws.rlpp.psynet.gg*") {
                    $mStore.Remove($c)
                    $deletedCount++
                }
            }
            $mStore.Close()
        }
    }
} catch {}
$certs = @(Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root, Cert:\LocalMachine\CA, Cert:\CurrentUser\CA -ErrorAction SilentlyContinue | Where-Object {
    $_.Subject -like "*VelocityRL*" -or $_.Issuer -like "*VelocityRL*" -or $_.Subject -like "*config.psynet.gg*" -or $_.Subject -like "*ws.rlpp.psynet.gg*"
})
foreach ($c in $certs) {
    Remove-Item -LiteralPath $c.PSPath -Force -ErrorAction SilentlyContinue
    $deletedCount++
}
foreach ($t in @("0DBB5FBF9A1E635A2414AE14BAEF375D25755BFB", "05969B177719D7613DBED10B7FBE4A0DD846EB7A", "38A28A81A89A71CA078369073BD2F0597422983C", "3AF665291A560DFE85D68950AF29FA588B567ACE", "E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76", "CFFF312D754F62344E30E11D128CDB1F35CF8FC8", "290193877074751336AECEE8554F0D065F8F11CE", "9DB9369DF51127837DC086DBA047B8DBB4A626D3", "A3B9C9546F22BC05C21BBF427ED966EF2FE0F211", "1D4DA3995F3CF0905932A3678C4029E610784EF8", "11B5D05A6588541C1E0A61604A9B47FFDEA48BB9")) {
    certutil -f -delstore Root $t | Out-Null
    certutil -user -f -delstore Root $t | Out-Null
    certutil -f -delstore CA $t | Out-Null
    certutil -user -f -delstore CA $t | Out-Null
}
Write-Output "Deleted $deletedCount certificate(s)"
exit 0
"#;
        let status = run_elevated_script(script_text);


        crate::applog::event("psynet: deleted VelocityRL CA certificates from Windows store");
        status.map(|_| "VelocityRL certificates deleted successfully.".into())
    }
    #[cfg(not(windows))]
    {
        Ok("Not applicable on this platform.".into())
    }
}

