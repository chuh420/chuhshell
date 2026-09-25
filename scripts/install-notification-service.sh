#!/usr/bin/env bash
set -euo pipefail

binary="$HOME/.local/bin/chuhshell"
service="$HOME/.local/share/dbus-1/services/org.freedesktop.Notifications.service"

if [[ ! -x "$binary" ]]; then
    printf 'Install chuhshell at %s before enabling D-Bus activation.\n' "$binary" >&2
    exit 1
fi

install -d -m 0755 "$(dirname "$service")"
printf '[D-BUS Service]\nName=org.freedesktop.Notifications\nExec=%s\n' "$binary" >"$service"
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null
printf 'Installed %s\n' "$service"
