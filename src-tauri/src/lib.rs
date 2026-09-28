use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

mod applog;
mod integrity;
mod jobobject;
mod presets;
pub mod psynet;
pub mod upk;
pub mod workshop;
pub mod tracker;
mod winprobe;
pub mod proxy;
pub mod features;


#[allow(dead_code)]
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
    #[serde(default)]
    changelog_on_startup: bool,

    #[serde(default)]
    launch_on_startup: bool,
    #[serde(default)]
    minimize_to_tray: bool,
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
    #[serde(alias = "name", alias = "Product", alias = "label", alias = "long_label")]
    product: String,
    #[serde(default, alias = "src", alias = "thumbnail", alias = "image_url")]
    image_url: String,
    #[serde(default, alias = "AssetPackage", alias = "asset_package")]
    asset_package: String,
    #[serde(default, alias = "AssetPath", alias = "asset_path")]
    asset_path: String,
    #[serde(default, alias = "ObjectName", alias = "object_name")]
    object_name: Option<String>,
    #[serde(default, alias = "ObjectClass", alias = "object_class")]
    object_class: Option<String>,
    #[serde(default, alias = "IsMultiAssetPackage", alias = "is_multi_asset_package")]
    is_multi_asset_package: Option<bool>,
    #[serde(default, alias = "PackageItemCount", alias = "package_item_count")]
    package_item_count: Option<usize>,
    #[serde(default, alias = "CompatibleBodyId", alias = "compatible_body_id")]
    compatible_body_id: Option<i64>,
    #[serde(default, alias = "CompatibleBodyName", alias = "compatible_body_name")]
    compatible_body_name: Option<String>,
    #[serde(default, alias = "Type", alias = "Slot", alias = "slot")]
    slot: String,
    #[serde(default, alias = "Quality", alias = "quality")]
    quality: String,
    #[serde(default, alias = "Paintable", alias = "paintable", deserialize_with = "opt_paintable")]
    #[serde(skip_serializing_if = "Option::is_none")]
    paintable: Option<bool>,
    #[serde(default, alias = "Attributes", alias = "attributes")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attributes: Vec<ItemAttribute>,

    #[serde(default, alias = "DLC", alias = "dlc")]
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

