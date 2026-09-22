use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

mod applog;
mod integrity;
mod jobobject;
mod presets;
mod psynet;
pub mod upk;
pub mod workshop;
pub mod tracker;
mod winprobe;
pub mod proxy;
pub mod features;

pub(crate) fn default_true() -> bool { true }

pub(crate) fn app_config_dir_of(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

pub(crate) fn now_iso8601_utc() -> String {

    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn record_swap_history(app: &tauri::AppHandle, kind: &str, entries: &[SwapEntry], note: &str) {
    presets::append_history(app, kind, entries, note);
}

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    game_dir: String,
    #[serde(default)]
    privacy_agreed: bool,
    #[serde(default)]
    privacy_version: String,
    #[serde(default = "default_true")]
    changelog_on_startup: bool,

    #[serde(default)]
    launch_on_startup: bool,
    #[serde(default = "default_lang")]
    language: String,
}

fn default_lang() -> String {
    "en".to_string()
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct ItemAttribute {
    #[serde(default, alias = "Key")]
    key: String,
    #[serde(default, alias = "Value")]
    value: serde_json::Value,
}

fn opt_paintable<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match val {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Bool(b)) => Some(b),
        Some(serde_json::Value::Number(n)) => Some(n.as_i64() != Some(0)),
        Some(serde_json::Value::String(s)) => {
            let s = s.trim().to_lowercase();
            if matches!(s.as_str(), "true" | "yes" | "1" | "paintable") {
                Some(true)
            } else if matches!(s.as_str(), "false" | "no" | "0" | "unpaintable" | "none") {
                Some(false)
            } else {
                None
            }
        }
        Some(_) => None,
    })
}

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    #[serde(alias = "id-rl-garage", alias = "id", alias = "ID")]
    id: i32,
    #[serde(alias = "name", alias = "Product")]
    product: String,
    #[serde(default, alias = "src")]
    image_url: String,
    #[serde(default, alias = "AssetPackage", alias = "asset_package")]
    asset_package: String,
    #[serde(default, alias = "Type", alias = "Slot", alias = "slot")]
    slot: String,
    #[serde(default, alias = "Quality", alias = "quality")]
    quality: String,
    #[serde(default, alias = "Paintable", deserialize_with = "opt_paintable")]
    #[serde(skip_serializing_if = "Option::is_none")]
    paintable: Option<bool>,
    #[serde(default, alias = "Attributes")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attributes: Vec<ItemAttribute>,

    #[serde(default, alias = "DLC")]
    #[serde(skip_serializing_if = "String::is_empty")]
    dlc: String,
}

pub fn norm_item_slot(slot: &str) -> String {
    slot.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect()
}

pub(crate) fn is_non_swappable(item: &Item) -> bool {
    let pkg = item.asset_package.to_lowercase();

    if pkg == "bots_sf.upk" || pkg.starts_with("bot") || pkg == "tagame.upk" || pkg == "tagame" || pkg.starts_with("tagame") {
        return true;
    }

    if pkg.is_empty() || pkg == "none" {
        return true;
    }
    false
}

