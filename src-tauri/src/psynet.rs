use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tauri::State;

#[derive(Default)]
pub struct PsyNetState {

    pub running: Mutex<bool>,
}

/// Single-flight for proxy start/stop so mashed Save cannot overlap elevated scripts.
static PROXY_LIFECYCLE: Mutex<()> = Mutex::new(());

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
pub struct NameSpoofPayload {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub display_name: String,

    #[serde(default)]
    pub real_name: String,

    #[serde(default)]
    pub player_id: String,

    #[serde(default)]
    pub replace_all_player_names: bool,

    #[serde(default)]
    pub broker: bool,

    #[serde(default)]
    pub classprop_name: bool,

    #[serde(default)]
    pub websocket: bool,
    #[serde(default)]
    pub ws_enabled: bool,
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
    pub name_spoof: Option<NameSpoofPayload>,

    #[serde(default)]
    pub logo_spoof: Option<LogoSpoofPayload>,

    #[serde(default)]
    pub blog_spoof: Option<BlogSpoofPayload>,

    #[serde(default)]
    pub camera_spoof: Option<CameraSpoofPayload>,

    #[serde(default)]
    pub ping_spoof: Option<PingSpoofPayload>,

    #[serde(default)]
    pub fake_ranks: Option<FakeRanksPayload>,

    #[serde(default)]
    pub inventory_spoof: Option<InventorySpoofPayload>,
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

fn proxy_dir_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tools")
            .join("psynet_proxy")
            .join("go_mitm"),
    );

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {

            candidates.push(
                dir.join("..")
                    .join("..")
                    .join("..")
                    .join("tools")
                    .join("psynet_proxy")
                    .join("go_mitm"),
            );
            candidates.push(
                dir.join("..")
                    .join("..")
                    .join("..")
                    .join("..")
                    .join("tools")
                    .join("psynet_proxy")
                    .join("go_mitm"),
            );
            candidates.push(dir.join("tools").join("psynet_proxy").join("go_mitm"));
            candidates.push(dir.join("psynet_proxy"));
        }
    }

    let mut seen = std::collections::HashSet::<String>::new();
    candidates
        .into_iter()
        .filter(|c| seen.insert(c.to_string_lossy().into_owned()))
        .collect()
}

fn find_proxy_dir() -> Result<PathBuf, String> {
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;

    for c in proxy_dir_candidates() {
        let canon = normalize_win_path(fs::canonicalize(&c).unwrap_or(c));
        let exe = canon.join("psynet_proxy.exe");
        let starter = canon.join("start_from_app.ps1");
        if !exe.is_file() || !starter.is_file() {
            continue;
        }
        let Ok(meta) = fs::metadata(&exe) else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, t)| mtime > *t) {
            best = Some((canon, mtime));
        }
    }

    if let Some((dir, _mtime)) = best {
        return Ok(dir);
    }

    Err("Could not find tools/psynet_proxy/go_mitm (need psynet_proxy.exe + start_from_app.ps1). Build: cd tools/psynet_proxy/go_mitm; go build -o psynet_proxy.exe .".into())
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("psynet_config.json")
}

fn read_config_player_id(dir: &Path) -> Option<String> {
    let path = config_path(dir);
    let text = fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = v
        .get("name_spoof")?
        .get("player_id")?
        .as_str()?
        .trim();
    if pid.is_empty() {
        None
    } else {
        Some(pid.to_string())
    }
}

fn status_for(dir: Option<PathBuf>, process_alive: bool) -> PsyNetStatus {
    let hosts_redirected = psynet_hosts_redirected();
    let (port443_ok, port443_owner) = loopback443_status();
    // viewer_ok historically meant http://127.0.0.1:8081/; the traffic viewer is gone.
    // Treat loopback :443 (MITM listen) as the proxy health signal.
    let viewer_ok = process_alive && port443_ok;

    let running = process_alive && port443_ok;
    let last_capture_secs_ago = None;
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
    }
}