#[allow(dead_code)]
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
    let slot = item.slot.trim().to_lowercase().replace([' ', '_', '-'], "");
    let unpaintable_slots = [
        "playeranthem", "anthem", "audio", "engineaudio",
        "playertitle", "title",
        "crate", "blueprint", "currency", "drop",
    ];
    if unpaintable_slots.contains(&slot.as_str()) {
        return false;
    }
    true
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ItemsResponse {
    Database {
        #[serde(alias = "Items", alias = "items")]
        items: Vec<Item>,
        #[serde(default)]
        meta: Option<serde_json::Value>,
        #[serde(default)]
        categories: Option<serde_json::Value>,
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
    swap_from_paint: String,
    #[serde(default)]
    swap_to: String,
    #[serde(default)]
    swap_to_image: String,
    #[serde(default)]
    slot: String,
    #[serde(default)]
    paint_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwapEntry {
    pub owned_id:  i32,
    pub wanted_id: i32,
    #[serde(default)]
    pub owned_name:  String,
    #[serde(default)]
    pub wanted_name: String,
    #[serde(default)]
    pub owned_paint_id: Option<i32>,
    #[serde(default)]
    pub owned_custom_hex: Option<String>,
    #[serde(default)]
    pub paint_id: i32,
    #[serde(default)]
    pub custom_paint_hex: Option<String>,
    #[serde(default)]
    pub asset_package: String,
    #[serde(default)]
    pub slot: Option<String>,
}

static ITEMS_CACHE: std::sync::RwLock<Option<std::collections::HashMap<u32, Vec<Item>>>> = std::sync::RwLock::new(None);

pub fn rl_lang_id(lang: &str) -> u32 {
    match lang.trim().to_lowercase().as_str() {
        "de" | "deu" | "german" => 1,
        "nl" | "dut" | "dutch" | "nederlands" => 2,
        "es" | "esn" | "spanish" | "espanol" | "español" => 3,
        "fr" | "fra" | "french" | "francais" | "français" => 4,
        "it" | "ita" | "italian" | "italiano" => 5,
        "ja" | "jpn" | "japanese" => 6,
        "ko" | "kor" | "korean" => 7,
        "pl" | "pol" | "polish" | "polski" => 8,
        "pt" | "ptb" | "portuguese" | "portugues" | "português" => 9,
        "ru" | "rus" | "russian" => 10,
        "tr" | "trk" | "turkish" | "turkce" | "türkçe" => 11,
        _ => 0, // "INT" / "en" / English default
    }
}

pub fn items_api_url_for_lang(lang_id: u32) -> String {
    if lang_id == 0 {
        "https://api.velocityrl.tech/items.json".to_string()
    } else {
        format!("https://api.velocityrl.tech/items.json?l={lang_id}")
    }
}

fn get_cached_items(lang_id: u32) -> Option<Vec<Item>> {
    ITEMS_CACHE.read().ok().and_then(|guard| {
        guard.as_ref().and_then(|map| map.get(&lang_id).cloned())
    })
}

fn set_cached_items(lang_id: u32, items: Vec<Item>) {
    if let Ok(mut guard) = ITEMS_CACHE.write() {
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        map.insert(lang_id, items);
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

fn read_catalog_meta(dir: &Path, lang_id: u32) -> CatalogMetadata {
    let filename = if lang_id == 0 {
        "items.meta.json".to_string()
    } else {
        format!("items_{lang_id}.meta.json")
    };
    let path = dir.join(&filename);
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(meta) = serde_json::from_slice::<CatalogMetadata>(&bytes) {
            return meta;
        }
    }
    if lang_id == 0 {
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
    }
    CatalogMetadata::default()
}

fn write_catalog_meta(dir: &Path, meta: &CatalogMetadata, lang_id: u32) {
    let filename = if lang_id == 0 {
        "items.meta.json".to_string()
    } else {
        format!("items_{lang_id}.meta.json")
    };
    let path = dir.join(&filename);
    if let Ok(data) = serde_json::to_vec(meta) {
        let _ = fs::write(&path, data);
    }
    if lang_id == 0 {
        if let Some(etag) = &meta.etag {
            let _ = fs::write(dir.join("items.ver"), etag);
        }
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

#[allow(dead_code)]
fn load_raw_items_json(app: &tauri::AppHandle) -> Result<String, String> {
    let (data_dir, config_dir) = get_catalog_dirs(app);
    let mut candidates = Vec::new();

    if let Ok(cfg_content) = fs::read_to_string(config_dir.as_ref().unwrap_or(&data_dir).join("config.json")) {
        if let Ok(cfg) = serde_json::from_str::<Config>(&cfg_content) {
            let lid = rl_lang_id(&cfg.language);
            if lid != 0 {
                candidates.push(data_dir.join(format!("items_{lid}.json")));
                if let Some(ref cfg_d) = config_dir {
                    candidates.push(cfg_d.join(format!("items_{lid}.json")));
                }
            }
        }
    }

    candidates.push(data_dir.join("items.json"));
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
            ItemsResponse::Database { items, .. } => items,
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
    lang_id: u32,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let _ = fs::create_dir_all(&data_dir);
        let serialized = serde_json::to_vec(&serde_json::json!({ "Items": items }))
            .map_err(|e| format!("JSON serialize error: {e}"))?;

        let file_name = if lang_id == 0 {
            "items.json".to_string()
        } else {
            format!("items_{lang_id}.json")
        };

        let primary_path = data_dir.join(&file_name);
        fs::write(&primary_path, &serialized)
            .map_err(|e| format!("Failed to write {}: {e}", primary_path.display()))?;

        if let Some(ref cfg) = config_dir {
            if cfg != &data_dir {
                let _ = fs::create_dir_all(cfg);
                let _ = fs::write(cfg.join(&file_name), &serialized);
            }
        }

        if let Some(meta) = meta {
            write_catalog_meta(&data_dir, &meta, lang_id);
            if let Some(ref cfg) = config_dir {
                if cfg != &data_dir {
                    write_catalog_meta(cfg, &meta, lang_id);
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

async fn background_check_items_update(data_dir: PathBuf, config_dir: Option<PathBuf>, lang_id: u32) {
    let client = match reqwest::Client::builder()
        .user_agent(app_user_agent())
        .timeout(std::time::Duration::from_secs(6))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let meta = read_catalog_meta(&data_dir, lang_id);
    let api_url = items_api_url_for_lang(lang_id);
    let github_url = "https://raw.githubusercontent.com/CrunchyRL/RLUPKTools/refs/heads/main/items.json";

    let result = match fetch_catalog_update(&client, &api_url, &meta).await {
        Ok(res) => Ok(res),
        Err(e) => {
            log::warn!("Primary catalog update check failed for lang {lang_id} ({e}), trying fallback");
            if lang_id == 0 {
                fetch_catalog_update(&client, github_url, &meta).await
            } else {
                Err(e)
            }
        }
    };

    match result {
        Ok(None) => {
            // 304 Not Modified: cache is current and retained
        }
        Ok(Some((bytes, new_meta))) => {
            if let Ok(items) = parse_items_slice(bytes).await {
                set_cached_items(lang_id, items.clone());
                let _ = persist_catalog(items, data_dir, config_dir, Some(new_meta), lang_id).await;
            }
        }
        Err(e) => {
            log::warn!("Catalog update check failed for lang {lang_id}: {e}");
        }
    }
}

#[tauri::command]
async fn get_items(app: tauri::AppHandle, lang: Option<String>) -> Result<Vec<Item>, String> {
    let effective_lang = if let Some(l) = lang {
        l
    } else if let Ok(cfg) = get_config(app.clone()).await {
        cfg.language
    } else {
        "en".to_string()
    };

    let lang_id = rl_lang_id(&effective_lang);

    if let Some(cached) = get_cached_items(lang_id) {
        return Ok(cached);
    }

    let (data_dir, config_dir) = get_catalog_dirs(&app);
    let file_name = if lang_id == 0 {
        "items.json".to_string()
    } else {
        format!("items_{lang_id}.json")
    };

    let cache_path = data_dir.join(&file_name);

    let candidate_cache = if cache_path.is_file() {
        Some(cache_path.clone())
    } else if let Some(ref cfg) = config_dir {
        let cfg_path = cfg.join(&file_name);
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
                set_cached_items(lang_id, items.clone());

                let d_dir = data_dir.clone();
                let c_dir = config_dir.clone();
                tauri::async_runtime::spawn(async move {
                    background_check_items_update(d_dir, c_dir, lang_id).await;
                });

                return Ok(items);
            }
        }
    }

    if lang_id == 0 {
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
                        set_cached_items(lang_id, items.clone());

                        let d_dir = data_dir.clone();
                        let c_dir = config_dir.clone();
                        let items_for_save = items.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = persist_catalog(items_for_save, d_dir.clone(), c_dir.clone(), None, lang_id).await;
                            background_check_items_update(d_dir, c_dir, lang_id).await;
                        });

                        return Ok(items);
                    }
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .user_agent(app_user_agent())
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;

    let api_url = items_api_url_for_lang(lang_id);
    let github_url = "https://raw.githubusercontent.com/CrunchyRL/RLUPKTools/refs/heads/main/items.json";

    let empty_meta = CatalogMetadata::default();
    let fetch_res = match fetch_catalog_update(&client, &api_url, &empty_meta).await {
        Ok(Some(res)) => Some(res),
        _ => {
            if lang_id == 0 {
                fetch_catalog_update(&client, github_url, &empty_meta).await.ok().flatten()
            } else {
                fetch_catalog_update(&client, "https://api.velocityrl.tech/items.json", &empty_meta).await.ok().flatten()
            }
        }
    };

    if let Some((bytes, meta)) = fetch_res {
        let items = parse_items_slice(bytes).await?;
        set_cached_items(lang_id, items.clone());

        let d_dir = data_dir.clone();
        let c_dir = config_dir.clone();
        let items_for_save = items.clone();
        tauri::async_runtime::spawn(async move {
            let _ = persist_catalog(items_for_save, d_dir, c_dir, Some(meta), lang_id).await;
        });

        return Ok(items);
    }

    if lang_id != 0 {
        if let Some(cached) = get_cached_items(0) {
            return Ok(cached);
        }
        let fallback_en = data_dir.join("items.json");
        if fallback_en.is_file() {
            if let Ok(bytes) = fs::read(&fallback_en) {
                if let Ok(items) = parse_items_slice(bytes).await {
                    return Ok(items);
                }
            }
        }
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
        Ok(Config {
            game_dir: "".to_string(),
            privacy_agreed: false,
            privacy_version: "".to_string(),
            changelog_on_startup: true,
            launch_on_startup: false,
            minimize_to_tray: false,
            language: "en".to_string(),
        })
    }
}

#[cfg(windows)]
const STARTUP_TASK_NAME: &str = "VelocityRL";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
fn scheduled_task_exists() -> bool {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(["/Query", "/TN", STARTUP_TASK_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.status();
    matches!(out, Ok(s) if s.success())
}

#[tauri::command]
async fn get_launch_on_startup() -> Result<bool, String> {
    #[cfg(windows)]
    {
        Ok(scheduled_task_exists())
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let desktop = std::path::Path::new(&home).join(".config/autostart/velocityrl.desktop");
        Ok(desktop.exists())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Ok(false)
    }
}

#[tauri::command]
async fn set_launch_on_startup(enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
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
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let autostart_dir = std::path::Path::new(&home).join(".config/autostart");
        let desktop_file = autostart_dir.join("velocityrl.desktop");

        if enable {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let _ = std::fs::create_dir_all(&autostart_dir);
            let content = format!(
                "[Desktop Entry]\nType=Application\nName=VelocityRL\nExec=\"{}\"\nHidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n",
                exe.display()
            );
            std::fs::write(&desktop_file, content).map_err(|e| e.to_string())?;
        } else if desktop_file.exists() {
            let _ = std::fs::remove_file(&desktop_file);
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("Startup configuration not supported on this platform".into())
    }
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

    let items = get_items(app.clone(), None).await.unwrap_or_default();
    let swaps = load_swaps(&app);
    let mut backups = Vec::new();
    let mut seen_ids = std::collections::HashSet::<i32>::new();

    for swap in &swaps {
        let owned_item = items.iter().find(|i| i.id == swap.owned_id);
        let wanted_item = items.iter().find(|i| i.id == swap.wanted_id);
        let display_name = if !swap.owned_name.is_empty() {
            swap.owned_name.clone()
        } else {
            owned_item.map(|i| i.product.clone()).unwrap_or_else(|| format!("Item #{}", swap.owned_id))
        };
        let image_url = owned_item.map(|i| i.image_url.clone()).unwrap_or_default();
        let swap_from = display_name.clone();
        let swap_to = if !swap.wanted_name.is_empty() {
            swap.wanted_name.clone()
        } else {
            wanted_item.map(|i| i.product.clone()).unwrap_or_else(|| format!("Item #{}", swap.wanted_id))
        };
        let swap_to_image = wanted_item.map(|i| i.image_url.clone()).unwrap_or_default();

        let slot = wanted_item.or(owned_item).map(|i| i.slot.clone()).unwrap_or_default();
        let paint_name = if let Some(hex) = swap.custom_paint_hex.as_ref().filter(|h| !h.trim().is_empty()) {
            hex.clone()
        } else if swap.paint_id > 0 {
            upk::swapper::paint_label(swap.paint_id).to_string()
        } else {
            String::new()
        };

        let from_paint_name = if let Some(hex) = swap.owned_custom_hex.as_ref().filter(|h| !h.trim().is_empty()) {
            hex.clone()
        } else if let Some(pid) = swap.owned_paint_id.filter(|p| *p > 0) {
            upk::swapper::paint_label(pid).to_string()
        } else {
            String::new()
        };

        if seen_ids.insert(swap.owned_id) {
            backups.push(BackupFile {
                name: display_name,
                path: format!("item_{}", swap.owned_id),
                image_url,
                swap_from,
                swap_from_paint: from_paint_name,
                swap_to,
                swap_to_image,
                slot,
                paint_name,
            });
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
    // Compute Engine.upk fingerprint now so we can detect future RL updates.
    let cooked = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rl_fp = integrity::rl_update_fingerprint_for(Path::new(&cooked));
    integrity::mark_palette_on_with_rl(&mut state, &st.fingerprint, &rl_fp);
    save_integrity(&app, &state)?;
    let _ = psynet::merge_palette_spoof(true);

    // Re-apply any active loadout swaps so palette and item swaps coexist perfectly
    let swaps = load_swaps(&app);
    if let Ok(cooked_path) = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)) {
        let _ = sync_all_swaps_to_tagame(&app, &cooked_path, &swaps).await;
    }

    Ok(st)
}

#[tauri::command]
async fn restore_rich_palette(app: tauri::AppHandle) -> Result<upk::PaletteStatus, String> {
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

    // Re-apply any active loadout swaps after restoring palette
    let swaps = load_swaps(&app);
    if let Ok(cooked_path) = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)) {
        let _ = sync_all_swaps_to_tagame(&app, &cooked_path, &swaps).await;
    }

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
    let list: Vec<SwapEntry> = swaps_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for entry in list.into_iter().rev() {
        if seen.insert(entry.owned_id) {
            deduped.push(entry);
        }
    }
    deduped.reverse();
    deduped
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

pub(crate) async fn sync_all_swaps_to_tagame(
    app: &tauri::AppHandle,
    cooked: &Path,
    swaps: &[SwapEntry],
) -> Result<(), String> {
    if swaps.is_empty() {
        let _ = upk::tagame_swapper::restore_tagame_upk(cooked);
        return Ok(());
    }

    let items = get_items(app.clone(), None).await.unwrap_or_default();
    let mut tagame_items = Vec::new();

    for s in swaps {
        let owned_item = items.iter().find(|i| i.id == s.owned_id);
        let wanted_item = items.iter().find(|i| i.id == s.wanted_id);

        let slot_str = s.slot.clone()
            .or_else(|| owned_item.map(|i| i.slot.clone()))
            .or_else(|| wanted_item.map(|i| i.slot.clone()))
            .unwrap_or_else(|| "Body".to_string());

        let slot_index = presets::slot_index_from_str(&slot_str) as i32;

        let pkg = wanted_item
            .map(|w| w.asset_package.clone())
            .unwrap_or_else(|| s.asset_package.clone());

        let product_id = match s.wanted_id {
            999902 => 2526,
            _ => s.wanted_id,
        };

        tagame_items.push(upk::TagameSwapItem {
            slot: slot_str,
            slot_index: Some(slot_index),
            owned_id: Some(s.owned_id),
            product_id,
            paint_id: if s.paint_id > 0 { Some(s.paint_id) } else { None },
            custom_paint_hex: s.custom_paint_hex.clone(),
            package_name: Some(pkg),
        });
    }

    let keys_txt = include_str!("../resources/keys.txt");
    let keys_map_json = include_str!("../resources/keys_map.json");

    upk::tagame_swapper::apply_tagame_modifications(
        cooked,
        &tagame_items,
        keys_txt,
        keys_map_json,
    ).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn delete_swap(app: tauri::AppHandle, owned_id: i32) -> Result<(), String> {
    let mut swaps = load_swaps(&app);
    swaps.retain(|s| s.owned_id != owned_id);
    save_swaps(&app, &swaps);

    if let Ok(config) = get_config(app.clone()).await {
        if !config.game_dir.is_empty() {
            if let Ok(cooked) = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)) {
                let _ = sync_all_swaps_to_tagame(&app, &cooked, &swaps).await;
            }
        }
    }
    Ok(())
}

pub(crate) fn run_swap_caught(
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
    owned_paint_id: Option<i32>,
    owned_custom_hex: Option<String>,
    paint_id: Option<i32>,
    custom_paint_hex: Option<String>,
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
    if !(0..=29).contains(&paint_id) {
        return Err(format!("invalid paint id {paint_id} (use 0 for None/Match, or 1–29)"));
    }

    let all_items = get_items(app.clone(), None).await
        .map_err(|e| format!("Failed to load items database: {}", e))?;

    let oid: i32 = owned_id.parse().unwrap_or(0);
    let wid: i32 = wanted_id.parse().unwrap_or(0);
    let owned = all_items.iter().find(|i| i.id == oid)
        .ok_or_else(|| format!("Owned item ID {owned_id} not found in database"))?;
    let wanted = all_items.iter().find(|i| i.id == wid)
        .ok_or_else(|| format!("Target item ID {wanted_id} not found in database"))?;

    if paint_id > 0 && !item_is_paintable(wanted) {
        paint_id = 0;
    }

    let cooked = match upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)) {
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

    let clean_hex = custom_paint_hex.filter(|h| !h.trim().is_empty());
    let clean_owned_hex = owned_custom_hex.filter(|h| !h.trim().is_empty());

    applog::event(&format!(
        "apply_swap: starting owned_id={} wanted_id={} paint_id={} custom_hex={:?} cooked='{}'",
        owned_id, wanted_id, paint_id, clean_hex, cooked.display()
    ));

    let mut swaps = load_swaps(&app);
    swaps.retain(|s| s.owned_id != oid);
    let new_entry = SwapEntry {
        owned_id: oid,
        wanted_id: wid,
        owned_name: owned.product.clone(),
        wanted_name: wanted.product.clone(),
        owned_paint_id,
        owned_custom_hex: clean_owned_hex,
        paint_id,
        custom_paint_hex: clean_hex.clone(),
        asset_package: wanted.asset_package.clone(),
        slot: Some(owned.slot.clone()),
    };
    swaps.push(new_entry.clone());
    save_swaps(&app, &swaps);

    sync_all_swaps_to_tagame(&app, &cooked, &swaps).await?;

    record_swap_history(&app, "swap", std::slice::from_ref(&new_entry), "");

    applog::event(&format!("apply_swap: succeeded for owned_id={} wanted_id={}", owned_id, wanted_id));
    let paint_suffix = if let Some(hex) = clean_hex {
        format!(" ({hex})")
    } else if paint_id > 0 {
        format!(" ({})", upk::swapper::paint_label(paint_id))
    } else {
        String::new()
    };
    Ok(format!("Successfully swapped {} with {}{}", owned.product, wanted.product, paint_suffix))
}

pub(crate) fn build_swap_opts(game_dir: PathBuf, items_json: String) -> upk::SwapOptions {
    upk::SwapOptions {
        game_dir,
        items_json,
        keys_txt: include_str!("../resources/keys.txt").to_string(),
        keys_map_json: include_str!("../resources/keys_map.json").to_string(),
    }
}

#[tauri::command]
async fn restore_single_backup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not configured".into());
    }
    let cooked = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));

    let bak_path = if path.ends_with(".bak") {
        PathBuf::from(&path)
    } else {
        PathBuf::from(format!("{path}.bak"))
    };

    let bak_str = bak_path.to_string_lossy().into_owned();
    let _ = upk::swapper::restore_single(&bak_str);

    let mut swaps = load_swaps(&app);
    let orig_len = swaps.len();

    let target_id = path
        .strip_prefix("item_")
        .and_then(|s| s.parse::<i32>().ok());

    if let Some(id) = target_id {
        swaps.retain(|s| s.owned_id != id && s.wanted_id != id);
        record_swap_history(&app, "restore", &[], &format!("restored item #{id}"));
    } else {
        swaps.retain(|s| {
            s.asset_package.to_lowercase() != path.to_lowercase()
                && s.owned_name.to_lowercase() != path.to_lowercase()
        });
    }

    if swaps.len() != orig_len || swaps.is_empty() {
        save_swaps(&app, &swaps);
        if swaps.is_empty() {
            let _ = upk::tagame_swapper::restore_tagame_upk(&cooked);
        } else {
            let _ = sync_all_swaps_to_tagame(&app, &cooked, &swaps).await;
        }
    }

    applog::event(&format!("restore_single_backup: restored {path}"));
    Ok(())
}

