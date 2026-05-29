use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;
use sha2::{Sha256, Digest};
use hex;

// ── Embedded engine ───────────────────────────────────────────────────────────
// The engine binary is compiled into this library at build time.
// This eliminates all runtime path-finding: the engine is always present.
#[cfg(target_os = "windows")]
const ENGINE_BYTES: &[u8] = include_bytes!("../bin/velocity-engine-x86_64-pc-windows-msvc.exe");
#[cfg(not(target_os = "windows"))]
const ENGINE_BYTES: &[u8] = include_bytes!("../bin/velocity-engine-x86_64-unknown-linux-gnu");

fn engine_hash() -> String {
    let mut h = Sha256::new();
    h.update(ENGINE_BYTES);
    hex::encode(h.finalize())
}

/// Extract the embedded engine to AppLocalData if missing or outdated, return its path.
async fn get_engine_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let local_dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&local_dir).map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    let name = "velocity-engine.exe";
    #[cfg(not(target_os = "windows"))]
    let name = "velocity-engine";

    let path = local_dir.join(name);
    let expected = engine_hash();

    let needs_write = if path.exists() {
        match fs::read(&path) {
            Ok(bytes) => {
                let mut h = Sha256::new();
                h.update(&bytes);
                hex::encode(h.finalize()) != expected
            }
            Err(_) => true,
        }
    } else {
        true
    };

    if needs_write {
        fs::write(&path, ENGINE_BYTES).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(path)
}

/// Run the engine with the given args and optional env vars.
async fn run_engine(
    path: PathBuf,
    args: Vec<String>,
    env_vars: Vec<(String, String)>,
) -> Result<std::process::Output, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&path);
        for arg in &args { cmd.arg(arg); }
        for (k, v) in &env_vars { cmd.env(k, v); }
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    game_dir: String,
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
async fn check_integrity(app: tauri::AppHandle) -> Result<bool, String> {
    // Engine is embedded at compile time — extract it and confirm it's ready.
    get_engine_path(&app).await?;
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
    let engine_path = get_engine_path(&app).await?;
    let output = run_engine(
        engine_path,
        vec!["--fetch".into(), "--account".into(), account],
        vec![("EPIC_TOKEN".into(), token)],
    ).await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!("Fetch error: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[tauri::command]
async fn replace_export(app: tauri::AppHandle, target_pkg: String, target_path: String, donor_pkg: String, donor_path: String) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Err("Game directory not set".to_string()); }

    let engine_path = get_engine_path(&app).await?;
    let items_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("items.json");

    let output = run_engine(
        engine_path,
        vec![
            "--replace-export".into(),
            "--items".into(), items_path.to_string_lossy().to_string(),
            "--target".into(), target_pkg,
            "--target-path".into(), target_path,
            "--donor".into(), donor_pkg,
            "--donor-path".into(), donor_path,
            "--overwrite".into(),
            "--donor-dir".into(), config.game_dir.clone(),
            "--output-dir".into(), config.game_dir.clone(),
        ],
        vec![],
    ).await?;

    if output.status.success() {
        Ok("Export replacement completed".to_string())
    } else {
        Err(format!("Engine error: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[tauri::command]
async fn set_custom_pfp(app: tauri::AppHandle, donor_upk: String) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Err("Game directory not set".to_string()); }

    let engine_path = get_engine_path(&app).await?;

    let output = run_engine(
        engine_path,
        vec![
            "--custom-pfp".into(), donor_upk,
            "--overwrite".into(),
            "--donor-dir".into(), config.game_dir.clone(),
            "--output-dir".into(), config.game_dir.clone(),
        ],
        vec![],
    ).await?;

    if output.status.success() {
        Ok("Custom PFP applied".to_string())
    } else {
        Err(format!("Engine error: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[tauri::command]
async fn change_display_name(app: tauri::AppHandle, old_name: String, new_name: String) -> Result<String, String> {
    let local_dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    let script_path = local_dir.join("rl_memory_patcher.py");

    // We should ensure the script is extracted if we're bundling it,
    // but for this task we assume it's in the python/ directory during dev
    // and correctly placed in production.

    // For now, let's try to run it from the python/ dir if it exists, otherwise from local_dir
    let python_script = if PathBuf::from("python/rl_memory_patcher.py").exists() {
        PathBuf::from("python/rl_memory_patcher.py")
    } else {
        script_path
    };

    let output = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new("python")
            .arg(python_script)
            .arg("--old")
            .arg(old_name)
            .arg("--new")
            .arg(new_name)
            .output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!("Memory patcher error: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[tauri::command]
async fn apply_swap(app: tauri::AppHandle, owned_id: String, wanted_id: String) -> Result<String, String> {
    let config = get_config(app.clone()).await?;
    if config.game_dir.is_empty() { return Err("Game directory not set".to_string()); }

    let engine_path = get_engine_path(&app).await?;
    let items_path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("items.json");

    let output = run_engine(
        engine_path,
        vec![
            "--no-gui".into(),
            "--items".into(), items_path.to_string_lossy().to_string(),
            "--target".into(), owned_id.clone(),
            "--donor".into(), wanted_id.clone(),
            "--overwrite".into(),
            "--donor-dir".into(), config.game_dir.clone(),
            "--output-dir".into(), config.game_dir.clone(),
        ],
        vec![],
    ).await?;

    if output.status.success() {
        Ok("Swap completed successfully".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let game_dir_name = PathBuf::from(&config.game_dir)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let exit_code = output.status.code().unwrap_or(-1);
        send_diagnostic(json!({
            "event":     "swap_fail",
            "context":   "apply_swap",
            "message":   format!("Engine exited with code {}", exit_code),
            "stderr":    stderr,
            "stdout":    stdout,
            "owned_id":  owned_id,
            "wanted_id": wanted_id,
            "game_dir":  game_dir_name,
            "exit_code": exit_code,
        })).await;
        Err(format!("Engine error: {}", stderr))
    }
}

#[tauri::command]
async fn restore_single_backup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let config = get_config(app).await?;
    if config.game_dir.is_empty() { return Err("Game directory not configured".into()); }
    let allowed_dir = PathBuf::from(&config.game_dir).canonicalize().map_err(|e| e.to_string())?;

    let bak_path = PathBuf::from(&path).canonicalize().map_err(|_| "Invalid backup path".to_string())?;
    if !bak_path.starts_with(&allowed_dir) {
        return Err("Access denied: path is outside the game directory".into());
    }
    if bak_path.extension().map_or(true, |ext| ext != "bak") {
        return Err("Access denied: only .bak files can be restored".into());
    }

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

#[tauri::command]
async fn report_diagnostic(payload: serde_json::Value) -> Result<(), String> {
    send_diagnostic(payload).await;
    Ok(())
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
            change_display_name,
            restore_backups,
            restore_single_backup,
            check_integrity,
            cleanup_temp_files,
            fetch_catalog,
            report_diagnostic,
            check_for_updates,
            install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
