use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;

use crate::app::AppState;
use crate::background_apps::BackgroundManager;
use crate::modules::{self, DeviceEvent, NetworkInfo};
use crate::niri;
use crate::notification_center::NotificationCenter;
use crate::notifications::{self, ConnectionNotice, Notice, NoticeKind, PowerNotice};
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
    device_rx: Receiver<DeviceEvent>,
}

type NetworkState = Option<(bool, Option<String>)>;

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
    Brightness,
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

fn update_modules(refs: &ModuleRefs, state: &Rc<AppState>, last_network: &mut NetworkState) {
    if let Some(text) = refs.audio_rx.try_iter().last().flatten()
        && refs.audio_text.borrow().as_ref() != Some(&text)
    {
        *refs.audio_text.borrow_mut() = Some(text.clone());
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
        let connected = info.status == "connected";
        match notifications::network_transition(
            last_network.as_ref(),
            connected,
            info.ssid.as_deref(),
        ) {
            Some(ConnectionNotice::Connected(ssid)) => notifications::show(
                state,
                Notice::transient(NoticeKind::Network, "wi-fi connected").with_detail(ssid),
            ),
            Some(ConnectionNotice::Disconnected) => notifications::show(
                state,
                Notice::transient(NoticeKind::Network, "wi-fi disconnected"),
            ),
            None => {}
        }
        *last_network = Some((connected, info.ssid.clone()));
        if refs.network.text() != info.text
            || refs.network.tooltip_text().as_deref() != Some(&info.tooltip)
        {
            refs.network.set_text(&info.text);
            refs.network.set_tooltip_text(Some(&info.tooltip));
            if connected {
                refs.network.remove_css_class("disconnected");
            } else {
                refs.network.add_css_class("disconnected");
            }
        }
    }
}