#[tauri::command]
async fn restore_backups(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    let cooked = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));

    let _ = upk::tagame_swapper::restore_tagame_upk(&cooked);
    let _ = upk::swapper::restore_all(&cooked.to_string_lossy());

    let count = load_swaps(&app).len();
    save_swaps(&app, &[]);
    record_swap_history(&app, "restore_all", &[], "restored all active swaps");

    let mut state = load_integrity(&app);
    state.swap_packages.clear();
    state.swap_fingerprints.clear();
    let _ = save_integrity(&app, &state);

    applog::event(&format!("restore_backups: restored {count} swaps"));
    Ok(format!("Restored {} item(s) to default", count))
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
        return Err("No recorded swaps to re-apply. Swap items again from the Swapper tab.".into());
    }
    let cooked = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));

    sync_all_swaps_to_tagame(&app, &cooked, &swaps).await?;
    Ok(format!("Synchronized {} active swap(s)", swaps.len()))
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

#[tauri::command]
async fn apply_tagame_swaps(
    app: tauri::AppHandle,
    swaps: Vec<upk::TagameSwapItem>,
) -> Result<upk::TagameSwapperStatus, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set. Open Settings and select CookedPCConsole folder.".into());
    }
    let cooked = upk::tagame_swapper::resolve_cooked_dir(Path::new(&config.game_dir))
        .map_err(|e| e.to_string())?;

    let keys_txt = include_str!("../resources/keys.txt");
    let keys_map_json = include_str!("../resources/keys_map.json");

    let status = upk::tagame_swapper::apply_tagame_modifications(
        &cooked,
        &swaps,
        keys_txt,
        keys_map_json,
    ).map_err(|e| e.to_string())?;

    applog::event(&format!(
        "tagame_swapper: applied {} swaps",
        swaps.len()
    ));
    Ok(status)
}

