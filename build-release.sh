#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

VERSION_FILE="$SCRIPT_DIR/VERSION"
if [[ ! -f "$VERSION_FILE" ]]; then
    echo "ERROR: VERSION file not found at repo root." >&2
    exit 1
fi

VERSION="$(tr -d '[:space:]' < "$VERSION_FILE")"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "ERROR: VERSION file contains '$VERSION' - expected semver like 2.1.0" >&2
    exit 1
fi
echo "==> Version: $VERSION"

# Sync tauri.conf.json
CONF_PATH="$SCRIPT_DIR/src-tauri/tauri.conf.json"
if [[ -f "$CONF_PATH" ]]; then
    sed -i -E "s/(\"version\":[[:space:]]*\")[^\"]+(\")/\1$VERSION\2/" "$CONF_PATH"
    echo "    tauri.conf.json synced to $VERSION"
fi

# Sync Cargo.toml
CARGO_PATH="$SCRIPT_DIR/src-tauri/Cargo.toml"
if [[ -f "$CARGO_PATH" ]]; then
    sed -i -E "0,/^version = \"[^\"]+\"/ s/^version = \"[^\"]+\"/version = \"$VERSION\"/" "$CARGO_PATH"
    echo "    Cargo.toml synced to $VERSION"
fi

# Sync package.json
PKG_PATH="$SCRIPT_DIR/package.json"
if [[ -f "$PKG_PATH" ]]; then
    sed -i -E "s/(\"version\":[[:space:]]*\")[^\"]+(\")/\1$VERSION\2/" "$PKG_PATH"
    echo "    package.json synced to $VERSION"
fi

echo "==> Building VelocityRL Linux..."
if command -v npm &> /dev/null && [ -d "$SCRIPT_DIR/node_modules" ]; then
    npm run tauri build "$@"
else
    cd "$SCRIPT_DIR/src-tauri"
    cargo build --release "$@"
fi
