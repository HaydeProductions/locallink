#!/usr/bin/env bash
set -euo pipefail

echo "Stopping LocalLink core..."

cmd.exe /c "taskkill /F /T /IM locallink-core.exe" >/dev/null 2>&1 || true

echo "Done."
