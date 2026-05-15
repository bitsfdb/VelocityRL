use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;
use sha2::{Sha256, Digest};
use hex;

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    game_dir: String,
    #[serde(default)]
    db_url: String,
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
}

static ITEMS_CACHE: std::sync::OnceLock<Vec<Item>> = std::sync::OnceLock::new();

#[tauri::command]
async fn get_items(app: tauri::AppHandle) -> Result<Vec<Item>, String> {
    if let Some(cached) = ITEMS_CACHE.get() {
        return Ok(cached.clone());
    }

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let cache_path = config_dir.join("items.json");

    let url = "https://velocityrl.me/items.json";

    let client = reqwest::Client::new();
    
    if let Ok(resp) = client.get(url).send().await {
        if let Ok(content) = resp.text().await {
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

    let resource_path = app.path().resolve("python/items.json", tauri::path::BaseDirectory::Resource).ok();
    if let Some(path) = resource_path {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
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
    }

    Err("Failed to parse items database".into())
}

#[tauri::command]
async fn get_config(app: tauri::AppHandle) -> Result<Config, String> {
    let config_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("config.json");
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        let config: Config = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(config)
    } else {
        Ok(Config { game_dir: "".to_string(), db_url: "".to_string() })
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
            if path.extension().map_or(false, |ext| ext == "bak") {
                let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                let clean_name = file_name.to_lowercase()
                    .replace(".bak", "")
                    .replace(".upk", "");
                
                let display_name = items.iter()
                    .find(|i| {
                        let db_pkg = i.asset_package.to_lowercase().replace(".upk", "");
                        if db_pkg.is_empty() || db_pkg == "none" { return false; }
                        
                        if db_pkg == clean_name { return true; }
                        
                        if db_pkg.len() > 4 && (clean_name.contains(&db_pkg) || db_pkg.contains(&clean_name)) {
                            return true;
                        }
                        
                        false
                    })
                    .map(|i| i.product.clone())
                    .unwrap_or(file_name);

                backups.push(BackupFile {
                    name: display_name,
                    path: path.to_string_lossy().to_string(),
                });
            }
        }
    }
    Ok(backups)
}

#[tauri::command]
async fn check_integrity(app: tauri::AppHandle) -> Result<bool, String> {
    let paths = vec![
        app.path().resolve("python/engine.sha256", tauri::path::BaseDirectory::Resource),
        app.path().resolve("_up_/python/engine.sha256", tauri::path::BaseDirectory::Resource),
        app.path().resolve("engine.sha256", tauri::path::BaseDirectory::Resource),
    ];

    let mut final_path = None;
    for p in paths {
        if let Ok(path) = p {
            if path.exists() {
                final_path = Some(path);
                break;
            }
        }
    }

    let hash_path = match final_path {
        Some(p) => p,
        None => return Ok(true),
    };
    
    let expected_hash = fs::read_to_string(hash_path).map_err(|e| e.to_string())?.trim().to_lowercase();
    
    let sidecar_name = "velocity-engine";
    #[cfg(target_os = "windows")]
    let sidecar_file = format!("{}-x86_64-pc-windows-msvc.exe", sidecar_name);
    #[cfg(not(target_os = "windows"))]
    let sidecar_file = sidecar_name;

    let sidecar_path = app.path().resolve(format!("bin/{}", sidecar_file), tauri::path::BaseDirectory::Resource)
        .or_else(|_| app.path().resolve(format!("_up_/src-tauri/bin/{}", sidecar_file), tauri::path::BaseDirectory::Resource))
        .map_err(|e| format!("Could not locate engine: {}", e))?;

    if !sidecar_path.exists() {
        return Ok(true);
    }

    let file_bytes = fs::read(sidecar_path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let actual_hash = hex::encode(hasher.finalize()).to_lowercase();
    
    if actual_hash != expected_hash {
        return Err(format!("Integrity mismatch! Engine compromised."));
    }
    Ok(true)
}

#[tauri::command]
async fn cleanup_temp_files(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Ok("No directory to clean".to_string()); }
    let dir = PathBuf::from(&config.game_dir);
    let mut count = 0;
    let now = std::time::SystemTime::now();
    let one_day = std::time::Duration::from_secs(24 * 3600);

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy(),
                None => continue,
            };
            if name.ends_with("_decrypted.upk") || name.ends_with("_decompressed.upk") {
                if let Ok(metadata) = fs::metadata(&path) {
                    if let Ok(modified) = metadata.modified() {
                        if now.duration_since(modified).unwrap_or(std::time::Duration::ZERO) > one_day {
                            let _ = fs::remove_file(path);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(format!("Cleaned up {} temp files", count))
}

#[tauri::command]
async fn fetch_catalog(app: tauri::AppHandle, token: String, account: String) -> Result<String, String> {
    let sidecar = app.shell().sidecar("velocity-engine").map_err(|e| e.to_string())?;
    let output = sidecar
        .arg("--fetch")
        .arg("--token").arg(token)
        .arg("--account").arg(account)
        .output().await.map_err(|e| e.to_string())?;
    
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    } else {
        Err(format!("Fetch error: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[tauri::command]
async fn apply_swap(app: tauri::AppHandle, owned_id: String, wanted_id: String) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Err("Game directory not set".to_string()); }
    let items_path = app.path().resolve("python/items.json", tauri::path::BaseDirectory::Resource)
        .or_else(|_| app.path().resolve("_up_/python/items.json", tauri::path::BaseDirectory::Resource))
        .map_err(|e| e.to_string())?;
    
    let sidecar = app.shell().sidecar("velocity-engine").map_err(|e| e.to_string())?;
    let output = sidecar
        .arg("--no-gui")
        .arg("--items").arg(items_path)
        .arg("--target").arg(owned_id)
        .arg("--donor").arg(wanted_id)
        .arg("--overwrite")
        .arg("--donor-dir").arg(&config.game_dir)
        .arg("--output-dir").arg(&config.game_dir)
        .output().await.map_err(|e| e.to_string())?;
    if output.status.success() { Ok("Swap completed successfully".to_string()) }
    else { Err(format!("Engine error: {}", String::from_utf8_lossy(&output.stderr))) }
}

#[tauri::command]
async fn restore_single_backup(_app: tauri::AppHandle, path: String) -> Result<(), String> {
    let bak_path = PathBuf::from(&path);
    if !bak_path.exists() { return Err("Backup file not found".into()); }
    
    let original_path = bak_path.with_extension("");
    fs::copy(&bak_path, &original_path).map_err(|e| e.to_string())?;
    fs::remove_file(&bak_path).map_err(|e| e.to_string())?;
    
    Ok(())
}

#[tauri::command]
async fn restore_backups(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Err("Game directory not set".to_string()); }
    let dir = PathBuf::from(&config.game_dir);
    let mut count = 0;
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "bak") {
            let original = path.with_extension("");
            fs::copy(&path, &original).map_err(|e| e.to_string())?;
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    Ok(format!("Restored {} backups", count))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            get_items,
            get_config,
            save_config,
            get_backups,
            apply_swap,
            restore_backups,
            restore_single_backup,
            check_integrity,
            cleanup_temp_files,
            fetch_catalog
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
