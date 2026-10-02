# chuhshell

this project is mainly for my own use and was never really intended for distribution. feel free to do whatever you want with it: use it, copy it, modify it, share it or take it as a reference for your own project. i don’t mind.

a personal desktop shell for niri, written in rust with gtk4. the panel, app launcher, notifications and controls live in one process and share the same look.

## using it

the panel shows your workspaces, background apps, clock, notifications, volume, brightness, keyboard layout, temperature, wifi and battery. it works across multiple monitors. you can turn individual modules on or off from chuh menu.

press mod and space to open chuh menu in the middle of the screen. use up and down to move, enter or right to select, left to go back and escape to close. some sections are still marked wip and do not do anything yet.

press mod and d to open the app launcher. it has fuzzy search and remembers the apps you use most. pin apps with the button on the right to keep them at the top. right focuses the pin button, enter toggles it, and left returns to search. down from the last app focuses the sort button; up returns to the list. use the sort button to switch between most used and a–z; pins and sorting are saved in `~/.config/chuhshell/launcher.json` (or under `XDG_CONFIG_HOME`). the app launcher section in chuh menu lets you hide or show apps. choose configure to drag the launcher to a new position or resize it by its bottom edge.

keybindings in chuh menu lists all niri bindings from the active config and its includes, plus chuhshell’s internal shortcuts. search by key, action or source, then enter or record a new combination and save. descriptions and source files are shown beside each binding; overridden bindings are marked. niri changes are validated before writing and reload automatically. internal shortcuts are stored in `keybindings` in the shell config and apply immediately. comma-separated alternatives are supported for internal shortcuts. standard gtk text editing and focus navigation remain provided by gtk.

settings contains bluetooth controls for power, scanning, pairing, connections and trusted devices. bluez handles the radio; pin and confirmation prompts stay inside chuhshell. you can also disconnect devices or forget a pairing.

info contains weather, a monthly calendar, clipboard history and todo. weather starts in aktobe; search for another city or use the city from the system timezone. temperature units follow the system locale, with celsius and fahrenheit overrides. forecasts come from open-meteo. the timezone city is approximate. the calendar lets you browse months and return to today.

todo saves tasks under your xdg data home. add a task, mark it done or delete it from the info menu.

appearance opens a wallpaper preview strip. use left/right arrows to browse and enter to apply, or click a preview and apply. ctrl and b opens it directly (`chuhshell wallpaper`). place images in `~/Pictures/Wallpapers` and provide `~/.local/bin/wallpaper.sh`; the shell calls it with the selected filename. scripts have a 30-second timeout and failures are shown in the menu.

mod and c opens clipboard history. it keeps up to 100 text and png entries in memory for the current session (2 mib per item, 20 mib for retained contents and search data). search and press enter to copy an item and close the menu. press delete to remove an item, or use the buttons to pause recording and clear history. clipboard contents are never saved to disk.

under bar, configure lets you arrange modules in the left, center and right groups. drag a module from the panel to a + slot, or select its name and then a slot. the panel’s size and position are fixed, and the module order applies to every monitor.

in either editor, save keeps your changes and cancel or escape discards them. reset restores the default arrangement before saving.

click wifi to manage networks without opening a terminal. you can scan, connect, disconnect and manage saved connections. networkmanager handles passwords. enterprise networks need a profile configured beforehand.

notifications from apps appear as popups and stay in the notification drawer until cleared or the shell restarts. volume, brightness and other system indicators stay separate. the background apps menu lets you open or quit recognized apps that are still running without a window.

## installation

to build on arch, you need rust 1.92 or newer, pkgconf, gtk4, gtk4-layer-shell and glib 2.80 or newer. desktop integrations use niri, wireplumber, pactl, networkmanager, brightnessctl, udevadm, bluez, wl-clipboard and curl. enable bluetooth with `sudo systemctl enable --now bluetooth.service`. clicking the temperature opens btop in foot, so those two are optional. the intended font is inputsans nerd font.

run `scripts/install.sh` from the project directory inside your niri session. it builds and installs chuhshell, sets up the user service and notifications, and sets mod and space for the menu and mod and c for clipboard history. on the first install, these replace any existing actions on those shortcuts. reinstalling preserves your niri bindings. the old mod and shift and d binding is removed. mod and d should run `chuhshell launcher`.

