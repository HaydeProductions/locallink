#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

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
if [[ -n "$APPDATA_ROOT" ]]; then
  LOG_DIR="$APPDATA_ROOT/LocalLink/logs"
else
  LOG_DIR="$ROOT/.locallink-dev-logs"
fi
mkdir -p "$LOG_DIR"
DEV_LOG="$LOG_DIR/dev-launch-$(date +%Y%m%d-%H%M%S).log"

echo "LocalLink dev launch log: $DEV_LOG"
exec > >(tee -a "$DEV_LOG") 2>&1

echo "==== LocalLink dev launch started $(date -Iseconds) ===="
echo "Repo: $ROOT"
echo "Branch: $(git -C "$ROOT" branch --show-current 2>/dev/null || echo unknown)"
echo "Commit: $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo "Logs: $LOG_DIR"

bash "$ROOT/scripts/kill-core.sh"
bash "$ROOT/scripts/build.sh"
bash "$ROOT/scripts/run.sh"

echo "==== LocalLink dev launch command finished $(date -Iseconds) ===="
