mod app;
mod apps;
mod bar;
mod css;
mod fuzzy;
mod launcher;
mod modules;
mod niri;
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

    let state = Rc::new(AppState::default());
    let state_cli = Rc::clone(&state);
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
            bar::create(app, &state_cli);
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}
