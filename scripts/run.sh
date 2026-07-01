#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/dist/LocalLink/LocalLink.exe"

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

if [[ ! -f "$APP" ]]; then
  echo "LocalLink.exe was not found at: $APP" >&2
  echo "Run ./scripts/build-run.sh first." >&2
  exit 1
fi

APPDATA_ROOT="$(resolve_appdata_root)"
if [[ -n "$APPDATA_ROOT" ]]; then
  LOG_DIR="$APPDATA_ROOT/LocalLink/logs"
else
  LOG_DIR="$ROOT/.locallink-dev-logs"
fi
mkdir -p "$LOG_DIR"
UI_LOG="$LOG_DIR/ui-process-$(date +%Y%m%d-%H%M%S).log"

echo "Starting LocalLink..."
echo "UI stdout/stderr log: $UI_LOG"
"$APP" >"$UI_LOG" 2>&1 &
