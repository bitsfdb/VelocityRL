use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn compute_build(root: &Path) -> (String, String) {
    let env_num = std::env::var("VRL_BUILD_NUMBER").ok().filter(|s| !s.is_empty());
    let env_hash = std::env::var("VRL_BUILD_HASH").ok().filter(|s| !s.is_empty());

    let number = env_num.unwrap_or_else(|| {
        git(root, &["rev-list", "--count", "HEAD"]).unwrap_or_else(|| "0".to_string())
    });
    let hash = env_hash.unwrap_or_else(|| {
        git(root, &["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "dev".to_string())
    });
    (number, hash)
}

fn main() {
    println!("cargo:rerun-if-changed=windows/app.manifest");
    println!("cargo:rerun-if-changed=../ui");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-env-changed=VRL_BUILD_NUMBER");
    println!("cargo:rerun-if-env-changed=VRL_BUILD_HASH");
    println!("cargo:rerun-if-env-changed=VRL_BUILD_ID");

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest.parent().unwrap_or(manifest);
    if repo_root.join(".git").exists() {
        println!("cargo:rerun-if-changed=../.git/HEAD");
        println!("cargo:rerun-if-changed=../.git/refs/heads");
    }

    let (_git_num, hash) = compute_build(repo_root);
    let build_id = std::env::var("VRL_BUILD_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "-659612010".to_string());

    let number = std::env::var("VRL_BUILD_NUMBER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| build_id.clone());

    println!("cargo:rustc-env=VRL_BUILD_NUMBER={number}");
    println!("cargo:rustc-env=VRL_BUILD_HASH={hash}");
    println!("cargo:rustc-env=VRL_BUILD_ID={build_id}");

    let mut windows = tauri_build::WindowsAttributes::new();
    windows = windows.app_manifest(include_str!("windows/app.manifest"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run tauri build script");
}
