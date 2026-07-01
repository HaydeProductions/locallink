#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/dist/LocalLink/LocalLinkDebugger.exe"

if [[ ! -f "$APP" ]]; then
  echo "LocalLinkDebugger.exe was not found at: $APP" >&2
  echo "Run git launch or ./scripts/build.sh first." >&2
  exit 1
fi

echo "Starting LocalLink Debugger..."
"$APP" >/dev/null 2>&1 &
