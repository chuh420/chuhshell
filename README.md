# chuhshell

A personal desktop shell for Niri, written in Rust with GTK4 and gtk4-layer-shell.
It combines a panel, application launcher, system OSD, application notifications
and background application controls in one process.

## Interface

A panel is created on each connected monitor. Workspaces belong to their output.
The clock stays between background apps on the left and notifications on the
right. Side modules scroll horizontally when there is insufficient space.
The clock switches between time and date when clicked.

The remaining modules display audio, brightness, keyboard layout, CPU temperature,
Wi-Fi and battery state. Scroll audio or brightness to adjust them; click audio
to mute, Wi-Fi to open `nmtui`, and temperature to open `btop` in `foot`.
Missing services and unavailable readings are distinguished from disconnected
network devices. System commands run sequentially outside the GTK thread with
timeouts. Monitor changes do not start additional system observers.

The launcher supports fuzzy search, launch-frequency ranking and a separate
hide/show mode. Arrow keys and Page Up/Down navigate matching results, Enter
launches, and Escape closes. Empty searches cannot activate filtered-out rows.
Desktop visibility rules come from GIO; the shared catalog supports nested
.desktop directories and refreshes when applications change. Launching uses GIO
and reports errors without counting failed requests as successful launches.

Background apps lists recognized desktop applications with no open Niri windows.
`Open` activates their desktop entry. `Quit` sends SIGTERM to identified root
processes after rechecking their identities and windows; Linux pidfds prevent
signalling a reused PID. Ambiguous executables are omitted. Recognition uses
canonical executable paths, Flatpak metadata, verified launch metadata and
previously observed Niri windows. Some applications launched through wrappers
may only be recognized after an open window has been observed. This is not a
complete system process list or a tray implementation. Alt+F4 remains a normal
Niri window close.

Application notifications use `org.freedesktop.Notifications`. System OSD events
(volume, brightness, layout, network, power and peripherals) remain separate.
Notification history is bounded and lasts for the current shell session.
Popups have constrained previews, individual dismissal and height-aware placement.
The drawer scrolls, supports individual deletion, and keeps `Clear notifications`
at the bottom. Menus attach to their panel buttons, close on Escape/outside click,
and only one menu is open at a time.

Notification replacement, close/action signals, resident/transient hints,
desktop-entry hints and validated image data are supported. Images and text have
size limits. Expired notification actions are not reused; archived entries can
open their source application when the sender supplies its desktop ID.

## Build and installation

On Arch, build dependencies include `rust`, `pkgconf`, `gtk4`, `gtk4-layer-shell`
and GLib 2.80 or newer. Runtime integrations use Niri, `wpctl` (WirePlumber),
`pactl` (libpulse and a compatible server), NetworkManager, `brightnessctl` and
`udevadm`. `foot` and `btop` are needed only for their module actions. The intended
font is InputSans Nerd Font. `gtk-launch` and GTK3 are not required.

```sh
cargo build --release --locked
scripts/install.sh
```

The local installer places the binary in `~/.local/bin`, installs a user service
and D-Bus activation file, updates the existing Niri startup entry, validates
Niri's configuration, and starts chuhshell. It imports the current display and
Niri socket into the user service environment. Run it from the active Niri
session as the desktop user. It does not change autologin.

The user service restarts after failures. Managed system observers are terminated
when the shell stops, while applications launched by the shell remain independent.
Installation backups are stored under `$XDG_STATE_HOME/chuhshell/installation`
(default `~/.local/state/chuhshell/installation`). Installation failures roll back
replaced files. `scripts/uninstall.sh` restores backups and preserves files edited
after installation. Review any preserved Niri startup entry before the next login.

A local Arch package can also be built with `cd packaging && makepkg -s`. Its
PKGBUILD builds the surrounding checkout and installs to `/usr`; it does not
configure the current session. With that installation, enable the user service
and start it from Niri after the compositor has established its environment.
The repository currently has no declared redistribution license.

For an existing manual installation, `scripts/install-notification-service.sh`
only configures notification activation and keeps a backup of a replaced file.

## Commands and diagnostics

```sh
chuhshell launcher
chuhshell manage
chuhshell notifications
chuhshell background-apps
chuhshell doctor
chuhshell --help
chuhshell --version
journalctl --user -u chuhshell.service
```

Bind Mod+D to `chuhshell launcher` and Mod+Shift+D to `chuhshell manage`.
Media commands are `volume-up`, `volume-down`, `volume-mute`, `microphone-mute`,
`brightness-key-up` and `brightness-key-down`. Brightness also accepts
`brightness-up/down` and `brightness-scroll-up/down`. Unknown commands and failed
remote system actions return a nonzero exit status.

## Configuration and state

Optional settings are read at startup from `$XDG_CONFIG_HOME/chuhshell/config.json`
(default `~/.config/chuhshell/config.json`). An absent file uses automatic device
selection, all monitors and a 200-entry notification history. Unknown keys and
invalid configuration are reported. History limits are clamped to 10–1000.

```json
{
  "monitors": [],
  "battery": null,
  "backlight": null,
  "wifi": null,
  "temperature_sensor": null,
  "notification_history_limit": 200
}
```

Device overrides name sysfs devices/interfaces; temperature_sensor is a path to
a temperature input file. Monitor entries are connector names such as `eDP-1`.
These settings have no graphical editor; restart the service after changing them.

Hidden application IDs are stored in `chuhshell/hidden-apps` under the config
home. Launch counts are stored in `chuhshell/launch-counts.json` under the state
home. XDG locations are respected by the notification installer as well.

TTY1 autologin is separate and optional: `scripts/setup-autologin.sh` validates
the zsh account, backs up the getty drop-in and login profile, and adds the
`niri --session` startup. It uses sudo for system configuration and never stores
a password. Reboot to activate. `scripts/setup-autologin.sh --undo` restores the
backups if the managed files have not subsequently changed.

## Architecture and checks

`Services` owns the shared system state and event delivery. Panel windows are
views over that state. The catalog is shared between the launcher and background
application manager. System commands have a bounded serial queue; external
observers and bounded command processes have explicit cleanup. UI updates are
driven by delivered events instead of polling queues every 32/100 milliseconds.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python scripts/test_install.py
scripts/check-headless.sh
```

The last check needs `labwc` and `dbus-run-session`. It creates two headless
outputs and private runtime/config/data/state directories and a private D-Bus
session. It checks actual GTK filtering, popup sizing, notification replacement,
expiration, history limits, D-Bus calls, menus and multiple panel views. It does
not interact with the running desktop shell. These checks also run in CI.
