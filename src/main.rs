mod app;
mod apps;
mod background_apps;
mod bar;
mod css;
mod fuzzy;
mod launcher;
mod modules;
mod niri;
mod notification_center;
mod notifications;
mod ui;

use std::rc::Rc;

use gio::prelude::*;

use app::{AppState, LauncherMode};

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("dev.chuh.chuhshell")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| css::install());

    let center = notification_center::NotificationCenter::new(&app);
    let startup_center = Rc::clone(&center);
    app.connect_startup(move |_| startup_center.start());

    let state = Rc::new(AppState::default());
    let state_cli = Rc::clone(&state);
    let center_cli = Rc::clone(&center);
    app.connect_command_line(move |app, command_line| {
        let arguments = command_line.arguments();
        let command = arguments
            .get(1)
            .map(|argument| argument.to_string_lossy().into_owned());
        if command
            .as_deref()
            .is_some_and(|command| notifications::handle_command(&state_cli, command))
        {
            return glib::ExitCode::SUCCESS;
        }
        if command.as_deref() == Some("notifications") {
            if state_cli.bar.borrow().is_none() {
                bar::create(app, &state_cli, &center_cli);
            }
            center_cli.toggle_drawer();
            return glib::ExitCode::SUCCESS;
        }
        if command.as_deref() == Some("background-apps") {
            if state_cli.bar.borrow().is_none() {
                bar::create(app, &state_cli, &center_cli);
            }
            if let Some(manager) = state_cli.background_manager.borrow().as_ref() {
                manager.toggle();
            }
            return glib::ExitCode::SUCCESS;
        }
        let mode = arguments.iter().skip(1).find_map(|argument| {
            match argument.to_string_lossy().as_ref() {
                "launcher" => Some(LauncherMode::Normal),
                "manage" => Some(LauncherMode::Manage),
                _ => None,
            }
        });
        if let Some(mode) = mode {
            launcher::show(app, &state_cli, mode);
        } else if state_cli.bar.borrow().is_none() {
            bar::create(app, &state_cli, &center_cli);
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}
