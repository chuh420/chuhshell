mod app;
mod apps;
mod background_apps;
mod bar;
mod bar_editor;
mod bar_settings;
mod bluetooth;
mod clipboard;
mod config;
mod controls;
mod css;
mod doctor;
mod fuzzy;
mod info;
mod keybindings;
mod launcher;
mod layout;
mod menu;
mod modules;
mod monitor;
mod network;
mod niri;
mod notification_center;
mod notifications;
mod paths;
mod power;
mod process;
mod services;
mod storage;
mod todo;
mod ui;
#[cfg(test)]
mod ui_tests;
mod weather;

use app::{AppState, LauncherMode};
use gio::prelude::*;
use std::rc::Rc;

fn valid_command(command: &str) -> bool {
    matches!(
        command,
        "wallpaper"
            | "clipboard"
            | "menu"
            | "launcher"
            | "manage"
            | "notifications"
            | "background-apps"
    ) || notifications::is_command(command)
}

fn main() -> glib::ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|s| s == "installation-plan") {
        let result = (|| {
            let destination = arguments.get(1).ok_or("Missing installation destination")?;
            let mut root = None;
            let mut preserve = false;
            let mut options = arguments[2..].iter();
            while let Some(option) = options.next() {
                match option.as_str() {
                    "--preserve" => preserve = true,
                    "--config" => {
                        root = Some(std::path::Path::new(
                            options.next().ok_or("Missing config path")?,
                        ))
                    }
                    _ => return Err("Unknown installation option".to_string()),
                }
            }
            crate::keybindings::niri::installation_plan(
                std::path::Path::new(destination),
                root,
                preserve,
            )
        })();
        return match result {
            Ok(plan) => {
                println!("{plan}");
                glib::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if arguments.as_slice() == ["doctor", "--json"] {
        return doctor::run(true);
    }
    if arguments.as_slice() == ["clipboard-capture"] {
        clipboard::capture();
        return glib::ExitCode::SUCCESS;
    }
    if arguments.len() == 1 {
        match arguments[0].as_str() {
            "--version" => {
                println!("chuhshell {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!(
                    "chuhshell [menu|wallpaper|clipboard|launcher|manage|notifications|background-apps|doctor [--json]]\nMedia commands: volume-up, volume-down, volume-mute, microphone-mute, brightness-up, brightness-down, brightness-key-up, brightness-key-down, brightness-scroll-up, brightness-scroll-down"
                );
                return glib::ExitCode::SUCCESS;
            }
            "doctor" => return doctor::run(false),
            _ => {}
        }
    }
    if arguments.len() > 1
        || arguments
            .first()
            .is_some_and(|command| !valid_command(command))
    {
        eprintln!("chuhshell: unknown command or extra arguments; use --help");
        return glib::ExitCode::FAILURE;
    }
    if let Err(error) = config::read() {
        eprintln!("chuhshell: invalid configuration: {error}");
        return glib::ExitCode::FAILURE;
    }
    let app = gtk::Application::builder()
        .application_id("dev.chuh.chuhshell")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|app| {
        css::install();
        let guard = app.hold();
        app.connect_shutdown(move |_| {
            let _ = &guard;
            crate::storage::shutdown();
            process::shutdown();
        });
        for signal in [libc::SIGTERM, libc::SIGINT] {
            let weak = app.downgrade();
            glib_unix::unix_signal_add_local(signal, move || {
                if let Some(app) = weak.upgrade() {
                    app.quit();
                }
                glib::ControlFlow::Break
            });
        }
    });
    let center = notification_center::NotificationCenter::new(&app);
    app.connect_startup({
        let center = Rc::clone(&center);
        move |_| center.start()
    });
    let state = Rc::new(AppState::default());
    app.connect_command_line(move |app, command_line| {
        let arguments = command_line.arguments();
        let command = arguments.get(1).map(|s| s.to_string_lossy().into_owned());
        if arguments.len() > 2
            || command
                .as_deref()
                .is_some_and(|command| !valid_command(command))
        {
            command_line.printerr_literal("Unknown command or extra arguments\n");
            return glib::ExitCode::FAILURE;
        }
        state.clipboard.start(&state);
        bar::create(app, &state, &center);
        if let Some(command) = command
            .as_deref()
            .filter(|command| notifications::is_command(command))
        {
            let rx = notifications::submit(&state, command);
            let state = Rc::clone(&state);
            let line = command_line.clone();
            let hold = app.hold();
            glib::MainContext::default().spawn_local(async move {
                let _hold = hold;
                match rx.recv().await {
                    Ok(Ok(notice)) => notifications::show(&state, notice),
                    result => {
                        let error = match result {
                            Ok(Err(error)) => error,
                            _ => "System command worker stopped".into(),
                        };
                        line.printerr_literal(&format!("chuhshell: {error}\n"));
                        line.set_exit_code(glib::ExitCode::FAILURE);
                    }
                }
            });
        } else {
            match command.as_deref() {
                Some("notifications") => center.toggle_drawer(),
                Some("background-apps") => {
                    if let Some(manager) = state.background_manager.borrow().as_ref() {
                        manager.toggle();
                    }
                }
                Some("clipboard") => menu::show_clipboard(app, &state),
                Some("wallpaper") => menu::show_wallpaper(app, &state),
                Some("menu") => menu::show(app, &state),
                Some("launcher") => launcher::show(app, &state, LauncherMode::Normal),
                Some("manage") => launcher::show(app, &state, LauncherMode::Manage),
                _ => {}
            }
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_commands_are_rejected() {
        assert!(!super::valid_command("nonsense"));
        assert!(super::valid_command("launcher"));
        assert!(super::valid_command("volume-up"));
    }
}
