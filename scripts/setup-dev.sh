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
git config --local alias.build-local '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/build.sh"; }; f'
git config --local alias.run-local '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/run.sh"; }; f'
git config --local alias.kill-local '!f() { root=$(git rev-parse --show-toplevel) && bash "$root/scripts/kill-core.sh"; }; f'

echo "Done. Available from any subfolder in this repo:"
echo "  git launch       # kill -> build -> run"
echo "  git build-local  # build/package only"
echo "  git run-local    # run built UI only"
echo "  git kill-local   # stop LocalLink processes"