fn attr_flag(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Null => Some(true),
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) => Some(n.as_i64() != Some(0)),
        serde_json::Value::String(s) => {
            let s = s.trim().to_lowercase();
            if s.is_empty() {
                Some(true)
            } else if matches!(s.as_str(), "true" | "yes" | "1" | "paintable") {
                Some(true)
            } else if matches!(s.as_str(), "false" | "no" | "0" | "unpaintable" | "none") {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn item_is_paintable(item: &Item) -> bool {
    if let Some(flag) = item.paintable {
        return flag;
    }
    for a in &item.attributes {
        let k = a.key.to_lowercase();
        if k == "paintable" || k == "painted" || k == "paint" {
            if let Some(flag) = attr_flag(&a.value) {
                return flag;
            }
        }
    }
    false
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ItemsResponse {
    Database {
        #[serde(alias = "Items", alias = "items")]
        items: Vec<Item>
    },
    List(Vec<Item>),
}

#[derive(Serialize, Deserialize)]
struct BackupFile {
    name: String,
    path: String,
    #[serde(default)]
    image_url: String,
    #[serde(default)]
    swap_from: String,
    #[serde(default)]
    swap_to: String,
    #[serde(default)]
    swap_to_image: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SwapEntry {
    pub owned_id:  i32,
    pub wanted_id: i32,
    #[serde(default)]
    pub owned_name:  String,
    #[serde(default)]
    pub wanted_name: String,
    #[serde(default)]
    pub paint_id: i32,
    #[serde(default)]
    pub asset_package: String,
}

static ITEMS_CACHE: std::sync::RwLock<Option<Vec<Item>>> = std::sync::RwLock::new(None);

fn get_cached_items() -> Option<Vec<Item>> {
    ITEMS_CACHE.read().ok().and_then(|guard| guard.clone())
}

fn set_cached_items(items: Vec<Item>) {
    if let Ok(mut guard) = ITEMS_CACHE.write() {
        *guard = Some(items);
    }
}

const DIAGNOSTIC_URL: Option<&str> = option_env!("DIAGNOSTIC_URL");
const DIAGNOSTIC_SECRET: Option<&str> = option_env!("DIAGNOSTIC_SECRET");

async fn send_diagnostic(mut payload: serde_json::Value) {
    let (Some(url), Some(secret)) = (DIAGNOSTIC_URL, DIAGNOSTIC_SECRET) else { return };
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("version").or_insert_with(|| json!(env!("CARGO_PKG_VERSION")));
        obj.entry("os").or_insert_with(|| json!(std::env::consts::OS));
        obj.entry("arch").or_insert_with(|| json!(std::env::consts::ARCH));
        obj.entry("timestamp").or_insert_with(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            json!(ts)
        });
    }
    let client = reqwest::Client::builder()
        .user_agent(app_user_agent())
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let _ = client
        .post(url)
        .header("Authorization", format!("Bearer {}", secret))
        .json(&payload)
        .send()
        .await;
}

fn populate_thumbnails(items: &mut [Item]) {
    const THUMB_BASE: &str = "https://api.velocityrl.tech/thumbnails/";
    for item in items.iter_mut() {
        if item.image_url.is_empty() && !item.asset_package.is_empty() {
            let stem = item.asset_package
                .to_lowercase()
                .replace("_sf.upk", "")
                .replace(".upk", "");
            item.image_url = format!("{}{}_t.png", THUMB_BASE, stem);
        }
    }
}

pub(crate) fn app_user_agent() -> String {
    format!(
        "VelocityRL/{} ({}; {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct CatalogMetadata {
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    last_modified: Option<String>,
}

fn read_catalog_meta(dir: &Path) -> CatalogMetadata {
    let path = dir.join("items.meta.json");
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(meta) = serde_json::from_slice::<CatalogMetadata>(&bytes) {
            return meta;
        }
    }
    let ver_path = dir.join("items.ver");
    if let Ok(ver) = fs::read_to_string(&ver_path) {
        let v = ver.trim();
        if !v.is_empty() {
            return CatalogMetadata {
                etag: Some(v.to_string()),
                last_modified: None,
            };
        }
    }
    CatalogMetadata::default()
}

fn write_catalog_meta(dir: &Path, meta: &CatalogMetadata) {
    let path = dir.join("items.meta.json");
    if let Ok(data) = serde_json::to_vec(meta) {
        let _ = fs::write(&path, data);
    }
    if let Some(etag) = &meta.etag {
        let _ = fs::write(dir.join("items.ver"), etag);
    }
}

fn get_catalog_dirs(app: &tauri::AppHandle) -> (PathBuf, Option<PathBuf>) {
    let data_dir = app
        .path()
        .app_data_dir()
        .or_else(|_| app.path().app_config_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    let config_dir = app.path().app_config_dir().ok();
    (data_dir, config_dir)
}

fn load_raw_items_json(app: &tauri::AppHandle) -> Result<String, String> {
    let (data_dir, config_dir) = get_catalog_dirs(app);
    let mut candidates = vec![data_dir.join("items.json")];
    if let Some(cfg) = config_dir {
        candidates.push(cfg.join("items.json"));
    }
    for p in candidates {
        if p.is_file() {
            if let Ok(s) = fs::read_to_string(&p) {
                return Ok(s);
            }
        }
    }
    Err("Items database missing — check your internet connection and try again.".to_string())
}

async fn parse_items_slice(bytes: Vec<u8>) -> Result<Vec<Item>, String> {
    tokio::task::spawn_blocking(move || {
        let resp = serde_json::from_slice::<ItemsResponse>(&bytes)
            .map_err(|e| format!("JSON parse error: {e}"))?;
        let mut items = match resp {
            ItemsResponse::Database { items } => items,
            ItemsResponse::List(items) => items,
        };
        populate_thumbnails(&mut items);
        Ok(items)
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

async fn persist_catalog(
    items: Vec<Item>,
    data_dir: PathBuf,
    config_dir: Option<PathBuf>,
    meta: Option<CatalogMetadata>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let _ = fs::create_dir_all(&data_dir);
        let serialized = serde_json::to_vec(&serde_json::json!({ "Items": items }))
            .map_err(|e| format!("JSON serialize error: {e}"))?;

        let primary_path = data_dir.join("items.json");
        fs::write(&primary_path, &serialized)
            .map_err(|e| format!("Failed to write {}: {e}", primary_path.display()))?;

        if let Some(ref cfg) = config_dir {
            if cfg != &data_dir {
                let _ = fs::create_dir_all(cfg);
                let _ = fs::write(cfg.join("items.json"), &serialized);
            }
        }

        if let Some(meta) = meta {
            write_catalog_meta(&data_dir, &meta);
            if let Some(ref cfg) = config_dir {
                if cfg != &data_dir {
                    write_catalog_meta(cfg, &meta);
                }
            }
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

async fn fetch_catalog_update(
    client: &reqwest::Client,
    url: &str,
    meta: &CatalogMetadata,
) -> Result<Option<(Vec<u8>, CatalogMetadata)>, String> {
    let mut req = client.get(url);
    if let Some(etag) = &meta.etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, etag.as_str());
    }
    if let Some(lm) = &meta.last_modified {
        req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm.as_str());
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        log::info!("Catalog at {url} is not modified (HTTP 304), retaining cached version");
        return Ok(None);
    }

    if !resp.status().is_success() {
        return Err(format!("HTTP {} from {}", resp.status(), url));
    }

    let new_etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let new_lm = resp
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();

    Ok(Some((
        bytes,
        CatalogMetadata {
            etag: new_etag.or_else(|| meta.etag.clone()),
            last_modified: new_lm.or_else(|| meta.last_modified.clone()),
        },
    )))
}

async fn background_check_items_update(data_dir: PathBuf, config_dir: Option<PathBuf>) {
    let client = match reqwest::Client::builder()
        .user_agent(app_user_agent())
        .timeout(std::time::Duration::from_secs(6))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let meta = read_catalog_meta(&data_dir);
    let api_url = "https://api.velocityrl.tech/items.json";
    let github_url = "https://raw.githubusercontent.com/CrunchyRL/RLUPKTools/refs/heads/main/items.json";

    let result = match fetch_catalog_update(&client, api_url, &meta).await {
        Ok(res) => Ok(res),
        Err(e) => {
            log::warn!("Primary catalog update check failed ({e}), trying GitHub fallback");
            fetch_catalog_update(&client, github_url, &meta).await
        }
    };

    match result {
        Ok(None) => {
            // 304 Not Modified: cache is current and retained
        }
        Ok(Some((bytes, new_meta))) => {
            if let Ok(items) = parse_items_slice(bytes).await {
                set_cached_items(items.clone());
                let _ = persist_catalog(items, data_dir, config_dir, Some(new_meta)).await;
            }
        }
        Err(e) => {
            log::warn!("Catalog update check failed: {e}");
        }
    }
}

#[tauri::command]
async fn get_items(app: tauri::AppHandle) -> Result<Vec<Item>, String> {
    if let Some(cached) = get_cached_items() {
        return Ok(cached);
    }

    let (data_dir, config_dir) = get_catalog_dirs(&app);
    let cache_path = data_dir.join("items.json");

    let candidate_cache = if cache_path.is_file() {
        Some(cache_path.clone())
    } else if let Some(ref cfg) = config_dir {
        let cfg_path = cfg.join("items.json");
        if cfg_path.is_file() {
            Some(cfg_path)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(local_path) = candidate_cache {
        if let Ok(bytes) = fs::read(&local_path) {
            if let Ok(items) = parse_items_slice(bytes).await {
                set_cached_items(items.clone());

                let d_dir = data_dir.clone();
                let c_dir = config_dir.clone();
                tauri::async_runtime::spawn(async move {
                    background_check_items_update(d_dir, c_dir).await;
                });

                return Ok(items);
            }
        }
    }

    let mut bundled_candidates = Vec::new();
    if let Ok(res_dir) = app.path().resource_dir() {
        bundled_candidates.push(res_dir.join("items.json"));
        bundled_candidates.push(res_dir.join("resources").join("items.json"));
    }
    bundled_candidates.push(PathBuf::from("resources").join("items.json"));

    for bundled in bundled_candidates {
        if bundled.is_file() {
            if let Ok(bytes) = fs::read(&bundled) {
                if let Ok(items) = parse_items_slice(bytes).await {
                    set_cached_items(items.clone());

                    let d_dir = data_dir.clone();
                    let c_dir = config_dir.clone();
                    let items_for_save = items.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = persist_catalog(items_for_save, d_dir.clone(), c_dir.clone(), None).await;
                        background_check_items_update(d_dir, c_dir).await;
                    });

                    return Ok(items);
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .user_agent(app_user_agent())
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let api_url = "https://api.velocityrl.tech/items.json";
    let github_url = "https://raw.githubusercontent.com/CrunchyRL/RLUPKTools/refs/heads/main/items.json";

    let empty_meta = CatalogMetadata::default();
    let fetch_res = match fetch_catalog_update(&client, api_url, &empty_meta).await {
        Ok(Some(res)) => Some(res),
        _ => fetch_catalog_update(&client, github_url, &empty_meta).await.ok().flatten(),
    };

    if let Some((bytes, meta)) = fetch_res {
        let items = parse_items_slice(bytes).await?;
        set_cached_items(items.clone());

        let d_dir = data_dir.clone();
        let c_dir = config_dir.clone();
        let items_for_save = items.clone();
        tauri::async_runtime::spawn(async move {
            let _ = persist_catalog(items_for_save, d_dir, c_dir, Some(meta)).await;
        });

        return Ok(items);
    }

    Err("Failed to load items database".into())
}

#[tauri::command]
async fn get_config(app: tauri::AppHandle) -> Result<Config, String> {
    let config_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("config.json");
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        let config: Config = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(config)
    } else {
        Ok(Config { game_dir: "".to_string(), privacy_agreed: false, privacy_version: "".to_string(), changelog_on_startup: true, launch_on_startup: false, language: "en".to_string() })
    }
}

const STARTUP_TASK_NAME: &str = "VelocityRL";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn scheduled_task_exists() -> bool {
    if !cfg!(windows) {
        return false;
    }
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(["/Query", "/TN", STARTUP_TASK_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.status();
    matches!(out, Ok(s) if s.success())
}

#[tauri::command]
async fn get_launch_on_startup() -> Result<bool, String> {
    Ok(scheduled_task_exists())
}

#[tauri::command]
async fn set_launch_on_startup(enable: bool) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Startup task is only supported on Windows".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if enable {
        let mut cmd = std::process::Command::new("schtasks");
        cmd.args([
            "/Create",
            "/TN", STARTUP_TASK_NAME,
            "/TR", &format!("\"{}\"", exe.display()),
            "/SC", "ONLOGON",
            "/RL", "HIGHEST",
            "/F",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "Could not create startup task: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    } else if scheduled_task_exists() {
        let mut cmd = std::process::Command::new("schtasks");
        cmd.args(["/Delete", "/TN", STARTUP_TASK_NAME, "/F"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "Could not remove startup task: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

fn normalize_game_dir(game_dir: &str) -> String {
    if game_dir.is_empty() {
        return String::new();
    }
    upk::palette::resolve_cooked_dir(Path::new(game_dir))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| game_dir.to_string())
}

#[tauri::command]
async fn save_config(app: tauri::AppHandle, mut config: Config) -> Result<String, String> {
    config.game_dir = normalize_game_dir(&config.game_dir);
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let config_path = config_dir.join("config.json");
    let content = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    fs::write(config_path, content).map_err(|e| e.to_string())?;
    Ok(config.game_dir)
}

#[tauri::command]
async fn get_backups(app: tauri::AppHandle) -> Result<Vec<BackupFile>, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Ok(vec![]); }

    let items = get_items(app.clone()).await.unwrap_or_default();
    let swaps = load_swaps(&app);
    let mut backups = Vec::new();
    let dir = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .map_err(|e| e.to_string())?;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name_lower = path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_lowercase())
                .unwrap_or_default();

            if file_name_lower.ends_with(".upk.bak") {

                if file_name_lower == "tagame.upk.bak"
                    || file_name_lower == "engine.upk.bak"
                    || file_name_lower.starts_with("labs_underpass_p")
                {
                    continue;
                }

                let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                let clean_name = file_name.to_lowercase()
                    .replace(".upk.bak", "")
                    .replace(".upk", "");

                let matched_item = items.iter()
                    .find(|i| {
                        let db_pkg = i.asset_package.to_lowercase().replace(".upk", "");
                        if db_pkg.is_empty() || db_pkg == "none" { return false; }
                        if db_pkg == clean_name { return true; }
                        if db_pkg.len() > 4 && (clean_name.contains(&db_pkg) || db_pkg.contains(&clean_name)) {
                            return true;
                        }
                        false
                    });

                let display_name = matched_item.map(|i| i.product.clone()).unwrap_or(file_name);
                let image_url = matched_item.map(|i| i.image_url.clone()).unwrap_or_default();

                let swap_entry = matched_item.and_then(|item| swaps.iter().find(|s| s.owned_id == item.id));
                let (swap_from, swap_to) = swap_entry
                    .map(|s| (s.owned_name.clone(), s.wanted_name.clone()))
                    .unwrap_or_default();
                let swap_to_image = swap_entry
                    .and_then(|s| items.iter().find(|i| i.id == s.wanted_id))
                    .map(|i| i.image_url.clone())
                    .unwrap_or_default();

                backups.push(BackupFile {
                    name: display_name,
                    path: path.to_string_lossy().to_string(),
                    image_url,
                    swap_from,
                    swap_to,
                    swap_to_image,
                });
            }
        }
    }
    Ok(backups)
}

fn integrity_state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(integrity::integrity_path(&dir))
}

fn load_integrity(app: &tauri::AppHandle) -> integrity::IntegrityState {
    integrity_state_path(app)
        .ok()
        .map(|p| integrity::IntegrityState::load(&p))
        .unwrap_or_default()
}

fn save_integrity(app: &tauri::AppHandle, state: &integrity::IntegrityState) -> Result<(), String> {
    let path = integrity_state_path(app)?;
    state.save(&path)
}

#[tauri::command]
async fn check_integrity(app: tauri::AppHandle) -> Result<integrity::RepairReport, String> {
    let config = get_config(app.clone()).await?;
    let mut state = load_integrity(&app);

    for s in load_swaps(&app) {
        if !s.asset_package.is_empty() {
            integrity::mark_swap_package(&mut state, &s.asset_package, None);
        }
    }
    let game = PathBuf::from(&config.game_dir);
    Ok(integrity::check_repair(&game, &state))
}

#[tauri::command]
async fn acknowledge_repair(app: tauri::AppHandle) -> Result<(), String> {
    let config = get_config(app.clone()).await?;
    let mut state = load_integrity(&app);
    integrity::acknowledge_repair(Path::new(&config.game_dir), &mut state);
    save_integrity(&app, &state)
}

#[tauri::command]
async fn get_palette_status(app: tauri::AppHandle) -> Result<upk::PaletteStatus, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".into());
    }
    let state = load_integrity(&app);
    let fp = if state.palette_active {
        Some(state.palette_fingerprint.as_str())
    } else {
        None
    };
    Ok(upk::palette::read_palette_status(Path::new(&config.game_dir), fp))
}

fn palette_blocked_by_game(action: &str) -> Option<String> {
    psynet::rocket_league_lock_holder()
        .map(|who| format!("Rocket League is running ({who}). Close it, then {action}."))
}

fn explain_upk_lock(err: String, what: &str) -> String {
    if err.contains("os error 32") || err.contains("os error 33") {
        return format!(
            "{err} — {what} is locked by another program. Close Rocket League and the Epic launcher, then retry."
        );
    }
    err
}

fn explain_palette_error(err: String) -> String {
    if err.contains("os error 32") || err.contains("os error 33") {
        return format!(
            "{err} — TAGame.upk is locked by another program. Close Rocket League and the Epic launcher, then retry."
        );
    }
    err
}

#[tauri::command]
async fn apply_rich_palette(app: tauri::AppHandle) -> Result<upk::PaletteStatus, String> {
    if let Some(msg) = palette_blocked_by_game("Apply") {
        return Err(msg);
    }
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".into());
    }
    let st = upk::palette::apply_rich_palette_to_file(
        Path::new(&config.game_dir),
        include_str!("../resources/keys.txt"),
        include_str!("../resources/keys_map.json"),
    )
    .map_err(|e| explain_palette_error(e.to_string()))?;
    let mut state = load_integrity(&app);
    integrity::mark_palette_on(&mut state, &st.fingerprint);
    save_integrity(&app, &state)?;
    let _ = psynet::merge_palette_spoof(true);
    Ok(st)
}

#[tauri::command]
async fn restore_rich_palette(app: tauri::AppHandle) -> Result<upk::PaletteStatus, String> {
    if let Some(msg) = palette_blocked_by_game("Restore") {
        return Err(msg);
    }
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".into());
    }
    let st = upk::palette::restore_palette_backup(Path::new(&config.game_dir))
        .map_err(|e| explain_palette_error(e.to_string()))?;
    let mut state = load_integrity(&app);
    integrity::mark_palette_off(&mut state);
    save_integrity(&app, &state)?;
    let _ = psynet::merge_palette_spoof(false);
    Ok(st)
}

#[tauri::command]
async fn validate_game_dir(path: String) -> Result<String, String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }
    if !p.is_dir() {
        return Err(format!("Path is not a directory: {}", path));
    }
    let cooked = upk::palette::resolve_cooked_dir(p).map_err(|e| e.to_string())?;
    let tagame = cooked.join("TAGame.upk");
    if !tagame.exists() {
        return Err(
            "TAGame.upk not found — select …/TAGame/CookedPCConsole (or the game root; we resolve it)."
                .into(),
        );
    }
    let has_upk = fs::read_dir(&cooked)
        .map_err(|e| e.to_string())?
        .flatten()
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("upk"));
    if !has_upk {
        return Err("No .upk files found — make sure this is the CookedPCConsole folder.".into());
    }
    Ok(cooked.to_string_lossy().into_owned())
}

