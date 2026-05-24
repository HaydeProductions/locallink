#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/LocalLink"

cd "$ROOT"

echo "Stopping any running LocalLink processes..."
cmd.exe /c "taskkill /F /T /IM LocalLink.exe" >/dev/null 2>&1 || true
cmd.exe /c "taskkill /F /T /IM locallink-core.exe" >/dev/null 2>&1 || true
cmd.exe /c "taskkill /F /T /IM locallink-addon-clipboard.exe" >/dev/null 2>&1 || true
sleep 1

echo "Building LocalLink workspace release..."
cargo build --release

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
echo "Starting LocalLink..."

"$DIST/LocalLink.exe" >/dev/null 2>&1 &
