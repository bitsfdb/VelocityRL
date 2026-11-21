# Pre-patched UPK Incoming Directory

Place newly pre-patched UPK files here when a new Rocket League update drops:

- `TAGame.upk` (required or updated)
- `ProjectX.upk` (optional, if changed)
- `RocketLeague.exe` (optional — if placed here, the updater will auto-extract the exact game version and prime version strings)

### How to Deploy to VPS:
Double-click `update_vps_upk.bat` in the repository root, or run:
```bash
python scripts/update_prepatched_vps.py
```

The script will:
1. Detect `TAGame.upk` and `ProjectX.upk` in this folder.
2. Auto-detect the Rocket League version (from `RocketLeague.exe` here, or from your local Epic Games / Steam install, or from `--version`).
3. Calculate SHA-256 hashes and file sizes.
4. Upload files to the VelocityRL API server (`api.velocityrl.tech`).
5. Update the prepatched manifest atomically.
6. Verify the live API endpoint.