#[tauri::command]
async fn restore_tagame_swaps(app: tauri::AppHandle) -> Result<upk::TagameSwapperStatus, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set. Open Settings and select CookedPCConsole folder.".into());
    }
    let cooked = upk::tagame_swapper::resolve_cooked_dir(Path::new(&config.game_dir))
        .map_err(|e| e.to_string())?;

    let status = upk::tagame_swapper::restore_tagame_upk(&cooked)
        .map_err(|e| e.to_string())?;

    applog::event("tagame_swapper: restored TAGame.upk");
    Ok(status)
}

#[tauri::command]
async fn get_tagame_swapper_status(app: tauri::AppHandle) -> Result<upk::TagameSwapperStatus, String> {
    let config = get_config(app.clone()).await?;
    let tagame_path = if !config.game_dir.is_empty() {
        if let Ok(cooked) = upk::tagame_swapper::resolve_cooked_dir(Path::new(&config.game_dir)) {
            cooked.join("TAGame.upk").to_string_lossy().into_owned()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let backup_present = if !tagame_path.is_empty() {
        let p = Path::new(&tagame_path);
        p.parent().map(|d| d.join("TAGame.upk.bak").is_file()).unwrap_or(false)
    } else {
        false
    };

    Ok(upk::TagameSwapperStatus {
        applied: false,
        backup_present,
        tagame_path,
        active_swaps: Vec::new(),
        message: "Ready".to_string(),
    })
}

#[tauri::command]
async fn get_detected_car_body(app: tauri::AppHandle) -> Result<upk::DetectedCarInfo, String> {
    let swaps = load_swaps(&app);
    Ok(upk::decal_compiler::detect_active_car(&swaps))
}

#[tauri::command]
async fn get_custom_decal_config(app: tauri::AppHandle) -> Result<upk::CustomDecalConfig, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let path = config_dir.join("custom_decal.json");
    if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str::<upk::CustomDecalConfig>(&text).map_err(|e| e.to_string())
    } else {
        let swaps = load_swaps(&app);
        let car = upk::decal_compiler::detect_active_car(&swaps);
        Ok(upk::CustomDecalConfig {
            enabled: false,
            decal_name: None,
            car_id: Some(car.car_id),
            car_name: Some(car.car_name),
            auto_detect: true,
            diffuse_path: None,
            skin_path: None,
            roughness_path: None,
            metallic_path: None,
            normal_path: None,
            preview_base64: None,
        })
    }
}

#[tauri::command]
async fn save_custom_decal_config(app: tauri::AppHandle, config: upk::CustomDecalConfig) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let path = config_dir.join("custom_decal.json");
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    applog::event(&format!("custom_decal: saved config (enabled={}, car={:?})", config.enabled, config.car_name));
    Ok(())
}

#[tauri::command]
async fn import_decal_json_path(app: tauri::AppHandle, json_file_path: String) -> Result<upk::ParsedDecalPackage, String> {
    let p = Path::new(&json_file_path);
    if !p.is_file() {
        return Err(format!("File does not exist: {json_file_path}"));
    }
    let parent = p.parent();
    let content = fs::read_to_string(p).map_err(|e| format!("Failed to read decal JSON: {e}"))?;
    let parsed = upk::decal_compiler::parse_decal_json(&content, parent)?;

    let (data_dir, _) = get_catalog_dirs(&app);
    let decal_cache = data_dir.join("cache").join("decals");
    fs::create_dir_all(&decal_cache).map_err(|e| e.to_string())?;

    let mut diffuse_dds_path = None;
    let mut skin_dds_path = None;

    if let Some(ref d_path) = parsed.diffuse_path {
        if let Ok(bytes) = fs::read(d_path) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_diffuse.dds", parsed.package_name));
                if fs::write(&out_dds, &dds).is_ok() {
                    diffuse_dds_path = Some(out_dds.to_string_lossy().into_owned());
                }
            }
        }
    }

    if let Some(ref s_path) = parsed.skin_path {
        if let Ok(bytes) = fs::read(s_path) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_skin.dds", parsed.package_name));
                if fs::write(&out_dds, &dds).is_ok() {
                    skin_dds_path = Some(out_dds.to_string_lossy().into_owned());
                }
            }
        }
    }

    let cfg = upk::CustomDecalConfig {
        enabled: true,
        decal_name: Some(parsed.decal_name.clone()),
        car_id: Some(parsed.car_id),
        car_name: Some(parsed.car_name.clone()),
        auto_detect: false,
        diffuse_path: diffuse_dds_path.or_else(|| parsed.diffuse_path.clone()),
        skin_path: skin_dds_path.or_else(|| parsed.skin_path.clone()),
        roughness_path: None,
        metallic_path: None,
        normal_path: parsed.normal_path.clone(),
        preview_base64: parsed.preview_base64.clone(),
    };
    save_custom_decal_config(app.clone(), cfg).await?;

    applog::event(&format!("custom_decal: imported package '{}' for car {}", parsed.decal_name, parsed.car_name));
    Ok(parsed)
}

