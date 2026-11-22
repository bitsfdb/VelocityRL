//! Installs complete API artifacts. No client-side bytecode or texture patching.
use crate::integrity::{self, PrepatchedManifest, PrepatchedSyncStatus};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::{Path, PathBuf}};

const API: &str = "https://api.velocityrl.tech";
const FILES: [&str; 2] = ["ProjectX.upk", "TAGame.upk"];
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn validate_manifest(m: &PrepatchedManifest, version: &str) -> Result<(), String> {
    if version.is_empty() || m.game_version.trim().is_empty() || version != m.game_version {
        return Err("Rocket League and API build versions must match exactly".into());
    }
    if m.files.len() != FILES.len() || FILES.iter().any(|name| !m.files.contains_key(*name)) {
        return Err("API must publish a complete ProjectX.upk and TAGame.upk release".into());
    }
    for info in m.files.values() {
        if info.size == 0 || info.size > 4 * 1024 * 1024 * 1024 {
            return Err("Invalid API artifact size".into());
        }
        if info.sha256.len() != 64 || !info.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid API artifact SHA-256".into());
        }
        artifact_url(&info.url)?;
    }
    Ok(())
}

fn artifact_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(&format!("{API}/")).unwrap()
        .join(value).map_err(|_| "Invalid artifact URL")?;
    if url.scheme() != "https" || url.host_str() != Some("api.velocityrl.tech")
        || url.port_or_known_default() != Some(443) || !url.username().is_empty()
        || url.password().is_some() || url.fragment().is_some()
        || !url.path().starts_with("/v2/rl/prepatched/") {
        return Err("Artifact URL must use the VelocityRL HTTPS prepatched API".into());
    }
    Ok(url)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Journal { version: String, files: Vec<String>, committed: bool }

fn journal_path(stage: &Path) -> PathBuf { stage.join("transaction.json") }

fn file_stamp(path: &Path) -> Result<(u64, std::time::SystemTime), String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    Ok((meta.len(), meta.modified().map_err(|e| e.to_string())?))
}

fn save_journal(stage: &Path, journal: &Journal) -> Result<(), String> {
    let path = stage.join("transaction.next");
    let mut file = fs::File::create(&path).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(journal).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file); // Windows cannot rename a file held open without delete sharing.
    fs::rename(path, journal_path(stage)).map_err(|e| e.to_string())
}

fn recover(cooked: &Path, stage: &Path, version: &str) -> Result<(), String> {
    let path = journal_path(stage);
    if !path.exists() { return Ok(()); }
    let journal: Journal = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Invalid installation recovery journal: {e}. Verify game files in the launcher."))?;
    if journal.files.iter().any(|n| !FILES.contains(&n.as_str())) {
        return Err("Invalid recovery file list. Verify game files in the launcher.".into());
    }
    if !journal.committed {
        if journal.version != version {
            return Err("Game updated during an interrupted installation. Verify game files in the launcher before retrying.".into());
        }
        for name in &journal.files {
            let original = stage.join(format!("{name}.original"));
            if original.exists() {
                fs::rename(&original, cooked.join(name)).map_err(|e| format!("Cannot recover {name}: {e}. Close the game and retry; do not launch it yet."))?;
                integrity::invalidate_file_hash(&cooked.join(name));
            }
        }
    }
    fs::remove_file(journal_path(stage)).map_err(|e| e.to_string())?;
    Ok(())
}

// All disk I/O and hashing run on a blocking worker, never on the webview thread.
pub async fn sync(app: Option<&tauri::AppHandle>, game: &Path) -> Result<PrepatchedSyncStatus, String> {
    sync_with_force(app, game, false).await
}

pub async fn sync_forced(app: Option<&tauri::AppHandle>, game: &Path) -> Result<PrepatchedSyncStatus, String> {
    sync_with_force(app, game, true).await
}

async fn sync_with_force(app: Option<&tauri::AppHandle>, game: &Path, force: bool) -> Result<PrepatchedSyncStatus, String> {
    let _guard = INSTALL.lock().await;
    let app = app.cloned();
    let game = game.to_owned();
    tauri::async_runtime::spawn_blocking(move || tauri::async_runtime::block_on(sync_inner(app.as_ref(), &game, force)))
        .await.map_err(|e| format!("Prepatched worker failed: {e}"))?
}

