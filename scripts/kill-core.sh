#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/LocalLink"

if command -v cygpath >/dev/null 2>&1; then
  DIST_WIN="$(cygpath -w "$DIST")"
else
  DIST_WIN="$DIST"
fi

kill_image() {
  local name="$1"

  echo "Stopping $name if running..."

  if command -v timeout >/dev/null 2>&1; then
    timeout 8s taskkill.exe //F //T //IM "$name" >/dev/null 2>&1 || true
  else
    taskkill.exe //F //T //IM "$name" >/dev/null 2>&1 || true
  fi
}

# Kill the known shipped process names first. Include the tray; otherwise the
# dist folder stays busy and rebuilds cannot remove it.
kill_image "LocalLinkTray.exe"
kill_image "LocalLink.exe"
kill_image "locallink-core.exe"
kill_image "locallink-addon-clipboard.exe"
kill_image "locallink-addon-echo.exe"

# Fallback: kill any LocalLink/add-on process Windows reports without relying
# on taskkill image-name matching. This also catches future add-ons and any
# process running from dist/LocalLink.
if command -v powershell.exe >/dev/null 2>&1; then
  powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "
    \$ErrorActionPreference = 'SilentlyContinue'
    \$dist = '$DIST_WIN'
    \$names = @(
      'LocalLinkTray',
      'LocalLink',
      'locallink-core',
      'locallink-addon-clipboard',
      'locallink-addon-echo'
    )

    Get-Process |
      Where-Object {
        \$names -contains \$_.ProcessName -or
        \$_.ProcessName -like 'locallink-addon-*' -or
        (\$_.Path -and \$_.Path.StartsWith(\$dist, [System.StringComparison]::OrdinalIgnoreCase))
      } |
      ForEach-Object {
        Write-Host ('Stopping ' + \$_.ProcessName + ' pid=' + \$_.Id)
        Stop-Process -Id \$_.Id -Force
      }
  " || true
fi

# Give Windows a moment to release executable handles before build.sh removes dist.
sleep 1

echo "Done."