fn swaps_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("swaps.json"))
}

fn load_swaps(app: &tauri::AppHandle) -> Vec<SwapEntry> {
    swaps_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_swaps(app: &tauri::AppHandle, swaps: &[SwapEntry]) {
    if let Some(path) = swaps_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(swaps) {
            let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = fs::write(path, json);
        }
    }
}

#[tauri::command]
async fn get_swaps(app: tauri::AppHandle) -> Result<Vec<SwapEntry>, String> {
    Ok(load_swaps(&app))
}

#[tauri::command]
async fn delete_swap(app: tauri::AppHandle, owned_id: i32) -> Result<(), String> {
    let mut swaps = load_swaps(&app);
    swaps.retain(|s| s.owned_id != owned_id);
    save_swaps(&app, &swaps);
    Ok(())
}

fn run_swap_caught(
    owned_id: &str,
    wanted_id: &str,
    paint_id: i32,
    opts: &upk::SwapOptions,
) -> Result<String, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        upk::swap_asset(owned_id, wanted_id, paint_id, opts)
    })) {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(explain_upk_lock(e.to_string(), "the UPK")),
        Err(_) => Err(
            "Swap failed unexpectedly. If a .bak exists, restore it from the Restore tab — the app did not crash."
                .into(),
        ),
    }
}

