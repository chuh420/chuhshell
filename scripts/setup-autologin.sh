#!/usr/bin/env bash
set -euo pipefail
username="$(id -un)"
if [[ "$EUID" -eq 0 ]]; then
    printf 'Run this script as the desktop user; it invokes sudo where needed.\n' >&2
    exit 1
fi
entry="$(getent passwd "$username")"
home_dir="$(cut -d: -f6 <<< "$entry")"
login_shell="$(cut -d: -f7 <<< "$entry")"
if [[ "$login_shell" != */zsh || ! -d "$home_dir" ]]; then
    printf 'This setup requires a zsh login account with an existing home directory.\n' >&2
    exit 1
fi
profile="$home_dir/.zprofile"
dropin_dir=/etc/systemd/system/getty@tty1.service.d
dropin="$dropin_dir/20-autologin.conf"
backup_dir="${XDG_STATE_HOME:-$home_dir/.local/state}/chuhshell/autologin-backup"
if [[ "${1:-}" == --undo ]]; then
    [[ -f "$backup_dir/installed" ]] || { printf 'No autologin backup found.\n' >&2; exit 1; }
    if [[ -f "$backup_dir/profile.sha256" ]] && ! sha256sum --status --check "$backup_dir/profile.sha256"; then
        printf 'The login profile changed since setup; preserve it and remove the Niri startup block manually.\n' >&2
        exit 1
    fi
    if [[ -f "$backup_dir/dropin.sha256" ]] && ! sudo sha256sum --status --check "$backup_dir/dropin.sha256"; then
        printf 'The getty drop-in changed since setup; refusing to overwrite it.\n' >&2
        exit 1
    fi
    if [[ -f "$backup_dir/profile" ]]; then cp -p "$backup_dir/profile" "$profile"; elif [[ -f "$backup_dir/profile-created" ]]; then rm -f "$profile"; fi
    if [[ -f "$backup_dir/dropin" ]]; then sudo install -m644 "$backup_dir/dropin" "$dropin"; else sudo rm -f "$dropin"; fi
    sudo systemctl daemon-reload
    rm -f "$backup_dir/installed"
    printf 'Restored the previous login configuration.\n'
    exit 0
fi
[[ $# -eq 0 ]] || { printf 'Usage: setup-autologin.sh [--undo]\n' >&2; exit 1; }
command -v niri >/dev/null
if [[ -f "$backup_dir/installed" ]]; then
    printf 'Autologin is already managed by this script.\n'
    exit 0
fi
install -d -m700 "$backup_dir"
rm -f "$backup_dir/profile" "$backup_dir/profile-created" "$backup_dir/dropin"
if [[ -f "$profile" ]]; then cp -p "$profile" "$backup_dir/profile"; else touch "$backup_dir/profile-created"; fi
if [[ -f "$dropin" ]]; then sudo cat "$dropin" > "$backup_dir/dropin"; fi
sudo install -d -m755 "$dropin_dir"
printf '[Service]\nExecStart=\nExecStart=-/usr/bin/agetty --autologin %s --noclear %%I $TERM\n' "$username" | sudo tee "$dropin" >/dev/null
if ! [[ -f "$profile" ]] || ! grep -Fq 'exec niri --session' "$profile"; then
    printf '\nif [[ -o interactive && "$TTY" == /dev/tty1 && -z "$WAYLAND_DISPLAY" && -z "$DISPLAY" ]]; then\n    exec niri --session\nfi\n' >> "$profile"
fi
sha256sum "$profile" > "$backup_dir/profile.sha256"
sudo sha256sum "$dropin" > "$backup_dir/dropin.sha256"
sudo systemctl daemon-reload
sudo systemctl enable getty@tty1.service
touch "$backup_dir/installed"
printf 'Configured tty1 autologin for %s. Reboot to activate; use --undo to restore backups.\n' "$username"
