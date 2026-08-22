# AGENTS.md

## Cursor Cloud specific instructions

### What this repo is
VelocityRL is a **Tauri 2 desktop app** for swapping Rocket League in-game assets. Components:
- `src-tauri/` — Rust backend (crate `velocity-rl`, lib `app_lib`). The UPK swap engine is **native Rust** (`src-tauri/src/upk/`); swaps are performed by `upk::swap_asset`, not by an external process.
- `ui/` — the frontend: plain static `index.html` / `main.js` / `style.css`. There is **no frontend build step** (the `build` npm script and `beforeBuildCommand` are no-ops).
- `python/` — a **standalone/legacy** interactive CLI (`python/cli.py` + `rl_asset_swapper.py`) and the bundled item database `items.json`. The desktop app does not invoke Python at runtime.

### Toolchain caveats (already provisioned in the base snapshot)
- **Rust must be stable ≥ 1.85.** The committed `src-tauri/Cargo.lock` pins dependencies that require `edition2024`, so the older Rust that may ship in the base image is too new-a-requirement to satisfy. The environment's default toolchain is set to `stable` via rustup. If a build ever fails with "feature `edition2024` is required", run `rustup default stable` (or `rustup toolchain install stable`).
- **Tauri Linux GUI system libraries** are installed (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`). These are needed to build/run the desktop app on Linux.

### Running the app (dev)
- `npm run dev` (= `tauri dev`). The GUI opens a native window; set `DISPLAY=:1` so it renders on the VM desktop used for computer-use. First `cargo` build takes ~1–2 min; subsequent runs are fast.
- `libEGL warning: DRI3 error ...` on startup is harmless (software-rendering fallback).
- On first run the app shows a mandatory privacy "I Agree" dialog, then a "What's New" changelog, then an optional "System Settings" (game path) dialog that can be dismissed with X. Item search/database works offline (bundled `python/items.json`) **without** configuring a game directory.

### Real swaps vs. what's testable here
Executing an actual asset swap needs a real Rocket League `CookedPCConsole` folder of encrypted `.upk` files, which is **not** available in the VM. Everything up to staging a swap (searching the item DB, selecting an owned item + target asset, enabling "Swap Items") works without game files.

### Lint / test / build
- Lint: `cd src-tauri && cargo clippy` (warnings only; nothing is denied). CI itself only runs `cargo check --release` (see `.github/workflows/ci.yml`). There is **no JS linter**.
- Tests: there are **no automated tests** in this repo.
- Build (dev): `cd src-tauri && cargo build`. Release bundling (`cargo build --release` / `tauri build`) additionally expects a sidecar binary at `src-tauri/bin/velocity-engine-<target-triple>`; this is **only** required for release bundling, not for `cargo check`/`cargo build`/`tauri dev`.

### Python CLI (optional/standalone)
`python3 python/cli.py` imports and runs without extra packages, but performing real swaps through it needs `pip install cryptography Pillow requests` (there is no `requirements.txt`).
