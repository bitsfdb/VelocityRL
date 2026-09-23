use crate::upk::{palette, parser};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Compute a fast fingerprint (size:mtime_secs) of Engine.upk in the cooked dir.
/// Engine.upk changes on every RL game update — cheap to compute (no file read needed).
pub fn rl_update_fingerprint_for(cooked_dir: &Path) -> String {
    let engine_upk = cooked_dir.join("Engine.upk");
    let Ok(meta) = std::fs::metadata(&engine_upk) else {
        return String::new();
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}:{}", meta.len(), mtime)
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
