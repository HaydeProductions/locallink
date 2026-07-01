#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/LocalLink"
APPDATA_ROOT="${APPDATA:-}"

cd "$ROOT"

echo "Building LocalLink workspace release..."
cargo build --release

echo "Packaging LocalLink..."
rm -rf "$DIST"
mkdir -p "$DIST/addons/clipboard-sync"
mkdir -p "$DIST/addons/space-probe"
mkdir -p "$DIST/scripts"

cp "$ROOT/target/release/locallink-core.exe" "$DIST/locallink-core.exe"
cp "$ROOT/target/release/locallink-ui.exe" "$DIST/LocalLink.exe"
cp "$ROOT/target/release/locallink-tray.exe" "$DIST/LocalLinkTray.exe"
cp "$ROOT/target/release/locallink-addon-clipboard.exe" "$DIST/addons/clipboard-sync/locallink-addon-clipboard.exe"
cp "$ROOT/target/release/locallink-addon-space-probe.exe" "$DIST/addons/space-probe/locallink-addon-space-probe.exe"
cp "$ROOT/scripts/windows-network-check.ps1" "$DIST/scripts/windows-network-check.ps1"
cp "$ROOT/scripts/windows-network-setup.ps1" "$DIST/scripts/windows-network-setup.ps1"
cp "$ROOT/scripts/windows-network-repair.ps1" "$DIST/scripts/windows-network-repair.ps1"

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

cp "$ROOT/locallink-addon-space-probe/manifest.json" "$DIST/addons/space-probe/manifest.json"

if [[ -n "$APPDATA_ROOT" ]]; then
  USER_ADDONS="$APPDATA_ROOT/LocalLink/addons"
  mkdir -p "$USER_ADDONS/space-probe"
  cp "$ROOT/target/release/locallink-addon-space-probe.exe" "$USER_ADDONS/space-probe/locallink-addon-space-probe.exe"
  cp "$ROOT/locallink-addon-space-probe/manifest.json" "$USER_ADDONS/space-probe/manifest.json"
  echo "Installed debug add-on space-probe to: $USER_ADDONS/space-probe"
else
  echo "APPDATA is not set; packaged space-probe in dist but did not install it for the running app."
fi

cat > "$DIST/README.txt" <<'TXT'
LocalLink development package

Run tray controller:
  ./LocalLinkTray.exe

Run UI directly:
  ./LocalLink.exe

Run core directly:
  ./locallink-core.exe

Run clipboard sync addon:
  ./addons/clipboard-sync/locallink-addon-clipboard.exe

Run space probe debug addon:
  ./addons/space-probe/locallink-addon-space-probe.exe

Space probe logs:
  %LOCALAPPDATA%\LocalLink\logs\space-probe-*.log
  or %APPDATA%\LocalLink\logs\space-probe-*.log

Network setup scripts:
  ./scripts/windows-network-check.ps1
  ./scripts/windows-network-repair.ps1
  ./scripts/windows-network-setup.ps1

AppData:
  %APPDATA%\LocalLink
TXT

echo "Built package at: $DIST"
