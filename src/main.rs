mod app;
mod apps;
mod background_apps;
mod bar;
mod bar_editor;
mod bar_settings;
mod config;
mod css;
mod fuzzy;
mod launcher;
mod layout;
mod menu;
mod modules;
mod network;
mod niri;
mod notification_center;
mod notifications;
mod process;
mod services;
mod ui;
#[cfg(test)]
mod ui_tests;

use app::{AppState, LauncherMode};
use gio::prelude::*;
use std::rc::Rc;

fn valid_command(command: &str) -> bool {
    matches!(
        command,
        "menu" | "launcher" | "manage" | "notifications" | "background-apps"
    ) || notifications::is_command(command)
}

fn doctor() -> glib::ExitCode {
    let mut healthy = true;
    match config::read() {
        Ok(_) => println!("Configuration: OK ({})", config::path().display()),
        Err(error) => {
            println!("Configuration: {error}");
            healthy = false;
        }
    }
    let connected = niri::window_processes().is_some();
    println!("Niri IPC: {}", if connected { "OK" } else { "unavailable" });
    healthy &= connected;
    for program in [
        "wpctl",
        "pactl",
        "nmcli",
        "brightnessctl",
        "udevadm",
        "foot",
    ] {
        let found = std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|path| path.join(program).is_file())
        });
        println!("{program}: {}", if found { "OK" } else { "missing" });
        healthy &= found;
    }
    match process::run(
        "busctl",
        &[
            "--user",
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            "org.freedesktop.Notifications",
        ],
    ) {
        Ok(owner) => println!("Notification service owner: {owner}"),
        Err(error) => {
            println!("Notification service: {error}");
            healthy = false;
        }
    }
    if healthy {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    }
}

fn main() -> glib::ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() == 1 {
        match arguments[0].as_str() {
            "--version" => {
                println!("chuhshell {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!(
                    "chuhshell [menu|launcher|manage|notifications|background-apps|doctor]\nMedia commands: volume-up, volume-down, volume-mute, microphone-mute, brightness-up, brightness-down, brightness-key-up, brightness-key-down, brightness-scroll-up, brightness-scroll-down"
                );
                return glib::ExitCode::SUCCESS;
            }
            "doctor" => return doctor(),
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
