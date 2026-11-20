# changelog

this file follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-11-20

the first tagged release. chuhshell is in beta: Hyprland is the daily driver, niri and sway were checked in nested sessions only.

### added

- bar: workspaces with kanji numerals, the playing track, clock, CPU and RAM rings, keyboard layout, Wi-Fi, VPN, Bluetooth, volume, battery and tray chips, split into groups by thin separators.
- control center: resource rings, network and Bluetooth tiles, keep awake, do not disturb, microphone and wallpaper colors switches, volume and brightness sliders, a media card and shortcuts to the other panels.
- notifications: a freedesktop notification server with popups, a history panel and do not disturb.
- OSD for volume, brightness and keyboard layout.
- tray: a status notifier watcher with dbusmenu menus.
- panels for Wi-Fi (iwd or NetworkManager, with a password prompt), Bluetooth (BlueZ, with a pairing agent), Proton VPN, calendar, battery with power profiles, audio, clipboard history (cliphist), power and system info.
- launcher: desktop entry search, hidden apps, pinned apps on top (Ctrl+P or right click), apps started in their own systemd scopes.
- wallpapers: a wallpaper surface with a circle reveal, a picker with search, live video wallpapers through mpvpaper and posters made with ffmpeg.
- themes panel: nine built-in palettes, colors from the wallpaper through matugen with scheme choice, app color files rendered from matugen templates, cursor themes with size, recoloring and animation.
- MPRIS media: the bar chip and the control panel card follow the playing player.
- compositors: Hyprland, niri and sway backends, picked from the environment at startup.
- config: `~/.config/chuhshell/config.toml` with every key optional, live reload, a notification on errors, `chuhshell config default|check|path`, hooks for wallpaper and theme changes.
- IPC: `chuhshell msg <command>` over a unix socket for binds and scripts.
- session: logind suspend, reboot and power off, lock, screenshot and mixer commands from the config.
- availability: chips and panel items hide when their service or tool is missing, with one log line per missing tool.
- packaging: a systemd user unit, AUR packages `chuhshell` and `chuhshell-git`, CI with fmt, clippy and tests.

### performance

- about 32 MB RSS and 0.02 CPU-seconds per idle minute on one 1920x1080 output.
- partial damage rendering, a sliced shadow cache, direct blits for pixel-aligned rasters and one text shaping pass with a glyph cache.
- large images shrink right after decoding, and the shell trims malloc arenas after closing panels.

[0.1.0]: https://github.com/chuh420/chuhshell/releases/tag/v0.1.0
