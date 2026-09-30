#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
exec "$repo_dir/scripts/install.sh" "$@"
