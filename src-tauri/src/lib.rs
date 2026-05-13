use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    game_dir: String,
}

#[derive(Serialize, Deserialize)]
struct Item {
    ID: i32,
    Product: String,
    AssetPackage: String,
    Slot: String,
    Quality: String,
}

#[derive(Serialize, Deserialize)]
struct ItemsDatabase {
    Items: Vec<Item>,
}

#[tauri::command]
async fn get_items(app: tauri::AppHandle) -> Result<Vec<Item>, String> {
    let resource_path = app.path().resolve("python/items.json", tauri::path::BaseDirectory::Resource)
        .map_err(|e| e.to_string())?;
    
    let content = fs::read_to_string(resource_path).map_err(|e| e.to_string())?;
    let db: ItemsDatabase = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(db.Items)
}

#[tauri::command]
async fn get_config(app: tauri::AppHandle) -> Result<Config, String> {
    let config_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("config.json");
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        let config: Config = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(config)
    } else {
        Ok(Config { game_dir: "".to_string() })
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
async fn apply_swap(app: tauri::AppHandle, owned_id: String, wanted_id: String) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }

    let python_script = app.path().resolve("python/rl_asset_swapper.py", tauri::path::BaseDirectory::Resource)
        .map_err(|e| e.to_string())?;

    // We assume 'python' is in PATH.
    let output = Command::new("python")
        .current_dir(python_script.parent().unwrap())
        .arg(&python_script)
        .arg("--no-gui")
        .arg("--target")
        .arg(owned_id)
        .arg("--donor")
        .arg(wanted_id)
        .arg("--overwrite")
        .arg("--donor-dir")
        .arg(&config.game_dir)
        .arg("--output-dir")
        .arg(&config.game_dir)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok("Swap completed successfully".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("Python error: {}", err))
    }
}

#[tauri::command]
async fn restore_backups(app: tauri::AppHandle) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() {
        return Err("Game directory not set".to_string());
    }

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
            apply_swap,
            restore_backups
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
