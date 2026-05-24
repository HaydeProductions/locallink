#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/LocalLink"

cd "$ROOT"

echo "Building LocalLink workspace release..."
cargo build --release

echo "Packaging LocalLink..."
rm -rf "$DIST"
mkdir -p "$DIST/addons/clipboard-sync"

cp "$ROOT/target/release/locallink-core.exe" "$DIST/locallink-core.exe"
cp "$ROOT/target/release/locallink-ui.exe" "$DIST/LocalLink.exe"
cp "$ROOT/target/release/locallink-addon-clipboard.exe" "$DIST/addons/clipboard-sync/locallink-addon-clipboard.exe"

cat > "$DIST/addons/clipboard-sync/manifest.json" <<'JSON'
{
  "id": "clipboard-sync",
  "name": "Clipboard Sync",
  "version": "0.1.0",
  "description": "Keeps text clipboards synchronized across connected LocalLink devices. Newest clipboard wins.",
  "executable": "locallink-addon-clipboard.exe",
  "services": [
    "clipboard-sync"
  ],
  "enabled": false
}
JSON

cat > "$DIST/README.txt" <<'TXT'
LocalLink development package

Run UI:
  ./LocalLink.exe

Run core directly:
  ./locallink-core.exe

Run clipboard sync addon:
  ./addons/clipboard-sync/locallink-addon-clipboard.exe

AppData:
  %APPDATA%\LocalLink
TXT

echo "Built package at: $DIST"
