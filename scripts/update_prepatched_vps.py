#!/usr/bin/env python3
"""
VelocityRL Pre-patched UPK Updater & VPS Deployer
=================================================
Automates publishing updated pre-patched UPK files (e.g., TAGame.upk, ProjectX.upk)
to the VelocityRL API server for seamless client auto-updates.

Features:
- Auto-detects UPK files placed in `prepatched_incoming/` or specified via CLI
- Auto-detects Rocket League game version from:
    1. RocketLeague.exe in incoming folder
    2. Local Epic Games / Steam installation
    3. Manual --version flag
- Computes SHA-256 and byte sizes
- Securely uploads to VPS via SFTP with live progress bars
- Backs up previous UPKs on the server
- Atomically deploys new files and updates `/root/velrlapi/prepatched/manifest.json`
- Restarts `velrlapi.service` via SSH
- Performs live verification against https://api.velocityrl.tech/v2/rl/prepatched/manifest

Usage:
    # 1. Place TAGame.upk (and optionally ProjectX.upk / RocketLeague.exe) in prepatched_incoming/
    python scripts/update_prepatched_vps.py

    # Or specify custom paths:
    python scripts/update_prepatched_vps.py --tagame path/to/TAGame.upk --version 261031.63733.533021
"""

import argparse
import hashlib
import json
import os
import re
import sys
import time
import urllib.request
from pathlib import Path
from typing import Optional, Tuple, Dict, Any

try:
    import paramiko
except ImportError:
    print("[ERROR] 'paramiko' is required. Install it via: pip install paramiko")
    sys.exit(1)

# Default VPS credentials
DEFAULT_HOST = os.getenv("VRL_VPS_HOST", "")
DEFAULT_PORT = int(os.getenv("VRL_VPS_PORT", "22"))
DEFAULT_USER = os.getenv("VRL_VPS_USER", "")
DEFAULT_PASS = os.getenv("VRL_VPS_PASS")
DEFAULT_REMOTE_DIR = "/root/velrlapi/prepatched"
API_MANIFEST_URL = "https://api.velocityrl.tech/v2/rl/prepatched/manifest"

KNOWN_LOCAL_RL_PATHS = [
    Path(r"C:\Program Files\Epic Games\rocketleague\Binaries\Win64\RocketLeague.exe"),
    Path(r"C:\Program Files (x86)\Steam\steamapps\common\rocketleague\Binaries\Win64\RocketLeague.exe"),
    Path(r"D:\SteamLibrary\steamapps\common\rocketleague\Binaries\Win64\RocketLeague.exe"),
    Path(r"E:\SteamLibrary\steamapps\common\rocketleague\Binaries\Win64\RocketLeague.exe"),
]


def extract_version_from_exe(exe_path: Path) -> Tuple[Optional[str], Optional[str]]:
    r"""Extract game_version (\d{6}\.\d{5}\.\d{6}) and prime_version from RocketLeague.exe."""
    if not exe_path.is_file():
        return None, None

    print(f"[*] Scanning binary for game version: {exe_path}")
    try:
        with open(exe_path, "rb") as f:
            data = f.read()

        # 1. UTF-16LE version pattern: 6 digits . 5 digits . 6 digits
        m = re.search(rb'(?:[\x30-\x39]\x00){6}\.\x00(?:[\x30-\x39]\x00){5}\.\x00(?:[\x30-\x39]\x00){6}', data)
        game_version = m.group(0).decode("utf-16le") if m else None

        # 2. Prime version ASCII pattern
        m_prime = re.search(rb'\+\+Prime\+Update[0-9a-zA-Z\.\_\-\+]+', data)
        prime_version = m_prime.group(0).decode("ascii", errors="ignore") if m_prime else None

        return game_version, prime_version
    except Exception as e:
        print(f"[!] Warning: Failed to extract version from {exe_path}: {e}")
        return None, None


def find_local_rl_exe(incoming_dir: Path) -> Optional[Path]:
    """Find RocketLeague.exe in incoming dir or standard system paths."""
    incoming_exe = incoming_dir / "RocketLeague.exe"
    if incoming_exe.is_file():
        return incoming_exe

    for p in KNOWN_LOCAL_RL_PATHS:
        if p.is_file():
            return p

    return None


