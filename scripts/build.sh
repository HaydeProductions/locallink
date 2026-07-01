#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/LocalLink"
DEFAULT_ADDONS_ROOT="$ROOT/addons"

resolve_appdata_root() {
  local root=""

  if command -v powershell.exe >/dev/null 2>&1; then
    root="$(powershell.exe -NoProfile -Command '[Environment]::GetFolderPath("ApplicationData")' 2>/dev/null | tr -d '\r' | tail -n 1)"
  fi

  if [[ -z "$root" ]]; then
    root="${APPDATA:-${LOCALAPPDATA:-}}"
  fi

  if [[ -n "$root" ]] && command -v cygpath >/dev/null 2>&1; then
    root="$(cygpath -u "$root" 2>/dev/null || printf '%s' "$root")"
  fi

  printf '%s' "$root"
}

APPDATA_ROOT="$(resolve_appdata_root)"

json_value() {
  local file="$1"
  local key="$2"
  sed -nE "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"([^\"]+)\".*/\1/p" "$file" | head -n 1
}

cd "$ROOT"

echo "Building LocalLink workspace release..."
cargo build --release

echo "Packaging LocalLink..."
rm -rf "$DIST"
mkdir -p "$DIST/addons"
mkdir -p "$DIST/scripts"

cp "$ROOT/target/release/locallink-core.exe" "$DIST/locallink-core.exe"
cp "$ROOT/target/release/locallink-ui.exe" "$DIST/LocalLink.exe"
cp "$ROOT/target/release/locallink-tray.exe" "$DIST/LocalLinkTray.exe"
cp "$ROOT/scripts/windows-network-check.ps1" "$DIST/scripts/windows-network-check.ps1"
cp "$ROOT/scripts/windows-network-setup.ps1" "$DIST/scripts/windows-network-setup.ps1"
cp "$ROOT/scripts/windows-network-repair.ps1" "$DIST/scripts/windows-network-repair.ps1"

shopt -s nullglob
addon_manifests=("$DEFAULT_ADDONS_ROOT"/*/manifest.json)
shopt -u nullglob

if [[ ${#addon_manifests[@]} -eq 0 ]]; then
  echo "No add-on manifests found under $DEFAULT_ADDONS_ROOT" >&2
  exit 1
fi

if [[ -z "$APPDATA_ROOT" ]]; then
  echo "APPDATA/LOCALAPPDATA is not set; cannot install add-ons for the running app." >&2
  echo "Add-ons will still be packaged in dist." >&2
else
  USER_ADDONS="$APPDATA_ROOT/LocalLink/addons"
  mkdir -p "$USER_ADDONS"
  echo "Resolved LocalLink user add-ons directory: $USER_ADDONS"
fi

for manifest in "${addon_manifests[@]}"; do
  addon_source_dir="$(dirname "$manifest")"
  id="$(json_value "$manifest" id)"
  exe="$(json_value "$manifest" executable)"

  if [[ -z "$id" || -z "$exe" ]]; then
    echo "Add-on manifest must contain id and executable: $manifest" >&2
    exit 1
  fi

  built_exe="$ROOT/target/release/$exe"
  dist_addon_dir="$DIST/addons/$id"

  if [[ ! -f "$built_exe" ]]; then
    echo "Built add-on binary not found: $built_exe" >&2
    echo "Manifest: $manifest" >&2
    exit 1
  fi

  mkdir -p "$dist_addon_dir"
  cp "$built_exe" "$dist_addon_dir/$exe"
  cp "$manifest" "$dist_addon_dir/manifest.json"
  echo "Packaged add-on $id from $addon_source_dir to: $dist_addon_dir"

  if [[ -n "$APPDATA_ROOT" ]]; then
    user_addon_dir="$USER_ADDONS/$id"
    mkdir -p "$user_addon_dir"
    cp "$built_exe" "$user_addon_dir/$exe"
    cp "$manifest" "$user_addon_dir/manifest.json"
    echo "Installed add-on $id to: $user_addon_dir"
  fi
done

cat > "$DIST/README.txt" <<'TXT'
LocalLink development package

Run tray controller:
  ./LocalLinkTray.exe

Run UI directly:
  ./LocalLink.exe

Run core directly:
  ./locallink-core.exe

Default/debug add-ons are packaged in:
  ./addons

The build script also installs discovered add-ons into:
  %APPDATA%\LocalLink\addons

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
if [[ -n "$APPDATA_ROOT" ]]; then
  echo "Updated add-ons under: $USER_ADDONS"
fi