#[tauri::command]
async fn apply_custom_decal(
    app: tauri::AppHandle,
    diffuse_base64: Option<String>,
    skin_base64: Option<String>,
    roughness_base64: Option<String>,
    metallic_base64: Option<String>,
    _normal_base64: Option<String>,
) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".into());
    }

    let swaps = load_swaps(&app);
    let detected_car = upk::decal_compiler::detect_active_car(&swaps);

    let (data_dir, _) = get_catalog_dirs(&app);
    let decal_cache = data_dir.join("cache").join("decals");
    fs::create_dir_all(&decal_cache).map_err(|e| e.to_string())?;

    let mut preview_b64 = None;
    let mut diff_path = None;
    let mut s_path = None;

    if let Some(ref diff_b64) = diffuse_base64 {
        let clean = if let Some(idx) = diff_b64.find(',') { &diff_b64[idx + 1..] } else { diff_b64 };
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim())
            .map_err(|e| format!("Diffuse decode error: {e}"))?;
        let img = image::load_from_memory(&bytes).map_err(|e| format!("Failed to load diffuse image: {e}"))?;
        let rgba = img.to_rgba8();
        let dds = upk::decal_compiler::encode_to_dxt5_dds(&rgba);
        let out_dds = decal_cache.join(format!("{}_diffuse.dds", detected_car.package_name));
        fs::write(&out_dds, &dds).map_err(|e| e.to_string())?;
        diff_path = Some(out_dds.to_string_lossy().into_owned());
        preview_b64 = Some(format!("data:image/png;base64,{}", clean.trim()));
    }

    if let Some(ref skin_b64) = skin_base64 {
        let clean = if let Some(idx) = skin_b64.find(',') { &skin_b64[idx + 1..] } else { skin_b64 };
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim())
            .map_err(|e| format!("Skin decode error: {e}"))?;
        let img = image::load_from_memory(&bytes).map_err(|e| format!("Failed to load skin image: {e}"))?;
        let rgba = img.to_rgba8();
        let dds = upk::decal_compiler::encode_to_dxt5_dds(&rgba);
        let out_dds = decal_cache.join(format!("{}_skin.dds", detected_car.package_name));
        fs::write(&out_dds, &dds).map_err(|e| e.to_string())?;
        s_path = Some(out_dds.to_string_lossy().into_owned());
        if preview_b64.is_none() {
            preview_b64 = Some(format!("data:image/png;base64,{}", clean.trim()));
        }
    }

    let r_bytes = roughness_base64.and_then(|b| {
        let clean = if let Some(idx) = b.find(',') { &b[idx + 1..] } else { &b };
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim()).ok()
    });
    let m_bytes = metallic_base64.and_then(|b| {
        let clean = if let Some(idx) = b.find(',') { &b[idx + 1..] } else { &b };
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim()).ok()
    });

    if r_bytes.is_some() || m_bytes.is_some() {
        let packed_mask = upk::decal_compiler::pack_metallic_roughness_mask(
            r_bytes.as_deref(),
            m_bytes.as_deref(),
            2048,
            2048,
        )?;
        let mask_dds = upk::decal_compiler::encode_to_dxt5_dds(&packed_mask);
        let out_mask = decal_cache.join(format!("{}_mask.dds", detected_car.package_name));
        fs::write(&out_mask, &mask_dds).map_err(|e| e.to_string())?;
    }

    let decal_cfg = upk::CustomDecalConfig {
        enabled: true,
        decal_name: Some("Custom Decal".to_string()),
        car_id: Some(detected_car.car_id),
        car_name: Some(detected_car.car_name.clone()),
        auto_detect: true,
        diffuse_path: diff_path,
        skin_path: s_path,
        roughness_path: None,
        metallic_path: None,
        normal_path: None,
        preview_base64: preview_b64,
    };
    save_custom_decal_config(app.clone(), decal_cfg).await?;

    applog::event(&format!("custom_decal: compiled and configured custom decal for {}", detected_car.car_name));
    Ok(format!("Custom decal compiled and applied for {}", detected_car.car_name))
}

#[tauri::command]
async fn import_decal_zip_path(app: tauri::AppHandle, zip_file_path: String) -> Result<upk::ParsedDecalPackage, String> {
    let p = Path::new(&zip_file_path);
    if !p.is_file() {
        return Err(format!("File does not exist: {zip_file_path}"));
    }
    let (data_dir, _) = get_catalog_dirs(&app);
    let extract_dir = data_dir.join("cache").join("decals").join(format!("pkg_{}", presets::utc_filename_stamp()));
    let bytes = fs::read(p).map_err(|e| format!("Failed to read ZIP file: {e}"))?;
    let parsed = upk::decal_compiler::extract_and_parse_decal_zip(&bytes, &extract_dir)?;

    let decal_cache = data_dir.join("cache").join("decals");
    fs::create_dir_all(&decal_cache).map_err(|e| e.to_string())?;

    let mut diffuse_dds_path = None;
    let mut skin_dds_path = None;

    if let Some(ref d_path) = parsed.diffuse_path {
        if let Ok(b) = fs::read(d_path) {
            if let Ok(img) = image::load_from_memory(&b) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_diffuse.dds", parsed.package_name));
                if fs::write(&out_dds, &dds).is_ok() {
                    diffuse_dds_path = Some(out_dds.to_string_lossy().into_owned());
                }
            }
        }
    }

    if let Some(ref s_path) = parsed.skin_path {
        if let Ok(b) = fs::read(s_path) {
            if let Ok(img) = image::load_from_memory(&b) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_skin.dds", parsed.package_name));
                if fs::write(&out_dds, &dds).is_ok() {
                    skin_dds_path = Some(out_dds.to_string_lossy().into_owned());
                }
            }
        }
    }

    let cfg = upk::CustomDecalConfig {
        enabled: true,
        decal_name: Some(parsed.decal_name.clone()),
        car_id: Some(parsed.car_id),
        car_name: Some(parsed.car_name.clone()),
        auto_detect: false,
        diffuse_path: diffuse_dds_path.or_else(|| parsed.diffuse_path.clone()),
        skin_path: skin_dds_path.or_else(|| parsed.skin_path.clone()),
        roughness_path: None,
        metallic_path: None,
        normal_path: parsed.normal_path.clone(),
        preview_base64: parsed.preview_base64.clone(),
    };
    save_custom_decal_config(app.clone(), cfg).await?;

    applog::event(&format!("custom_decal: imported ZIP package '{}' for car {}", parsed.decal_name, parsed.car_name));
    Ok(parsed)
}

