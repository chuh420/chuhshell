use crate::{
    app::AppState, background_apps::BackgroundManager, modules, niri,
    notification_center::NotificationCenter, notifications, services::Services, ui,
};
use gtk::prelude::*;
use gtk4_layer_shell::{self as layer_shell, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn module(text: &str, class: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.add_css_class("module");
    button.set_valign(gtk::Align::Center);
    button.add_css_class(class);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn scroll(button: &gtk::Button, state: &Rc<AppState>, up: &'static str, down: &'static str) {
    let controller = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    let state = Rc::clone(state);
    controller.connect_scroll(move |_, _, y| {
        if y != 0.0 {
            notifications::handle_command(&state, if y < 0.0 { up } else { down });
        }
        glib::Propagation::Stop
    });
    button.add_controller(controller);
}

pub fn create(app: &gtk::Application, state: &Rc<AppState>, center: &Rc<NotificationCenter>) {
    if state.services.borrow().is_some() {
        return;
    }
    *state.services.borrow_mut() = Some(Services::new(state));
    *state.background_manager.borrow_mut() = Some(BackgroundManager::new(app));
    *state.network_menu.borrow_mut() = Some(crate::network::NetworkMenu::new());
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let monitors = display.monitors();
    reconcile(app, state, center, &monitors);
    let app = app.downgrade();
    let state = Rc::downgrade(state);
    let center = Rc::downgrade(center);
    monitors.connect_items_changed(move |monitors, _, _, _| {
        if let (Some(app), Some(state), Some(center)) =
            (app.upgrade(), state.upgrade(), center.upgrade())
        {
            reconcile(&app, &state, &center, monitors);
        }
    });
}

fn reconcile(
    app: &gtk::Application,
    state: &Rc<AppState>,
    center: &Rc<NotificationCenter>,
    monitors: &gio::ListModel,
) {
    let active: Vec<_> = (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
        .filter(|m| {
            crate::config::get().monitors.is_empty()
                || m.connector()
                    .is_some_and(|name| crate::config::get().monitors.iter().any(|s| s == &name))
        })
        .collect();
    state.bars.borrow_mut().retain(|(monitor, window)| {
        if active.contains(monitor) {
            true
        } else {
            window.close();
            false
        }
    });
    for monitor in active {
        if state.bars.borrow().iter().any(|(m, _)| m == &monitor) {
            continue;
        }
        let window = build(app, state, center, &monitor);
        state.bars.borrow_mut().push((monitor, window));
    }
}

fn sidebar(content: &gtk::Box, monitor: &gtk::gdk::Monitor) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
    scroll.set_propagate_natural_width(true);
    scroll.set_max_content_width(((monitor.geometry().width() - 290) / 2).max(30));
    scroll.set_child(Some(content));
    let weak = scroll.downgrade();
    monitor.connect_geometry_notify(move |monitor| {
        if let Some(scroll) = weak.upgrade() {
            scroll.set_max_content_width(((monitor.geometry().width() - 290) / 2).max(30));
        }
    });
    scroll
}

fn build(
    app: &gtk::Application,
    state: &Rc<AppState>,
    center: &Rc<NotificationCenter>,
    monitor: &gtk::gdk::Monitor,
) -> gtk::Window {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("chuhshell")
        .build();
    window.set_widget_name("bar");
    window.set_default_height(36);
    ui::set_layer_window(
        &window,
        "chuhshell",
        layer_shell::Layer::Top,
        &[
            layer_shell::Edge::Top,
            layer_shell::Edge::Left,
            layer_shell::Edge::Right,
        ],
        36,
        layer_shell::KeyboardMode::OnDemand,
    );
    window.set_monitor(Some(monitor));
    let layout = gtk::CenterBox::new();
    layout.set_margin_start(8);
    layout.set_margin_end(8);
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    layout.set_start_widget(Some(
        &state
            .bar_modules
            .wrap("workspaces", &sidebar(&left, monitor)),
    ));
    layout.set_end_widget(Some(&sidebar(&right, monitor)));
    let clock = module("", "clock", "Date and time");
    let date = Rc::new(Cell::new(false));
    update_clock(&clock, false);
    clock.connect_clicked({
        let date = Rc::clone(&date);
        move |button| {
            date.set(!date.get());
            update_clock(button, date.get());
        }
    });
    clock_tick(clock.downgrade(), date);
    let notification = module("󰂚", "notification-toggle", "Notifications");
    center.attach_button(&notification);
    notification.connect_clicked({
        let center = Rc::clone(center);
        move |button| center.toggle_at(button)
    });
    let background = module("󰀻", "background-apps-toggle", "Background apps");
    if let Some(manager) = state.background_manager.borrow().as_ref() {
        manager.attach_button(&background);
        background.connect_clicked({
            let manager = Rc::clone(manager);
            move |button| manager.toggle_at(button)
        });
    }
    let middle = gtk::Grid::new();
    middle.set_valign(gtk::Align::Center);
    middle.set_column_homogeneous(true);
    middle.set_column_spacing(4);
    for (column, id, button, align) in [
        (0, "background-apps", &background, gtk::Align::End),
        (1, "clock", &clock, gtk::Align::Center),
        (2, "notifications", &notification, gtk::Align::Start),
    ] {
        let container = state.bar_modules.wrap(id, button);
        container.set_halign(align);
        middle.attach(&container, column, 0, 1, 1);
    }
    layout.set_center_widget(Some(&middle));
    let audio = module("--", "audio", "Audio unavailable");
    let brightness = module("--", "brightness", "Screen brightness");
    let language = module("--", "language", "Keyboard layout");
    let temperature = module("--", "temperature", "CPU temperature");
    let network = module("󰖪", "network", "Wi-Fi unavailable");
    let battery = module("", "battery", "Battery");
    for (id, button) in [
        ("audio", &audio),
        ("brightness", &brightness),
        ("language", &language),
        ("temperature", &temperature),
        ("wifi", &network),
        ("battery", &battery),
    ] {
        right.append(&state.bar_modules.wrap(id, button));
    }
    audio.connect_clicked({
        let state = Rc::clone(state);
        move |_| {
            notifications::handle_command(&state, "volume-mute");
        }
    });
    scroll(&audio, state, "volume-up", "volume-down");
    scroll(
        &brightness,
        state,
        "brightness-scroll-up",
        "brightness-scroll-down",
    );
    network.connect_clicked({
        let menu = state.network_menu.borrow().as_ref().unwrap().clone();
        move |button| menu.toggle(button)
    });
    temperature.connect_clicked(|_| {
        modules::spawn_detached("foot", &["-e", "btop"]);
    });
    language.set_focusable(false);
    battery.set_focusable(false);
    let output = monitor.connector().map(|s| s.to_string());
    let old_workspaces = RefCell::new(Vec::new());
    let weak_window = window.downgrade();
    window.set_child(Some(&layout));
    window.present();
    state.services.borrow().as_ref().unwrap().subscribe(move |data| {
        if weak_window.upgrade().is_none_or(|window| !window.is_visible()) { return false; }
        if *old_workspaces.borrow() != data.niri.workspaces {
            *old_workspaces.borrow_mut() = data.niri.workspaces.clone();
            while let Some(child) = left.first_child() { left.remove(&child); }
            let mut workspaces = data.niri.workspaces.clone();
            workspaces.sort_by_key(|w| w.idx);
            for workspace in workspaces.iter().filter(|w| w.output == output) {
                let text = workspace.name.clone().unwrap_or_else(|| if workspace.is_active { "●" } else { "○" }.into());
                let button = gtk::Button::with_label(&text);
                button.add_css_class("workspace");
                button.set_valign(gtk::Align::Center);
                for (enabled, class) in [(workspace.active_window_id.is_none(), "empty"), (workspace.is_active, "active"), (workspace.is_focused, "focused"), (workspace.is_urgent, "urgent")] { if enabled { button.add_css_class(class); } }
                button.set_tooltip_text(Some(&format!("Workspace {}", workspace.idx)));
                let id = workspace.id;
                button.connect_clicked(move |_| { std::thread::spawn(move || { if !niri::command(&format!("{{\"Action\":{{\"FocusWorkspace\":{{\"reference\":{{\"Id\":{id}}}}}}}}}")) { eprintln!("chuhshell: could not focus workspace"); } }); });
                left.append(&button);
            }
        }
        if let Some(text) = &data.audio {
            let muted = text.contains("MUTED");
            let percent = text.split_whitespace().nth(1).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0) * 100.0;
            audio.set_label(&format!("{} {:.0}%", if muted { "󰝟" } else { "󰕾" }, percent));
            audio.set_tooltip_text(Some(if muted { "Audio muted" } else { "Audio volume" }));
        } else { audio.set_label("󰝟 --"); audio.set_tooltip_text(Some("Audio unavailable")); }
        brightness.set_label(&data.brightness.map_or_else(|| "--".into(), |(p, icon)| format!("{icon} {p}%")));
        brightness.set_tooltip_text(Some(if data.brightness.is_some() { "Screen brightness — scroll to adjust" } else { "Screen brightness unavailable" }));
        temperature.set_label(&data.temperature.map_or_else(|| "󰔏 --°C".into(), |t| format!("󰔏 {}°C", t / 1000)));
        let name = data.niri.layouts.names.get(data.niri.layouts.current_idx);
        language.set_label(&name.map_or_else(|| "󰌌 --".into(), |s| format!("󰌌 {}", notifications::layout_label(s))));
        if let Some(info) = &data.network { network.set_label(&info.text); network.set_tooltip_text(Some(&info.tooltip)); }
        battery.set_label(&data.battery.text); battery.set_tooltip_text(Some(&data.battery.tooltip)); battery.set_visible(!data.battery.text.is_empty());
        for class in ["warning", "critical"] { if data.battery.level == class { battery.add_css_class(class); } else { battery.remove_css_class(class); } }
        true
    });
    window.upcast()
}

fn update_clock(clock: &gtk::Button, date: bool) {
    if let Ok(now) = glib::DateTime::now_local() {
        if let Ok(text) = now.format(if date { "%a %d.%m" } else { "%H:%M" }) {
            clock.set_label(&format!("󰥔 {text}"));
        }
        if let Ok(text) = now.format("%A, %d %B %Y") {
            clock.set_tooltip_text(Some(&text));
        }
    }
}

fn clock_tick(clock: glib::WeakRef<gtk::Button>, date: Rc<Cell<bool>>) {
    let delay = glib::DateTime::now_local().map_or(30, |now| (60 - now.second()).clamp(1, 30));
    glib::timeout_add_local_once(std::time::Duration::from_secs(delay as u64), move || {
        if let Some(button) = clock.upgrade() {
            update_clock(&button, date.get());
            clock_tick(clock, date);
        }
    });
}