#[tauri::command]
async fn apply_swap(
    app: tauri::AppHandle,
    owned_id: String,
    wanted_id: String,
    paint_id: Option<i32>,
) -> Result<String, String> {
    if !features::is_build_supported() {
        return Err("VelocityRL build is outdated. Please update to the latest version.".into());
    }
    let mut config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        if let Ok(installs) = detect_game_dir().await {
            if let Some(first) = installs.first() {
                let _ = save_config(app.clone(), Config {
                    game_dir: first.path.clone(),
                    ..config.clone()
                }).await;
                config.game_dir = first.path.clone();
                applog::event(&format!("apply_swap: auto-configured game_dir to '{}'", config.game_dir));
            }
        }
    }
    if config.game_dir.is_empty() {
        return Err("Game directory not set. Open Settings and select your Rocket League CookedPCConsole folder.".to_string());
    }
    let mut paint_id = paint_id.unwrap_or(0);
    if !(0..=12).contains(&paint_id) {
        return Err(format!("invalid paint id {paint_id} (use 0 for None, or 1–12)"));
    }

    let all_items = get_items(app.clone()).await
        .map_err(|e| format!("Failed to load items database: {}", e))?;
    if paint_id > 0 {
        if let Ok(wid) = wanted_id.parse::<i32>() {
            if let Some(wanted) = all_items.iter().find(|i| i.id == wid) {
                if !item_is_paintable(wanted) {
                    paint_id = 0;
                }
            }
        }
    }

    let items_json = load_raw_items_json(&app)?;

    let game_dir = match upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)) {
        Ok(dir) => dir,
        Err(_) => {
            let p = PathBuf::from(&config.game_dir);
            if !p.exists() || !p.join("TAGame.upk").exists() {
                return Err(format!(
                    "Game directory not valid: '{}'. Please open Settings and select your Rocket League CookedPCConsole folder.",
                    config.game_dir
                ));
            }
            p
        }
    };

    applog::event(&format!(
        "apply_swap: starting owned_id={} wanted_id={} paint_id={} game_dir='{}'",
        owned_id, wanted_id, paint_id, game_dir.display()
    ));

    let opts = upk::SwapOptions {
        game_dir: game_dir.clone(),
        items_json,
        keys_txt: include_str!("../resources/keys.txt").to_string(),
        keys_map_json: include_str!("../resources/keys_map.json").to_string(),
    };
    let result = match run_swap_caught(&owned_id, &wanted_id, paint_id, &opts) {
        Ok(res) => {
            applog::event(&format!("apply_swap: succeeded for owned_id={} wanted_id={}", owned_id, wanted_id));
            res
        }
        Err(err) => {
            applog::event(&format!("apply_swap: failed for owned_id={} wanted_id={}: {}", owned_id, wanted_id, err));
            return Err(err);
        }
    };

    let oid: i32 = owned_id.parse().unwrap_or(0);
    let wid: i32 = wanted_id.parse().unwrap_or(0);
    let owned = all_items.iter().find(|i| i.id == oid);
    let owned_name = owned.map(|i| i.product.clone()).unwrap_or_default();
    let wanted_name = all_items.iter().find(|i| i.id == wid).map(|i| i.product.clone()).unwrap_or_default();
    let mut swaps = load_swaps(&app);
    swaps.retain(|s| s.owned_id != oid);
    swaps.push(SwapEntry {
        owned_id: oid,
        wanted_id: wid,
        owned_name,
        wanted_name,
        paint_id,
        asset_package: owned
            .map(|i| i.asset_package.clone())
            .unwrap_or_default(),
    });
    record_swap_history(&app, "swap", swaps.last().map(|s| std::slice::from_ref(s)).unwrap_or(&[]), "");
    save_swaps(&app, &swaps);

    if let Some(pkg) = owned.map(|i| i.asset_package.as_str()).filter(|p| !p.is_empty()) {
        let fp = integrity::upk_fingerprint(&opts.game_dir.join(pkg));
        let mut state = load_integrity(&app);
        integrity::mark_swap_package(&mut state, pkg, fp.as_deref());
        let _ = save_integrity(&app, &state);
    }

    Ok(result)
}

