#!/usr/bin/env bash
set -euo pipefail

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

echo "Done."