def compute_sha256(path: Path) -> str:
    """Compute SHA-256 hash in 64KB chunks."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest().lower()


class SFTPProgressBar:
    def __init__(self, filename: str, total_bytes: int):
        self.filename = filename
        self.total_bytes = total_bytes
        self.start_time = time.time()
        self.last_update = 0

    def __call__(self, transferred: int, total: int):
        now = time.time()
        if now - self.last_update < 0.2 and transferred < total:
            return
        self.last_update = now

        pct = (transferred / total) * 100 if total else 0
        elapsed = now - self.start_time
        speed = (transferred / (1024 * 1024)) / elapsed if elapsed > 0 else 0
        mb_trans = transferred / (1024 * 1024)
        mb_tot = total / (1024 * 1024)

        bar_len = 30
        filled = int(bar_len * (transferred / total)) if total else 0
        bar = "=" * filled + ">" + " " * (bar_len - filled - 1) if filled < bar_len else "=" * bar_len

        sys.stdout.write(f"\r  Uploading {self.filename}: [{bar}] {pct:5.1f}% ({mb_trans:.1f}/{mb_tot:.1f} MB, {speed:.2f} MB/s)")
        sys.stdout.flush()
        if transferred >= total:
            sys.stdout.write("\n")


def parse_args():
    parser = argparse.ArgumentParser(description="VelocityRL Pre-patched UPK Updater & VPS Deployer")
    repo_root = Path(__file__).resolve().parent.parent
    default_incoming = repo_root / "prepatched_incoming"

    parser.add_argument("--incoming", "-i", type=Path, default=default_incoming,
                        help=f"Directory containing new UPKs (default: {default_incoming})")
    parser.add_argument("--tagame", type=Path, default=None,
                        help="Path to new TAGame.upk")
    parser.add_argument("--projectx", type=Path, default=None,
                        help="Path to new ProjectX.upk")
    parser.add_argument("--exe", type=Path, default=None,
                        help="Path to RocketLeague.exe to extract version")
    parser.add_argument("--version", "-v", type=str, default=None,
                        help="Rocket League game version string (e.g. 261031.63733.533021)")
    parser.add_argument("--prime-version", type=str, default=None,
                        help="Rocket League prime version (e.g. ++Prime+Update60-CL-529371)")
    parser.add_argument("--feature-set", type=str, default="PrimeUpdate66313",
                        help="Feature set identifier (default: PrimeUpdate66313)")
    parser.add_argument("--host", default=DEFAULT_HOST, help="VPS IP/host")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT, help="VPS SSH port")
    parser.add_argument("--user", default=DEFAULT_USER, help="VPS SSH user")
    parser.add_argument("--password", default=DEFAULT_PASS, help="VPS SSH password")
    parser.add_argument("--dry-run", action="store_true",
                        help="Compute hashes and preview manifest without uploading")
    return parser.parse_args()


def main():
    print("=" * 70)
    print("   VelocityRL Pre-patched UPK Updater & Server Deployer")
    print("=" * 70)

    args = parse_args()
    incoming = args.incoming
    incoming.mkdir(parents=True, exist_ok=True)

    # 1. Resolve UPK files
    tagame_path = args.tagame or (incoming / "TAGame.upk" if (incoming / "TAGame.upk").is_file() else None)
    projectx_path = args.projectx or (incoming / "ProjectX.upk" if (incoming / "ProjectX.upk").is_file() else None)

    if not tagame_path and not projectx_path:
        print(f"[!] No UPK files found in '{incoming}' and none passed via CLI.")
        print("    Please place TAGame.upk and/or ProjectX.upk into:")
        print(f"    --> {incoming.resolve()}\n")
        sys.exit(1)

    # 2. Resolve game version
    game_version = args.version
    prime_version = args.prime_version

    if not game_version:
        exe_path = args.exe or find_local_rl_exe(incoming)
        if exe_path:
            gv, pv = extract_version_from_exe(exe_path)
            if gv:
                game_version = gv
                if not prime_version and pv:
                    prime_version = pv
                print(f"[+] Auto-detected Game Version:  {game_version}")
                if prime_version:
                    print(f"[+] Auto-detected Prime Version: {prime_version}")

    if not game_version:
        print("[!] Could not auto-detect Rocket League version.")
        try:
            game_version = input("Enter Rocket League Game Version (e.g. 261031.63733.533021): ").strip()
        except EOFError:
            pass
        if not game_version:
            print("[ERROR] Game version is required to tie UPKs to the Rocket League build.")
            sys.exit(1)

    if not prime_version:
        prime_version = "++Prime+Update60-CL-529371"

    # 3. Analyze local files
    deploy_files: Dict[str, Dict[str, Any]] = {}

    for name, path in [("TAGame.upk", tagame_path), ("ProjectX.upk", projectx_path)]:
        if path and path.is_file():
            size = path.stat().st_size
            if size < 100_000:
                print(f"[ERROR] {name} is suspiciously small ({size} bytes). Aborting.")
                sys.exit(1)
            print(f"[*] Hashing {name} ({size / (1024*1024):.2f} MB)...")
            sha256 = compute_sha256(path)
            deploy_files[name] = {
                "local_path": path,
                "size": size,
                "sha256": sha256,
                "url": f"https://api.velocityrl.tech/v2/rl/prepatched/{name}",
            }
            print(f"    -> SHA256: {sha256}")

    # Build manifest
    manifest_data = {
        "game_version": game_version,
        "prime_version": prime_version,
        "feature_set": args.feature_set,
        "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "files": {
            name: {
                "sha256": info["sha256"],
                "size": info["size"],
                "url": info["url"],
            }
            for name, info in deploy_files.items()
        }
    }

    print("\nTarget Manifest Preview:")
    print(json.dumps(manifest_data, indent=2))

    if args.dry_run:
        print("\n[DRY RUN] Skipping deployment to VPS.")
        return

    # 4. Connect to VPS
    print(f"\n[*] Connecting to VPS {args.host}:{args.port} as {args.user}...")
    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    try:
        ssh.connect(args.host, port=args.port, username=args.user, password=args.password, timeout=15)
    except Exception as e:
        print(f"[ERROR] Failed to connect to VPS: {e}")
        sys.exit(1)

    sftp = ssh.open_sftp()
    remote_dir = DEFAULT_REMOTE_DIR

    # Ensure remote directory exists
    try:
        sftp.stat(remote_dir)
    except IOError:
        sftp.mkdir(remote_dir)

    # If updating only one file, fetch existing manifest on server to merge
    remote_manifest_path = f"{remote_dir}/manifest.json"
    try:
        with sftp.file(remote_manifest_path, "r") as f:
            existing = json.loads(f.read().decode("utf-8"))
            if "files" in existing:
                for k, v in existing["files"].items():
                    if k not in manifest_data["files"]:
                        print(f"[*] Preserving existing manifest entry for {k}")
                        manifest_data["files"][k] = v
    except Exception:
        pass

    # 5. Upload files atomically
    for name, info in deploy_files.items():
        local_p = info["local_path"]
        remote_final = f"{remote_dir}/{name}"
        remote_tmp = f"{remote_dir}/{name}.upload_tmp_{int(time.time())}"
        remote_bak = f"{remote_dir}/{name}.bak"

        # Check existing remote file
        try:
            r_stat = sftp.stat(remote_final)
            # Create backup on server
            ssh.exec_command(f"cp -f {remote_final} {remote_bak}")
        except IOError:
            pass

        progress = SFTPProgressBar(name, info["size"])
        sftp.put(str(local_p), remote_tmp, callback=progress)

        # Atomic rename on VPS
        sftp.posix_rename(remote_tmp, remote_final)
        print(f"  [+] Deployed {name} -> {remote_final}")

    # 6. Upload new manifest.json atomically
    manifest_tmp = f"{remote_manifest_path}.tmp_{int(time.time())}"
    with sftp.file(manifest_tmp, "w") as f:
        f.write(json.dumps(manifest_data, indent=2))
    sftp.posix_rename(manifest_tmp, remote_manifest_path)
    print(f"  [+] Updated manifest -> {remote_manifest_path}")

    sftp.close()

    # 7. Restart API service on VPS
    print("[*] Restarting velrlapi.service on VPS...")
    stdin, stdout, stderr = ssh.exec_command("systemctl restart velrlapi && systemctl is-active velrlapi")
    out = stdout.read().decode().strip()
    if out == "active":
        print("  [+] velrlapi.service restarted successfully (status: active)")
    else:
        print(f"  [!] Service status output: {out}")
        err = stderr.read().decode().strip()
        if err:
            print(f"  [!] Service error: {err}")

    ssh.close()

    # 8. Live Public API Verification
    print(f"[*] Verifying live manifest via {API_MANIFEST_URL}...")
    time.sleep(1.0)
    try:
        req = urllib.request.Request(API_MANIFEST_URL, headers={"User-Agent": "VelocityRL-Deployer"})
        with urllib.request.urlopen(req, timeout=10) as resp:
            live_data = json.loads(resp.read().decode())
            if live_data.get("game_version") == manifest_data["game_version"]:
                print("  [SUCCESS] Live API successfully updated and verified!")
                print(f"  Game Version: {live_data.get('game_version')}")
                for fname, finfo in live_data.get("files", {}).items():
                    print(f"    - {fname}: {finfo.get('sha256')} ({finfo.get('size')} bytes)")
            else:
                print(f"  [!] Warning: Live version mismatch. Expected {manifest_data['game_version']}, got {live_data.get('game_version')}")
    except Exception as e:
        print(f"  [!] Verification request failed: {e}")

    print("\n" + "=" * 70)
    print("Pre-patched UPKs deployed! Users opening VelocityRL will now auto-update.")
    print("=" * 70)


if __name__ == "__main__":
    main()
