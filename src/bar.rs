use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;

use crate::app::AppState;
use crate::modules::{self, BatteryStatus, NetworkInfo};
use crate::niri;
use crate::ui::set_layer_window;

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
    battery_rx: Receiver<BatteryStatus>,
    brightness_rx: Receiver<Option<(u8, &'static str)>>,
}

fn brightness_text(value: Option<(u8, &'static str)>) -> String {
    value.map_or_else(
        || "--".to_owned(),
        |(percent, icon)| format!("{icon} {percent}%"),
    )
}

fn temperature_text(value: Option<i64>) -> String {
    value.map_or_else(
        || "󰔏 --°C".to_owned(),
        |temp| format!("󰔏 {}°C", temp / 1000),
    )
}

enum ScrollAction {
    Volume,
    Brightness { device: Option<String> },
}

fn module(text: &str, class: &str, tooltip: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("module");
    label.add_css_class(class);
    label.set_tooltip_text(Some(tooltip));
    label
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
                let _ = niri::command(&command);
            });
        });
        container.append(&button);
    }
}

fn update_layout(state: &Rc<AppState>, label: &gtk::Label) {
    let names = state.layout_names.borrow();
    if names.is_empty() {
        label.set_text("󰌌 --");
        return;
    }
    let name = names
        .get(state.current_layout.get())
        .map(String::as_str)
        .unwrap_or("?");
    let short = match name.to_lowercase().as_str() {
        "russian" => "ru".to_owned(),
        "english (us)" => "us".to_owned(),
        _ => name
            .split_whitespace()
            .next_back()
            .unwrap_or(name)
            .to_lowercase(),
    };
    label.set_text(&format!("󰌌 {short}"));
    label.set_tooltip_text(Some(&name.to_lowercase()));
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
            .and_then(|value| value.parse::<f32>().ok())
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

    if let Some(value) = refs.brightness_rx.try_iter().last() {
        match value {
            Some((percent, icon)) => {
                refs.brightness
                    .set_text(&brightness_text(Some((percent, icon))));
                refs.brightness
                    .set_tooltip_text(Some("screen brightness — scroll to adjust"));
                refs.brightness.set_visible(true);
            }
            None => {
                refs.brightness.set_text(&brightness_text(None));
                refs.brightness
                    .set_tooltip_text(Some("screen brightness unavailable"));
            }
        }
    }

    if let Some(value) = refs.cpu_rx.try_iter().last() {
        match value {
            Some(temp) => {
                refs.temperature.set_text(&temperature_text(Some(temp)));
                refs.temperature
                    .set_tooltip_text(Some(&format!("cpu temperature: {}°c", temp / 1000)));
                refs.temperature.set_visible(true);
            }
            None => {
                refs.temperature.set_text(&temperature_text(None));
                refs.temperature
                    .set_tooltip_text(Some("cpu temperature unavailable"));
            }
        }
    }

    if let Some(info) = refs.network_rx.try_iter().last() {
        refs.network.set_text(&info.text);
        refs.network.set_tooltip_text(Some(&info.tooltip));
        refs.network.remove_css_class("disconnected");
        if info.status == "disconnected" {
            refs.network.add_css_class("disconnected");
        }
    }

    if let Some(battery) = refs.battery_rx.try_iter().last() {
        refs.battery.set_text(&battery.text);
        refs.battery.set_tooltip_text(Some(&battery.tooltip));
        refs.battery.remove_css_class("warning");
        refs.battery.remove_css_class("critical");
        if !battery.level.is_empty() && battery.level != "normal" {
            refs.battery.add_css_class(&battery.level);
        }
        refs.battery.set_visible(!battery.text.is_empty());
    }
}

fn add_scroll_controller(widget: &impl IsA<gtk::Widget>, action: ScrollAction) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.connect_scroll(move |_, _, y| {
        let direction = if y < 0.0 { "+" } else { "-" };
        match &action {
            ScrollAction::Volume => {
                let _ = Command::new("wpctl")
                    .args([
                        "set-volume",
                        "@DEFAULT_AUDIO_SINK@",
                        &format!("5%{direction}"),
                    ])
                    .spawn();
            }
            ScrollAction::Brightness { device } => {
                let mut command = Command::new("brightnessctl");
                if let Some(device) = device {
                    command.args(["-d", device.as_str()]);
                }
                let _ = command.args(["set", &format!("5%{direction}")]).spawn();
            }
        }
        glib::Propagation::Stop
    });
    widget.add_controller(controller);
}

pub fn create(app: &gtk::Application, state: &Rc<AppState>) {
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
    add_scroll_controller(&audio, ScrollAction::Volume);
    add_scroll_controller(
        &brightness,
        ScrollAction::Brightness {
            device: modules::backlight_device(),
        },
    );

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

    let (niri_tx, niri_rx) = std::sync::mpsc::channel();
    niri::spawn_poller(niri_tx);
    glib::timeout_add_local(Duration::from_millis(16), {
        let state = Rc::clone(state);
        let workspaces = left.clone();
        let layout_label = language.clone();
        move || {
            if let Some(snapshot) = niri_rx.try_iter().last() {
                if *state.workspaces.borrow() != snapshot.workspaces {
                    *state.workspaces.borrow_mut() = snapshot.workspaces;
                    update_workspaces(&state, &workspaces);
                }
                if *state.layout_names.borrow() != snapshot.layouts.names
                    || state.current_layout.get() != snapshot.layouts.current_idx
                {
                    *state.layout_names.borrow_mut() = snapshot.layouts.names;
                    state.current_layout.set(snapshot.layouts.current_idx);
                    update_layout(&state, &layout_label);
                }
                if let Some(initial_window) = state.launcher_focus_window.get() {
                    if state.focused_window_id() != initial_window {
                        let launcher = state.launcher.borrow().clone();
                        if let Some(launcher) = launcher {
                            launcher.close();
                        }
                    }
                } else if let Some(current_window) = state.focused_window_id() {
                    state.launcher_focus_window.set(Some(Some(current_window)));
                }
            }
            glib::ControlFlow::Continue
        }
    });

    let (audio_tx, audio_rx) = std::sync::mpsc::sync_channel(1);
    modules::spawn_audio_poller(audio_tx);
    let (network_tx, network_rx) = std::sync::mpsc::sync_channel(1);
    modules::spawn_network_poller(network_tx);
    let (cpu_tx, cpu_rx) = std::sync::mpsc::sync_channel(1);
    modules::spawn_temperature_poller(cpu_tx);
    let (battery_tx, battery_rx) = std::sync::mpsc::sync_channel(1);
    modules::spawn_battery_poller(battery_tx);
    let (brightness_tx, brightness_rx) = std::sync::mpsc::sync_channel(1);
    modules::spawn_brightness_poller(brightness_tx);

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
    glib::timeout_add_local(Duration::from_millis(50), move || {
        update_modules(&refs_update);
        glib::ControlFlow::Continue
    });

    let clock = refs.clock.clone();
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
    *state.bar.borrow_mut() = Some(window.upcast());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_state_never_keeps_a_stale_reading() {
        assert_eq!(brightness_text(Some((42, "󰃝"))), "󰃝 42%");
        assert_eq!(brightness_text(None), "--");
    }

    #[test]
    fn temperature_state_never_keeps_a_stale_reading() {
        assert_eq!(temperature_text(Some(52_000)), "󰔏 52°C");
        assert_eq!(temperature_text(None), "󰔏 --°C");
    }
}
