#!/usr/bin/env bash
set -euo pipefail
binary="$HOME/.local/bin/chuhshell"
service="${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services/org.freedesktop.Notifications.service"
[[ -x "$binary" ]] || { printf 'Install chuhshell first using scripts/install.sh.\n' >&2; exit 1; }
install -d "$(dirname "$service")"
if [[ -f "$service" && ! -f "$service.chuhshell-backup" ]]; then cp -p "$service" "$service.chuhshell-backup"; fi
printf '[D-BUS Service]\nName=org.freedesktop.Notifications\nExec="%s"\n' "$binary" > "$service"
if systemctl --user cat chuhshell.service >/dev/null 2>&1; then printf 'SystemdService=chuhshell.service\n' >> "$service"; fi
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null
printf 'Installed %s\n' "$service"
