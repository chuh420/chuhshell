# chuhshell

a personal desktop shell for niri, written in rust with gtk4. the panel, launcher, menus and notifications share one process and one look.

i made this for my own desktop. i think it should work on other people's desktops too, but i'm not sure. it only supports the niri window manager. feel free to use it, change it, share it or borrow ideas — i don't mind what you do with it.

## what it does

- a panel across multiple monitors, with workspaces, background apps, clock, notifications, volume, brightness, keyboard layout, temperature, wifi and battery.
- an app launcher with fuzzy search, pinned apps, usage-based or alphabetical sorting, and controls to hide apps.
- wifi and bluetooth controls, including pairing prompts inside the shell. enterprise wifi needs an existing networkmanager profile.
- weather from open-meteo, a calendar, saved tasks and clipboard history.
- app notification popups and a drawer, separate from the volume and brightness osd.
- editors for panel modules, launcher placement and niri keybindings. some menu sections are still marked wip.

## getting around

mod + space opens chuh menu, mod + d opens the app launcher, mod + c opens clipboard history, and ctrl + b opens the wallpaper picker.

in menus, use up/down to move, enter or right to select, left to go back and escape to close. the launcher’s pin and sort buttons also work with the keyboard.

chuh menu lets you toggle and arrange panel modules, move or resize the launcher, and edit shortcuts. save applies changes; cancel or escape discards them. niri bindings include their source files and are validated before saving.

clipboard history keeps up to 100 text and png entries in memory, never on disk. search and press enter to copy; delete removes an entry. you can pause recording or clear the history.

for wallpapers, put images in `~/Pictures/Wallpapers` and provide `~/.local/bin/wallpaper.sh`. the picker calls that script with the selected filename and shows failures.

## installation

on arch, building needs rust 1.92+, pkgconf, gtk4, gtk4-layer-shell and glib 2.80+. desktop controls use niri, wireplumber, pactl, networkmanager, brightnessctl, udevadm, bluez, wl-clipboard and curl. foot and btop are optional, for the temperature shortcut. the intended font is inputsans nerd font.

run this from the project directory in your niri session:

```sh
scripts/install.sh
```

the installer builds and installs the shell, sets up its user service and notification integration, and restarts it. the first install sets mod + space and mod + c, replacing existing actions on those shortcuts; reinstalls preserve your bindings. set mod + d to run `chuhshell launcher`. the old mod + shift + d binding is removed.

the active niri config is detected automatically. to choose one explicitly:

```sh
scripts/install.sh --config /path/to/config.kdl
```

for bluetooth, enable the system service with `sudo systemctl enable --now bluetooth.service`.

`scripts/uninstall.sh` restores installation backups. optional tty1 autologin is set up with `scripts/setup-autologin.sh` and reverted with `scripts/setup-autologin.sh --undo`; it uses your current account and never reads or stores your password.

if installation or autologin is interrupted, rerun the same script or use its uninstall/undo command. keep the backups and recovery journal until recovery finishes. later edits are preserved or reported as conflicts.

## settings

settings live in `~/.config/chuhshell/config.json` (or under `XDG_CONFIG_HOME`). see [config.example.json](config.example.json) for an example. menu changes apply immediately; most manual edits need a service restart.

you can choose modules, their order, launcher layout, monitors, devices and the notification history limit (200 by default). weather starts in aktobe; choose another city in the menu. `weather_units` accepts `"system"`, `"celsius"` or `"fahrenheit"`.

launcher pins and sorting are saved separately in `launcher.json` in the same directory. tasks are saved under your xdg data home. notification history lasts until cleared or the shell restarts.

settings writes are atomic. invalid or oversized user data is preserved and reported instead of overwritten. if a write succeeds but its directory cannot be synced, the new settings stay active and a durability warning appears in the service log.

## troubleshooting and development

`chuhshell --help` lists commands. `chuhshell doctor` checks desktop integrations and notification ownership; add `--json` for machine-readable output. read service logs with:

```sh
journalctl --user -u chuhshell.service
```

local checks:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python scripts/test_install.py
scripts/check-headless.sh
```

the headless checks need labwc, wlr-randr and dbus-run-session. they test gtk and navigation with two virtual monitors in an isolated session. ordinary `cargo test` skips the gtk regression test.

optional headless modes:

- `CHUHSHELL_PROFILE=1 CHUHSHELL_TEST_RELEASE=1` for performance measurements.
- `CHUHSHELL_SOAK_SECONDS=1800` for a 30-minute load check.
- `CHUHSHELL_TEST_SCREENSHOTS=/path` for screenshots with grim; add `CHUHSHELL_TEST_BASELINES=/path` for comparison. match fonts, renderer and geometry.

`scripts/report-environment.sh` records toolchain and package versions. build a release with `cargo build --release --locked`.

for a local arch package, run `python scripts/package-source.py /tmp/chuhshell-package`, then `makepkg` in that directory. this generates the source archive and checksum without installing or restarting the shell.
