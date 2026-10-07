# chuhshell

a personal desktop shell for niri, written in rust with gtk4. the panel, launcher, menus and notifications share one process and one look.

i made this for my own desktop. i think it should work on other people's desktops too, but i'm not sure. it only supports the niri window manager. feel free to use it, change it, share it or borrow ideas - i don't mind what you do with it.

## what it does

- a panel across multiple monitors, with workspaces, background apps, clock, notifications, volume, brightness, keyboard layout, temperature, wifi and battery.
- an app launcher with fuzzy search, pinned apps, usage-based or alphabetical sorting, and controls to hide apps.
- wifi and bluetooth controls, including pairing prompts inside the shell. enterprise wifi needs an existing networkmanager profile.
- keyboard layout selection and volume/brightness sliders, with scroll controls on the panel.
- a system monitor menu with cpu/gpu usage, ram usage and cpu/gpu temperatures.
- a battery menu with uptime and economy, balance and performance profiles through power-profiles-daemon.
- weather from open-meteo, a calendar, saved tasks and clipboard history.
- app notification popups and a drawer, separate from the volume and brightness osd.
- editors for panel modules, launcher placement and niri keybindings.

## getting around

mod + space opens chuh menu, mod + d opens the app launcher, mod + c opens clipboard history, and ctrl + b opens the wallpaper picker.

in menus, use up/down to move, enter or right to select, left to go back and escape to close. the launcher’s pin and sort buttons also work with the keyboard. results use pages of up to 32 rows; up/down crosses page boundaries, and search covers the full catalog.

chuh menu lets you toggle and arrange panel modules, move or resize the launcher, and edit shortcuts. save applies changes; cancel or escape discards them. niri bindings include their source files and are validated before saving.

click the keyboard layout module to select a configured layout. click volume or brightness for a slider; scrolling still adjusts them. volume stays within 0-100%, and moving its slider unmutes audio. brightness stays within 1-100%. if the audio observer fails, the panel keeps the last confirmed value and shows the error in its tooltip; the volume slider stays disabled until fresh data arrives.

click the temperature module for live system metrics, refreshed every two seconds. gpu collection runs only while the menu is open. gpu usage falls back to the busiest engine across accessible applications when the driver has no global usage counter. missing sensors show unavailable; integrated intel graphics may have no separate gpu temperature sensor.

the battery menu selects performance on external power, balance on battery, and economy below 20% on battery. manual selection lasts until the power source or charge threshold changes; the automatic button immediately restores automatic selection. unsupported profiles are disabled and service errors appear in the menu. install and enable power-profiles-daemon for these controls.

clipboard history keeps up to 100 text and png entries in memory, never on disk. search and press enter to copy; delete removes an entry. you can pause recording or clear the history.

for wallpapers, put images in `~/Pictures/Wallpapers` and provide `~/.local/bin/wallpaper.sh`. the picker calls that script with the selected filename and shows failures. it lists up to 256 images and loads previews as you browse; larger folders show a limit notice.

## installation

on arch, building needs rust 1.92+, pkgconf, gtk4, gtk4-layer-shell and glib 2.80+. desktop controls use niri, wireplumber, pactl, networkmanager, brightnessctl, udevadm, bluez, wl-clipboard and curl. the preferred font is inputsans nerd font. css falls back to input sans, dejavu sans, symbols nerd font mono and the system sans-serif. unpatched ttf-input supplies text only; panel icons require nerd fonts 3 glyphs (including material design icons in the supplementary private-use area). the package includes ttf-dejavu and ttf-nerd-fonts-symbols-mono for text and icon fallback. manual installations need these fallback packages or a patched nerd font.

run this from the project directory in your niri session:

```sh
scripts/install.sh
```

the installer builds and installs the shell, sets up its user service and notification integration, and restarts it. the service records supported xdg, path and locale variables from that session; rerun the installer after changing those variables. display, niri and runtime variables are imported into the user manager and remain dynamic across logins. the first install sets mod + space and mod + c, replacing existing actions on those shortcuts; reinstalls preserve your bindings. set mod + d to run `chuhshell launcher`. the old mod + shift + d binding is removed.