the installer discovers the active niri config and validates all changed includes before writing. use `scripts/install.sh --config /path/to/config.kdl` to select a config explicitly. interrupted installations are recovered on the next install or uninstall; later user edits are preserved.

run `scripts/uninstall.sh` to restore installation backups. autologin is optional: `scripts/setup-autologin.sh` enables passwordless local login on tty1, and `scripts/setup-autologin.sh --undo` restores the previous configuration. the script reads the current account name; it never reads or stores the account password. its local recovery journal also supports interrupted setup. the script resolves `ZDOTDIR` through zsh, validates profile syntax before writing and remembers the original profile path for recovery. comments or inactive niri commands do not suppress the managed tty1 block.

## settings

settings live in `~/.config/chuhshell/config.json`, or under your config home if you set `XDG_CONFIG_HOME`. module visibility, module order, launcher layout and weather preferences are saved there. set `weather_units` to `"celsius"` for °c, `"fahrenheit"` for °f or `"system"` to follow your locale. you can also choose monitors and devices and change the notification history limit, which defaults to 200. [config.example.json](config.example.json) shows a minimal configuration. empty or relative xdg home variables are ignored in favor of the standard directories under the user’s home; relative entries in `XDG_DATA_DIRS` are ignored. most manual changes need a service restart; weather preferences are read when its page opens. menu changes apply in the running shell, and the launcher keeps the same relative position on whichever monitor opens it.

## saving and recovery

settings are replaced atomically. a failure before replacement leaves the old file and reports an error. if replacement succeeds but directory sync fails, the new file and in-memory state stay active; a durability warning is written to the service log. a power loss may still lose that unconfirmed write. invalid or oversized todo files are reported and preserved.

installation and autologin treat durability failures as transaction failures and use their recovery journals. rerun the same installer or use its undo/uninstall command to recover; keep the backup directory and journal until recovery finishes. independently edited files are preserved or reported as conflicts.

limits include 500 tasks of 200 characters, 256 kib per desktop file, 4096 catalog entries and 16 mib of catalog text, 1 mib per niri frame and 1152 kib per notification request. popup creation is limited to five per rolling second; accepted notifications still enter the bounded history.

## packaging

`packaging/PKGBUILD` is a template for a local source snapshot. run `python scripts/package-source.py /tmp/chuhshell-package`, then run `makepkg` in that directory. the helper produces a deterministic archive and a recipe with its sha256 checksum; the build uses only that archive under `srcdir`. package creation does not install or restart the shell. `scripts/install-notification-service.sh` now delegates to the main installer, using its backups and recovery journal.

## development

for available commands, run `chuhshell --help`. `chuhshell doctor` checks the main integrations and verifies that chuhshell owns notifications. optional tools produce warnings. `chuhshell doctor --json` provides the same checks as json; errors produce a nonzero exit status. service logs are available through `journalctl --user -u chuhshell.service`.

for development, use `cargo fmt --check`, `cargo clippy --locked --all-targets` and `cargo test --locked --all-targets`. installer checks run with `python scripts/test_install.py`. `scripts/check-headless.sh` checks the interface in an isolated session and needs labwc, wlr-randr and dbus-run-session. virtual monitor off/on is checked without touching the live session.


use `CHUHSHELL_PROFILE=1 CHUHSHELL_TEST_RELEASE=1 scripts/check-headless.sh` for launcher/search measurements with 100, 500 and 1000 apps. `CHUHSHELL_PROFILE_CATALOG_DIRS` accepts colon-separated application directories for a cold catalog read. results include nearest-rank p95, main-loop gaps and rss; timing has no ci threshold.

set `CHUHSHELL_TEST_SCREENSHOTS=/path/to/output` to capture screenshots with grim. set `CHUHSHELL_TEST_BASELINES=/path/to/baselines` to compare them automatically: dimensions must match and at most 1% of pixels may differ by more than 24 per channel. use matching fonts, renderer and output geometry. ci compares two isolated runs and retains both sets; persistent baselines can be supplied for visual regression checks.

`CHUHSHELL_SOAK_SECONDS=1800 scripts/check-headless.sh` runs a 30-minute isolated notification, module and navigation load. rss samples are printed once per minute. `scripts/report-environment.sh` records toolchain and system package versions alongside reports. ci runs separate security, release and minimum-rust checks; a manual workflow can enable the optional performance job.
