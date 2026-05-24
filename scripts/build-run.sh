#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/LocalLink"

cd "$ROOT"

kill_if_running() {
  local name="$1"

  echo "Stopping $name if running..."

  if command -v timeout >/dev/null 2>&1; then
    timeout 5s taskkill.exe //F //IM "$name" >/dev/null 2>&1 || true
  else
    taskkill.exe //F //IM "$name" >/dev/null 2>&1 || true
  fi
}

kill_if_running "LocalLink.exe"
kill_if_running "locallink-core.exe"
kill_if_running "locallink-addon-clipboard.exe"
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