the active niri config is detected automatically. to choose one explicitly:

```sh
scripts/install.sh --config /path/to/config.kdl
```

for battery power profiles, install and enable the system service:

```sh
sudo pacman -S power-profiles-daemon
sudo systemctl enable --now power-profiles-daemon.service
```

for bluetooth, enable the system service with `sudo systemctl enable --now bluetooth.service`.

`scripts/uninstall.sh` restores installation backups. optional tty1 autologin is set up with `scripts/setup-autologin.sh` and reverted with `scripts/setup-autologin.sh --undo`; it uses your current account and never reads or stores your password.

if installation or autologin is interrupted, rerun the same script or use its uninstall/undo command. keep the backups and recovery journal until recovery finishes. later edits are preserved or reported as conflicts.

## settings

settings live in `~/.config/chuhshell/config.json` (or under `XDG_CONFIG_HOME`). see [config.example.json](config.example.json) for an example. menu changes apply immediately; most manual edits need a service restart.

you can choose modules, their order, launcher layout, monitors, devices and the notification history limit (200 by default). weather starts in aktobe; choose another city in the menu. `weather_units` accepts `"system"`, `"celsius"` or `"fahrenheit"`.

launcher pins and sorting are saved separately in `launcher.json` in the same directory. tasks are saved under your xdg data home. notification history lasts until cleared or the shell restarts. task edits detect external changes; reopen the task list before retrying a conflict.

the running shell shares one validated settings snapshot. weather and shortcuts read that snapshot; failed settings updates keep it active. commands sent to an existing shell do not reread config.json. settings writes are atomic. invalid or oversized user data is preserved and reported instead of overwritten. if a write succeeds but its directory cannot be synced, the new settings stay active and a durability warning appears in the service log.

## troubleshooting and development

`chuhshell --help` lists commands. `chuhshell doctor` checks desktop integrations, notification ownership, power profiles, the cpu sensor, backlight access, idle/session-lock protocols and pending installation/autologin recovery journals. disabled panel modules are skipped; optional services and absent hardware produce warnings. brightness checks read sysfs and inspect the active logind session without changing brightness; add `--json` for machine-readable output. read service logs with:

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
- `scripts/check-visual.sh` compares launcher fixtures against `CHUHSHELL_VISUAL_BASE` (locally `HEAD`, in ci the pull request base or previous push commit). it needs ttf-dejavu and uses the same isolated fonts, geometry and renderer for both builds; screenshots and the baseline commit are saved in `target/visual-artifacts/` (ci uses `artifacts/`). comparisons measure the visible content area.

`scripts/report-environment.sh` records toolchain and package versions. build a release with `cargo build --release --locked`.

for a local arch package, run `python scripts/package-source.py /tmp/chuhshell-package`, then `makepkg` in that directory. this generates the source archive and checksum without installing or restarting the shell.

idle settings live under trigger → idle, with an enabled flag and minute timer for screensaver (15 minutes). system → screensaver starts it immediately. it uses the delta corps priest 1 figlet artwork and dismisses on input. wayland session-lock and idle-notify support are required. a separate lock client releases the screensaver on normal shutdown or when the main shell crashes. killing the lock client itself can leave the session locked.

trigger → reminder schedules one-time notifications with text and a local date/time. upcoming reminders appear below the form and can be deleted. reminders are saved in `$XDG_DATA_HOME/chuhshell/reminders.json` and kept pending until the notification center accepts them. pending and overdue reminders arrive after restart; a crash before acknowledgement may show a reminder again.

todo and keybinding writes share the settings queue and finish before normal shutdown. the user service allows up to five minutes to drain accepted writes.

background app scans pause while the module and drawer are hidden. scans have time, process, read and cache limits; an incomplete scan is shown in the ui and cannot be used to quit an app. application lookup and launch run on a separate bounded worker.
