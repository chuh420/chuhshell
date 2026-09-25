#!/usr/bin/env bash
set -euo pipefail

username="$(id -un)"
home_dir="$(getent passwd "$username" | cut -d: -f6)"
profile="$home_dir/.zprofile"
dropin_dir="/etc/systemd/system/getty@tty1.service.d"
dropin="$dropin_dir/20-autologin.conf"

if [[ -z "$home_dir" || ! -d "$home_dir" ]]; then
    printf 'Could not find the home directory for %s\n' "$username" >&2
    exit 1
fi

sudo install -d -m 0755 "$dropin_dir"
printf '[Service]\nExecStart=\nExecStart=-/usr/bin/agetty --autologin %s --noclear %%I $TERM\n' "$username" | sudo tee "$dropin" >/dev/null

touch "$profile"
session_start='if [[ -o interactive && "$TTY" == /dev/tty1 && -z "$WAYLAND_DISPLAY" && -z "$DISPLAY" ]]; then
    exec niri --session
fi'

if ! grep -Fq 'exec niri --session' "$profile"; then
    printf '\n%s\n' "$session_start" >>"$profile"
fi

sudo systemctl daemon-reload
sudo systemctl enable getty@tty1.service

printf 'Configured automatic login for %s on tty1 and Niri session startup. Reboot to activate it.\n' "$username"