fn ensure_broker_for_ws_spoofs(obj: &mut serde_json::Map<String, serde_json::Value>) {
    let mut ns = obj
        .get("name_spoof")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(m) = ns.as_object_mut() {
        m.insert("broker".into(), serde_json::json!(true));

        m.insert("rewrite_ws_url".into(), serde_json::json!(false));

        m.insert("enabled".into(), serde_json::json!(false));
        m.insert("websocket".into(), serde_json::json!(false));
        m.insert("openssl_trust".into(), serde_json::json!(false));
    }
    obj.insert("name_spoof".into(), ns);
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
    if let Some(v) = ov.display_mmr {
        m.insert("display_mmr".into(), serde_json::json!(v));
    }
    if let Some(v) = ov.mu {
        m.insert("mu".into(), serde_json::json!(v));
    }
    if let Some(v) = ov.sigma {
        m.insert("sigma".into(), serde_json::json!(v));
    }
    if let Some(v) = ov.tier {
        m.insert("tier".into(), serde_json::json!(v));
    }
    if let Some(v) = ov.division {
        m.insert("division".into(), serde_json::json!(v));
    }
    if let Some(v) = ov.win_streak {
        m.insert("win_streak".into(), serde_json::json!(v));
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
        title_color: None,
    }])
}

fn write_spoof(dir: &Path, payload: &SpoofPayload) -> Result<PathBuf, String> {
    let path = config_path(dir);

    let method = "raw";

    let mut body = if path.is_file() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = body.as_object_mut() {
        obj.insert("method".into(), serde_json::json!(method));

        obj.remove("observe_only");
        if let Some(ns) = &payload.name_spoof {
            let display = if !ns.display_name.trim().is_empty() {
                ns.display_name.clone()
            } else {
                payload.custom_name.clone()
            };

            // Force-off name/ping/inventory only — always keep broker true for PerCon rewrite.
            let mut name_obj = serde_json::json!({
                "classprop_name": false,
                "broker": true,
                "rewrite_ws_url": false,
                "enabled": false,
                "display_name": display,
                "real_name": ns.real_name.trim(),
                "replace_all_player_names": false,
                "websocket": false,
                "openssl_trust": false,
            });
            let pid = ns.player_id.trim();
            if let Some(m) = name_obj.as_object_mut() {
                if !pid.is_empty() && !pid.to_ascii_lowercase().contains("|temp|") {
                    m.insert("player_id".into(), serde_json::json!(pid));
                }
            }
            obj.insert("name_spoof".into(), name_obj);

            obj.insert("custom_name".into(), serde_json::json!(display));
        }
        if let Some(ls) = &payload.logo_spoof {
            // Merge-only: omitted logo_spoof leaves the existing disk key untouched.
            obj.insert(
                "logo_spoof".into(),
                serde_json::json!({
                    "enabled": ls.enabled,
                    "logo_url": ls.logo_url.trim(),
                }),
            );
        }
        if let Some(bs) = &payload.blog_spoof {
            // Merge-only: omitted blog_spoof leaves the existing disk key untouched.
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

        obj.insert(
            "ping_spoof".into(),
            serde_json::json!({
                "enabled": false,
                "ms": 0,
            }),
        );
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
                    if !map.is_empty() {
                        m.insert("playlists".into(), serde_json::Value::Object(map));
                    }
                }
                if let Some(rl) = &fr.reward_levels {
                    let mut rl_obj = serde_json::Map::new();
                    if let Some(v) = rl.season_level {
                        rl_obj.insert("season_level".into(), serde_json::json!(v));
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
            if fr.enabled {
                ensure_broker_for_ws_spoofs(obj);
            }
        }

        obj.insert(
            "inventory_spoof".into(),
            serde_json::json!({
                "enabled": false,
                "items": [],
            }),
        );
        if let Some(swaps) = resolve_swaps(payload) {
            obj.insert("enabled".into(), serde_json::json!(payload.enabled));
            obj.insert("swaps".into(), serde_json::to_value(&swaps).unwrap_or(serde_json::json!([])));
            if let Some(first) = swaps.first() {
                obj.insert("equip_title_id".into(), serde_json::json!(first.equip_title_id));
                obj.insert("display_title_id".into(), serde_json::json!(first.display_title_id));
                obj.insert("custom_text".into(), serde_json::json!(first.custom_text));
                obj.insert("category".into(), serde_json::json!(first.category));
            } else {
                obj.insert("equip_title_id".into(), serde_json::json!(""));
                obj.insert("display_title_id".into(), serde_json::json!(""));
                obj.insert("custom_text".into(), serde_json::json!(""));
                obj.insert("category".into(), serde_json::json!(""));
            }
        }

        // 2.0: always enable broker so AuthPlayer PerCon rewrites to http://127.0.0.1.
        ensure_broker_for_ws_spoofs(obj);
    }
    fs::write(&path, serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(windows)]
fn tasklist_text() -> String {
    use std::os::windows::process::CommandExt;
    Command::new("tasklist")
        .arg("/NH")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
        .unwrap_or_default()
}

fn proxy_process_alive() -> bool {
    #[cfg(windows)]
    {
        tasklist_text().contains("psynet_proxy.exe")
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
const RL_PROCESS_NAMES: [&str; 3] = [
    "rocketleague.exe",
    "rocketleague_eac.exe",
    "rocketleague_eos.exe",
];

#[cfg(windows)]
pub fn rocket_league_process() -> Option<(String, u32)> {
    use std::os::windows::process::CommandExt;
    let out = Command::new("tasklist")
        .args(["/NH", "/FO", "CSV"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    for line in text.lines() {
        let mut fields = line.split("\",\"").map(|f| f.trim_matches('"').trim());
        let (Some(name), Some(pid)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !RL_PROCESS_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        return Some((name.to_string(), pid));
    }
    None
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

/// Same pairs start_from_app.ps1 writes for config MITM (never ws.rlpp / api.rlpp).
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
    }
    #[cfg(not(windows))]
    {
        false
    }
}

static HOSTS_ENSURE: Mutex<()> = Mutex::new(());

#[cfg(windows)]
fn loopback443_status() -> (bool, Option<String>) {
    use std::os::windows::process::CommandExt;

    let script = r#"
$rows = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue |
  Where-Object { $_.LocalAddress -in @('127.0.0.1','::1') })
if ($rows.Count -eq 0) { Write-Output 'NONE|0'; exit 0 }
$names = @()
foreach ($c in $rows) {
  $o = Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
  $n = if ($o) { $o.ProcessName } else { '?' }
  $names += $n
}
$primary = ($names | Select-Object -First 1)
Write-Output ($primary + '|' + $rows[0].OwningProcess)
"#;
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return (false, None);
    };
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if line.is_empty() || line.starts_with("NONE") {
        return (false, None);
    }
    let mut parts = line.split('|');
    let name = parts.next().unwrap_or("?").trim().to_string();
    let ok = name.eq_ignore_ascii_case("psynet_proxy");
    (ok, Some(name))
}

#[cfg(not(windows))]
fn loopback443_status() -> (bool, Option<String>) {
    (false, None)
}

fn kill_proxy_processes() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "psynet_proxy.exe", "/T"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

/// Called on any exit path (clean shutdown or force-kill via ctrlc handler).
pub fn kill_proxy_on_exit() {
    crate::applog::event("psynet: killing proxy on exit");
    kill_proxy_processes();
    #[cfg(windows)]
    if proxy_process_alive() {
        kill_proxy_elevated();
    }
}

fn stop_proxy_before_start(dir: &Path, revert_hosts: bool) {
    kill_proxy_processes();
    #[cfg(windows)]
    {
        let stop = dir.join("stop_proxy.ps1");
        if stop.is_file() {
            let mut args: Vec<&str> = vec!["-WaitMs", "10000", "-Quiet"];
            if revert_hosts {
                args.push("-RevertHosts");
            }
            let _ = run_elevated_ps1_args(&stop, &args);
        } else {
            kill_proxy_elevated();
            wait_proxy_ports_released(10_000);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, revert_hosts);
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
}

#[cfg(windows)]
fn wait_proxy_ports_released(timeout_ms: u64) {
    use std::os::windows::process::CommandExt;

    let script = format!(
        r#"$deadline = [Environment]::TickCount + {timeout_ms}
while ([Environment]::TickCount -lt $deadline) {{
    & taskkill.exe /F /IM psynet_proxy.exe /T 2>$null | Out-Null
    $procs = @(Get-Process -Name psynet_proxy -ErrorAction SilentlyContinue)
    $on443 = @(Get-NetTCPConnection -LocalPort 443 -State Listen -ErrorAction SilentlyContinue | Where-Object {{
        $o = Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue
        $o -and $o.ProcessName -eq 'psynet_proxy'
    }})
    if ($procs.Count -eq 0 -and $on443.Count -eq 0) {{ exit 0 }}
    Start-Sleep -Milliseconds 250
}}
exit 1"#
    );
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(windows)]
fn kill_proxy_elevated() {
    use std::os::windows::process::CommandExt;

    let inner =
        "Get-Process -Name psynet_proxy -ErrorAction SilentlyContinue | Stop-Process -Force";
    let status = if is_process_elevated() {
        Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", inner])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
    } else {
        let cmd = format!(
            "$p = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile','-WindowStyle','Hidden','-Command','{inner}'); if ($null -eq $p) {{ exit 1223 }}; exit $p.ExitCode"
        );
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &cmd,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
    };
    let _ = status;
}

#[cfg(windows)]
fn is_process_elevated() -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_ascii_lowercase();
            Some(s == "true")
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn setup_log_path(script: &Path) -> PathBuf {
    script
        .parent()
        .map(|p| p.join("start_from_app.log"))
        .unwrap_or_else(|| script.with_extension("log"))
}

#[cfg(windows)]
fn read_setup_log(script: &Path) -> Option<String> {
    let log = setup_log_path(script);
    let mut text = None;
    for attempt in 0..6 {
        match fs::read_to_string(&log) {
            Ok(s) => {
                text = Some(s);
                break;
            }
            Err(_) if attempt < 5 => {
                std::thread::sleep(std::time::Duration::from_millis(80 * (attempt + 1) as u64));
            }
            Err(_) => return None,
        }
    }
    text.and_then(|s| {

        let s = s.strip_prefix('\u{feff}').unwrap_or(&s);
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        let lines: Vec<&str> = trimmed.lines().collect();
        let errors: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| l.contains("ERROR:"))
            .collect();
        if !errors.is_empty() {
            return Some(errors.join("\n"));
        }
        let start = lines.len().saturating_sub(12);
        Some(lines[start..].join("\n"))
    })
}

#[cfg(windows)]
fn powershell_parse_errors(script: &Path) -> Option<String> {
    let script_str = script.to_string_lossy().replace('\'', "''");
    let cmd = format!(
        "$errs = $null; $null = [System.Management.Automation.Language.Parser]::ParseFile('{script_str}', [ref]$null, [ref]$errs); if ($errs -and $errs.Count -gt 0) {{ $errs | ForEach-Object {{ $_.ToString() }} }} "
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &cmd])
        .output()
        .ok()?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        None
    } else {
        let lines: Vec<&str> = trimmed.lines().collect();
        let start = lines.len().saturating_sub(16);
        Some(lines[start..].join("\n"))
    }
}

#[cfg(windows)]
fn run_elevated_ps1_args(script: &Path, extra_args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let script = normalize_win_path(script.to_path_buf());
    if !script.is_file() {
        return Err(format!("missing script: {}", script.display()));
    }

    let mut args: Vec<String> = vec![
        "-NoProfile".into(),
        "-WindowStyle".into(),
        "Hidden".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.to_string_lossy().into_owned(),
    ];
    for a in extra_args {
        args.push((*a).into());
    }

    let status = if is_process_elevated() {
        Command::new("powershell")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("script failed: {e}"))?
    } else {
        let script_str = script.to_string_lossy().replace('\'', "''");
        let arg_list = extra_args
            .iter()
            .map(|a| format!("'{a}'"))
            .collect::<Vec<_>>()
            .join(",");
        let arg_list = if arg_list.is_empty() {
            String::new()
        } else {
            format!(", {arg_list}")
        };
        let cmd = format!(
            "$p = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File','{script_str}'{arg_list}); if ($null -eq $p) {{ exit 1223 }}; exit $p.ExitCode"
        );
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &cmd,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("elevate failed: {e}"))?
    };

    if status.success() {
        Ok(())
    } else if status.code() == Some(1223) {
        Err("UAC was cancelled".into())
    } else {
        Err(format!("script failed (exit {:?})", status.code()))
    }
}

