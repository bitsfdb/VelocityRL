#!/usr/bin/env python3
"""
VelocityRL Server-side Pre-patched UPK Updater
==============================================
Runs directly on the VPS to update pre-patched UPKs and manifest.json.

Usage:
    # 1. Place new UPK files into /root/velrlapi/prepatched_incoming/
    python3 /root/velrlapi/update_prepatched.py

    # Or specify custom paths / version:
    python3 /root/velrlapi/update_prepatched.py --version 261031.63733.533021
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

BASE_DIR = Path("/root/velrlapi")
PREPATCHED_DIR = BASE_DIR / "prepatched"
INCOMING_DIR = BASE_DIR / "prepatched_incoming"
MANIFEST_FILE = PREPATCHED_DIR / "manifest.json"

KNOWN_SERVER_EXE_PATHS = [
    BASE_DIR / "games" / "RocketLeague.exe",
    BASE_DIR / "games" / "rocketleague" / "Binaries" / "Win64" / "RocketLeague.exe",
]


def compute_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest().lower()


def extract_version_from_exe(exe_path: Path):
    if not exe_path.is_file():
        return None, None
    try:
        with open(exe_path, "rb") as f:
            data = f.read()

        m = re.search(rb'(?:[\x30-\x39]\x00){6}\.\x00(?:[\x30-\x39]\x00){5}\.\x00(?:[\x30-\x39]\x00){6}', data)
        gv = m.group(0).decode("utf-16le") if m else None

        m_prime = re.search(rb'\+\+Prime\+Update[0-9a-zA-Z\.\_\-\+]+', data)
        pv = m_prime.group(0).decode("ascii", errors="ignore") if m_prime else None

        return gv, pv
    except Exception as e:
        print(f"[!] Warning reading {exe_path}: {e}")
        return None, None


def main():
    parser = argparse.ArgumentParser(description="VelocityRL Pre-patched UPK Server Updater")
    parser.add_argument("--incoming", type=Path, default=INCOMING_DIR,
                        help=f"Directory to check for incoming UPKs (default: {INCOMING_DIR})")
    parser.add_argument("--tagame", type=Path, default=None,
                        help="Path to new TAGame.upk")
    parser.add_argument("--projectx", type=Path, default=None,
                        help="Path to new ProjectX.upk")
    parser.add_argument("--exe", type=Path, default=None,
                        help="Path to RocketLeague.exe to extract version")
    parser.add_argument("--version", "-v", type=str, default=None,
                        help="Explicit Rocket League game version string")
    parser.add_argument("--prime-version", type=str, default=None,
                        help="Explicit Rocket League prime version string")
    parser.add_argument("--feature-set", type=str, default="PrimeUpdate66313")
    args = parser.parse_args()

    PREPATCHED_DIR.mkdir(parents=True, exist_ok=True)
    args.incoming.mkdir(parents=True, exist_ok=True)

    tagame_path = args.tagame or (args.incoming / "TAGame.upk" if (args.incoming / "TAGame.upk").is_file() else None)
    projectx_path = args.projectx or (args.incoming / "ProjectX.upk" if (args.incoming / "ProjectX.upk").is_file() else None)

    # If no files in incoming, check if files are already in prepatched directory
    if not tagame_path and not projectx_path:
        print(f"[*] No new files found in incoming directory '{args.incoming}'.")
        print("    Checking existing files in prepatched dir...")
        if (PREPATCHED_DIR / "TAGame.upk").is_file():
            tagame_path = PREPATCHED_DIR / "TAGame.upk"
        if (PREPATCHED_DIR / "ProjectX.upk").is_file():
            projectx_path = PREPATCHED_DIR / "ProjectX.upk"

    if not tagame_path and not projectx_path:
        print("[ERROR] No UPK files found to update.")
        sys.exit(1)

    # Game version detection
    game_version = args.version
    prime_version = args.prime_version

    if not game_version:
        exe_candidates = [args.incoming / "RocketLeague.exe"] + KNOWN_SERVER_EXE_PATHS
        if args.exe:
            exe_candidates.insert(0, args.exe)

        for c in exe_candidates:
            if c.is_file():
                gv, pv = extract_version_from_exe(c)
                if gv:
                    game_version = gv
                    if not prime_version and pv:
                        prime_version = pv
                    print(f"[+] Detected game version {gv} from {c}")
                    break

    if not game_version and MANIFEST_FILE.is_file():
        try:
            curr = json.loads(MANIFEST_FILE.read_text())
            game_version = curr.get("game_version")
            if not prime_version:
                prime_version = curr.get("prime_version")
            print(f"[*] Using existing manifest game version: {game_version}")
        except Exception:
            pass

    if not game_version:
        game_version = "261031.63733.533021"
        print(f"[!] Falling back to default version {game_version}")

    if not prime_version:
        prime_version = "++Prime+Update60-CL-529371"

    deploy_files = {}

    for name, p in [("TAGame.upk", tagame_path), ("ProjectX.upk", projectx_path)]:
        if p and p.is_file():
            dest = PREPATCHED_DIR / name
            # If coming from incoming directory, move or copy
            if p.parent != PREPATCHED_DIR:
                if dest.is_file():
                    shutil.copy2(dest, PREPATCHED_DIR / f"{name}.bak")
                tmp_dest = PREPATCHED_DIR / f"{name}.tmp_{int(time.time())}"
                shutil.copy2(p, tmp_dest)
                os.replace(tmp_dest, dest)
                print(f"[+] Deployed {p} -> {dest}")

            size = dest.stat().st_size
            sha256 = compute_sha256(dest)
            deploy_files[name] = {
                "sha256": sha256,
                "size": size,
                "url": f"https://api.velocityrl.tech/v2/rl/prepatched/{name}",
            }
            print(f"    - {name}: {sha256} ({size / (1024*1024):.2f} MB)")

    # Read existing manifest to preserve any untouched files
    existing_files = {}
    if MANIFEST_FILE.is_file():
        try:
            ex_data = json.loads(MANIFEST_FILE.read_text())
            existing_files = ex_data.get("files", {})
        except Exception:
            pass

    for k, v in existing_files.items():
        if k not in deploy_files:
            deploy_files[k] = v

    manifest_data = {
        "game_version": game_version,
        "prime_version": prime_version,
        "feature_set": args.feature_set,
        "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "files": deploy_files,
    }

    # Atomic write of manifest.json
    tmp_manifest = PREPATCHED_DIR / f"manifest.json.tmp_{int(time.time())}"
    tmp_manifest.write_text(json.dumps(manifest_data, indent=2), encoding="utf-8")
    os.replace(tmp_manifest, MANIFEST_FILE)
    print(f"[+] Updated manifest: {MANIFEST_FILE}")

    # Restart velrlapi service
    print("[*] Restarting velrlapi.service...")
    subprocess.run(["systemctl", "restart", "velrlapi"], check=True)
    res = subprocess.run(["systemctl", "is-active", "velrlapi"], capture_output=True, text=True)
    print(f"[+] velrlapi.service is: {res.stdout.strip()}")

    print("\n[SUCCESS] Manifest and UPKs successfully updated and live.")


if __name__ == "__main__":
    main()
