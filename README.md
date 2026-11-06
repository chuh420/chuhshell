# chuhshell

a Wayland desktop shell for Hyprland, niri and sway, written in Rust. it draws the bar, panels, notifications, OSD and wallpaper of a material desktop in one process, with no Qt, GTK or GPU stack.

![control center](docs/screenshots/control.webp)

on one 1920x1080 output it idles at about 28 MB RSS and 0.2% CPU.

## status

chuhshell is in beta. I built it for my own desktop and then moved every personal path and tool into a config file. expect bugs.

- Hyprland is my daily driver and the best tested.
- niri and sway were only checked in a nested session: the shell starts, picks the compositor, draws the bar and follows workspaces and window titles. I have not used them for real work.
- I tested the NetworkManager backend against real NetworkManager 1.58 on virtual radios (see [development](#development)): open, WPA2, WPA3, mixed WPA2/WPA3 and hidden networks, wrong passwords, daemon restarts. I have not used it on real hardware yet.

## features

- **bar**: workspaces, active window title, clock, CPU and RAM, keyboard layout, Wi-Fi, Bluetooth, volume, battery, tray and a keep awake toggle.
- **control center**: network and Bluetooth tiles, volume and brightness sliders, keep awake, shortcuts to notifications, clipboard, screenshots, wallpapers and the power menu.
- **launcher**: Unicode search over desktop entries, pinned and hidden apps, terminal entries in your terminal, every app in its own `app.slice` scope.
- **notifications**: a full `org.freedesktop.Notifications` server with actions, images, markup, history and do not disturb.
- **tray**: StatusNotifierItem host with dbusmenu submenus.
- **Wi-Fi and Bluetooth**: iwd or NetworkManager, and BlueZ, with scan, connect, forget, a password agent and pairing prompts.
- **audio**: PipeWire output and input with device selection.
- **battery**: UPower details, level and power history, power profiles.
- **clipboard**: cliphist history with image thumbnails.
- **wallpapers**: a gallery with search, live wallpapers through mpvpaper, a circle reveal on apply and an optional palette from the wallpaper through matugen.
- **themes**: six built-in palettes and per-color overrides.
- **cursors**: a cursor theme picker that can recolor themes to the current palette.
- **calendar**, **power menu** and a **system** panel with cores, temperatures, disk, load and top processes.

## screenshots

<table>
  <tr>
    <td><img src="docs/screenshots/launcher.webp" alt="launcher"></td>
    <td><img src="docs/screenshots/wallpapers.webp" alt="wallpapers"></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/calendar.webp" alt="calendar"></td>
    <td><img src="docs/screenshots/battery.webp" alt="battery"></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/audio.webp" alt="sound"></td>
    <td></td>
  </tr>
</table>

## supported systems

| | supported | picked by |
| --- | --- | --- |
| compositor | Hyprland, niri, sway | `HYPRLAND_INSTANCE_SIGNATURE`, `NIRI_SOCKET` or `SWAYSOCK` |
| Wi-Fi | iwd, NetworkManager (open, WEP, WPA2, WPA3, OWE, saved enterprise profiles) | the service that owns its name on the system bus; NetworkManager wins when both run |

Hyprland shows a fixed number of workspaces plus any busy ones above it. niri and sway show their real workspace list. under another compositor the bar runs without workspaces and window titles.

chuhshell hides a chip or panel when its service is missing: no UPower battery, no BlueZ adapter, no Wi-Fi service, no backlight or `brightnessctl`, no `cliphist`, no power-profiles-daemon, no matugen. each missing tool gets one line in the log.

## requirements

to build:

- Rust 1.99 or newer
- PipeWire headers and clang (for the `pipewire` crate bindings)
- libxkbcommon

to run:

- Hyprland, niri or sway
- fonts: Adwaita Sans and JetBrainsMono Nerd Font (or set your own in the config)
- PipeWire with WirePlumber (`wpctl`)

optional, each one turns on a feature:

| tool | feature |
| --- | --- |
| UPower, power-profiles-daemon | battery chip and panel, power profiles |
| iwd or NetworkManager, `ip`, `rfkill` | Wi-Fi panel, airplane mode |
| BlueZ | Bluetooth panel |
| `brightnessctl` | brightness slider and keys |
| `cliphist`, `wl-clipboard` | clipboard panel |
| ImageMagick, ffmpeg | wallpaper previews, posters for live wallpapers |
| mpvpaper | live wallpapers |
| matugen | colors from the wallpaper |
| hyprlock or swaylock | lock (otherwise `loginctl lock-session`) |
| grim, slurp, `wl-copy` | the default screenshot command |
| pavucontrol | the default mixer |
| `systemd-run` | apps in their own scopes |

suspend, reboot and power off go through logind.

## install

on Arch, build the package from `dist/PKGBUILD`:

```sh
cd dist && makepkg -si
```

the package installs the binary to `/usr/bin` and the unit to `/usr/lib/systemd/user`. by hand:

```sh
cargo build --release
install -Dm755 target/release/chuhshell ~/.local/bin/chuhshell
install -Dm644 dist/chuhshell.service ~/.config/systemd/user/chuhshell.service
systemctl --user daemon-reload
systemctl --user start chuhshell.service
```

the unit starts after `graphical-session.target` and stops with it. it conflicts with `swaync`, because both claim the notification D-Bus name. add conflicts for your own units in a drop-in under `~/.config/systemd/user/chuhshell.service.d/`. start the unit from your compositor config or session script. logs go to `journalctl --user -u chuhshell.service`; set `CHUHSHELL_LOG=debug` for more.

turn off compositor animations for the `material-*` layer namespaces. the shell animates its own surfaces.

## config

the shell reads `~/.config/chuhshell/config.toml` (`$XDG_CONFIG_HOME` is respected). every key is optional, and a missing file means defaults. save the file and the shell picks up the change. when the file has an error, the shell keeps the last working settings and shows a notification.

```sh
chuhshell config default   # print a config with every key
chuhshell config check     # validate your file
chuhshell config path      # print where the shell looks
```

```toml
[general]
scale = 1.22                       # ui scale, 0.5 to 4
terminal = "foot"                  # default: $TERMINAL, xdg-terminal-exec, then kitty, foot, alacritty, wezterm, ghostty
clock_format = "%a, %-d %b  ·  %H:%M"
wallpapers_dir = "~/Pictures/Wallpapers"   # default: XDG_PICTURES_DIR/Wallpapers

[fonts]
ui = "Adwaita Sans"
icons = "JetBrainsMono Nerd Font Mono"
icon_fallback = "JetBrainsMonoNL Nerd Font"

[bar]
workspaces = 5                     # fixed slots on Hyprland

[launcher]
pinned = ["kitty", "firefox"]      # matched against desktop file ids, shown first

[keyboard.labels]
"English (US)" = "US"              # layout name fragment = label

[theme]
name = "nord"                      # gruvbox, catppuccin-mocha, nord, tokyo-night, rose-pine, everforest
auto = false                       # colors from the wallpaper through matugen
scheme = "tonal-spot"              # tonal-spot, content, fidelity, vibrant, rainbow, neutral
templates = "~/.config/matugen/templates"

[theme.colors]
green = "#a3be8c"                  # override any of bg, dim, surface, elevated, line, text, muted, green, yellow, red, blue

[commands]
lock = "hyprlock"
screenshot = 'grim -g "$(slurp)" - | wl-copy'
mixer = "pavucontrol"

[hooks]
wallpaper_changed = "~/bin/wallpaper-sync"   # $1 = shown image (a poster for videos), $2 = selected file
theme_changed = "~/bin/theme-reload"         # $1 = auto or static
```

commands and hooks run through `sh -c`.

### files

| path | content |
| --- | --- |
| `~/.config/chuhshell/config.toml` | config |
| `~/.local/state/chuhshell/` | wallpaper, theme mode, cursor, hidden apps, battery history |
| `~/.local/state/chuhshell/theme/` | matugen output in `auto/`, your static snapshots, and `current` linking to one of them |
| `~/.cache/chuhshell/` | previews, posters, clipboard thumbnails |

in auto mode the shell renders its own color template plus every file in `theme.templates` into `theme/auto/`. point your apps at `~/.local/state/chuhshell/theme/current/` and reload them from the `theme_changed` hook.

## usage

running `chuhshell` with no arguments starts the shell. a second instance exits if one already answers.

`chuhshell msg COMMAND` talks to the running shell over `$XDG_RUNTIME_DIR/chuhshell.sock`, so key bindings can call it directly:

| command | action |
| --- | --- |
| `launcher`, `menu`, `calendar`, `wallpapers`, `clipboard`, `power` | open a centered panel |
| `wifi`, `bluetooth`, `audio`, `battery`, `system`, `notifications` | open an anchored panel |
| `close` | close the open panel |
| `awake`, `dnd` | toggle keep awake or do not disturb |
| `volumeUp`, `volumeDown`, `volumeMute`, `micMute` | change audio and show the OSD |
| `brightnessUp`, `brightnessDown` | change brightness and show the OSD |
| `lock`, `suspend`, `screenshot`, `mixer` | run the configured command or the logind call |
| `wallpaper [PATH]` | apply a wallpaper, or print the current one |
| `osd KIND TEXT` | show a custom OSD message |
| `tray ID` | open a tray item menu |
| `theme [auto\|static\|toggle]` | wallpaper colors on or off |
| `theme scheme [NAME\|next]` | pick the matugen scheme |
| `inspect` | print the shell state as JSON |

```lua
bind("SUPER + D", hl.dsp.exec_cmd("chuhshell msg launcher"))
bind("SUPER + Space", hl.dsp.exec_cmd("chuhshell msg menu"))
bind("XF86AudioRaiseVolume", hl.dsp.exec_cmd("chuhshell msg volumeUp"), { locked = true, repeating = true })
bind("Print", hl.dsp.exec_cmd("chuhshell msg screenshot"))
```

in the launcher, `Ctrl + H` hides the selected app and `Ctrl + Shift + H` shows hidden ones. the wallpaper gallery reads your wallpapers folder and its `live wallpapers` subfolder; cursor themes go in `cursors` inside it. live wallpapers pause while the active workspace has windows or the screen is locked.

## how it works

| crate | role |
| --- | --- |
| `config` | XDG paths and the TOML config |
| `platform` | Wayland: outputs, layer surfaces, subsurfaces, popups, shm buffers, damage, input, cursor shape, idle inhibit |
| `ui` | toolkit: node tree, layout, tiny-skia painting, cosmic-text text, animations ported from Qt easing and springs, widgets |
| `services` | compositors, PipeWire, UPower, iwd, NetworkManager, BlueZ, logind, notifications, tray, apps, clipboard, wallpapers, themes |
| `shell` | the binary: shell state, surfaces, panels and IPC |

calloop drives everything on one thread. zbus futures run on its executor, and image decoding and file scans go to a small worker pool. the shell repaints only damaged regions after frame callbacks and frees surfaces you close.

## development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

the tests include property tests for the parsers and a notification server test on a private D-Bus session bus, so you need `dbus-daemon`.

`tools/netlab` builds a Wi-Fi lab out of `mac80211_hwsim` radios: hostapd serves `lab-open`, `lab-wpa2`, `lab-sae`, `lab-mixed`, `lab-hidden` and `lab-hidden-open` in one network namespace, and NetworkManager or iwd runs in another with its own system bus. your own Wi-Fi keeps working. it needs root, `hostapd`, `dnsmasq`, `networkmanager` and `iwd`.

```sh
tools/netlab up nm
cargo test -p services --test network_lab -- --ignored --test-threads=1
tools/netlab down
```

to see the lab in the shell, start it with `CHUHSHELL_NETWORK_BUS=unix:path=/run/chuhshell-lab/bus`. only the Wi-Fi and Bluetooth service use that bus.

`cargo test -p shell --test ipc_stress -- --ignored` hammers the socket of the running shell with silent clients, slow clients, oversized and broken requests.

## license

licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
