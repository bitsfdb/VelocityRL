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

pub fn mark_palette_on(state: &mut IntegrityState, fingerprint: &str) {
    state.palette_active = true;
    state.palette_fingerprint = fingerprint.to_string();
}

pub fn mark_palette_off(state: &mut IntegrityState) {
    state.palette_active = false;
    state.palette_fingerprint.clear();
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
        format!("{SWAP_VERIFY_MESSAGE} Rich palette was also reset.")
    } else if swaps_wiped > 0 {
        SWAP_VERIFY_MESSAGE.to_string()
    } else {
        "Epic Repair wiped rich palette".into()
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
}