#[tauri::command]
async fn restore_single_backup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not configured".into());
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| upk::restore_single(&path))) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(explain_upk_lock(e.to_string(), "the UPK")),
        Err(_) => return Err("Restore failed unexpectedly. Close Rocket League and try again.".into()),
    }

    let stem = std::path::Path::new(&path)
        .file_name().unwrap_or_default().to_string_lossy()
        .to_lowercase().replace(".upk.bak","").replace(".upk","");
    let items = get_items(app.clone()).await.unwrap_or_default();
    if let Some(item) = items.iter().find(|i| i.asset_package.to_lowercase().replace(".upk","") == stem) {
        let mut swaps = load_swaps(&app);
        swaps.retain(|s| s.owned_id != item.id);
        record_swap_history(&app, "restore", &[], &format!("restored {}", item.product));
        save_swaps(&app, &swaps);
        let mut state = load_integrity(&app);
        integrity::clear_swap_package(&mut state, &item.asset_package);
        let _ = save_integrity(&app, &state);
    }
    Ok(())
}

#[tauri::command]
async fn restore_backups(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    let count = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        upk::restore_all(&config.game_dir)
    })) {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => return Err(explain_upk_lock(e.to_string(), "a UPK")),
        Err(_) => return Err("Restore-all failed unexpectedly.".into()),
    };
    save_swaps(&app, &[]);
    let mut state = load_integrity(&app);
    state.swap_packages.clear();
    state.swap_fingerprints.clear();
    let _ = save_integrity(&app, &state);
    Ok(format!("Restored {} backups", count))
}

fn build_swap_opts(game_dir: PathBuf, items_json: String) -> upk::SwapOptions {
    upk::SwapOptions {
        game_dir,
        items_json,
        keys_txt: include_str!("../resources/keys.txt").to_string(),
        keys_map_json: include_str!("../resources/keys_map.json").to_string(),
    }
}

#[tauri::command]
async fn reswap_all(app: tauri::AppHandle) -> Result<String, String> {
    if !features::is_build_supported() {
        return Err("VelocityRL build is outdated. Please update to the latest version.".into());
    }
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    let swaps = load_swaps(&app);
    if swaps.is_empty() {
        return Err(
            "No recorded swaps to re-apply. Swap items again from the Swapper tab.".into(),
        );
    }
    let _ = get_items(app.clone()).await;
    let items_json = load_raw_items_json(&app)?;
    let game_dir = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));
    let items = get_items(app.clone()).await.unwrap_or_default();
    let opts = build_swap_opts(game_dir.clone(), items_json);

    let mut ok = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for s in &swaps {
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
            let bak = integrity::bak_path_for(&game_dir.join(&pkg));
            if bak.exists() {
                if let Some(bak_s) = bak.to_str() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        upk::restore_single(bak_s)
                    }));
                }
            }
        }
        let paint = if (0..=12).contains(&s.paint_id) { s.paint_id } else { 0 };
        match run_swap_caught(&s.owned_id.to_string(), &s.wanted_id.to_string(), paint, &opts) {
            Ok(_) => {
                ok += 1;
                if !pkg.is_empty() {
                    let fp = integrity::upk_fingerprint(&game_dir.join(&pkg));
                    let mut state = load_integrity(&app);
                    integrity::mark_swap_package(&mut state, &pkg, fp.as_deref());
                    let _ = save_integrity(&app, &state);
                }
            }
            Err(e) => {
                let name = if s.owned_name.is_empty() {
                    format!("#{}", s.owned_id)
                } else {
                    s.owned_name.clone()
                };
                errors.push(format!("{name}: {e}"));
            }
        }
    }

    if ok == 0 {
        return Err(if errors.is_empty() {
            "Reswap did not apply any swaps.".into()
        } else {
            errors.join("\n")
        });
    }
    if errors.is_empty() {
        Ok(format!("Re-applied {ok} swap(s). Restart Rocket League to see them."))
    } else {
        Ok(format!(
            "Re-applied {ok} swap(s); {} failed: {}",
            errors.len(),
            errors.join("; ")
        ))
    }
}

#[tauri::command]
fn copy_to_clipboard(text: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::io::Write;
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "clip"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("clipboard failed: {e}"))?;
        if let Some(stdin) = child.stdin.as_mut() {

            let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            stdin.write_all(&utf16).map_err(|e| format!("clipboard write failed: {e}"))?;
        }
        let _ = child.wait();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("clipboard is Windows-only".into())
    }
}

#[tauri::command]
fn force_exit(_app: tauri::AppHandle) {
    applog::event("exit: force_exit invoked — killing proxy and exiting process");
    psynet::kill_proxy_on_exit();
    std::process::exit(0);
}

#[tauri::command]
fn open_external_url(url: String) {
    applog::event(&format!("open_external_url: {url}"));
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer.exe").arg(&url).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
}

#[tauri::command]
async fn repair_engine_refs(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    let game_dir = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));
    let cooked = game_dir.clone();

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let engine_path = cooked.join("Engine.upk");
        let tagame_path = cooked.join("TAGame.upk");
        let (eng, _) = upk::parser::parse_prefix(&fs::read(&engine_path).map_err(|e| format!("read Engine.upk: {e}"))?)
            .map_err(|e| format!("parse Engine.upk: {e}"))?;
        let mut tag = fs::read(&tagame_path).map_err(|e| format!("read TAGame.upk: {e}"))?;
        let (sum, _) = upk::parser::parse_prefix(&tag).map_err(|e| format!("parse TAGame.upk: {e}"))?;
        if sum.engine_version == eng.engine_version && sum.cooker_version == eng.cooker_version {
            return Ok("TAGame.upk already matches Engine.upk — nothing to do.".into());
        }
        let old = (sum.engine_version, sum.cooker_version);
        let changed = upk::swapper::patch_engine_versions_in_prefix(&mut tag, &sum, eng.engine_version, eng.cooker_version);
        if !changed {
            return Err("could not patch TAGame.upk version fields".into());
        }
        fs::write(&tagame_path, &tag).map_err(|e| format!("write TAGame.upk: {e}"))?;
        Ok(format!("patched TAGame.upk engine/cooker version {:?} -> {:?}", old, (eng.engine_version, eng.cooker_version)))
    }));
    match res {
        Ok(Ok(msg)) => {
            applog::event(&format!("repair_engine_refs: {msg}"));
            Ok(msg)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Engine reference repair failed unexpectedly.".into()),
    }
}