#[tauri::command]
async fn upload_decal_package_bytes(
    app: tauri::AppHandle,
    filename: String,
    base64_data: String,
) -> Result<upk::ParsedDecalPackage, String> {
    let raw_bytes = if let Some(idx) = base64_data.find(',') {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &base64_data[idx + 1..])
            .map_err(|e| format!("Base64 decode error: {e}"))?
    } else {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &base64_data)
            .map_err(|e| format!("Base64 decode error: {e}"))?
    };

    let (data_dir, _) = get_catalog_dirs(&app);
    let extract_dir = data_dir.join("cache").join("decals").join(format!("pkg_{}", presets::utc_filename_stamp()));
    fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;

    let is_zip = filename.to_lowercase().ends_with(".zip");
    let parsed = if is_zip {
        upk::decal_compiler::extract_and_parse_decal_zip(&raw_bytes, &extract_dir)?
    } else {
        let json_str = std::str::from_utf8(&raw_bytes).map_err(|e| format!("Invalid JSON text encoding: {e}"))?;
        upk::decal_compiler::parse_decal_json(json_str, Some(&extract_dir))?
    };

    let decal_cache = data_dir.join("cache").join("decals");
    fs::create_dir_all(&decal_cache).map_err(|e| e.to_string())?;

    let mut diffuse_dds_path = None;
    let mut skin_dds_path = None;

    if let Some(ref d_path) = parsed.diffuse_path {
        if let Ok(b) = fs::read(d_path) {
            if let Ok(img) = image::load_from_memory(&b) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_diffuse.dds", parsed.package_name));
                if fs::write(&out_dds, &dds).is_ok() {
                    diffuse_dds_path = Some(out_dds.to_string_lossy().into_owned());
                }
            }
        }
    }

    if let Some(ref s_path) = parsed.skin_path {
        if let Ok(b) = fs::read(s_path) {
            if let Ok(img) = image::load_from_memory(&b) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_skin.dds", parsed.package_name));
                if fs::write(&out_dds, &dds).is_ok() {
                    skin_dds_path = Some(out_dds.to_string_lossy().into_owned());
                }
            }
        }
    }

    let cfg = upk::CustomDecalConfig {
        enabled: true,
        decal_name: Some(parsed.decal_name.clone()),
        car_id: Some(parsed.car_id),
        car_name: Some(parsed.car_name.clone()),
        auto_detect: false,
        diffuse_path: diffuse_dds_path.or_else(|| parsed.diffuse_path.clone()),
        skin_path: skin_dds_path.or_else(|| parsed.skin_path.clone()),
        roughness_path: None,
        metallic_path: None,
        normal_path: parsed.normal_path.clone(),
        preview_base64: parsed.preview_base64.clone(),
    };
    save_custom_decal_config(app.clone(), cfg).await?;

    applog::event(&format!("custom_decal: uploaded and parsed package '{}' for car {}", parsed.decal_name, parsed.car_name));
    Ok(parsed)
}

#[tauri::command]
async fn swap_custom_decal_to_donor(
    app: tauri::AppHandle,
    donor_item_id: i32,
    decal_name: String,
    diffuse_base64: Option<String>,
    skin_base64: Option<String>,
    roughness_base64: Option<String>,
    metallic_base64: Option<String>,
    _normal_base64: Option<String>,
) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set. Open Settings and select CookedPCConsole folder.".into());
    }
    let cooked = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));

    let all_items = get_items(app.clone(), None).await
        .map_err(|e| format!("Failed to load items database: {e}"))?;
    let donor = all_items.iter().find(|i| i.id == donor_item_id)
        .ok_or_else(|| format!("Donor decal ID {donor_item_id} not found"))?;

    let (data_dir, _) = get_catalog_dirs(&app);
    let decal_cache = data_dir.join("cache").join("decals");
    fs::create_dir_all(&decal_cache).map_err(|e| e.to_string())?;

    let mut preview_b64 = None;

    let current_cfg = get_custom_decal_config(app.clone()).await.ok();
    if diffuse_base64.is_none() && skin_base64.is_none() {
        if let Some(ref cfg) = current_cfg {
            if let Some(ref d_path) = cfg.diffuse_path {
                let donor_diff_dds = decal_cache.join(format!("{}_diffuse.dds", donor.asset_package));
                let _ = fs::copy(d_path, &donor_diff_dds);
            }
            if let Some(ref s_path) = cfg.skin_path {
                let donor_skin_dds = decal_cache.join(format!("{}_skin.dds", donor.asset_package));
                let _ = fs::copy(s_path, &donor_skin_dds);
            }
            if let Some(ref prev) = cfg.preview_base64 {
                preview_b64 = Some(prev.clone());
            }
        }
    }

    if let Some(ref diff_b64) = diffuse_base64 {
        let clean = if let Some(idx) = diff_b64.find(',') { &diff_b64[idx + 1..] } else { diff_b64 };
        if let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim()) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_diffuse.dds", donor.asset_package));
                if fs::write(&out_dds, &dds).is_ok() {
                    preview_b64 = Some(format!("data:image/png;base64,{}", clean.trim()));
                }
            }
        }
    }

    if let Some(ref skin_b64) = skin_base64 {
        let clean = if let Some(idx) = skin_b64.find(',') { &skin_b64[idx + 1..] } else { skin_b64 };
        if let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim()) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let dds = upk::decal_compiler::encode_to_dxt5_dds(&img.to_rgba8());
                let out_dds = decal_cache.join(format!("{}_skin.dds", donor.asset_package));
                let _ = fs::write(&out_dds, &dds);
                if preview_b64.is_none() {
                    preview_b64 = Some(format!("data:image/png;base64,{}", clean.trim()));
                }
            }
        }
    }

    let r_bytes = roughness_base64.and_then(|b| {
        let clean = if let Some(idx) = b.find(',') { &b[idx + 1..] } else { &b };
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim()).ok()
    });
    let m_bytes = metallic_base64.and_then(|b| {
        let clean = if let Some(idx) = b.find(',') { &b[idx + 1..] } else { &b };
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, clean.trim()).ok()
    });

    if r_bytes.is_some() || m_bytes.is_some() {
        if let Ok(packed_mask) = upk::decal_compiler::pack_metallic_roughness_mask(
            r_bytes.as_deref(),
            m_bytes.as_deref(),
            2048,
            2048,
        ) {
            let mask_dds = upk::decal_compiler::encode_to_dxt5_dds(&packed_mask);
            let out_mask = decal_cache.join(format!("{}_mask.dds", donor.asset_package));
            let _ = fs::write(&out_mask, &mask_dds);
        }
    }

    let mut swaps = load_swaps(&app);
    swaps.retain(|s| s.owned_id != donor_item_id);
    let new_entry = SwapEntry {
        owned_id: donor_item_id,
        wanted_id: 990301,
        owned_name: donor.product.clone(),
        wanted_name: decal_name.clone(),
        owned_paint_id: None,
        owned_custom_hex: None,
        paint_id: 0,
        custom_paint_hex: None,
        asset_package: donor.asset_package.clone(),
        slot: Some("Decal".to_string()),
    };
    swaps.push(new_entry.clone());
    save_swaps(&app, &swaps);

    let decal_cfg = upk::CustomDecalConfig {
        enabled: true,
        decal_name: Some(decal_name.clone()),
        car_id: Some(donor.id),
        car_name: Some(donor.product.clone()),
        auto_detect: false,
        diffuse_path: Some(decal_cache.join(format!("{}_diffuse.dds", donor.asset_package)).to_string_lossy().into_owned()),
        skin_path: Some(decal_cache.join(format!("{}_skin.dds", donor.asset_package)).to_string_lossy().into_owned()),
        roughness_path: None,
        metallic_path: None,
        normal_path: None,
        preview_base64: preview_b64,
    };
    let _ = save_custom_decal_config(app.clone(), decal_cfg.clone()).await;

    // Normalization to <CookedPCConsole>/decals/ and write decals.ini
    let cooked_decals_dir = cooked.join("decals");
    let _ = fs::create_dir_all(&cooked_decals_dir);

    if let Some(ref d_path) = decal_cfg.diffuse_path {
        let p = Path::new(d_path);
        if let Some(fname) = p.file_name() {
            let target = cooked_decals_dir.join(fname);
            let _ = fs::copy(p, &target);
        }
    }

    if let Some(ref s_path) = decal_cfg.skin_path {
        let p = Path::new(s_path);
        if let Some(fname) = p.file_name() {
            let target = cooked_decals_dir.join(fname);
            let _ = fs::copy(p, &target);
        }
    }

    let _ = sync_all_swaps_to_tagame(&app, &cooked, &swaps).await;
    record_swap_history(&app, "custom_decal_swap", std::slice::from_ref(&new_entry), "");

    applog::event(&format!("swap_custom_decal_to_donor: swapped '{}' -> '{}'", donor.product, decal_name));
    Ok(format!("Swapped {} with {} successfully! Restart Rocket League to see it.", donor.product, decal_name))
}

