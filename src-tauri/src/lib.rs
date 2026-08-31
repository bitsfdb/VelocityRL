use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

mod applog;
mod integrity;
mod psynet;
pub mod upk;

fn default_true() -> bool { true }

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    game_dir: String,
    #[serde(default)]
    privacy_agreed: bool,
    #[serde(default)]
    privacy_version: String,
    #[serde(default = "default_true")]
    changelog_on_startup: bool,
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

fn norm_item_slot(slot: &str) -> String {
    slot.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect()
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
}

#[derive(Serialize, Deserialize, Clone)]
struct SwapEntry {
    owned_id:  i32,
    wanted_id: i32,
    #[serde(default)]
    owned_name:  String,
    #[serde(default)]
    wanted_name: String,
    #[serde(default)]
    paint_id: i32,
    #[serde(default)]
    asset_package: String,
}

static ITEMS_CACHE: std::sync::OnceLock<Vec<Item>> = std::sync::OnceLock::new();

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

#[tauri::command]
async fn get_items(app: tauri::AppHandle) -> Result<Vec<Item>, String> {
    if let Some(cached) = ITEMS_CACHE.get() {
        return Ok(cached.clone());
    }

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let cache_path = config_dir.join("items.json");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let ver_url = "https://api.velocityrl.tech/items.ver";
    let api_url = "https://api.velocityrl.tech/items.json";
    let github_url = "https://raw.githubusercontent.com/CrunchyRL/RLUPKTools/refs/heads/main/items.json";

    let mut should_download = true;
    let cache_ver_path = config_dir.join("items.ver");

    // Check version if local cache exists
    if cache_path.exists() {
        if let Ok(resp) = client.get(ver_url).send().await {
            if let Ok(remote_ver) = resp.text().await {
                let remote_ver = remote_ver.trim();
                if let Ok(local_ver) = fs::read_to_string(&cache_ver_path) {
                    if local_ver.trim() == remote_ver {
                        should_download = false;
                    }
                }
                if should_download {
                    fs::write(&cache_ver_path, remote_ver).ok();
                }
            }
        }
    }

    let mut fetched_content = None;
    if should_download {
        if let Ok(resp) = client.get(api_url).send().await {
            if let Ok(text) = resp.text().await {
                fetched_content = Some(text);
            }
        }

        if fetched_content.is_none() {
            if let Ok(resp) = client.get(github_url).send().await {
                if let Ok(text) = resp.text().await {
                    fetched_content = Some(text);
                }
            }
        }
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

    if let Some(content) = fetched_content {
        if let Ok(resp) = serde_json::from_str::<ItemsResponse>(&content) {
            let mut items = match resp {
                ItemsResponse::Database { items } => items,
                ItemsResponse::List(items) => items,
            };
            populate_thumbnails(&mut items);
            fs::create_dir_all(&config_dir).ok();
            let serialized = serde_json::to_string(&serde_json::json!({"Items": items})).unwrap_or_default();
            fs::write(&cache_path, &serialized).ok();
            let _ = ITEMS_CACHE.set(items.clone());
            return Ok(items);
        }
    }

    if cache_path.exists() {
        if let Ok(content) = fs::read_to_string(&cache_path) {
            if let Ok(resp) = serde_json::from_str::<ItemsResponse>(&content) {
                let mut items = match resp {
                    ItemsResponse::Database { items } => items,
                    ItemsResponse::List(items) => items,
                };
                populate_thumbnails(&mut items);
                let _ = ITEMS_CACHE.set(items.clone());
                return Ok(items);
            }
        }
    }

    if let Ok(resource_path) = app.path().resource_dir() {
        let bundled = resource_path.join("items.json");
        if bundled.exists() {
            if let Ok(content) = fs::read_to_string(&bundled) {
                if let Ok(resp) = serde_json::from_str::<ItemsResponse>(&content) {
                    let mut items = match resp {
                        ItemsResponse::Database { items } => items,
                        ItemsResponse::List(items) => items,
                    };

                    populate_thumbnails(&mut items);
                    
                    fs::create_dir_all(&config_dir).ok();
                    let serialized = serde_json::to_string(&serde_json::json!({"Items": items})).unwrap_or_default();
                    fs::write(&cache_path, &serialized).ok();
                    let _ = ITEMS_CACHE.set(items.clone());
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
        Ok(Config { game_dir: "".to_string(), privacy_agreed: false, privacy_version: "".to_string(), changelog_on_startup: true })
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

    let items = get_items(app.clone()).await.unwrap_or_default();
    let mut backups = Vec::new();
    let dir = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .map_err(|e| e.to_string())?;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.ends_with(".upk.bak"))
            {
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

                backups.push(BackupFile {
                    name: display_name,
                    path: path.to_string_lossy().to_string(),
                    image_url,
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
    Ok(upk::palette::status(Path::new(&config.game_dir), fp))
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
    let st = upk::palette::apply(
        Path::new(&config.game_dir),
        include_str!("../../python/keys.txt"),
        include_str!("../../python/keys_map.json"),
    )
    .map_err(|e| explain_palette_error(e.to_string()))?;
    let mut state = load_integrity(&app);
    integrity::mark_palette_on(&mut state, &st.fingerprint);
    save_integrity(&app, &state)?;
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
    let st = upk::palette::restore(Path::new(&config.game_dir))
        .map_err(|e| explain_palette_error(e.to_string()))?;
    let mut state = load_integrity(&app);
    integrity::mark_palette_off(&mut state);
    save_integrity(&app, &state)?;
    Ok(st)
}

#[tauri::command]
async fn cleanup_temp_files(_app: tauri::AppHandle) -> Result<String, String> {
    Ok("OK".to_string())
}

#[tauri::command]
async fn fetch_catalog(_app: tauri::AppHandle, _token: String, _account: String) -> Result<String, String> {
    Err("Not yet implemented — Rust UPK engine coming soon".to_string())
}

#[tauri::command]
async fn replace_export(
    _app: tauri::AppHandle,
    _target_pkg: String,
    _target_path: String,
    _donor_pkg: String,
    _donor_path: String,
) -> Result<String, String> {
    Err("Not yet implemented — Rust UPK engine coming soon".to_string())
}

#[tauri::command]
async fn set_custom_pfp(_app: tauri::AppHandle, _png_path: String) -> Result<String, String> {
    Err("Not yet implemented — Rust UPK engine coming soon".to_string())
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
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
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

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let items_json = fs::read_to_string(config_dir.join("items.json"))
        .map_err(|_| "Items database missing — check your internet connection and try again.".to_string())?;

    let game_dir = upk::palette::resolve_cooked_dir(Path::new(&config.game_dir))
        .unwrap_or_else(|_| PathBuf::from(&config.game_dir));
    let opts = upk::SwapOptions {
        game_dir,
        items_json,
        keys_txt: include_str!("../../python/keys.txt").to_string(),
        keys_map_json: include_str!("../../python/keys_map.json").to_string(),
    };
    let result = run_swap_caught(&owned_id, &wanted_id, paint_id, &opts)?;

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
        keys_txt: include_str!("../../python/keys.txt").to_string(),
        keys_map_json: include_str!("../../python/keys_map.json").to_string(),
    }
}

#[tauri::command]
async fn reswap_all(app: tauri::AppHandle) -> Result<String, String> {
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
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let items_json = fs::read_to_string(config_dir.join("items.json")).map_err(|_| {
        "Items database missing — check your internet connection and try again.".to_string()
    })?;
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
async fn report_diagnostic(payload: serde_json::Value) -> Result<(), String> {
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

    let steam_candidates = [
        r"C:\Program Files (x86)\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole",
        r"C:\Program Files\Steam\steamapps\common\rocketleague\TAGame\CookedPCConsole",
        r"D:\SteamLibrary\steamapps\common\rocketleague\TAGame\CookedPCConsole",
        r"E:\SteamLibrary\steamapps\common\rocketleague\TAGame\CookedPCConsole",
        r"F:\SteamLibrary\steamapps\common\rocketleague\TAGame\CookedPCConsole",
        r"G:\SteamLibrary\steamapps\common\rocketleague\TAGame\CookedPCConsole",
    ];
    let epic_candidates = [
        r"C:\Program Files\Epic Games\rocketleague\TAGame\CookedPCConsole",
        r"C:\Program Files (x86)\Epic Games\rocketleague\TAGame\CookedPCConsole",
        r"D:\Epic Games\rocketleague\TAGame\CookedPCConsole",
        r"E:\Epic Games\rocketleague\TAGame\CookedPCConsole",
        r"F:\Epic Games\rocketleague\TAGame\CookedPCConsole",
    ];

    let add_unique = |list: &mut Vec<DetectedInstall>, label: &str, path: String| {
        if !list.iter().any(|e| e.path == path) {
            list.push(DetectedInstall { label: label.to_string(), path });
        }
    };

    for path in &steam_candidates {
        if std::path::Path::new(path).exists() {
            add_unique(&mut results, "Steam", path.to_string());
        }
    }
    for path in &epic_candidates {
        if std::path::Path::new(path).exists() {
            add_unique(&mut results, "Epic Games", path.to_string());
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
        .first()
        .cloned()
        .ok_or("missing window config")?;
    let mut last_err: Option<String> = None;
    for attempt in 1u32..=6 {
        match tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
            .focused(false)
            .build()
        {
            Ok(win) => {
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
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("velocityrl".into()),
                    }),
                ])
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(psynet::PsyNetState::default())
        .setup(|app| {
            let dir = applog::init(app.handle());
            create_main_window(app)?;
            applog::event(&format!("app setup complete; logs at {}", dir.display()));
            // Kill proxy on force-close (Ctrl+C / Task Manager / kill)
            std::thread::spawn(|| {
                if let Ok(()) = ctrlc::set_handler(|| {
                    psynet::kill_proxy_on_exit();
                    std::process::exit(0);
                }) {}
            });
            std::thread::spawn(|| {
                match psynet::ensure_config_hosts() {
                    Ok(true) => {
                        applog::event("psynet: boot hosts already set (config.psynet.gg)")
                    }
                    Ok(false) => applog::event("psynet: boot hosts added config.psynet.gg"),
                    Err(e) => applog::event(&format!("psynet: boot hosts failed: {e}")),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_items,
            get_config,
            save_config,
            get_backups,
            apply_swap,
            reswap_all,
            replace_export,
            set_custom_pfp,
            restore_backups,
            restore_single_backup,
            check_integrity,
            acknowledge_repair,
            get_palette_status,
            apply_rich_palette,
            restore_rich_palette,
            cleanup_temp_files,
            fetch_catalog,
            report_diagnostic,
            check_for_updates,
            install_update,
            get_swaps,
            delete_swap,
            detect_game_dir,
            validate_game_dir,
            applog::append_launch_log,
            applog::get_logs_dir,
            psynet::save_psynet_spoof,
            psynet::get_psynet_spoof,
            psynet::get_psynet_status,
            psynet::ensure_psynet_hosts,
            psynet::start_psynet_proxy,
            psynet::stop_psynet_proxy,
            psynet::is_rocket_league_running,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            applog::on_run_event(app_handle, &event);
        });
}