#[tauri::command]
async fn reset_tagame_for_verify(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    let game_dir = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));
    let tagame = game_dir.join("TAGame.upk");
    let bak = integrity::bak_path_for(&tagame);
    if !bak.exists() {
        return Err("No TAGame.upk backup found — nothing to reset.".into());
    }
    let bak_s = bak.to_string_lossy().into_owned();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        upk::restore_single(&bak_s)
    })) {
        Ok(Ok(())) => {

            let mut swaps = load_swaps(&app);
            swaps.clear();
            save_swaps(&app, &swaps);
            applog::event("reset_tagame_for_verify: TAGame.upk restored from backup");
            Ok("TAGame.upk restored. Now verify game files in Epic/Steam.".into())
        }
        Ok(Err(e)) => Err(explain_upk_lock(e.to_string(), "TAGame.upk")),
        Err(_) => Err("Reset failed unexpectedly. Close Rocket League and try again.".into()),
    }
}

#[tauri::command]
async fn sync_palette_psynet_config(app: tauri::AppHandle) -> Result<(), String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Ok(());
    }
    let actual_st = upk::palette::read_palette_status(Path::new(&config.game_dir), None);
    let applied = actual_st.applied;
    let mut state = load_integrity(&app);
    if state.palette_active != applied {
        state.palette_active = applied;
        if applied {
            state.palette_fingerprint = actual_st.fingerprint;
        } else {
            state.palette_fingerprint.clear();
        }
        let _ = save_integrity(&app, &state);
    }
    let enabled = psynet::merge_palette_spoof(applied).is_ok();
    if enabled {
        applog::event(&format!("psynet: palette_spoof synced -> {applied}"));
    }
    Ok(())
}

fn user_rl_logs_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let profile = std::env::var("USERPROFILE").ok()?;
        let p = PathBuf::from(profile);
        let candidates = [
            p.join("Documents").join("My Games").join("Rocket League").join("TAGame").join("Logs"),
            p.join("OneDrive").join("Documents").join("My Games").join("Rocket League").join("TAGame").join("Logs"),
            p.join("OneDrive").join("Documentos").join("My Games").join("Rocket League").join("TAGame").join("Logs"),
            p.join("Documentos").join("My Games").join("Rocket League").join("TAGame").join("Logs"),
        ];
        for c in &candidates {
            if c.exists() {
                return Some(c.clone());
            }
        }
    }
    None
}

#[tauri::command]
async fn export_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
    use std::io::Write;
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let logs_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| config_dir.clone());

    let out_dir = config_dir.join("diagnostics");
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let stamp = presets::utc_filename_stamp();
    let zip_path = out_dir.join(format!("velocityrl-diagnostics-{stamp}.zip"));

    let zip_file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let add_file = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, path: &Path| {
        let read_result = fs::read(path).or_else(|_| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(7)
                    .open(path)
                {
                    let mut data = Vec::new();
                    if std::io::Read::read_to_end(&mut file, &mut data).is_ok() {
                        return Ok(data);
                    }
                }
            }
            Err(std::io::Error::new(std::io::ErrorKind::Other, "could not read file"))
        });

        if let Ok(data) = read_result {
            if zip.start_file(name, opts).is_ok() {
                let _ = zip.write_all(&data);
            }
        }
    };

    for f in ["config.json", "swaps.json", "items.ver", "integrity.json"] {
        add_file(&mut zip, f, &config_dir.join(f));
    }
    if let Ok(entries) = fs::read_dir(&logs_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    add_file(&mut zip, &format!("logs/{name}"), &p);
                }
            }
        }
    }

    if let Some(rl_logs) = user_rl_logs_dir() {
        let launch_log = rl_logs.join("Launch.log");
        if launch_log.exists() {
            add_file(&mut zip, "game_logs/Launch.log", &launch_log);
        }
        if let Ok(entries) = fs::read_dir(&rl_logs) {
            let mut backup_logs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map_or(false, |s| s.starts_with("Launch-backup") && s.ends_with(".log"))
                })
                .collect();
            backup_logs.sort_by(|a, b| b.cmp(a));
            for backup in backup_logs.into_iter().take(2) {
                if let Some(name) = backup.file_name().and_then(|n| n.to_str()) {
                    add_file(&mut zip, &format!("game_logs/{name}"), &backup);
                }
            }
        }
    }

    if let Ok(cfg) = psynet::get_psynet_config_json().await {
        if zip.start_file("psynet_config.json", opts).is_ok() {
            let _ = zip.write_all(cfg.as_bytes());
        }
    }
    if let Ok(dir) = psynet::resolve_proxy_dir_for_diag() {
        if zip.start_file("proxy_dir.txt", opts).is_ok() {
            let _ = zip.write_all(dir.to_string_lossy().as_bytes());
        }
        for log_name in ["psynet_proxy.log", "start_from_app.log", "real_skill.json"] {
            let log_file = dir.join(log_name);
            add_file(&mut zip, &format!("proxy/{log_name}"), &log_file);
        }
    }
    if let Ok(info) = applog::get_debug_info(app.clone()) {
        let mut sys = String::new();
        for (k, v) in &info {
            sys.push_str(&format!("{k}: {v}\n"));
        }
        sys.push_str(&format!(
            "app_version: {}\nbuild_number: {}\nbuild_hash: {}\n",
            env!("CARGO_PKG_VERSION"),
            env!("VRL_BUILD_NUMBER"),
            env!("VRL_BUILD_HASH")
        ));
        if zip.start_file("system.txt", opts).is_ok() {
            let _ = zip.write_all(sys.as_bytes());
        }
    }

    let health = psynet::verify_config_psynet_live().await;
    if let Ok(h_json) = serde_json::to_string_pretty(&health) {
        if zip.start_file("config_psynet_health.json", opts).is_ok() {
            let _ = zip.write_all(h_json.as_bytes());
        }
    }

    let _ = zip.finish();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let escaped = zip_path.to_string_lossy().replace('\'', "''");
        let ps_cmd = format!("Set-Clipboard -Path '{escaped}'");
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps_cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    let path_str = zip_path.to_string_lossy().into_owned();
    applog::event(&format!("diagnostics: exported {path_str} and copied to clipboard"));
    Ok(path_str)
}

#[tauri::command]
async fn report_diagnostic(payload: serde_json::Value) -> Result<(), String> {
    applog::event(&format!(
        "frontend diagnostic: event={} context={} message={}",
        payload.get("event").and_then(|v| v.as_str()).unwrap_or(""),
        payload.get("context").and_then(|v| v.as_str()).unwrap_or(""),
        payload.get("message").and_then(|v| v.as_str()).unwrap_or("")
    ));
    send_diagnostic(payload).await;
    Ok(())
}

#[derive(Serialize, Clone)]
struct DetectedInstall {
    label: String,
    path: String,
}

