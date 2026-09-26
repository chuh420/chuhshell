#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
test_dir="$(mktemp -d)"
cleanup() {
    if [[ -n "${compositor_pid:-}" ]]; then kill "$compositor_pid" 2>/dev/null || true; wait "$compositor_pid" 2>/dev/null || true; fi
    rm -rf "$test_dir"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$test_dir/runtime"
export XDG_CONFIG_HOME="$test_dir/config"
export XDG_DATA_HOME="$test_dir/data"
export XDG_STATE_HOME="$test_dir/state"
export XDG_DATA_DIRS="$test_dir/empty"
export NIRI_SOCKET="$test_dir/no-niri.sock"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME/chuhshell" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$test_dir/labwc" "$test_dir/bin"
chmod 700 "$XDG_RUNTIME_DIR"
printf '%s\n' '{"notification_history_limit":10}' > "$XDG_CONFIG_HOME/chuhshell/config.json"
for helper in pactl udevadm; do
    printf '#!/bin/sh\nexec sleep 300\n' > "$test_dir/bin/$helper"
    chmod +x "$test_dir/bin/$helper"
done
printf '#!/bin/sh\nprintf "Volume: 0.4\\n"\n' > "$test_dir/bin/wpctl"
printf '#!/bin/sh\nexit 1\n' > "$test_dir/bin/nmcli"
chmod +x "$test_dir/bin/wpctl" "$test_dir/bin/nmcli"
export PATH="$test_dir/bin:$PATH"
export GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas GTK_A11Y=none GDK_DEBUG=no-portals
export WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=2 WLR_RENDERER=pixman GSK_RENDERER=cairo
unset WAYLAND_DISPLAY DISPLAY
labwc -C "$test_dir/labwc" > "$test_dir/compositor.log" 2>&1 &
compositor_pid=$!
for attempt in $(seq 1 100); do
    if [[ -S "$XDG_RUNTIME_DIR/wayland-0" ]]; then break; fi
    if ! kill -0 "$compositor_pid" 2>/dev/null; then cat "$test_dir/compositor.log"; exit 1; fi
    sleep 0.1
done
export WAYLAND_DISPLAY=wayland-0
cd "$repo_dir"
dbus-run-session -- cargo test --locked -- ui_regressions --ignored --test-threads=1 --nocapture
