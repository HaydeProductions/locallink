#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/dist/LocalLink/LocalLink.exe"

if [[ ! -f "$APP" ]]; then
  echo "LocalLink.exe was not found at: $APP" >&2
  echo "Run ./scripts/build-run.sh first." >&2
  exit 1
fi

echo "Starting LocalLink..."
"$APP" >/dev/null 2>&1 &