#[cfg(windows)]
fn run_elevated_ps1(script: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let script = normalize_win_path(script.to_path_buf());
    if !script.is_file() {
        return Err(format!("missing script: {}", script.display()));
    }

    let status = if is_process_elevated() {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script.to_string_lossy(),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("setup failed: {e}"))?
    } else {

        let script_str = script.to_string_lossy().replace('\'', "''");
        let cmd = format!(
            "$p = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File','{script_str}'); if ($null -eq $p) {{ Write-Error 'UAC cancelled or elevation failed'; exit 1223 }}; exit $p.ExitCode"
        );
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &cmd,
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

    let detail = read_setup_log(&script)
        .or_else(|| powershell_parse_errors(&script))
        .unwrap_or_default();
    if !detail.is_empty() {
        return Err(format!(
            "Proxy setup failed (exit {code:?}).\n{detail}"
        ));
    }
    Err(format!(
        "Proxy setup failed (exit {code:?}). Approve UAC when prompted, then retry."
    ))
}

#[cfg(not(windows))]
fn run_elevated_ps1(_script: &Path) -> Result<(), String> {
    Err("PsyNet proxy is Windows-only for now.".into())
}

#[cfg(windows)]
const ENSURE_CONFIG_HOSTS_PS: &str = r#"$ErrorActionPreference = "Stop"
$hostsPath = Join-Path $env:SystemRoot "System32\drivers\etc\hosts"
if (-not (Test-Path -LiteralPath $hostsPath)) { throw "hosts file not found" }
function Add-HostsLine([string]$Path, [string]$Line) {
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        try {
            $fs = [System.IO.FileStream]::new(
                $Path,
                [System.IO.FileMode]::Append,
                [System.IO.FileAccess]::Write,
                ([System.IO.FileShare]"ReadWrite, Delete")
            )
            try {
                $bytes = [System.Text.Encoding]::ASCII.GetBytes("`r`n$Line")
                $fs.Write($bytes, 0, $bytes.Length)
                return
            } finally { $fs.Dispose() }
        } catch [System.IO.IOException] {
            if ($attempt -eq 5) { throw }
            Start-Sleep -Milliseconds 500
        }
    }
}
$raw = [System.IO.File]::ReadAllText($hostsPath)
foreach ($pair in @(
    @{ Ip = "127.0.0.1"; Host = "config.psynet.gg" },
    @{ Ip = "::1"; Host = "config.psynet.gg" }
)) {
    $pat = [regex]::Escape($pair.Ip) + "\s+" + [regex]::Escape($pair.Host)
    if ($raw -notmatch $pat) {
        Add-HostsLine -Path $hostsPath -Line "$($pair.Ip) $($pair.Host)"
        $raw += "`r`n$($pair.Ip) $($pair.Host)"
    }
}
ipconfig /flushdns | Out-Null
exit 0
"#;

