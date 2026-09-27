# chuhshell

this project is mainly for my own use and was never really intended for distribution. feel free to do whatever you want with it: use it, copy it, modify it, share it or take it as a reference for your own project. i don’t mind.

a personal desktop shell for niri, written in rust with gtk4. the panel, app launcher, notifications and controls live in one process and share the same look.

## using it

the panel shows your workspaces, background apps, clock, notifications, volume, brightness, keyboard layout, temperature, wifi and battery. it works across multiple monitors. you can turn individual modules on or off from chuh menu.

press mod and space to open chuh menu in the middle of the screen. use up and down to move, enter or right to select, left to go back and escape to close. some sections are still marked wip and do not do anything yet.

press mod and d to open the app launcher. it has fuzzy search and remembers the apps you use most. the app launcher section in chuh menu lets you hide or show apps. choose configure to drag the launcher to a new position or resize it by its bottom edge.

settings contains bluetooth controls for power, scanning, pairing, connections and trusted devices. bluez handles the radio; pin and confirmation prompts stay inside chuhshell. you can also disconnect devices or forget a pairing.

info contains weather, a monthly calendar and clipboard history. weather starts in aktobe; search for another city or use the city from the system timezone. temperature units follow the system locale, with celsius and fahrenheit overrides. forecasts come from open-meteo. the timezone city is approximate. the calendar lets you browse months and return to today.

mod and c opens clipboard history. it keeps up to 100 text and png entries in memory for the current session (2 mib per item, 20 mib total). search and press enter to copy an item and close the menu. press delete to remove an item, or use the buttons to pause recording and clear history. clipboard contents are never saved to disk.

under bar, configure lets you arrange modules in the left, center and right groups. drag a module from the panel to a + slot, or select its name and then a slot. the panel’s size and position are fixed, and the module order applies to every monitor.

in either editor, save keeps your changes and cancel or escape discards them. reset restores the default arrangement before saving.

click wifi to manage networks without opening a terminal. you can scan, connect, disconnect and manage saved connections. networkmanager handles passwords. enterprise networks need a profile configured beforehand.

notifications from apps appear as popups and stay in the notification drawer until cleared or the shell restarts. volume, brightness and other system indicators stay separate. the background apps menu lets you open or quit recognized apps that are still running without a window.

## installation

to build on arch, you need rust, pkgconf, gtk4, gtk4-layer-shell and glib 2.80 or newer. desktop integrations use niri, wireplumber, pactl, networkmanager, brightnessctl, udevadm, bluez, wl-clipboard and curl. enable bluetooth with `sudo systemctl enable --now bluetooth.service`. clicking the temperature opens btop in foot, so those two are optional. the intended font is inputsans nerd font.

run `scripts/install.sh` from the project directory inside your niri session. it builds and installs chuhshell, sets up the user service and notifications, and sets mod and space for the menu and mod and c for clipboard history. these replace any existing actions on those shortcuts. the old mod and shift and d binding is removed. mod and d should run `chuhshell launcher`.

run `scripts/uninstall.sh` to restore installation backups. autologin is optional and has its own script, `scripts/setup-autologin.sh`.

## settings

settings live in `~/.config/chuhshell/config.json`, or under your config home if you set `XDG_CONFIG_HOME`. module visibility, module order, launcher layout and weather preferences are saved there. set `weather_units` to `"celsius"` for °c, `"fahrenheit"` for °f or `"system"` to follow your locale. you can also choose monitors and devices and change the notification history limit, which defaults to 200. most manual changes need a service restart; weather preferences are read when its page opens. menu changes apply in the running shell, and the launcher keeps the same relative position on whichever monitor opens it.

## development

for available commands, run `chuhshell --help`. `chuhshell doctor` checks the main integrations. service logs are available through `journalctl --user -u chuhshell.service`.

for development, use `cargo fmt --check`, `cargo clippy --locked --all-targets` and `cargo test --locked --all-targets`. installer checks run with `python scripts/test_install.py`. `scripts/check-headless.sh` checks the interface in an isolated session and needs labwc and dbus-run-session.