#[tauri::command]
async fn detect_game_dir() -> Result<Vec<DetectedInstall>, String> {
    let mut results: Vec<DetectedInstall> = Vec::new();

    let add_unique = |list: &mut Vec<DetectedInstall>, label: &str, path: String| {
        if !list.iter().any(|e| e.path.eq_ignore_ascii_case(&path)) {
            list.push(DetectedInstall { label: label.to_string(), path });
        }
    };

    #[cfg(target_os = "windows")]
    {
        if let Some((_, pid)) = crate::psynet::rocket_league_process() {
            if let Some(exe_path) = crate::winprobe::process_path(pid) {
                if let Ok(cooked) = upk::palette::resolve_cooked_dir(&exe_path) {
                    add_unique(&mut results, "Running Rocket League", cooked.to_string_lossy().into_owned());
                }
            }
        }
    }

    for drive in ["C", "D", "E", "F", "G", "H", "X", "Z"] {
        let steam_cands = [
            format!(r"{drive}:\Program Files (x86)\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Program Files\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\SteamLibrary\steamapps\common\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Games\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole"),
        ];
        for path in steam_cands {
            let p = std::path::Path::new(&path);
            if p.join("TAGame.upk").exists() {
                add_unique(&mut results, "Steam", path);
            }
        }

        let epic_cands = [
            format!(r"{drive}:\Program Files\Epic Games\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Program Files (x86)\Epic Games\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Epic Games\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\games\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Games\rocketleague\TAGame\CookedPCConsole"),
            format!(r"{drive}:\Games\Epic Games\rocketleague\TAGame\CookedPCConsole"),
        ];
        for path in epic_cands {
            let p = std::path::Path::new(&path);
            if p.join("TAGame.upk").exists() {
                add_unique(&mut results, "Epic Games", path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for subkey in &[
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 252950",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 252950",
        ] {
            if let Ok(key) = hklm.open_subkey(subkey) {
                if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                    let p = std::path::PathBuf::from(loc).join("TAGame").join("CookedPCConsole");
                    if p.exists() {
                        add_unique(&mut results, "Steam", p.to_string_lossy().into_owned());
                    }
                }
            }
        }

        let manifest_dir = std::path::Path::new(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests");
        if manifest_dir.exists() {
            if let Ok(entries) = fs::read_dir(manifest_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("item") { continue; }
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                            let is_rl = json.get("AppName")
                                .and_then(|v| v.as_str())
                                .map_or(false, |s| s.eq_ignore_ascii_case("Sugar"))
                                || json.get("DisplayName")
                                    .and_then(|v| v.as_str())
                                    .map_or(false, |s| s.to_lowercase().contains("rocket league"));
                            if is_rl {
                                if let Some(loc) = json.get("InstallLocation").and_then(|v| v.as_str()) {
                                    let p = std::path::PathBuf::from(loc).join("TAGame").join("CookedPCConsole");
                                    if p.exists() {
                                        add_unique(&mut results, "Epic Games", p.to_string_lossy().into_owned());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

#[derive(Serialize)]
struct BuildInfo {
    version: &'static str,
    build_number: &'static str,
    build_hash: &'static str,
    build_id: i64,
}

#[tauri::command]
fn get_build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        build_number: env!("VRL_BUILD_NUMBER"),
        build_hash: env!("VRL_BUILD_HASH"),
        build_id: features::VRL_BUILD_ID,
    }
}

#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater_builder().build() {
        Ok(u) => u,
        Err(e) => {
            applog::event(&format!("updater: builder failed (ignored): {e}"));
            return Ok(None);
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            applog::event(&format!("updater: update available v{}", update.version));
            Ok(Some(update.version))
        }
        Ok(None) => {
            applog::event("updater: no update available");
            Ok(None)
        }
        Err(e) => {
            applog::event(&format!("updater: check failed (ignored): {e}"));
            Ok(None)
        }
    }
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            applog::event(&format!("updater: downloading v{}", update.version));
            update
                .download_and_install(
                    |_chunk, _total| {},
                    || {},
                )
                .await
                .map_err(|e| {
                    applog::event(&format!("updater: install failed: {e}"));
                    e.to_string()
                })?;

            applog::event("updater: install finished");
            Ok(())
        }
        Ok(None) => {
            applog::event("updater: install skipped (no update)");
            Ok(())
        }
        Err(e) => {
            applog::event(&format!("updater: install check failed: {e}"));
            Err(e.to_string())
        }
    }
}

fn create_main_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .or_else(|| app.config().app.windows.first())
        .cloned()
        .ok_or("missing window config")?;
    let mut last_err: Option<String> = None;
    for attempt in 1u32..=6 {
        match tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
            .focused(true)
            .on_download(workshop::bakkes_download_interceptor())
            .build()
        {
            Ok(win) => {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
                if attempt > 1 {
                    applog::event(&format!("webview created on attempt {attempt}"));
                }
                return Ok(());
            }
            Err(e) => {
                let msg = e.to_string();
                applog::event(&format!("webview create attempt {attempt}/6 failed: {msg}"));
                last_err = Some(msg);
                std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempt)));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "failed to create webview".into()).into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {

    jobobject::init_job_object();

    tauri::Builder::default()

        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            applog::event(&format!("single-instance: argv={:?}", argv));

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .filter(|metadata| {

                    !metadata
                        .target()
                        .starts_with("tokio_tungstenite")
                        && !metadata.target().starts_with("tungstenite")
                        && !metadata.target().starts_with("rustls")
                })
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("velocityrl".into()),
                    }),
                ])
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(psynet::PsyNetState::default())
        .setup(|app| {
            let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
            let dir = applog::init(app.handle());
            create_main_window(app)?;
            let tracker_state = tracker::init(app.handle());
            app.manage(tracker_state);

            let _ = tracker::create_overlay_window(app);

            // Load proxy dir override from config if present
            psynet::load_proxy_dir_override(app.handle());

            // Sync color palette status to psynet proxy so OrangeTeamV2 override is active only when applied
            let mut integrity = load_integrity(app.handle());
            let app_handle = app.handle().clone();
            let applied = if let Ok(config) = tauri::async_runtime::block_on(get_config(app_handle.clone())) {
                if !config.game_dir.is_empty() {
                    let st = upk::palette::read_palette_status(Path::new(&config.game_dir), None);
                    if integrity.palette_active != st.applied {
                        integrity.palette_active = st.applied;
                        if st.applied {
                            integrity.palette_fingerprint = st.fingerprint;
                        } else {
                            integrity.palette_fingerprint.clear();
                        }
                        let _ = save_integrity(&app_handle, &integrity);
                    }
                    st.applied
                } else {
                    false
                }
            } else {
                integrity.palette_active
            };
            let _ = psynet::merge_palette_spoof(applied);

            applog::event(&format!(
                "app setup complete; build {} (v{}, hash {}) logs at {}",
                env!("VRL_BUILD_NUMBER"),
                env!("CARGO_PKG_VERSION"),
                env!("VRL_BUILD_HASH"),
                dir.display()
            ));

            std::thread::spawn(|| {
                if let Ok(()) = ctrlc::set_handler(|| {
                    psynet::kill_proxy_on_exit();
                    std::process::exit(0);
                }) {}
            });
            std::thread::spawn(|| {
                // Bind the native MITM first; only rewrite hosts after :443 is healthy.
                // Orphan config.psynet.gg → 127.0.0.1 is what testers see as EOS/online failure.
                tauri::async_runtime::spawn(async {
                    let _ = psynet::clear_rocket_league_cache();
                    if crate::proxy::is_proxy_running() {
                        applog::event("psynet: proxy already running at boot");
                        return;
                    }
                    #[cfg(windows)]
                    if let Some(pid) = crate::winprobe::loopback_443_owner() {
                        if pid != std::process::id() {
                            let name = crate::winprobe::process_name(pid).unwrap_or_else(|| "unknown".to_string());
                            if name.eq_ignore_ascii_case("velocity-rl.exe")
                                || name.eq_ignore_ascii_case("velocityrl.exe")
                                || name.eq_ignore_ascii_case("psynet_proxy.exe")
                                || name.eq_ignore_ascii_case("mitmproxy.exe")
                            {
                                applog::event(&format!(
                                    "psynet: boot terminating stale process ({name}, PID {pid}) on :443"
                                ));
                                crate::winprobe::terminate_process(pid);
                                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                            }
                        }
                    }
                    if let Some(cfg) = psynet::load_active_spoof_from_disk() {
                        crate::proxy::set_spoof_config(cfg).await;
                    }
                    psynet::ensure_wininet_revocation_disabled();
                    match crate::proxy::start_native_proxy().await {
                        Ok(()) => applog::event("psynet: native proxy auto-started on port 443"),
                        Err(e) => {
                            applog::event(&format!("psynet: proxy auto-start failed: {e}"));
                            let _ = psynet::revert_config_hosts();
                            return;
                        }
                    }
                    match crate::proxy::start_ws_broker().await {
                        Ok(port) => applog::event(&format!(
                            "psynet: WS broker auto-started on 127.0.0.1:{port}"
                        )),
                        Err(e) => {
                            // PsyNetUrl rewrite targets the ephemeral broker — without it,
                            // in-game Auth/WS fail while config MITM still looks fine in a browser.
                            applog::event(&format!(
                                "psynet: WS broker auto-start failed — stopping proxy: {e}"
                            ));
                            crate::proxy::stop_native_proxy(true);
                            let _ = psynet::revert_config_hosts();
                            return;
                        }
                    }
                    if let Err(e) = crate::proxy::verify_proxy_loopback_health().await {
                        applog::event(&format!(
                            "psynet: boot loopback health probe failed — stopping proxy: {e}"
                        ));
                        crate::proxy::stop_native_proxy(true);
                        let _ = psynet::revert_config_hosts();
                        return;
                    }
                    match psynet::ensure_config_hosts() {
                        Ok(true) => {
                            applog::event("psynet: boot hosts already set (config.psynet.gg)")
                        }
                        Ok(false) => applog::event("psynet: boot hosts added config.psynet.gg"),
                        Err(e) => {
                            applog::event(&format!(
                                "psynet: boot hosts failed after listen — stopping proxy: {e}"
                            ));
                            crate::proxy::stop_native_proxy(true);
                            let _ = psynet::revert_config_hosts();
                            return;
                        }
                    }
                    let health = psynet::verify_config_psynet_live().await;
                    if !health.ok {
                        applog::event(&format!(
                            "psynet: boot config.psynet.gg verification warning: {}",
                            health.details
                        ));
                    }
                });
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        applog::event("exit: main window close requested — terminating application");
                        psynet::kill_proxy_on_exit();
                        std::process::exit(0);
                    } else if window.label() == "tracker_overlay" {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    if window.label() == "main" {
                        psynet::kill_proxy_on_exit();
                        std::process::exit(0);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_items,
            get_config,
            save_config,
            get_launch_on_startup,
            set_launch_on_startup,
            get_backups,
            apply_swap,
            reswap_all,
            restore_backups,
            restore_single_backup,
            check_integrity,
            acknowledge_repair,
            get_palette_status,
            apply_rich_palette,
            restore_rich_palette,
            report_diagnostic,
            check_for_updates,
            get_build_info,
            install_update,
            get_swaps,
            delete_swap,
            presets::get_presets,
            presets::save_preset,
            presets::delete_preset,
            presets::apply_preset,
            presets::export_preset_code,
            presets::import_preset_code,
            presets::peek_preset_code,
            presets::preset_download_missing_maps,
            presets::random_swap_plan,
            presets::apply_swap_plan,
            presets::get_swap_history,
            presets::clear_swap_history,
            detect_game_dir,
            validate_game_dir,
            applog::append_launch_log,
            applog::set_app_locale_logs,
            applog::get_logs_dir,
            applog::get_log_tail,
            applog::open_log_folder,
            psynet::save_psynet_spoof,
            psynet::get_psynet_spoof,
            psynet::get_psynet_status,
            psynet::check_config_psynet,
            psynet::ensure_psynet_hosts,
            psynet::start_psynet_proxy,
            psynet::stop_psynet_proxy,
            psynet::restart_psynet_proxy,
            psynet::is_rocket_league_running,
            psynet::clear_rocket_league_cache,
            psynet::get_psynet_config_json,
            psynet::save_psynet_config_json,
            psynet::get_proxy_dir_override,
            psynet::save_proxy_dir,
            psynet::delete_ca_certificates,
            export_diagnostics,
            copy_to_clipboard,
            force_exit,
            open_external_url,
            repair_engine_refs,
            reset_tagame_for_verify,
            sync_palette_psynet_config,
            features::get_features,
            workshop::workshop_get_auth,
            workshop::workshop_search_maps,
            workshop::workshop_fetch_bakkes_maps,
            workshop::workshop_fetch_bakkes_versions,
            workshop::workshop_get_installed,
            workshop::workshop_install_custom_map,
            workshop::workshop_import_bakkes_zip,
            workshop::workshop_install_map_from_url,
            workshop::workshop_get_map_library,
            workshop::workshop_install_from_library,
            workshop::workshop_remove_from_library,
            workshop::workshop_get_map_presets,
            workshop::workshop_save_map_preset,
            workshop::workshop_apply_map_preset,
            workshop::workshop_delete_map_preset,
            workshop::workshop_install_map,
            workshop::workshop_restore,
            workshop::workshop_set_map_thumbnail,
            tracker::get_overlay_state,
            tracker::get_connection_status,
            tracker::reset_session,
            tracker::update_player_skill,
            tracker::tracker_set_playlist,
            tracker::set_player_identity,
            tracker::set_overlay_config,
            tracker::set_click_through,
            tracker::test_simulate_match,
            tracker::tracker_open_overlay_window,
            tracker::tracker_close_overlay_window,
            tracker::tracker_set_overlay_locked,
            tracker::tracker_position_overlay,
            tracker::tracker_start_dragging,
            tracker::tracker_move_overlay_by,
            tracker::tracker_center_overlay,
            tracker::tracker_load_session,
            tracker::tracker_save_session,
            tracker::tracker_ensure_stats_api,
            tracker::tracker_set_overlay_size,
            tracker::tracker_apply_scale_opacity,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            applog::on_run_event(app_handle, &event);
        });
}
