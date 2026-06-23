#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ ! -d .git ]]; then
  echo "This script must be run from inside a Git checkout." >&2
  exit 1
fi

echo "Installing repo-local Git shortcuts..."

git config --local alias.launch '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/build-run.sh"; }; f'
git config --local alias.build '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/build.sh"; }; f'
git config --local alias.run '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/run.sh"; }; f'
git config --local alias.kill '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/kill-core.sh"; }; f'
git config --local alias.net-check '!f() { root=$(git rev-parse --show-toplevel) && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$root/scripts/windows-network-check.ps1" "$@"; }; f'
git config --local alias.net-repair '!f() { root=$(git rev-parse --show-toplevel) && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$root/scripts/windows-network-repair.ps1" "$@"; }; f'
git config --local alias.net-setup '!f() { root=$(git rev-parse --show-toplevel) && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$root/scripts/windows-network-setup.ps1" "$@"; }; f'
git config --local alias.aliases "!git config --local --get-regexp '^alias\\.' | sed 's/^alias\\.//'"

git config --local --unset alias.build-local >/dev/null 2>&1 || true
git config --local --unset alias.run-local >/dev/null 2>&1 || true
git config --local --unset alias.kill-local >/dev/null 2>&1 || true

echo "Done. Available from any subfolder in this repo:"
echo "  git launch      # kill -> build -> run"
echo "  git build       # build/package only"
echo "  git run         # run built UI only"
echo "  git kill        # stop LocalLink processes"
echo "  git net-check   # inspect Windows LocalLink network requirements"
echo "  git net-repair  # check requirements and request admin repair if needed"
echo "  git net-setup   # run the Windows network setup script directly"
echo "  git aliases     # list repo-local Git shortcuts"
