# chuhshell

this is a personal project made for myself and my own desktop setup.

it contains a panel and an application launcher for niri, written in rust with
gtk4 and gtk4-layer-shell. the project is mainly for personal use and may
change to suit my needs.

## features

the bar is a 36px layer-shell panel on top of every output, styled after the
rosé pine palette. it shows workspaces with their empty, active, focused and
urgent states, and clicking one focuses it. next to them is a clock that
toggles between the time and the date on click, with the full date in its
tooltip. the remaining modules are audio with volume and mute state, screen
brightness, the current keyboard layout, the cpu package temperature, wi-fi
signal and ip, and the battery with a time estimate. audio and brightness
react to scrolling, and most modules open a matching tool on click.

the launcher is a fuzzy application launcher with two modes. the normal mode
lists visible applications by launch frequency, filters them as you type and
launches the selected one. search match quality takes priority over frequency.
the manage mode toggles applications between visible and hidden.

chuhshell displays its own on-screen notifications for volume, microphone,
brightness and keyboard layout changes. it also reports wi-fi connections and
network names, power and charging changes, and usb device connections. repeated
updates refresh the current notification instead of flashing a new window.

## controls

mod+d opens the launcher and mod+shift+d opens it in manage mode. inside, the
arrow keys move the selection, page up and page down move five rows at a time,
enter launches the selected application, and escape closes it. typing searches
applications directly, while arrow keys and scrolling navigate the results.
the configured media keys control volume, microphone mute and screen
brightness, with chuhshell showing the resulting on-screen notification.

## configuration

hidden applications are read from and written to
`~/.config/chuhshell/hidden-apps`, one desktop-file id per line, with lines
starting with `#` ignored. the bar detects the battery, ac adapter, backlight,
wireless interface and cpu thermal sensor at runtime, so it is not tied to
specific device names. launch counts are stored in
`~/.local/state/chuhshell/launch-counts.json` (or under `XDG_STATE_HOME`). usb
connection events are read from udev.

## building and running

build with `cargo build --release` and install the binary with
`install -Dm755 target/release/chuhshell ~/.local/bin/chuhshell`.

to use it with niri, add `spawn-at-startup "chuhshell"` to the config and bind
`Mod+D` to `chuhshell launcher` and `Mod+Shift+D` to `chuhshell manage`. media
key bindings can call `chuhshell volume-up`, `chuhshell volume-down`,
`chuhshell volume-mute`, `chuhshell microphone-mute`, `chuhshell brightness-key-up`
and `chuhshell brightness-key-down`.

the bar expects `nmcli` for wi-fi information, `wpctl` and `pactl` for audio,
`brightnessctl` for brightness controls, `udevadm` for usb events, `foot` and
`nmtui` for the network menu, and `btop` for the temperature module action. the
configured `InputSans Nerd Font` font should be installed for the intended
appearance. on-screen notifications are drawn by chuhshell and do not require
a separate osd daemon.

## structure

the code is split by feature. `main.rs` is the entry point and routes
commands. `app.rs` holds the shared state. `bar.rs` and `launcher.rs` build the
two windows, `niri.rs` talks to niri over its ipc and event stream, and
`modules.rs` runs the system pollers and usb event monitor. `notifications.rs`
builds the on-screen notifications. `apps.rs` reads desktop entries and the
hidden list, `fuzzy.rs` does the matching, and `css.rs` and `ui.rs` hold the
stylesheet and layer-shell helpers.