/// Ensure config.psynet.gg -> loopback in the Windows hosts file.
/// No UAC if both 127.0.0.1 and ::1 entries are already present.
/// Returns true when hosts were already correct (no elevation).
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
        if config_hosts_complete() {
            crate::applog::event("psynet: config.psynet.gg hosts already present");
            return Ok(true);
        }
        crate::applog::event(
            "psynet: config hosts missing — elevating to add",
        );
        let tmp = std::env::temp_dir().join(format!(
            "velocityrl_ensure_hosts_{}.ps1",
            std::process::id()
        ));
        fs::write(&tmp, ENSURE_CONFIG_HOSTS_PS)
            .map_err(|e| format!("could not write hosts helper: {e}"))?;
        let result = run_elevated_ps1(&tmp);
        let _ = fs::remove_file(&tmp);
        result?;
        if config_hosts_complete() {
            crate::applog::event("psynet: config.psynet.gg hosts written");
            Ok(false)
        } else {
            Err(
                "UAC finished but config.psynet.gg was not added to hosts. Approve the prompt and retry."
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
    let dir = find_proxy_dir()?;
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
    let _guard = PROXY_LIFECYCLE.lock().map_err(|e| e.to_string())?;
    let dir = find_proxy_dir()?;
    let path = write_spoof(&dir, &payload)?;
    let _ = state;
    // Go watchCfg() polls psynet_config.json — no proxy restart needed.
    crate::applog::event(&format!(
        "psynet: wrote spoof config {} (hot-reload)",
        path.display()
    ));
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn get_psynet_status(state: State<'_, PsyNetState>) -> Result<PsyNetStatus, String> {
    let alive = proxy_process_alive();
    {
        let mut g = state.running.lock().map_err(|e| e.to_string())?;
        *g = alive;
    }
    let dir = find_proxy_dir().ok();
    Ok(status_for(dir, alive))
}

#[tauri::command]
pub async fn start_psynet_proxy(
    state: State<'_, PsyNetState>,
    payload: Option<SpoofPayload>,
) -> Result<PsyNetStatus, String> {
    crate::applog::event("psynet: start requested");
    let _guard = PROXY_LIFECYCLE.lock().map_err(|e| e.to_string())?;
    let dir = find_proxy_dir().map_err(|e| {
        crate::applog::event(&format!("psynet: find_proxy_dir failed: {e}"));
        e
    })?;
    let exe = dir.join("psynet_proxy.exe");
    if let Ok(meta) = fs::metadata(&exe) {
        crate::applog::event(&format!(
            "psynet: proxy dir {} (exe mtime {:?}, {} bytes)",
            dir.display(),
            meta.modified().ok(),
            meta.len()
        ));
    }
    if !exe.is_file() {
        let msg = format!(
            "Missing {}. Build with: cd tools/psynet_proxy/go_mitm; go build -o psynet_proxy.exe .",
            exe.display()
        );
        crate::applog::event(&format!("psynet: {msg}"));
        return Err(msg);
    }

    if let Some(p) = payload {
        write_spoof(&dir, &p).map_err(|e| {
            crate::applog::event(&format!("psynet: write_spoof failed: {e}"));
            e
        })?;
    } else if !config_path(&dir).is_file() {
        write_spoof(
            &dir,
            &SpoofPayload {
                enabled: true,
                equip_title_id: "Team_Iraq_World_Cup_2026".into(),
                display_title_id: "RLCS_X_Champion".into(),
                custom_text: "RLCS X Champion".into(),
                category: "RLCS_Champion".into(),
                custom_name: String::new(),
                name_spoof: None,
                logo_spoof: None,
                blog_spoof: None,
                camera_spoof: None,
                ping_spoof: None,
                fake_ranks: None,
                inventory_spoof: None,
                swaps: Some(vec![TitleSwapEntry {
                    equip_title_id: "Team_Iraq_World_Cup_2026".into(),
                    display_title_id: "RLCS_X_Champion".into(),
                    custom_text: "RLCS X Champion".into(),
                    category: "RLCS_Champion".into(),
                    title_color: None,
                }]),
                method: "raw".into(),
            },
        )
        .map_err(|e| {
            crate::applog::event(&format!("psynet: write default spoof failed: {e}"));
            e
        })?;
    }

    // Already healthy: config write above is enough (Go watchCfg hot-reloads).
    let alive_now = proxy_process_alive();
    let (port443_ok_now, _) = loopback443_status();
    if alive_now && port443_ok_now {
        crate::applog::event(
            "psynet: proxy already listening on loopback :443 — skip restart (config hot-reload)",
        );
        *state.running.lock().map_err(|e| e.to_string())? = true;
        return Ok(status_for(Some(dir), true));
    }

    crate::applog::event("psynet: stopping any existing psynet_proxy before start");
    stop_proxy_before_start(&dir, false);

    let starter = dir.join("start_from_app.ps1");
    run_elevated_ps1(&starter).map_err(|e| {
        crate::applog::event(&format!("psynet: elevated setup failed: {e}"));
        e
    })?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let alive = proxy_process_alive();
        let (port443_ok, port443_owner) = loopback443_status();
        let foreign_conflict = port443_owner
            .as_deref()
            .map(|n| {
                !n.is_empty()
                    && !n.eq_ignore_ascii_case("none")
                    && !n.eq_ignore_ascii_case("psynet_proxy")
            })
            .unwrap_or(false);
        if foreign_conflict {
            let msg = format!(
                "Another process owns loopback :443 ({}). Quit that process, then start the proxy again.",
                port443_owner.as_deref().unwrap_or("unknown")
            );
            crate::applog::event(&format!("psynet: {msg}"));
            return Err(msg);
        }
        if alive && port443_ok {
            break;
        }
        if !alive {
            let detail = read_setup_log(&starter).unwrap_or_default();
            let msg = if detail.is_empty() {
                "Proxy did not stay running. Quit any other process using port 443, approve UAC when prompted, then retry.".into()
            } else {
                format!("Proxy did not stay running.\n{detail}")
            };
            crate::applog::event(&format!("psynet: {msg}"));
            return Err(msg);
        }
        if std::time::Instant::now() >= deadline {
            let owner = port443_owner
                .as_deref()
                .filter(|o| !o.is_empty())
                .unwrap_or("not bound yet");
            let msg = format!(
                "Proxy process is up but loopback :443 is held by {owner}. Quit that process, approve UAC, and restart the proxy."
            );
            crate::applog::event(&format!("psynet: {msg}"));
            return Err(msg);
        }
    }

    let running = proxy_process_alive();
    *state.running.lock().map_err(|e| e.to_string())? = running;
    crate::applog::event(&format!(
        "psynet: proxy running; hosts_redirected={} port443_ok={}",
        psynet_hosts_redirected(),
        loopback443_status().0
    ));

    Ok(status_for(Some(dir), running))
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
    crate::applog::event("psynet: stop requested");
    let _guard = PROXY_LIFECYCLE.lock().map_err(|e| e.to_string())?;
    let do_revert = revert_hosts.unwrap_or(false);
    let dir = find_proxy_dir().ok();

    if let Some(ref d) = dir {
        stop_proxy_before_start(d, do_revert);
    } else {
        kill_proxy_processes();
        #[cfg(windows)]
        if proxy_process_alive() {
            kill_proxy_elevated();
        }
    }

    *state.running.lock().map_err(|e| e.to_string())? = false;

    let alive = proxy_process_alive();
    crate::applog::event(&format!(
        "psynet: stop complete; process_alive={alive} revert_hosts={do_revert}"
    ));

    Ok(status_for(dir, alive))
}