pub fn user_rl_logs_dir() -> Option<PathBuf> {
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
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").ok()?;
        let home_path = Path::new(&home);
        let prefixes = [
            home_path.join("Games/Heroic/Prefixes/Rocket League/drive_c"),
            home_path.join("Games/Heroic/Prefixes/default/Rocket League/drive_c"),
            home_path.join("Games/Heroic/Prefixes/rocketleague/drive_c"),
            home_path.join("Games/Heroic/Prefixes/rocketleague/pfx/drive_c"),
            home_path.join(".var/app/com.heroicgameslauncher.hgl/Prefixes/Rocket League/drive_c"),
            home_path.join(".local/share/Steam/steamapps/compatdata/252950/pfx/drive_c"),
            home_path.join(".steam/steam/steamapps/compatdata/252950/pfx/drive_c"),
            home_path.join(".steam/root/steamapps/compatdata/252950/pfx/drive_c"),
            home_path.join(".var/app/com.valvesoftware.Steam/data/Steam/steamapps/compatdata/252950/pfx/drive_c"),
            home_path.join("Games/rocketleague/drive_c"),
            home_path.join("Games/rocket-league/drive_c"),
            home_path.join("Games/epic-games-store/drive_c"),
            home_path.join(".wine/drive_c"),
        ];
        for pfx in &prefixes {
            let users_dir = pfx.join("users");
            if let Ok(entries) = std::fs::read_dir(&users_dir) {
                for u in entries.flatten() {
                    let log_cand = u.path().join("Documents/My Games/Rocket League/TAGame/Logs");
                    if log_cand.exists() {
                        return Some(log_cand);
                    }
                }
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

    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            let home_path = std::path::Path::new(&home);

            let steam_roots = [
                home_path.join(".local/share/Steam"),
                home_path.join(".steam/steam"),
                home_path.join(".steam/root"),
                home_path.join(".var/app/com.valvesoftware.Steam/data/Steam"),
            ];

            let mut library_paths = Vec::new();
            for sr in &steam_roots {
                library_paths.push(sr.join("steamapps"));
                let vdf_path = sr.join("steamapps/libraryfolders.vdf");
                if let Ok(vdf) = std::fs::read_to_string(&vdf_path) {
                    for line in vdf.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("\"path\"") {
                            if let Some(p) = trimmed.split('"').nth(3) {
                                library_paths.push(std::path::PathBuf::from(p).join("steamapps"));
                            }
                        }
                    }
                }
            }

            for sa in &library_paths {
                let cooked = sa.join("common/rocketleague/TAGame/CookedPCConsole");
                if cooked.join("TAGame.upk").exists() {
                    add_unique(&mut results, "Steam", cooked.to_string_lossy().into_owned());
                }
            }

            let heroic_cands = [
                home_path.join("Games/Heroic/rocketleague/TAGame/CookedPCConsole"),
                home_path.join("Games/rocketleague/TAGame/CookedPCConsole"),
                home_path.join(".var/app/com.heroicgameslauncher.hgl/Games/rocketleague/TAGame/CookedPCConsole"),
            ];
            for cand in &heroic_cands {
                if cand.join("TAGame.upk").exists() {
                    add_unique(&mut results, "Heroic Games Launcher", cand.to_string_lossy().into_owned());
                }
            }

            let lutris_cands = [
                home_path.join("Games/rocketleague/drive_c/Program Files/Epic Games/rocketleague/TAGame/CookedPCConsole"),
                home_path.join("Games/epic-games-store/drive_c/Program Files/Epic Games/rocketleague/TAGame/CookedPCConsole"),
            ];
            for cand in &lutris_cands {
                if cand.join("TAGame.upk").exists() {
                    add_unique(&mut results, "Lutris (Epic Games)", cand.to_string_lossy().into_owned());
                }
            }

            let bottles_dir = home_path.join(".var/app/com.usebottles.bottles/data/bottles/bottles");
            if let Ok(entries) = std::fs::read_dir(&bottles_dir) {
                for entry in entries.flatten() {
                    let cand = entry.path().join("drive_c/Program Files/Epic Games/rocketleague/TAGame/CookedPCConsole");
                    if cand.join("TAGame.upk").exists() {
                        add_unique(&mut results, "Bottles", cand.to_string_lossy().into_owned());
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
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    jobobject::init_job_object();

    tauri::Builder::default()

        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            applog::event(&format!("single-instance: argv={:?}", argv));

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();

                use tauri::Emitter;
                for arg in &argv {
                    if arg.starts_with("velocityrl://") || arg.contains("velocityrl://") || arg.contains("steam_id=") {
                        let _ = win.emit("steam-auth-callback", arg.clone());
                    }
                }
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
            let (_data_dir, _) = get_catalog_dirs(app.handle());
            psynet::ensure_wininet_revocation_disabled();
            #[cfg(windows)]
            let _ = psynet::revert_config_hosts();
            psynet::set_system_proxy_enabled(false);

            // Spawn detached guardian watchdog to guarantee proxy/hosts cleanup even if process is forcefully killed or crashes
            if let Ok(exe) = std::env::current_exe() {
                let pid = std::process::id();
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    let _ = std::process::Command::new(&exe)
                        .args(["--watchdog", &pid.to_string()])
                        .creation_flags(CREATE_NO_WINDOW)
                        .spawn();
                }
                #[cfg(not(windows))]
                {
                    let _ = std::process::Command::new(&exe)
                        .args(["--watchdog", &pid.to_string()])
                        .spawn();
                }
            }

            // Register Ctrl-C handler for terminal/signal interrupts
            let _ = ctrlc::set_handler(move || {
                crate::proxy::stop_native_proxy(true);
                psynet::set_system_proxy_enabled(false);
                let _ = psynet::revert_config_hosts();
                std::process::exit(0);
            });

            // Register panic hook for graceful cleanup
            let orig_panic_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                crate::proxy::stop_native_proxy(true);
                psynet::set_system_proxy_enabled(false);
                let _ = psynet::revert_config_hosts();
                orig_panic_hook(info);
            }));

            // Install CA and CRL to the user certificate store immediately (non-blocking, no UAC).
            psynet::install_user_ca_direct();
            // If the system (LocalMachine Root) store is missing the CA, trigger an elevated
            // install now — before the proxy auto-start — so the cert is in place before RL connects.
            // This catches fresh installs where the NSIS hooks ran but the machine cert store
            // was wiped (e.g. by antivirus) before the first launch.
            let ca_ok_at_startup = psynet::is_ca_installed();
            applog::event(&format!(
                "startup: system CA installed={ca_ok_at_startup} hosts={}",
                psynet::config_hosts_complete_pub()
            ));
            #[cfg(windows)]
            if !ca_ok_at_startup {
                applog::event("startup: system CA missing — triggering background install");
                std::thread::spawn(|| {
                    psynet::install_ca_and_crl_elevated();
                });
            }
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
                    // Detect RL game updates: Engine.upk changes on every RL update.
                    // If it changed since we applied the palette, the new game binary
                    // will crash against the old patched TAGame.upk — auto-restore first.
                    if integrity.palette_active && !integrity.rl_update_fingerprint.is_empty() {
                        if let Ok(cooked) = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir)) {
                            let current_rl_fp = integrity::rl_update_fingerprint_for(&cooked);
                            if !current_rl_fp.is_empty() && current_rl_fp != integrity.rl_update_fingerprint {
                                applog::event(&format!(
                                    "startup: RL update detected (Engine.upk changed {} -> {}), auto-restoring palette",
                                    integrity.rl_update_fingerprint, current_rl_fp
                                ));
                                match upk::palette::restore_palette_backup(Path::new(&config.game_dir)) {
                                    Ok(_) => {
                                        integrity::mark_palette_off(&mut integrity);
                                        let _ = save_integrity(&app_handle, &integrity);
                                        applog::event("startup: palette auto-restored after RL update");
                                    }
                                    Err(e) => {
                                        applog::event(&format!("startup: palette auto-restore failed: {e}"));
                                    }
                                }
                            }
                        }
                    }

                    let st = upk::palette::read_palette_status(Path::new(&config.game_dir), None);
                    if integrity.palette_active != st.applied {
                        integrity.palette_active = st.applied;
                        if st.applied {
                            integrity.palette_fingerprint = st.fingerprint;
                        } else {
                            integrity.palette_fingerprint.clear();
                            integrity.rl_update_fingerprint.clear();
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

            // Verify and synchronize TAGame.ini and TAGame.upk hooks on startup if swaps exist
            let app_h = app.handle().clone();
            std::thread::spawn(move || {
                let swaps = load_swaps(&app_h);
                if !swaps.is_empty() {
                    if let Ok(config) = tauri::async_runtime::block_on(get_config(app_h.clone())) {
                        if !config.game_dir.is_empty() {
                            if let Ok(cooked) = upk::tagame_swapper::resolve_cooked_dir(Path::new(&config.game_dir)) {
                                let _ = tauri::async_runtime::block_on(sync_all_swaps_to_tagame(&app_h, &cooked, &swaps));
                                applog::event("startup: verified and synced active swaps to TAGame.ini and TAGame.upk");
                            }
                        }
                    }
                }
            });

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
                    if let Some(pid) = crate::winprobe::loopback_443_owner() {
                        if pid != std::process::id() {
                            let name = crate::winprobe::process_name(pid).unwrap_or_else(|| "unknown".to_string());
                            if name.eq_ignore_ascii_case("velocity-rl.exe")
                                || name.eq_ignore_ascii_case("velocityrl.exe")
                                || name.eq_ignore_ascii_case("velocity-rl")
                                || name.eq_ignore_ascii_case("velocityrl")
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
                    psynet::clean_system_proxy();
                    let active_cfg = psynet::load_active_spoof_from_disk();
                    if let Some(ref cfg) = active_cfg {
                        crate::proxy::set_spoof_config(cfg.clone()).await;
                    }
                    psynet::ensure_wininet_revocation_disabled();
                    psynet::install_user_ca_direct();
                    match crate::proxy::start_native_proxy().await {
                        Ok(()) => {
                            applog::event("psynet: native proxy auto-started on port 443");
                            if let Some(cfg) = &active_cfg {
                                if cfg.name_spoof.as_ref().map(|n| n.enabled).unwrap_or(false) {
                                    psynet::set_system_proxy_enabled(true);
                                }
                            }
                        }
                        Err(e) => {
                            applog::event(&format!("psynet: proxy auto-start failed: {e}"));
                            psynet::clean_system_proxy();
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
                    }
                });
            });

            let tray_icon = app_handle
                .default_window_icon()
                .cloned()
                .or_else(|| {
                    Some(tauri::include_image!("icons/32x32.png"))
                });

            if let Some(icon) = tray_icon {
                if let (Ok(show_item), Ok(quit_item)) = (
                    tauri::menu::MenuItemBuilder::with_id("show", "Show VelocityRL").build(&app_handle),
                    tauri::menu::MenuItemBuilder::with_id("quit", "Exit VelocityRL").build(&app_handle),
                ) {
                    if let Ok(menu) = tauri::menu::MenuBuilder::new(&app_handle).items(&[&show_item, &quit_item]).build() {
                        let _ = tauri::tray::TrayIconBuilder::new()
                            .icon(icon)
                            .tooltip("VelocityRL")
                            .menu(&menu)
                            .show_menu_on_left_click(false)
                            .on_menu_event(|app, event| {
                                match event.id().as_ref() {
                                    "show" => {
                                        if let Some(w) = app.get_webview_window("main") {
                                            let _ = w.show();
                                            let _ = w.unminimize();
                                            let _ = w.set_focus();
                                        }
                                    }
                                    "quit" => {
                                        psynet::kill_proxy_on_exit();
                                        std::process::exit(0);
                                    }
                                    _ => {}
                                }
                            })
                            .on_tray_icon_event(|tray, event| {
                                if let tauri::tray::TrayIconEvent::Click {
                                    button: tauri::tray::MouseButton::Left,
                                    button_state: tauri::tray::MouseButtonState::Up,
                                    ..
                                } = event {
                                    let app = tray.app_handle();
                                    if let Some(w) = app.get_webview_window("main") {
                                        let _ = w.show();
                                        let _ = w.unminimize();
                                        let _ = w.set_focus();
                                    }
                                }
                            })
                            .build(&app_handle);
                    }
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        let app_h = window.app_handle().clone();
                        let min_to_tray = tauri::async_runtime::block_on(async move {
                            get_config(app_h).await.map(|c| c.minimize_to_tray).unwrap_or(false)
                        });
                        if min_to_tray {
                            api.prevent_close();
                            let _ = window.hide();
                        } else {
                            applog::event("exit: main window close requested — terminating application");
                            psynet::kill_proxy_on_exit();
                            std::process::exit(0);
                        }
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
            psynet::get_learned_identity,
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
            reset_tagame_for_verify,
            sync_palette_psynet_config,
            apply_tagame_swaps,
            restore_tagame_swaps,
            get_tagame_swapper_status,
            get_detected_car_body,
            get_custom_decal_config,
            save_custom_decal_config,
            import_decal_json_path,
            import_decal_zip_path,
            upload_decal_package_bytes,
            swap_custom_decal_to_donor,
            apply_custom_decal,
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
            match event {
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                    crate::proxy::stop_native_proxy(true);
                    psynet::set_system_proxy_enabled(false);
                    let _ = psynet::revert_config_hosts();
                }
                _ => {}
            }
        });
}