async fn sync_inner(app: Option<&tauri::AppHandle>, game: &Path, force: bool) -> Result<PrepatchedSyncStatus, String> {
    use futures_util::StreamExt;
    use tauri::Emitter;
    if let Err(message) = crate::game_file_access::ensure_idle() {
        return Ok(PrepatchedSyncStatus::Deferred { message });
    }
    let cooked = crate::upk::palette::resolve_cooked_dir(game).map_err(|e| e.to_string())?;
    let version = integrity::detect_local_game_version(game).filter(|v| !v.is_empty())
        .ok_or("Cannot identify Rocket League's installed build. No files were changed.")?;
    let stage = cooked.join(".velocity-prepatched");
    if fs::symlink_metadata(&stage).is_ok_and(|m| m.file_type().is_symlink() || !m.is_dir()) {
        return Err("Prepatched staging directory must be a regular directory".into());
    }
    fs::create_dir_all(&stage).map_err(|e| format!("Cannot stage game files: {e}"))?;
    recover(&cooked, &stage, &version)?;
    let client = reqwest::Client::builder().no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300)).build().map_err(|e| e.to_string())?;
    let response = client.get(format!("{API}/v2/rl/prepatched/manifest"))
        .timeout(std::time::Duration::from_secs(15)).send().await
        .map_err(|e| format!("API unavailable; no new files installed. Retry when online: {e}"))?
        .error_for_status().map_err(|e| format!("Prepatched API error: {e}"))?;
    // Bound the manifest independently of an untrusted Content-Length header.
    let mut manifest_bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if manifest_bytes.len() + chunk.len() > 64 * 1024 { return Err("API manifest is too large".into()); }
        manifest_bytes.extend_from_slice(&chunk);
    }
    let manifest: PrepatchedManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| format!("Invalid API manifest: {e}"))?;
    // Keep the observed build for accurate mismatch reporting in Settings.
    fs::write(stage.join("verified-manifest.json"), &manifest_bytes).map_err(|e| e.to_string())?;
    if manifest.game_version != version {
        return Ok(PrepatchedSyncStatus::VersionMismatch {
            local_version: version, server_version: manifest.game_version,
            message: "API files do not match this Rocket League build. Wait for a matching release; no files installed.".into(),
        });
    }
    validate_manifest(&manifest, &version)?;
    let mut changed = Vec::new();
    let initial_stamps: Vec<_> = FILES.iter().map(|name| file_stamp(&cooked.join(name))).collect::<Result<_, _>>()?;
    for name in FILES {
        let info = &manifest.files[name];
        let target = cooked.join(name);
        // Never create a missing installation or follow package symlinks.
        let meta = fs::symlink_metadata(&target).map_err(|e| format!("{name} missing or inaccessible: {e}. Verify game files in the launcher."))?;
        if !meta.is_file() || meta.file_type().is_symlink() { return Err(format!("{name} must be a regular installed game file")); }
        if !force && meta.len() == info.size && integrity::compute_file_sha256_cached(&target)
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&info.sha256)) { continue; }
        let staged = stage.join(format!("{name}.download"));
        let mut response = client.get(artifact_url(&info.url)?).send().await
            .map_err(|e| format!("Download failed for {name}: {e}"))?
            .error_for_status().map_err(|e| format!("Download failed for {name}: {e}"))?;
        if response.content_length().is_some_and(|len| len != info.size) { return Err(format!("API size mismatch for {name}")); }
        let mut file = fs::File::create(&staged).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut received = 0u64;
        let mut last_progress = std::time::Instant::now();
        while let Some(chunk) = response.chunk().await.map_err(|e| format!("Download interrupted for {name}: {e}"))? {
            received += chunk.len() as u64;
            if received > info.size { return Err(format!("Download exceeds expected size for {name}")); }
            file.write_all(&chunk).map_err(|e| format!("Cannot stage {name} (check free space/permissions): {e}"))?;
            hash.update(&chunk);
            if last_progress.elapsed().as_millis() > 100 {
                last_progress = std::time::Instant::now();
                if let Some(app) = app { let _ = app.emit("prepatched-progress", serde_json::json!({
                    "file": name, "downloaded": received, "total": info.size,
                    "percent": received * 100 / info.size, "stage": "downloading"
                })); }
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        if received != info.size || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&info.sha256) {
            return Err(format!("Downloaded {name} failed size/SHA-256 verification. Installed files unchanged."));
        }
        if let Some(app) = app {
            let _ = app.emit("prepatched-progress", serde_json::json!({
                "file": name, "downloaded": received, "total": info.size,
                "percent": 100, "stage": "verifying"
            }));
        }
        changed.push(name.to_string());
    }
    if changed.is_empty() {
        if let Some(app) = app {
            let _ = app.emit("prepatched-progress", serde_json::json!({
                "file": "game files", "downloaded": 0, "total": 0,
                "percent": 100, "stage": "completed"
            }));
        }
        return Ok(PrepatchedSyncStatus::UpToDate { version, message: "Both API files match the installed build and SHA-256.".into() });
    }
    if let Err(message) = crate::game_file_access::ensure_idle() { return Ok(PrepatchedSyncStatus::Deferred { message }); }
    if integrity::detect_local_game_version(game).as_deref() != Some(version.as_str()) {
        return Err("Rocket League updated during download. Retry with its new build; no files installed.".into());
    }
    for (name, initial) in FILES.iter().zip(&initial_stamps) {
        if &file_stamp(&cooked.join(name))? != initial {
            return Err(format!("{name} changed during download, possibly from launcher verification. Nothing installed; retry when it finishes."));
        }
    }
    let mut journal = Journal { version: version.clone(), files: changed.clone(), committed: false };
    // Old transaction leftovers are never reused as backups.
    for name in &changed {
        let old = stage.join(format!("{name}.original"));
        if old.exists() { fs::remove_file(old).map_err(|e| e.to_string())?; }
    }
    save_journal(&stage, &journal)?;
    if let Some(app) = app {
        let _ = app.emit("prepatched-progress", serde_json::json!({
            "file": "game files", "downloaded": 0, "total": 0,
            "percent": 100, "stage": "installing"
        }));
    }
    let result = (|| -> Result<(), String> {
        for name in &changed {
            crate::game_file_access::ensure_idle()?;
            fs::rename(cooked.join(name), stage.join(format!("{name}.original")))
                .map_err(|e| format!("Cannot move {name}: {e}. Close the game/launcher and retry."))?;
            fs::rename(stage.join(format!("{name}.download")), cooked.join(name))
                .map_err(|e| format!("Cannot install {name}: {e}"))?;
            integrity::invalidate_file_hash(&cooked.join(name));
        }
        journal.committed = true;
        save_journal(&stage, &journal)
    })();
    if let Err(error) = result {
        recover(&cooked, &stage, &version).map_err(|recovery| format!("{error}. Recovery failed: {recovery}"))?;
        return Err(format!("{error}. Installation rolled back."));
    }
    recover(&cooked, &stage, &version)?;
    if let Some(app) = app {
        let _ = app.emit("prepatched-progress", serde_json::json!({
            "file": "game files", "downloaded": 0, "total": 0,
            "percent": 100, "stage": "completed"
        }));
    }
    Ok(PrepatchedSyncStatus::Success {
        version, tagame_updated: changed.iter().any(|n| n == "TAGame.upk"),
        projectx_updated: changed.iter().any(|n| n == "ProjectX.upk"),
        message: "Installed complete, verified API artifacts. No local patches applied.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> PrepatchedManifest {
        serde_json::from_value(serde_json::json!({"game_version":"build-a", "files": {
            "TAGame.upk":{"sha256":"a".repeat(64),"size":42,"url":"/v2/rl/prepatched/TAGame.upk"},
            "ProjectX.upk":{"sha256":"b".repeat(64),"size":42,"url":"/v2/rl/prepatched/ProjectX.upk"}
        }})).unwrap()
    }
    #[test] fn rejects_mismatch_alias_and_incomplete_release() {
        let mut m = manifest(); m.prime_version = "build-b".into();
        assert!(validate_manifest(&m, "build-b").is_err());
        assert!(validate_manifest(&m, "build-a").is_ok());
        m.files.remove("ProjectX.upk"); assert!(validate_manifest(&m, "build-a").is_err());
    }
    #[test] fn rejects_bad_hash_size_paths_and_remote_hosts() {
        for url in ["http://api.velocityrl.tech/v2/rl/prepatched/a", "https://evil.test/a", "//evil.test/a", "/v2/rl/prepatched/../../a"] {
            assert!(artifact_url(url).is_err(), "{url}");
        }
        let mut m = manifest(); m.files.get_mut("TAGame.upk").unwrap().size = 0;
        assert!(validate_manifest(&m, "build-a").is_err());
        let mut m = manifest(); m.files.get_mut("TAGame.upk").unwrap().sha256 = "z".repeat(64);
        assert!(validate_manifest(&m, "build-a").is_err());
        let mut m = manifest(); let file = m.files.remove("TAGame.upk").unwrap(); m.files.insert("../TAGame.upk".into(), file);
        assert!(validate_manifest(&m, "build-a").is_err());
    }
    #[test] fn interrupted_install_recovers_originals_and_refuses_cross_version_restore() {
        let root = std::env::temp_dir().join(format!("vrl-transaction-{}-{}", std::process::id(), rand::random::<u64>()));
        let stage = root.join("stage"); fs::create_dir_all(&stage).unwrap();
        fs::write(root.join("TAGame.upk"), b"partial-new").unwrap();
        fs::write(stage.join("TAGame.upk.original"), b"original").unwrap();
        save_journal(&stage, &Journal {version:"a".into(),files:vec!["TAGame.upk".into()],committed:false}).unwrap();
        assert!(recover(&root, &stage, "b").is_err());
        assert_eq!(fs::read(root.join("TAGame.upk")).unwrap(), b"partial-new");
        recover(&root, &stage, "a").unwrap();
        assert_eq!(fs::read(root.join("TAGame.upk")).unwrap(), b"original");
        recover(&root, &stage, "a").unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test] fn completed_install_is_not_rolled_back_after_restart() {
        let root = std::env::temp_dir().join(format!("vrl-completed-{}-{}", std::process::id(), rand::random::<u64>()));
        let stage = root.join("stage"); fs::create_dir_all(&stage).unwrap();
        fs::write(root.join("TAGame.upk"), b"verified-new").unwrap();
        fs::write(stage.join("TAGame.upk.original"), b"old").unwrap();
        save_journal(&stage, &Journal {version:"a".into(),files:vec!["TAGame.upk".into()],committed:true}).unwrap();
        recover(&root, &stage, "b").unwrap();
        assert_eq!(fs::read(root.join("TAGame.upk")).unwrap(), b"verified-new");
        fs::remove_dir_all(root).unwrap();
    }
}
