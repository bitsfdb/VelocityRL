use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

mod upk;

// ── Types ─────────────────────────────────────────────────────────────────────

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
}

static ITEMS_CACHE: std::sync::OnceLock<Vec<Item>> = std::sync::OnceLock::new();

const DIAGNOSTIC_URL: Option<&str> = option_env!("DIAGNOSTIC_URL");
const DIAGNOSTIC_SECRET: Option<&str> = option_env!("DIAGNOSTIC_SECRET");

// ── Diagnostics ───────────────────────────────────────────────────────────────

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

// ── Commands ──────────────────────────────────────────────────────────────────

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

    let api_url = "https://api.velocityrl.tech/items.json";
    let github_url = "https://raw.githubusercontent.com/CrunchyRL/RLUPKTools/refs/heads/main/items.json";

    let mut fetched_content = None;
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

    if let Some(content) = fetched_content {
        if let Ok(resp) = serde_json::from_str::<ItemsResponse>(&content) {
            let items = match resp {
                ItemsResponse::Database { items } => items,
                ItemsResponse::List(items) => items,
            };
            fs::create_dir_all(&config_dir).ok();
            fs::write(&cache_path, &content).ok();
            let _ = ITEMS_CACHE.set(items.clone());
            return Ok(items);
        }
    }

    if cache_path.exists() {
        if let Ok(content) = fs::read_to_string(&cache_path) {
            if let Ok(resp) = serde_json::from_str::<ItemsResponse>(&content) {
                let items = match resp {
                    ItemsResponse::Database { items } => items,
                    ItemsResponse::List(items) => items,
                };
                let _ = ITEMS_CACHE.set(items.clone());
                return Ok(items);
            }
        }
    }

    // Last resort: use the bundled items.json shipped with the app
    if let Ok(resource_path) = app.path().resource_dir() {
        let bundled = resource_path.join("items.json");
        if bundled.exists() {
            if let Ok(content) = fs::read_to_string(&bundled) {
                if let Ok(resp) = serde_json::from_str::<ItemsResponse>(&content) {
                    let mut items = match resp {
                        ItemsResponse::Database { items } => items,
                        ItemsResponse::List(items) => items,
                    };
                    // Generate thumbnail URLs for items that don't have one
                    const THUMB_BASE: &str = "https://api.velocityrl.tech/thumbnails/";
                    for item in &mut items {
                        if item.image_url.is_empty() && !item.asset_package.is_empty() {
                            let stem = item.asset_package
                                .to_lowercase()
                                .replace("_sf.upk", "")
                                .replace(".upk", "");
                            item.image_url = format!("{}{}_t.png", THUMB_BASE, stem);
                        }
                    }
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

#[tauri::command]
async fn save_config(app: tauri::AppHandle, config: Config) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let config_path = config_dir.join("config.json");
    let content = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    fs::write(config_path, content).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_backups(app: tauri::AppHandle) -> Result<Vec<BackupFile>, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Ok(vec![]); }

    let items = get_items(app.clone()).await.unwrap_or_default();
    let mut backups = Vec::new();
    let dir = PathBuf::from(&config.game_dir);

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

#[tauri::command]
async fn check_integrity() -> Result<bool, String> {
    Ok(true)
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
async fn validate_game_dir(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }
    if !p.is_dir() {
        return Err(format!("Path is not a directory: {}", path));
    }
    let has_upk = fs::read_dir(p)
        .map_err(|e| e.to_string())?
        .flatten()
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("upk"));
    if !has_upk {
        return Err("No .upk files found — make sure this is the CookedPCConsole folder.".into());
    }
    Ok(())
}

// ── swaps.json helpers ────────────────────────────────────────────────────────

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

#[tauri::command]
async fn apply_swap(app: tauri::AppHandle, owned_id: String, wanted_id: String) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    // Fetch items if not cached yet (first launch)
    let all_items = get_items(app.clone()).await
        .map_err(|e| format!("Failed to load items database: {}", e))?;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let items_json = fs::read_to_string(config_dir.join("items.json"))
        .map_err(|_| "Items database missing — check your internet connection and try again.".to_string())?;

    let opts = upk::SwapOptions {
        game_dir: std::path::PathBuf::from(&config.game_dir),
        items_json,
        keys_txt: include_str!("../../python/keys.txt").to_string(),
        keys_map_json: include_str!("../../python/keys_map.json").to_string(),
    };
    let result = upk::swap_asset(&owned_id, &wanted_id, &opts).map_err(|e| e.to_string())?;

    // Save to swaps.json
    let oid: i32 = owned_id.parse().unwrap_or(0);
    let wid: i32 = wanted_id.parse().unwrap_or(0);
    let owned_name  = all_items.iter().find(|i| i.id == oid).map(|i| i.product.clone()).unwrap_or_default();
    let wanted_name = all_items.iter().find(|i| i.id == wid).map(|i| i.product.clone()).unwrap_or_default();
    let mut swaps = load_swaps(&app);
    swaps.retain(|s| s.owned_id != oid); // replace if already exists
    swaps.push(SwapEntry { owned_id: oid, wanted_id: wid, owned_name, wanted_name });
    save_swaps(&app, &swaps);

    Ok(result)
}

#[tauri::command]
async fn restore_single_backup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not configured".into());
    }
    upk::restore_single(&path).map_err(|e| e.to_string())?;
    // Remove from swaps.json — derive owned_id from the filename
    let stem = std::path::Path::new(&path)
        .file_name().unwrap_or_default().to_string_lossy()
        .to_lowercase().replace(".upk.bak","").replace(".upk","");
    let items = get_items(app.clone()).await.unwrap_or_default();
    if let Some(item) = items.iter().find(|i| i.asset_package.to_lowercase().replace(".upk","") == stem) {
        let mut swaps = load_swaps(&app);
        swaps.retain(|s| s.owned_id != item.id);
        save_swaps(&app, &swaps);
    }
    Ok(())
}

#[tauri::command]
async fn restore_backups(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }
    let count = upk::restore_all(&config.game_dir).map_err(|e| e.to_string())?;
    Ok(format!("Restored {} backups", count))
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

        // Steam registry
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

        // Epic Games: scan launcher manifest .item files
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
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(update.version)),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    if let Ok(Some(update)) = updater.check().await {
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_items,
            get_config,
            save_config,
            get_backups,
            apply_swap,
            replace_export,
            set_custom_pfp,
            restore_backups,
            restore_single_backup,
            check_integrity,
            cleanup_temp_files,
            fetch_catalog,
            report_diagnostic,
            check_for_updates,
            install_update,
            get_swaps,
            delete_swap,
            detect_game_dir,
            validate_game_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
