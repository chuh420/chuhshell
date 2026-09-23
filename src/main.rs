use gio::prelude::*;
use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;
use serde::Deserialize;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

const CSS: &str = r#"
* { font-family: "InputSans Nerd Font"; font-size: 12px; }
window#bar { background: #191724; border-bottom: 1px solid #403d52; }
.module { background: #1f1d2e; border-radius: 8px; padding: 4px 10px; margin: 4px 1px; color: #e0def4; }
.module:hover { background: #403d52; }
.workspaces { background: transparent; padding: 0; }
.workspace { background: transparent; color: #908caa; border: 0; border-radius: 8px; padding: 4px 9px; margin: 4px 1px; }
.workspace.empty { color: #6e6a86; }
.workspace.active { background: #26233a; color: #9ccfd8; }
.workspace.focused { background: #31748f; color: #191724; }
.workspace.urgent { color: #eb6f92; }
.clock { background: #1f1d2e; color: #e0def4; padding: 4px 8px; }
.audio { color: #ebbcba; min-width: 68px; }
.brightness, .temperature { color: #f6c177; }
.network, .language, .battery { color: #9ccfd8; }
.language, .battery { min-width: 68px; }
.battery.warning { color: #f6c177; }
.battery.critical { color: #eb6f92; }
.muted { color: #908caa; }
.tooltip { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 8px; padding: 6px 8px; }
window#launcher { background: #191724; border: 1px solid #403d52; border-radius: 12px; }
window#launcher label, window#launcher entry { font-size: 13pt; }
window#launcher .app-meta { font-size: 10pt; }
.launcher-box { padding: 14px; }
.search { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 8px; padding: 9px 12px; }
.search:focus { border-color: #c4a7e7; outline: none; box-shadow: none; }
window#launcher listbox row:focus { outline: none; }
.app-list { background: transparent; }
window#launcher listbox row { background: transparent; border-radius: 8px; }
window#launcher listbox row:hover { background: #26233a; }
window#launcher listbox row:selected { background: #26233a; color: #e0def4; }
window#launcher listbox row:selected label { color: #e0def4; }
window#launcher listbox row.selected { background-color: #26233a; color: #e0def4; }
window#launcher listbox row.selected label { color: #e0def4; }
window#launcher listbox row.selected-row { background-color: #26233a; color: #e0def4; }
window#launcher listbox row.selected-row label { color: #e0def4; }
.app-row { background-color: #1f1d2e; color: #e0def4; border-radius: 8px; padding: 8px 10px; }
window#launcher listbox row.selected-row .app-row { background-color: #26233a; }
.app-row.hidden-app { color: #908caa; }
.app-icon { margin-right: 12px; }
.app-meta { color: #6e6a86; font-size: 10px; }
"#;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Workspace {
    idx: i64,
    name: Option<String>,
    is_urgent: bool,
    is_active: bool,
    is_focused: bool,
    active_window_id: Option<u64>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct KeyboardLayouts {
    names: Vec<String>,
    current_idx: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkInfo {
    text: String,
    tooltip: String,
    status: String,
}

#[derive(Clone, Debug)]
struct AppEntry {
    id: String,
    name: String,
    icon: String,
    comment: String,
    exec: String,
    hidden: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LauncherMode {
    Normal,
    Manage,
}

#[derive(Default)]
struct AppState {
    bar: RefCell<Option<gtk::Window>>,
    launcher: RefCell<Option<gtk::Window>>,
    workspaces: RefCell<Vec<Workspace>>,
    layout_names: RefCell<Vec<String>>,
    current_layout: Cell<usize>,
    launcher_mode: Cell<Option<LauncherMode>>,
    launcher_apps: RefCell<Vec<AppEntry>>,
    launcher_focus_window: Cell<Option<Option<u64>>>,
}

fn config_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/fuzzel/hidden-apps")
}

fn read_hidden() -> HashSet<String> {
    fs::read_to_string(config_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn write_hidden(hidden: &HashSet<String>) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut ids: Vec<_> = hidden.iter().collect();
    ids.sort();
    let mut body = ids.into_iter().cloned().collect::<Vec<_>>().join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, body)?;
    fs::rename(temp, path)
}

fn desktop_value(section: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    section.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .filter(|value| !value.starts_with('['))
            .map(str::to_owned)
    })
}

fn load_apps() -> Vec<AppEntry> {
    let hidden = read_hidden();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    let mut dirs = vec![data_home.join("applications")];
    dirs.extend(
        data_dirs
            .split(':')
            .filter(|part| !part.is_empty())
            .map(|part| PathBuf::from(part).join("applications")),
    );

    let mut entries: HashMap<String, AppEntry> = HashMap::new();
    for dir in dirs {
        let Ok(files) = fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = files.flatten().map(|entry| entry.path()).collect();
        files.sort();
        for file in files {
            if file.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&file) else {
                continue;
            };
            let Some(section) = contents.split("[Desktop Entry]").nth(1) else {
                continue;
            };
            let section = section.split("\n[").next().unwrap_or(section);
            if desktop_value(section, "Type")
                .as_deref()
                .is_some_and(|v| v != "Application")
                || ["Hidden", "NoDisplay"].iter().any(|key| {
                    desktop_value(section, key)
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                })
            {
                continue;
            }
            let id = file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if id.is_empty() || entries.contains_key(id) {
                continue;
            }
            let name = desktop_value(section, "Name")
                .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
            let app = AppEntry {
                id: id.to_owned(),
                name,
                icon: desktop_value(section, "Icon").unwrap_or_default(),
                comment: desktop_value(section, "Comment").unwrap_or_default(),
                exec: desktop_value(section, "Exec").unwrap_or_default(),
                hidden: hidden.contains(id),
            };
            entries.insert(id.to_owned(), app);
        }
    }
    let mut apps: Vec<_> = entries.into_values().collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

fn niri_request(request: &str) -> Option<serde_json::Value> {
    let socket_path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(600)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(600)))
        .ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    niri_response(&line)
}

fn niri_response(response: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(response)
        .ok()?
        .get("Ok")
        .cloned()
}

fn niri_command(request: &str) -> bool {
    niri_request(request).is_some()
}

fn query_niri_snapshot() -> Option<(Vec<Workspace>, KeyboardLayouts)> {
    let workspaces = niri_request("\"Workspaces\"")
        .and_then(|value| value.get("Workspaces").cloned())
        .and_then(|value| serde_json::from_value::<Vec<Workspace>>(value).ok())?;
    let layouts = niri_request("\"KeyboardLayouts\"")
        .and_then(|value| value.get("KeyboardLayouts").cloned())
        .and_then(|value| serde_json::from_value::<KeyboardLayouts>(value).ok())?;
    Some((workspaces, layouts))
}

fn spawn_niri_poller(sender: SyncSender<(Vec<Workspace>, KeyboardLayouts)>) {
    thread::spawn(move || {
        loop {
            if let Some(snapshot) = query_niri_snapshot() {
                let _ = sender.try_send(snapshot);
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
}

fn spawn_network_poller(sender: SyncSender<NetworkInfo>) {
    thread::spawn(move || {
        loop {
            let (text, tooltip, status) = network_details();
            let _ = sender.try_send(NetworkInfo {
                text,
                tooltip,
                status,
            });
            thread::sleep(Duration::from_secs(5));
        }
    });
}

fn spawn_audio_poller(sender: SyncSender<Option<String>>) {
    thread::spawn(move || {
        loop {
            if let Some(value) = child_process("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
                let _ = sender.try_send(Some(value));
            }
            let Ok(mut child) = Command::new("pactl")
                .arg("subscribe")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if (line.contains("sink") || line.contains("server"))
                        && let Some(value) =
                            child_process("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
                    {
                        let _ = sender.try_send(Some(value));
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    });
}

fn spawn_brightness_poller(sender: SyncSender<Option<(u8, &'static str)>>) {
    thread::spawn(move || {
        loop {
            let max = fs::read_to_string("/sys/class/backlight/intel_backlight/max_brightness")
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok());
            let current = fs::read_to_string("/sys/class/backlight/intel_backlight/brightness")
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok());
            let value = max.zip(current).map(|(max, current)| {
                let percent = (current * 100 / max.max(1)) as u8;
                let icon = if percent <= 15 {
                    "󰃞"
                } else if percent <= 50 {
                    "󰃝"
                } else {
                    "󰃟"
                };
                (percent, icon)
            });
            let _ = sender.try_send(value);
            thread::sleep(Duration::from_millis(150));
        }
    });
}

fn update_workspaces(state: &Rc<AppState>, container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let mut workspaces = state.workspaces.borrow().clone();
    workspaces.sort_by_key(|workspace| workspace.idx);
    for workspace in workspaces {
        let label = workspace
            .name
            .as_ref()
            .map(|name| name.to_lowercase())
            .unwrap_or_else(|| {
                if workspace.is_urgent || workspace.is_active || workspace.is_focused {
                    "●".to_owned()
                } else {
                    "○".to_owned()
                }
            });
        let button = gtk::Button::with_label(&label);
        button.add_css_class("workspace");
        if workspace.active_window_id.is_none() {
            button.add_css_class("empty");
        }
        if workspace.is_active {
            button.add_css_class("active");
        }
        if workspace.is_focused {
            button.add_css_class("focused");
        }
        if workspace.is_urgent {
            button.add_css_class("urgent");
        }
        button.set_tooltip_text(Some(&format!("workspace {}", workspace.idx)));
        let idx = workspace.idx;
        button.connect_clicked(move |_| {
            let command = format!(
                "{{\"Action\":{{\"FocusWorkspace\":{{\"reference\":{{\"Index\":{idx}}}}}}}}}"
            );
            thread::spawn(move || {
                let _ = niri_command(&command);
            });
        });
        container.append(&button);
    }
}

fn focused_window_id(state: &Rc<AppState>) -> Option<u64> {
    state
        .workspaces
        .borrow()
        .iter()
        .find(|workspace| workspace.is_focused)
        .and_then(|workspace| workspace.active_window_id)
}

fn update_layout(state: &Rc<AppState>, label: &gtk::Label) {
    let names = state.layout_names.borrow();
    if names.is_empty() {
        label.set_text("󰌌 --");
    } else {
        let name = names
            .get(state.current_layout.get())
            .map(String::as_str)
            .unwrap_or("?");
        let short = match name.to_lowercase().as_str() {
            "russian" => "ru".to_owned(),
            "english (us)" => "us".to_owned(),
            _ => name
                .split_whitespace()
                .last()
                .unwrap_or(name)
                .to_lowercase(),
        };
        label.set_text(&format!("󰌌 {short}"));
        label.set_tooltip_text(Some(&name.to_lowercase()));
    }
}

fn module(text: &str, class: &str, tooltip: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("module");
    label.add_css_class(class);
    label.set_tooltip_text(Some(tooltip));
    label
}

fn child_process(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn battery_text() -> (String, String, String) {
    let base = Path::new("/sys/class/power_supply/BAT0");
    let read = |name: &str| {
        fs::read_to_string(base.join(name))
            .ok()
            .map(|value| value.trim().to_owned())
    };
    let Some(capacity) = read("capacity") else {
        return (String::new(), String::new(), String::new());
    };
    let capacity_num: u32 = capacity.parse().unwrap_or(0);
    let status = read("status").unwrap_or_default();
    let online = fs::read_to_string("/sys/class/power_supply/ADP1/online")
        .is_ok_and(|value| value.trim() == "1");
    let icon = if online && status.eq_ignore_ascii_case("charging") {
        "󰂄"
    } else if online {
        "󰚥"
    } else {
        match capacity_num {
            0..=10 => "󰂎",
            11..=30 => "󰁻",
            31..=50 => "󰁾",
            51..=75 => "󰂁",
            _ => "󰁹",
        }
    };
    let level = if capacity_num <= 10 {
        "critical"
    } else if capacity_num <= 20 {
        "warning"
    } else {
        "normal"
    };
    let estimate = if let (Some(now), Some(power)) = (
        read("energy_now").and_then(|v| v.parse::<u64>().ok()),
        read("power_now")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|p| *p > 0),
    ) {
        let hours = now / power;
        let minutes = (now % power) * 60 / power;
        format!("{hours}h {minutes:02}m")
    } else {
        status.to_lowercase()
    };
    (
        format!("{icon} {capacity}%"),
        level.to_owned(),
        format!("{capacity}% • {estimate}"),
    )
}

fn network_details() -> (String, String, String) {
    let ssid = child_process(
        "nmcli",
        &["-t", "-f", "ACTIVE,SSID,SIGNAL", "device", "wifi"],
    )
    .and_then(|data| {
        data.lines()
            .find(|line| line.starts_with("yes:"))
            .map(str::to_owned)
    });
    if let Some(line) = ssid {
        let mut parts = line.splitn(3, ':');
        let _ = parts.next();
        let name = parts.next().unwrap_or("wi-fi").to_lowercase();
        let signal: u8 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let icon = match signal {
            0..=24 => "󰤟",
            25..=49 => "󰤢",
            50..=74 => "󰤥",
            _ => "󰤨",
        };
        let ip = child_process(
            "nmcli",
            &["-t", "-f", "IP4.ADDRESS", "device", "show", "wlp1s0"],
        )
        .and_then(|v| {
            v.lines()
                .find_map(|line| line.strip_prefix("IP4.ADDRESS[1]:").map(str::to_owned))
        })
        .unwrap_or_default();
        return (
            icon.to_owned(),
            format!("{name}\n{signal}% • {ip}"),
            "connected".to_owned(),
        );
    }
    (
        "󰖪".to_owned(),
        "wi-fi: disconnected".to_owned(),
        "disconnected".to_owned(),
    )
}

fn update_modules(refs: &ModuleRefs) {
    if let Some(text) = refs.audio_rx.try_iter().last().flatten() {
        *refs.audio_text.borrow_mut() = Some(text);
    }
    if let Some(text) = refs.audio_text.borrow().as_ref() {
        let muted = text.contains("MUTED");
        let volume = text
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0);
        refs.audio.set_text(&format!(
            "{} {}%",
            if muted {
                "󰝟"
            } else if volume >= 0.5 {
                "󰕾"
            } else {
                "󰕿"
            },
            if muted { 0 } else { (volume * 100.0) as u32 }
        ));
        refs.audio.remove_css_class("muted");
        if muted {
            refs.audio.add_css_class("muted");
        }
    } else {
        refs.audio.set_text(" --");
    }

    if let Some(value) = refs.brightness_rx.try_iter().last().flatten() {
        let (percent, icon) = value;
        refs.brightness.set_text(&format!("{icon} {percent}%"));
        refs.brightness.set_visible(true);
    }

    if let Some(temp) = refs.cpu_rx.try_iter().last().flatten() {
        refs.temperature.set_text(&format!("󰔏 {}°C", temp / 1000));
        refs.temperature
            .set_tooltip_text(Some(&format!("cpu temperature: {}°c", temp / 1000)));
        refs.temperature.set_visible(true);
    }

    if let Some(info) = refs.network_rx.try_iter().last() {
        refs.network.set_text(&info.text);
        refs.network.set_tooltip_text(Some(&info.tooltip));
        refs.network.remove_css_class("disconnected");
        if info.status == "disconnected" {
            refs.network.add_css_class("disconnected");
        }
    }

    if let Some((battery, level, tooltip)) = refs.battery_rx.try_iter().last() {
        refs.battery.set_text(&battery);
        refs.battery.set_tooltip_text(Some(&tooltip));
        refs.battery.remove_css_class("warning");
        refs.battery.remove_css_class("critical");
        if !level.is_empty() && level != "normal" {
            refs.battery.add_css_class(&level);
        }
        refs.battery.set_visible(!battery.is_empty());
    }
}

struct ModuleRefs {
    audio: gtk::Label,
    brightness: gtk::Label,
    temperature: gtk::Label,
    network: gtk::Label,
    battery: gtk::Label,
    clock: gtk::Label,
    audio_text: RefCell<Option<String>>,
    audio_rx: Receiver<Option<String>>,
    network_rx: Receiver<NetworkInfo>,
    cpu_rx: Receiver<Option<i64>>,
    battery_rx: Receiver<(String, String, String)>,
    brightness_rx: Receiver<Option<(u8, &'static str)>>,
}

fn set_layer_window(
    window: &impl IsA<gtk::Window>,
    namespace: &str,
    layer: layer_shell::Layer,
    anchors: &[layer_shell::Edge],
    exclusive: i32,
    keyboard: layer_shell::KeyboardMode,
) {
    window.init_layer_shell();
    window.set_namespace(Some(namespace));
    window.set_layer(layer);
    for edge in [
        layer_shell::Edge::Top,
        layer_shell::Edge::Bottom,
        layer_shell::Edge::Left,
        layer_shell::Edge::Right,
    ] {
        window.set_anchor(edge, anchors.contains(&edge));
    }
    window.set_exclusive_zone(exclusive);
    window.set_keyboard_mode(keyboard);
}

fn create_bar(app: &gtk::Application, state: &Rc<AppState>) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("chuhshell")
        .build();
    window.set_widget_name("bar");
    window.set_default_height(36);
    set_layer_window(
        &window,
        "chuhshell",
        layer_shell::Layer::Top,
        &[
            layer_shell::Edge::Top,
            layer_shell::Edge::Left,
            layer_shell::Edge::Right,
        ],
        36,
        layer_shell::KeyboardMode::None,
    );

    let overlay = gtk::Overlay::new();
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    root.set_margin_start(8);
    root.set_margin_end(8);
    root.set_hexpand(true);
    overlay.set_child(Some(&root));
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    left.add_css_class("workspaces");
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    right.set_halign(gtk::Align::End);
    let clock = module("󰥔 --:--", "clock", "");
    clock.set_widget_name("clock");
    let clock_alt = Rc::new(Cell::new(false));
    let clock_alt_signal = Rc::clone(&clock_alt);
    let gesture = gtk::GestureClick::new();
    gesture.connect_released(move |_, _, _, _| clock_alt_signal.set(!clock_alt_signal.get()));
    clock.add_controller(gesture);
    clock.set_halign(gtk::Align::Center);
    clock.set_valign(gtk::Align::Center);
    overlay.add_overlay(&clock);

    let audio = module("--", "audio", "audio volume");
    let brightness = module("--", "brightness", "screen brightness — scroll to adjust");
    let language = module("󰌌 --", "language", "keyboard layout");
    language.set_width_chars(7);
    language.set_xalign(0.5);
    let temperature = module("", "temperature", "cpu temperature");
    let network = module("󰖪", "network", "wi-fi status");
    let battery = module("", "battery", "battery level");
    right.append(&audio);
    right.append(&brightness);
    right.append(&language);
    right.append(&temperature);
    right.append(&network);
    right.append(&battery);
    root.append(&left);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    root.append(&spacer);
    root.append(&right);
    window.set_child(Some(&overlay));

    let audio_click = gtk::GestureClick::new();
    audio_click.connect_released(move |_, _, _, _| {
        let _ = Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
            .spawn();
    });
    audio.add_controller(audio_click);
    add_scroll_controller(&audio, ScrollCommand::Volume);
    add_scroll_controller(&brightness, ScrollCommand::Brightness);

    let network_click = gtk::GestureClick::new();
    network_click.connect_released(move |_, _, _, _| {
        let _ = Command::new("foot").args(["-e", "nmtui"]).spawn();
    });
    network.add_controller(network_click);

    let temp_click = gtk::GestureClick::new();
    temp_click.connect_released(move |_, _, _, _| {
        let _ = Command::new("foot").args(["-e", "btop"]).spawn();
    });
    temperature.add_controller(temp_click);

    let (niri_tx, niri_rx) = mpsc::sync_channel(1);
    spawn_niri_poller(niri_tx);
    glib::timeout_add_local(Duration::from_millis(50), {
        let state = Rc::clone(state);
        let workspaces = left.clone();
        let layout_label = language.clone();
        move || {
            if let Some((workspaces_now, layouts_now)) = niri_rx.try_iter().last() {
                if *state.workspaces.borrow() != workspaces_now {
                    *state.workspaces.borrow_mut() = workspaces_now;
                    update_workspaces(&state, &workspaces);
                }
                if *state.layout_names.borrow() != layouts_now.names
                    || state.current_layout.get() != layouts_now.current_idx
                {
                    *state.layout_names.borrow_mut() = layouts_now.names;
                    state.current_layout.set(layouts_now.current_idx);
                    update_layout(&state, &layout_label);
                }
                if let Some(initial_window) = state.launcher_focus_window.get() {
                    let current_window = focused_window_id(&state);
                    if current_window != initial_window {
                        let launcher = { state.launcher.borrow().clone() };
                        if let Some(launcher) = launcher {
                            launcher.close();
                        }
                    }
                } else if let Some(current_window) = focused_window_id(&state) {
                    state.launcher_focus_window.set(Some(Some(current_window)));
                }
            }
            glib::ControlFlow::Continue
        }
    });

    let (audio_tx, audio_rx) = mpsc::sync_channel(1);
    spawn_audio_poller(audio_tx);
    let (network_tx, network_rx) = mpsc::sync_channel(1);
    spawn_network_poller(network_tx);
    let (cpu_tx, cpu_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        loop {
            let temperature = fs::read_to_string("/sys/class/thermal/thermal_zone3/temp")
                .ok()
                .and_then(|text| text.trim().parse::<i64>().ok());
            let _ = cpu_tx.try_send(temperature);
            thread::sleep(Duration::from_secs(3));
        }
    });
    let (battery_tx, battery_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        loop {
            let _ = battery_tx.try_send(battery_text());
            thread::sleep(Duration::from_secs(30));
        }
    });
    let (brightness_tx, brightness_rx) = mpsc::sync_channel(1);
    spawn_brightness_poller(brightness_tx);

    let refs = Rc::new(ModuleRefs {
        audio,
        brightness,
        temperature,
        network,
        battery,
        clock,
        audio_text: RefCell::new(None),
        audio_rx,
        network_rx,
        cpu_rx,
        battery_rx,
        brightness_rx,
    });
    update_modules(&refs);
    let refs_update = Rc::clone(&refs);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        update_modules(&refs_update);
        glib::ControlFlow::Continue
    });
    let clock = refs.clock.clone();
    let clock_alt = Rc::clone(&clock_alt);
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if let Ok(now) = glib::DateTime::now_local() {
            let format = if clock_alt.get() { "%a %d.%m" } else { "%H:%M" };
            let time = now
                .format(format)
                .map(|text| text.to_string())
                .unwrap_or_default();
            let weekday = now
                .format("%A, %d %B %Y")
                .map(|text| text.to_string())
                .unwrap_or_default();
            clock.set_text(&format!("󰥔 {time}"));
            clock.set_tooltip_text(Some(&weekday.to_lowercase()));
        }
        glib::ControlFlow::Continue
    });
    window.present();
    *state.bar.borrow_mut() = Some(window.clone().upcast());
}

#[derive(Clone, Copy)]
enum ScrollCommand {
    Volume,
    Brightness,
}

fn add_scroll_controller(widget: &impl IsA<gtk::Widget>, kind: ScrollCommand) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.connect_scroll(move |_, _, y| {
        let adjustment = if y < 0.0 { "+" } else { "-" };
        let mut command = match kind {
            ScrollCommand::Volume => Command::new("wpctl"),
            ScrollCommand::Brightness => Command::new("brightnessctl"),
        };
        match kind {
            ScrollCommand::Volume => {
                command.args([
                    "set-volume",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("5%{adjustment}"),
                ]);
            }
            ScrollCommand::Brightness => {
                command.args(["-d", "intel_backlight", "set", &format!("5%{adjustment}")]);
            }
        }
        let _ = command.spawn();
        glib::Propagation::Stop
    });
    widget.add_controller(controller);
}

fn create_launcher(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode) {
    let old = { state.launcher.borrow_mut().take() };
    if let Some(old) = old {
        old.close();
    }
    state.launcher_mode.set(Some(mode));
    let apps = load_apps();
    *state.launcher_apps.borrow_mut() = apps.clone();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("applications")
        .default_width(520)
        .build();
    window.set_widget_name("launcher");
    set_layer_window(
        &window,
        "chuhshell-launcher",
        layer_shell::Layer::Overlay,
        &[layer_shell::Edge::Top],
        0,
        layer_shell::KeyboardMode::OnDemand,
    );

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.add_css_class("launcher-box");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("найти приложение…"));
    search.add_css_class("search");
    outer.append(&search);
    let scrolled = gtk::ScrolledWindow::builder()
        .min_content_height(120)
        .max_content_height(420)
        .propagate_natural_height(true)
        .build();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("app-list");
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);
    window.set_child(Some(&outer));

    populate_launcher_list(&list, &apps, mode, "");
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    update_selected_row_styles(&list);
    list.connect_selected_rows_changed(update_selected_row_styles);

    let list_filter = list.clone();
    search.connect_search_changed(move |entry| {
        let query = entry.text().to_string();
        FILTER_APPS.with(|apps| populate_launcher_list(&list_filter, &apps.borrow(), mode, &query));
    });

    let state_for_activate = Rc::clone(state);
    let window_for_activate = window.clone();
    let search_for_activate = search.clone();
    list.connect_row_activated(move |list, row| {
        let Some(id) = row.widget_name().strip_prefix("app-").map(str::to_owned) else {
            return;
        };
        if mode == LauncherMode::Manage {
            let mut apps = state_for_activate.launcher_apps.borrow_mut();
            if let Some(entry) = apps.iter_mut().find(|entry| entry.id == id) {
                entry.hidden = !entry.hidden;
            }
            let hidden: HashSet<_> = apps
                .iter()
                .filter(|entry| entry.hidden)
                .map(|entry| entry.id.clone())
                .collect();
            let mut hidden = hidden;
            let known: HashSet<_> = apps.iter().map(|entry| entry.id.as_str()).collect();
            hidden.extend(
                read_hidden()
                    .into_iter()
                    .filter(|id| !known.contains(id.as_str())),
            );
            if write_hidden(&hidden).is_err() {
                return;
            }
            FILTER_APPS.with(|current| *current.borrow_mut() = apps.clone());
            let selected = row.index();
            populate_launcher_list(list, &apps, mode, &search_for_activate.text());
            if let Some(row) = list.row_at_index(
                selected.min(list.observe_children().n_items().saturating_sub(1) as i32),
            ) {
                list.select_row(Some(&row));
            }
        } else {
            let app_id = id.clone();
            let result = Command::new("gtk-launch").arg(&app_id).spawn();
            if result.is_ok() {
                window_for_activate.close();
            }
        }
    });

    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list_keys = list.clone();
    let search_keys = search.clone();
    let window_keys = window.clone();
    key.connect_key_pressed(move |_, key, _, modifiers| match key {
        gdk::Key::Escape => {
            window_keys.close();
            glib::Propagation::Stop
        }
        gdk::Key::Down => {
            let index = list_keys.selected_row().map_or(0, |row| row.index() + 1);
            if let Some(row) = list_keys.row_at_index(index) {
                list_keys.select_row(Some(&row));
            }
            glib::Propagation::Stop
        }
        gdk::Key::Up => {
            let index = list_keys.selected_row().map_or(0, |row| row.index() - 1);
            if let Some(row) = list_keys.row_at_index(index.max(0)) {
                list_keys.select_row(Some(&row));
            }
            glib::Propagation::Stop
        }
        gdk::Key::Return | gdk::Key::KP_Enter => {
            if let Some(row) = list_keys.selected_row() {
                list_keys.emit_by_name::<()>("row-activated", &[&row]);
            }
            glib::Propagation::Stop
        }
        key if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            && (key == gdk::Key::b || key == gdk::Key::f) =>
        {
            if key == gdk::Key::b {
                list_keys.grab_focus();
            } else {
                search_keys.grab_focus();
            }
            glib::Propagation::Stop
        }
        gdk::Key::Page_Down | gdk::Key::Page_Up => {
            let direction = if key == gdk::Key::Page_Down { 1 } else { -1 };
            let current = list_keys.selected_row().map_or(0, |row| row.index());
            let max = list_keys.observe_children().n_items().saturating_sub(1) as i32;
            if let Some(row) = list_keys.row_at_index((current + direction * 5).clamp(0, max)) {
                list_keys.select_row(Some(&row));
            }
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    window.add_controller(key);
    let was_active = Cell::new(false);
    window.connect_is_active_notify(move |window| {
        if window.is_active() {
            was_active.set(true);
        } else if was_active.replace(false) {
            window.close();
        }
    });
    window.connect_close_request({
        let state = Rc::clone(state);
        move |_| {
            let _ = state.launcher.borrow_mut().take();
            state.launcher_focus_window.set(None);
            state.launcher_mode.set(None);
            glib::Propagation::Proceed
        }
    });
    window.present();
    search.grab_focus();
    *state.launcher.borrow_mut() = Some(window.upcast());
}

fn update_selected_row_styles(list: &gtk::ListBox) {
    let selected = list.selected_row();
    let mut child = list.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let Some(row) = row.downcast_ref::<gtk::ListBoxRow>() else {
            continue;
        };
        if selected
            .as_ref()
            .is_some_and(|selected| selected.as_ptr() == row.as_ptr())
        {
            row.add_css_class("selected-row");
        } else {
            row.remove_css_class("selected-row");
        }
    }
}

fn append_app_row(list: &gtk::ListBox, entry: &AppEntry, mode: LauncherMode) {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(&format!("app-{}", entry.id));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.add_css_class("app-row");
    if entry.hidden {
        content.add_css_class("hidden-app");
    }
    if mode == LauncherMode::Manage {
        let icon = gtk::Label::new(Some(if entry.hidden { "󰈉" } else { "󰈈" }));
        icon.add_css_class("app-icon");
        content.append(&icon);
    } else {
        let icon = if entry.icon.is_empty() {
            gtk::Image::from_icon_name("application-x-executable")
        } else {
            gtk::Image::from_icon_name(&entry.icon)
        };
        icon.set_pixel_size(22);
        icon.add_css_class("app-icon");
        content.append(&icon);
    }
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&entry.name.to_lowercase()));
    name.set_xalign(0.0);
    details.append(&name);
    if !entry.comment.is_empty() {
        let comment = gtk::Label::new(Some(&entry.comment.to_lowercase()));
        comment.set_xalign(0.0);
        comment.set_ellipsize(gtk::pango::EllipsizeMode::End);
        comment.add_css_class("app-meta");
        details.append(&comment);
    }
    content.append(&details);
    row.set_child(Some(&content));
    row.set_tooltip_text(Some(&entry.exec));
    list.append(&row);
}

fn populate_launcher_list(list: &gtk::ListBox, apps: &[AppEntry], mode: LauncherMode, query: &str) {
    let mut matches: Vec<(i64, &AppEntry)> = apps
        .iter()
        .filter(|app| mode == LauncherMode::Manage || !app.hidden)
        .filter_map(|app| {
            fuzzy_score(&app.name.to_lowercase(), query)
                .or_else(|| fuzzy_score(&app.id, query))
                .map(|score| (score, app))
        })
        .collect();
    matches.sort_by(|(score_a, app_a), (score_b, app_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| app_a.name.to_lowercase().cmp(&app_b.name.to_lowercase()))
    });
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for (_, app) in matches {
        append_app_row(list, app, mode);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    update_selected_row_styles(list);
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<i64> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let candidate = candidate.to_lowercase();
    let chars: Vec<char> = candidate.chars().collect();
    let mut score = 0i64;
    let mut cursor = 0usize;
    let mut previous = None;
    for needle in query.chars() {
        let found = chars
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, ch)| **ch == needle)
            .map(|(index, _)| index)?;
        score += 10;
        if previous.is_some_and(|prev| found == prev + 1) {
            score += 9;
        }
        if found == 0
            || chars
                .get(found.wrapping_sub(1))
                .is_some_and(|ch| !ch.is_alphanumeric())
        {
            score += 7;
        }
        score -= found as i64 / 6;
        previous = Some(found);
        cursor = found + 1;
    }
    Some(score)
}

thread_local! { static FILTER_APPS: RefCell<Vec<AppEntry>> = const { RefCell::new(Vec::new()) }; }

fn show_launcher(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode) {
    state
        .launcher_focus_window
        .set(Some(focused_window_id(state)));
    FILTER_APPS.with(|apps| *apps.borrow_mut() = load_apps());
    create_launcher(app, state, mode);
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn main() -> glib::ExitCode {
    let flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    let app = gtk::Application::builder()
        .application_id("dev.chuh.chuhshell")
        .flags(flags)
        .build();
    let state = Rc::new(AppState::default());
    let state_cli = Rc::clone(&state);
    app.connect_command_line(move |app, command_line| {
        install_css();
        let args = command_line.arguments();
        let mode =
            args.iter()
                .skip(1)
                .find_map(|argument| match argument.to_string_lossy().as_ref() {
                    "launcher" => Some(LauncherMode::Normal),
                    "manage" => Some(LauncherMode::Manage),
                    _ => None,
                });
        if let Some(mode) = mode {
            show_launcher(app, &state_cli, mode);
        } else if state_cli.bar.borrow().is_none() {
            create_bar(app, &state_cli);
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niri_response_unwraps_keyboard_layout_reply() {
        let response =
            r#"{"Ok":{"KeyboardLayouts":{"names":["English (US)","Russian"],"current_idx":1}}}"#;
        let value = niri_response(response).expect("successful niri reply");
        let layouts: KeyboardLayouts = serde_json::from_value(
            value
                .get("KeyboardLayouts")
                .expect("keyboard-layout payload")
                .clone(),
        )
        .expect("valid keyboard-layout payload");

        assert_eq!(layouts.names, ["English (US)", "Russian"]);
        assert_eq!(layouts.current_idx, 1);
    }

    #[test]
    fn fuzzy_search_matches_subsequences_and_rejects_missing_characters() {
        assert!(fuzzy_score("Visual Studio Code", "vsc").is_some());
        assert!(fuzzy_score("Firefox", "xyz").is_none());
        assert!(fuzzy_score("Firefox", "").is_some());
    }
}
