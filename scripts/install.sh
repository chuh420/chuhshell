#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo_dir"
cargo build --release --locked
exec python3 scripts/install.py install
