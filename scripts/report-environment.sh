#!/usr/bin/env bash
set -euo pipefail
printf 'date: '
date --iso-8601=seconds
uname -a
rustc -Vv
cargo -V
pacman -Q || true
for program in labwc niri grim wlr-randr; do
    "$program" --version 2>&1 || true
done