fn update_battery(refs: &ModuleRefs, state: &Rc<AppState>, last_power: &mut Option<(bool, bool)>) {
    let battery = modules::battery_status();
    if refs.battery.text() != battery.text
        || refs.battery.tooltip_text().as_deref() != Some(&battery.tooltip)
        || *last_power != Some((battery.plugged, battery.charging))
    {
        let transition =
            notifications::power_transition(*last_power, battery.plugged, battery.charging);
        *last_power = Some((battery.plugged, battery.charging));
        if let Some(transition) = transition {
            let title = match transition {
                PowerNotice::Connected => "power connected",
                PowerNotice::Disconnected => "power disconnected",
                PowerNotice::ChargingStarted => "charging started",
                PowerNotice::ChargingComplete => "charging complete",
            };
            notifications::show(
                state,
                Notice::transient(NoticeKind::Power, title)
                    .with_detail(format!("{} · {}", battery.text, battery.tooltip)),
            );
        }
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

fn add_scroll_controller(
    widget: &impl IsA<gtk::Widget>,
    action: ScrollAction,
    state: Rc<AppState>,
) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.connect_scroll(move |_, _, y| {
        let direction = if y < 0.0 { "+" } else { "-" };
        match &action {
            ScrollAction::Volume => {
                notifications::handle_command(
                    &state,
                    if direction == "+" {
                        "volume-up"
                    } else {
                        "volume-down"
                    },
                );
            }
            ScrollAction::Brightness => {
                notifications::handle_command(
                    &state,
                    if direction == "+" {
                        "brightness-scroll-up"
                    } else {
                        "brightness-scroll-down"
                    },
                );
            }
        }
        glib::Propagation::Stop
    });
    widget.add_controller(controller);
}

pub fn create(app: &gtk::Application, state: &Rc<AppState>, center: &Rc<NotificationCenter>) {
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

    let audio = module("--", "audio", "audio volume");
    let brightness = module("--", "brightness", "screen brightness — scroll to adjust");
    let language = module("󰌌 --", "language", "keyboard layout");
    language.set_width_chars(7);
    language.set_xalign(0.5);
    let temperature = module("", "temperature", "cpu temperature");
    let network = module("󰖪", "network", "wi-fi status");
    let battery = module("", "battery", "battery level");
    let notification_label = module("󰂚", "notification-toggle", "Notifications");
    center.attach_label(&notification_label);
    let notification_click = gtk::GestureClick::new();
    notification_click.connect_released({
        let center = Rc::clone(center);
        move |_, _, _, _| center.toggle_drawer()
    });
    notification_label.add_controller(notification_click);
    let background_manager = BackgroundManager::new(app);
    *state.background_manager.borrow_mut() = Some(Rc::clone(&background_manager));
    let background_label = module("󰀻", "background-apps-toggle", "Background apps");
    background_manager.attach_label(&background_label);
    let background_click = gtk::GestureClick::new();
    background_click.connect_released({
        let manager = Rc::clone(&background_manager);
        move |_, _, _, _| manager.toggle()
    });
    background_label.add_controller(background_click);
    let center_modules = gtk::Grid::new();
    center_modules.set_column_homogeneous(true);
    center_modules.set_column_spacing(6);
    center_modules.set_halign(gtk::Align::Center);
    center_modules.set_valign(gtk::Align::Center);
    background_label.set_halign(gtk::Align::End);
    notification_label.set_halign(gtk::Align::Start);
    center_modules.attach(&background_label, 0, 0, 1, 1);
    center_modules.attach(&clock, 1, 0, 1, 1);
    center_modules.attach(&notification_label, 2, 0, 1, 1);
    overlay.add_overlay(&center_modules);
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
    audio_click.connect_released({
        let state = Rc::clone(state);
        move |_, _, _, _| {
            notifications::handle_command(&state, "volume-mute");
        }
    });
    audio.add_controller(audio_click);
    add_scroll_controller(&audio, ScrollAction::Volume, Rc::clone(state));
    add_scroll_controller(&brightness, ScrollAction::Brightness, Rc::clone(state));

    let network_click = gtk::GestureClick::new();
    network_click.connect_released(move |_, _, _, _| {
        modules::spawn_detached("foot", &["-e", "nmtui"]);
    });
    network.add_controller(network_click);

    let temp_click = gtk::GestureClick::new();
    temp_click.connect_released(move |_, _, _, _| {
        modules::spawn_detached("foot", &["-e", "btop"]);
    });
    temperature.add_controller(temp_click);

    let (niri_tx, niri_rx) = std::sync::mpsc::channel();
    niri::spawn_poller(niri_tx);
    let last_layout = Rc::new(Cell::new(None));
    glib::timeout_add_local(Duration::from_millis(32), {
        let state = Rc::clone(state);
        let workspaces = left.clone();
        let layout_label = language.clone();
        let last_layout = Rc::clone(&last_layout);
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
                    if let Some(previous) = last_layout.replace(Some(snapshot.layouts.current_idx))
                        && previous != snapshot.layouts.current_idx
                        && let Some(name) = state
                            .layout_names
                            .borrow()
                            .get(snapshot.layouts.current_idx)
                    {
                        notifications::show(
                            &state,
                            Notice::transient(
                                NoticeKind::Keyboard,
                                notifications::layout_label(name),
                            ),
                        );
                    }
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
    let (device_tx, device_rx) = std::sync::mpsc::sync_channel(32);
    modules::spawn_device_monitor(device_tx);

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
        device_rx,
    });
    let mut last_network = None;
    let mut last_power = None;
    update_modules(&refs, state, &mut last_network);
    update_battery(&refs, state, &mut last_power);
    let refs_update = Rc::clone(&refs);
    let state_for_notifications = Rc::clone(state);
    let state_for_modules = Rc::clone(state);
    let mut backlight = modules::backlight_device();
    let mut last_brightness = backlight.as_deref().and_then(modules::brightness_level);
    refs.brightness.set_text(&brightness_text(last_brightness));
    if last_brightness.is_none() {
        refs.brightness
            .set_tooltip_text(Some("screen brightness unavailable"));
    }
    let mut brightness_check = std::time::Instant::now();
    let mut battery_check = std::time::Instant::now();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let mut brightness_changed = false;
        let mut battery_changed = false;
        for event in refs_update.device_rx.try_iter() {
            match event {
                DeviceEvent::Peripheral(connected, name) => notifications::show(
                    &state_for_notifications,
                    Notice::transient(
                        NoticeKind::Peripheral,
                        if connected {
                            "device connected"
                        } else {
                            "device disconnected"
                        },
                    )
                    .with_detail(name),
                ),
                DeviceEvent::Brightness => brightness_changed = true,
                DeviceEvent::Power => battery_changed = true,
            }
        }
        if battery_check.elapsed() >= Duration::from_secs(30) {
            battery_changed = true;
        }
        if battery_changed {
            battery_check = std::time::Instant::now();
            update_battery(&refs_update, &state_for_modules, &mut last_power);
        }
        if brightness_check.elapsed() >= Duration::from_secs(15) {
            brightness_check = std::time::Instant::now();
            brightness_changed = true;
        }
        if brightness_changed {
            if backlight.is_none() {
                backlight = modules::backlight_device();
            }
            let mut value = backlight.as_deref().and_then(modules::brightness_level);
            if value.is_none() {
                backlight = modules::backlight_device();
                value = backlight.as_deref().and_then(modules::brightness_level);
            }
            if value != last_brightness {
                refs_update.brightness.set_text(&brightness_text(value));
                refs_update
                    .brightness
                    .set_tooltip_text(Some(if value.is_some() {
                        "screen brightness — scroll to adjust"
                    } else {
                        "screen brightness unavailable"
                    }));
                last_brightness = value;
            }
        }
        update_modules(&refs_update, &state_for_modules, &mut last_network);
        glib::ControlFlow::Continue
    });

    let clock = refs.clock.clone();
    let mut last_clock = String::new();
    let mut last_day = String::new();
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
            if time != last_clock {
                clock.set_text(&format!("󰥔 {time}"));
                last_clock = time;
            }
            if weekday != last_day {
                clock.set_tooltip_text(Some(&weekday.to_lowercase()));
                last_day = weekday;
            }
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
