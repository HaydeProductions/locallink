#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

bash "$ROOT/scripts/kill-core.sh"
bash "$ROOT/scripts/build.sh"
bash "$ROOT/scripts/run.sh"
