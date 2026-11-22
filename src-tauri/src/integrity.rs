/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use crate::upk::{palette, parser};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpkHashesAudit {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub rl_version: Option<String>,
    #[serde(default)]
    pub rl_exe_sha256: Option<String>,
    #[serde(default)]
    pub tagame_sha256: Option<String>,
    #[serde(default)]
    pub tagame_size: Option<u64>,
    #[serde(default)]
    pub projectx_sha256: Option<String>,
    #[serde(default)]
    pub projectx_size: Option<u64>,
    #[serde(default)]
    pub engine_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "details")]
pub enum PrepatchedSyncStatus {
    Deferred { message: String },
    Success {
        version: String,
        tagame_updated: bool,
        projectx_updated: bool,
        message: String,
    },
    VersionMismatch {
        local_version: String,
        server_version: String,
        message: String,
    },
    UpToDate {
        version: String,
        message: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepatchedFileInfo {
    pub sha256: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepatchedManifest {
    pub game_version: String,
    #[serde(default)]
    pub prime_version: String,
    #[serde(default)]
    pub feature_set: String,
    pub files: HashMap<String, PrepatchedFileInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrityState {
    #[serde(default)]
    pub palette_active: bool,
    #[serde(default)]
    pub palette_fingerprint: String,
    #[serde(default)]
    pub swap_packages: Vec<String>,
    #[serde(default)]
    pub swap_fingerprints: HashMap<String, String>,
    /// Fingerprint of Engine.upk (size:mtime_secs) at the time the palette was applied.
    /// If this changes, RL was updated and the patched TAGame.upk must be restored before
    /// launching — otherwise the new game binary will crash against the old patched data.
    #[serde(default)]
    pub rl_update_fingerprint: String,
    #[serde(default)]
    pub last_audit: Option<UpkHashesAudit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairReport {
    pub repaired: bool,
    pub palette_wiped: bool,
    pub swaps_wiped: usize,
    pub message: String,
}

pub const SWAP_VERIFY_MESSAGE: &str =
    "A verification of files has been detected. A reswap is advised to keep your swaps in game.";

impl IntegrityState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}

pub fn integrity_path(config_dir: &Path) -> PathBuf {
    config_dir.join("integrity.json")
}

/// Compute a fast fingerprint of Engine.upk and RocketLeague.exe.
/// If either file is updated (size or mtime changed), an RL update is detected.
pub fn rl_update_fingerprint_for(cooked_dir: &Path) -> String {
    let engine_upk = cooked_dir.join("Engine.upk");
    let rl_exe = cooked_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("Binaries").join("Win64").join("RocketLeague.exe"));

    let engine_part = std::fs::metadata(&engine_upk).ok().map(|meta| {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos().min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        format!("{}:{}", meta.len(), mtime)
    }).unwrap_or_default();

    let exe_part = rl_exe.and_then(|p| std::fs::metadata(p).ok()).map(|meta| {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos().min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        format!("{}:{}", meta.len(), mtime)
    }).unwrap_or_default();

    if !exe_part.is_empty() {
        format!("{engine_part}|{exe_part}")
    } else {
        engine_part
    }
}

#[allow(dead_code)]
pub fn mark_palette_on(state: &mut IntegrityState, fingerprint: &str) {
    state.palette_active = true;
    state.palette_fingerprint = fingerprint.to_string();
}

pub fn mark_palette_on_with_rl(state: &mut IntegrityState, fingerprint: &str, rl_fp: &str) {
    state.palette_active = true;
    state.palette_fingerprint = fingerprint.to_string();
    state.rl_update_fingerprint = rl_fp.to_string();
}

pub fn mark_palette_off(state: &mut IntegrityState) {
    state.palette_active = false;
    state.palette_fingerprint.clear();
    state.rl_update_fingerprint.clear();
}

pub fn mark_swap_package(state: &mut IntegrityState, package: &str, fingerprint: Option<&str>) {
    if package.is_empty() {
        return;
    }
    let p = package.to_string();
    if !state.swap_packages.iter().any(|x| x == &p) {
        state.swap_packages.push(p.clone());
    }
    if let Some(fp) = fingerprint.filter(|s| !s.is_empty()) {
        state.swap_fingerprints.insert(p, fp.to_string());
    }
}

pub fn clear_swap_package(state: &mut IntegrityState, package: &str) {
    state.swap_packages.retain(|x| x != package);
    state.swap_fingerprints.remove(package);
}

fn cooked_dir(game_dir: &Path) -> Option<PathBuf> {
    if game_dir.as_os_str().is_empty() || !game_dir.exists() {
        return None;
    }
    palette::resolve_cooked_dir(game_dir).ok()
}

pub fn bak_path_for(upk: &Path) -> PathBuf {
    let mut p = upk.to_path_buf();
    let mut name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.ends_with(".bak") {
        name.push_str(".bak");
    }
    p.set_file_name(name);
    p
}

pub fn upk_fingerprint(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let (summary, meta) = parser::parse_prefix(&data).ok()?;
    if summary.name_offset < 0 {
        return None;
    }
    let name_offset = summary.name_offset as usize;
    let enc_size = summary
        .total_header_size
        .checked_sub(meta.garbage_size)
        .and_then(|v| v.checked_sub(summary.name_offset))?;
    if enc_size <= 0 {
        return None;
    }
    let enc_aligned = (enc_size as usize + 15) & !15;
    Some(palette::fingerprint_bytes(&data, name_offset, enc_aligned))
}

pub fn swap_package_wiped(cooked: &Path, package: &str, expected_fp: Option<&str>) -> bool {
    if package.is_empty() {
        return false;
    }
    let upk = cooked.join(package);
    if !upk.exists() {
        return false;
    }
    let bak = bak_path_for(&upk);
    let live_fp = upk_fingerprint(&upk);

    if let Some(exp) = expected_fp.filter(|s| !s.is_empty()) {
        match live_fp.as_deref() {
            Some(live) if live == exp => return false,
            Some(_) | None => return true,
        }
    }

    if bak.exists() {
        let bak_fp = upk_fingerprint(&bak);
        return live_fp.is_some() && live_fp == bak_fp;
    }

    true
}

pub fn check_repair(game_dir: &Path, state: &IntegrityState) -> RepairReport {
    let Some(cooked) = cooked_dir(game_dir) else {
        return RepairReport {
            repaired: false,
            palette_wiped: false,
            swaps_wiped: 0,
            message: String::new(),
        };
    };

    let palette_wiped = state.palette_active
        && palette::repair_wiped_palette(&cooked, Some(state.palette_fingerprint.as_str()));

    let mut seen = std::collections::HashSet::new();
    let mut swaps_wiped = 0usize;
    for pkg in &state.swap_packages {
        if !seen.insert(pkg.clone()) {
            continue;
        }
        let exp = state.swap_fingerprints.get(pkg).map(|s| s.as_str());
        if swap_package_wiped(&cooked, pkg, exp) {
            swaps_wiped += 1;
        }
    }

    let repaired = palette_wiped || swaps_wiped > 0;
    let message = if !repaired {
        String::new()
    } else if swaps_wiped > 0 && palette_wiped {
        format!("{SWAP_VERIFY_MESSAGE} Color palette was also reset.")
    } else if swaps_wiped > 0 {
        SWAP_VERIFY_MESSAGE.to_string()
    } else {
        "Epic Repair wiped color palette".into()
    };

    RepairReport {
        repaired,
        palette_wiped,
        swaps_wiped,
        message,
    }
}

pub fn acknowledge_repair(game_dir: &Path, state: &mut IntegrityState) {
    let Some(cooked) = cooked_dir(game_dir) else {
        return;
    };
    if state.palette_active
        && palette::repair_wiped_palette(&cooked, Some(state.palette_fingerprint.as_str()))
    {
        mark_palette_off(state);
    }
    let fps = state.swap_fingerprints.clone();
    state.swap_packages.retain(|pkg| {
        let exp = fps.get(pkg).map(|s| s.as_str());
        !swap_package_wiped(&cooked, pkg, exp)
    });
    state.swap_fingerprints.retain(|pkg, fp| {
        !swap_package_wiped(&cooked, pkg, Some(fp.as_str()))
    });
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreSwapCheck {
    pub ok: bool,
    pub issues: Vec<String>,
}

#[allow(dead_code)]
const CRITICAL_PACKAGES: &[&str] = &[
    "Engine.upk",
    "TAGame.upk",
];

#[allow(dead_code)]
pub fn pre_swap_check(game_dir: &Path) -> PreSwapCheck {
    let mut issues = Vec::new();

    let cooked = match palette::resolve_cooked_dir(game_dir) {
        Ok(p) => p,
        Err(_) => {
            issues.push("Could not resolve CookedPCConsole directory.".into());
            return PreSwapCheck { ok: false, issues };
        }
    };

    for pkg in CRITICAL_PACKAGES {
        let upk = cooked.join(pkg);
        if !upk.exists() {
            issues.push(format!("Missing critical package: {pkg}"));
            continue;
        }
        match std::fs::metadata(&upk) {
            Ok(meta) => {
                if meta.len() == 0 {
                    issues.push(format!("{pkg} is empty (0 bytes) — game files may be corrupted."));
                } else if meta.len() < 1024 {
                    issues.push(format!("{pkg} is suspiciously small ({} bytes).", meta.len()));
                }
            }
            Err(e) => {
                issues.push(format!("Cannot read {pkg}: {e}"));
            }
        }

        if let Ok(data) = std::fs::read(&upk) {
            if let Err(e) = parser::parse_prefix(&data) {
                issues.push(format!("{pkg} has an invalid UPK header: {e}"));
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(&cooked) {
        let mut bak_count = 0usize;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".upk.bak") {
                bak_count += 1;

                let orig_name = name_str.trim_end_matches(".bak");
                if !cooked.join(orig_name).exists() {
                    issues.push(format!(
                        "Backup exists for {orig_name} but the original is missing — verify game files."
                    ));
                }
            }
        }
        if bak_count > 0 {
            issues.push(format!(
                "{bak_count} backup file(s) detected. If you recently verified game files, use Restore All before swapping."
            ));
        }
    }

    PreSwapCheck {
        ok: issues.is_empty(),
        issues,
    }
}

pub fn compute_file_sha256(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

static HASH_CACHE: std::sync::Mutex<Option<HashMap<PathBuf, (u64, u64, String)>>> = std::sync::Mutex::new(None);

pub(crate) fn invalidate_file_hash(path: &Path) {
    if let Ok(mut cache) = HASH_CACHE.lock() {
        if let Some(entries) = cache.as_mut() { entries.remove(path); }
    }
}

pub fn compute_file_sha256_cached(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0);

    if let Ok(guard) = HASH_CACHE.lock() {
        if let Some(ref map) = *guard {
            if let Some((cached_len, cached_mtime, ref hash)) = map.get(path) {
                if *cached_len == len && *cached_mtime == mtime {
                    return Some(hash.clone());
                }
            }
        }
    }

    let hash = compute_file_sha256(path)?;

    if let Ok(mut guard) = HASH_CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(path.to_path_buf(), (len, mtime, hash.clone()));
    }

    Some(hash)
}

pub fn resolve_rl_exe(game_dir: &Path) -> Option<PathBuf> {
    if let Ok(cooked) = palette::resolve_cooked_dir(game_dir) {
        if let Some(parent2) = cooked.parent().and_then(|p| p.parent()) {
            let candidate = parent2.join("Binaries").join("Win64").join("RocketLeague.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let candidates = [
        game_dir.join("Binaries").join("Win64").join("RocketLeague.exe"),
        game_dir.join("RocketLeague.exe"),
    ];
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

static VERSION_CACHE: std::sync::Mutex<Option<(PathBuf, u64, u64, String)>> = std::sync::Mutex::new(None);

pub fn detect_local_game_version(game_dir: &Path) -> Option<String> {
    let exe = resolve_rl_exe(game_dir)?;
    let meta = std::fs::metadata(&exe).ok()?;
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0);

    if let Ok(guard) = VERSION_CACHE.lock() {
        if let Some((ref cached_path, cached_len, cached_mtime, ref ver)) = *guard {
            if cached_path == &exe && cached_len == len && cached_mtime == mtime {
                return Some(ver.clone());
            }
        }
    }

    let data = std::fs::read(&exe).ok()?;

    // 1. Scan for UTF-16LE version pattern \d{6}\.\d{5}\.\d{6} (38 bytes)
    for window in data.windows(38) {
        let is_digit = |idx: usize| window[idx * 2] >= b'0' && window[idx * 2] <= b'9' && window[idx * 2 + 1] == 0;
        let is_dot = |idx: usize| window[idx * 2] == b'.' && window[idx * 2 + 1] == 0;

        let matches = (0..6).all(is_digit)
            && is_dot(6)
            && (7..12).all(is_digit)
            && is_dot(12)
            && (13..19).all(is_digit);

        if matches {
            let mut s = String::with_capacity(19);
            for i in 0..19 {
                s.push(window[i * 2] as char);
            }
            if let Ok(mut guard) = VERSION_CACHE.lock() {
                *guard = Some((exe.clone(), len, mtime, s.clone()));
            }
            return Some(s);
        }
    }

    // 2. Fallback: Scan for ++Prime+Update... ASCII
    const PRIME_PREFIX: &[u8] = b"++Prime+Update";
    if let Some(pos) = data.windows(PRIME_PREFIX.len()).position(|w| w == PRIME_PREFIX) {
        let mut end = pos + PRIME_PREFIX.len();
        while end < data.len() && (data[end].is_ascii_alphanumeric() || data[end] == b'+' || data[end] == b'.' || data[end] == b'-') {
            end += 1;
        }
        if let Ok(s) = std::str::from_utf8(&data[pos..end]) {
            let ver = s.to_string();
            if let Ok(mut guard) = VERSION_CACHE.lock() {
                *guard = Some((exe.clone(), len, mtime, ver.clone()));
            }
            return Some(ver);
        }
    }

    None
}

pub fn audit_and_log_hashes(game_dir: &Path) -> UpkHashesAudit {
    let now = chrono::Utc::now().timestamp();
    let rl_version = detect_local_game_version(game_dir);

    let cooked = palette::resolve_cooked_dir(game_dir).ok();
    let rl_exe = resolve_rl_exe(game_dir);

    let rl_exe_sha256 = rl_exe.as_deref().and_then(compute_file_sha256_cached);
    let tagame_path = cooked.as_ref().map(|c| c.join("TAGame.upk"));
    let tagame_sha256 = tagame_path.as_deref().and_then(compute_file_sha256_cached);
    let tagame_size = tagame_path.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());

    let projectx_path = cooked.as_ref().map(|c| c.join("ProjectX.upk"));
    let projectx_sha256 = projectx_path.as_deref().and_then(compute_file_sha256_cached);
    let projectx_size = projectx_path.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());

    let engine_path = cooked.as_ref().map(|c| c.join("Engine.upk"));
    let engine_sha256 = engine_path.as_deref().and_then(compute_file_sha256_cached);

    let audit = UpkHashesAudit {
        timestamp: now,
        rl_version: rl_version.clone(),
        rl_exe_sha256: rl_exe_sha256.clone(),
        tagame_sha256: tagame_sha256.clone(),
        tagame_size,
        projectx_sha256: projectx_sha256.clone(),
        projectx_size,
        engine_sha256,
    };

    let log_msg = format!(
        "[UPK_HASH_AUDIT] version={:?}, exe_sha256={:?}, tagame_sha256={:?} ({} bytes), projectx_sha256={:?} ({} bytes)",
        rl_version,
        rl_exe_sha256,
        tagame_sha256,
        tagame_size.unwrap_or(0),
        projectx_sha256,
        projectx_size.unwrap_or(0),
    );
    crate::applog::event(&log_msg);

    // Persist permanent audit log into logs directory
    if let Some(logs_dir) = crate::applog::get_logs_dir_path() {
        let audit_log_path = logs_dir.join("upk_hashes.log");
        let date_str = chrono::Utc::now().to_rfc3339();
        let file_line = format!("{date_str} {log_msg}\n");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&audit_log_path) {
            let _ = f.write_all(file_line.as_bytes());
        }
    }

    audit
}

pub async fn sync_prepatched_upks(app: Option<&tauri::AppHandle>, game_dir: &Path) -> Result<PrepatchedSyncStatus, String> {
    crate::prepatched::sync(app, game_dir).await
}

pub async fn force_sync_prepatched_upks(app: Option<&tauri::AppHandle>, game_dir: &Path) -> Result<PrepatchedSyncStatus, String> {
    crate::prepatched::sync_forced(app, game_dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_false_positive_without_swaps() {
        let state = IntegrityState::default();
        let report = check_repair(Path::new(""), &state);
        assert!(!report.repaired);
        assert_eq!(report.swaps_wiped, 0);
        assert!(report.message.is_empty());
    }

    #[test]
    fn bak_path_appends_bak() {
        let p = bak_path_for(Path::new("Body_Octane_SF.upk"));
        assert!(p.file_name().unwrap().to_string_lossy().ends_with(".upk.bak"));
    }

    #[test]
    fn pre_swap_check_empty_dir_returns_error() {
        let report = pre_swap_check(Path::new(""));
        assert!(!report.ok);
        assert!(!report.issues.is_empty());
    }
}
