#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
visual_dir="$(mktemp -d)"
trap 'rm -rf "$visual_dir"' EXIT
cd "$repo_dir"
base_ref="${CHUHSHELL_VISUAL_BASE:-HEAD}"
if [[ "$base_ref" =~ ^0+$ ]]; then base_ref=HEAD^; fi
base_commit="$(git rev-parse --verify "$base_ref^{commit}")"
artifact_dir="${CHUHSHELL_VISUAL_ARTIFACTS:-$repo_dir/target/visual-artifacts}"
mkdir -p "$visual_dir/base" "$artifact_dir/baselines" "$artifact_dir/screenshots"
python - "$artifact_dir" <<'PYCODE'
from pathlib import Path
import sys
root = Path(sys.argv[1])
for capture in ["launcher", "launcher-pin", "launcher-sort"]:
    for folder in ["baselines", "screenshots"]:
        (root / folder / (capture + ".png")).unlink(missing_ok=True)
PYCODE
git archive "$base_commit" | tar -x -C "$visual_dir/base"
printf '%s\n' "$base_commit" > "$artifact_dir/visual-base.txt"
font_file="$(fc-match -f '%{file}' 'DejaVu Sans')"
if [[ "$(fc-match -f '%{family}' 'DejaVu Sans')" != 'DejaVu Sans' ]]; then
    printf '%s\n' 'visual checks require ttf-dejavu' >&2
    exit 1
fi
mkdir -p "$visual_dir/fonts"
cp "$font_file" "$visual_dir/fonts/"
cat > "$visual_dir/fonts.conf" <<EOF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
<dir>$visual_dir/fonts</dir>
<cachedir>$visual_dir/font-cache</cachedir>
<alias><family>sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>
<alias><family>monospace</family><prefer><family>DejaVu Sans</family></prefer></alias>
</fontconfig>
EOF
export FONTCONFIG_FILE="$visual_dir/fonts.conf"
export LC_ALL=C.UTF-8 TZ=UTC GTK_THEME=Adwaita
python - "$visual_dir/base/scripts/check-headless.sh" "$repo_dir/target/visual-base" <<'PYCODE'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text().replace('$repo_dir/target/', sys.argv[2] + '/')
if 'wlr-randr --output HEADLESS-1' not in text:
    text = text.replace('export WAYLAND_DISPLAY=wayland-0', 'export WAYLAND_DISPLAY=wayland-0\nwlr-randr --output HEADLESS-1 --custom-mode 1280x720@60Hz --scale 1 --pos 0,0 --output HEADLESS-2 --custom-mode 1280x720@60Hz --scale 1 --pos 1280,0')
path.write_text(text)
PYCODE
export CARGO_TARGET_DIR="$repo_dir/target/visual-base"
if [[ ! -d "$CARGO_TARGET_DIR/debug" && -d "$repo_dir/target/debug" ]]; then
    mkdir -p "$CARGO_TARGET_DIR"
    cp -a --reflink=auto "$repo_dir/target/debug" "$CARGO_TARGET_DIR/"
fi
cargo clean --manifest-path "$visual_dir/base/Cargo.toml" --package chuhshell
CHUHSHELL_TEST_SCREENSHOTS="$artifact_dir/baselines" bash "$visual_dir/base/scripts/check-headless.sh"
unset CARGO_TARGET_DIR
CHUHSHELL_TEST_VISUAL_STABLE=1 CHUHSHELL_TEST_BASELINES="$artifact_dir/baselines" CHUHSHELL_TEST_SCREENSHOTS="$artifact_dir/screenshots" bash scripts/check-headless.sh
