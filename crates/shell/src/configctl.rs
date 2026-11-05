use std::{os::fd::AsFd, time::Duration};

use calloop::{
    PostAction,
    generic::Generic,
    {Interest, Mode},
};
use config::Config;
use services::{config_watch::Watcher, notifications::Urgency};
use ui::TextEngine;

use crate::{app::App, model::format_clock_with};

#[derive(Default)]
pub struct ConfigCtl {
    watcher: Option<Watcher>,
    reload_pending: bool,
    pending_error: Option<String>,
}

pub fn check(config: &Config) -> Result<(), String> {
    let now = services::clock::now();
    let mut errors = Vec::new();
    if let Err(err) = jiff::fmt::strtime::format(config.general.clock_format.as_str(), &now) {
        errors.push(format!("general.clock_format: {err}"));
    }
    if crate::theme::builtin(&config.theme.name).is_none() {
        let names: Vec<&str> = crate::theme::BUILTIN
            .iter()
            .map(|(name, _)| *name)
            .collect();
        errors.push(format!(
            "theme.name: \"{}\" is not one of {}",
            config.theme.name,
            names.join(", ")
        ));
    }
    if let Some(scheme) = config.theme.scheme.as_deref()
        && services::theme::parse_scheme(scheme).is_none()
    {
        errors.push(format!(
            "theme.scheme: \"{scheme}\" is not one of {}",
            services::theme::SCHEMES.join(", ")
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

pub fn load() -> Result<Config, String> {
    let config = config::load().map_err(|err| err.to_string())?;
    check(&config)?;
    Ok(config)
}

pub fn startup() -> Option<String> {
    match config::migrate(config::paths()) {
        Ok(copied) if !copied.is_empty() => {
            log::info!("migrated state files: {}", copied.join(", "));
        }
        Ok(_) => {}
        Err(err) => log::warn!("state migration: {err}"),
    }
    let (config, error) = match load() {
        Ok(config) => (config, None),
        Err(err) => {
            log::error!("config: {err}");
            (Config::default(), Some(err))
        }
    };
    crate::theme::apply_config(&config);
    config::set(config);
    error
}

pub fn cli(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("check") => {
            let path = config::paths().config_file();
            match load() {
                Ok(_) if path.is_file() => {
                    println!("{}: ok", path.display());
                    0
                }
                Ok(_) => {
                    println!("{} does not exist, using defaults", path.display());
                    0
                }
                Err(err) => {
                    eprintln!("{err}");
                    1
                }
            }
        }
        Some("default") => {
            print!("{}", config::DEFAULT_TEXT);
            0
        }
        Some("path") => {
            println!("{}", config::paths().config_file().display());
            0
        }
        _ => {
            eprintln!("usage: chuhshell config check|default|path");
            2
        }
    }
}

impl App {
    pub fn start_config(&mut self, startup_error: Option<String>) {
        self.config.pending_error = startup_error;
        match Watcher::new() {
            Ok(watcher) => match watcher.as_fd().try_clone_to_owned() {
                Ok(fd) => {
                    self.config.watcher = Some(watcher);
                    let source = Generic::new(fd, Interest::READ, Mode::Level);
                    let result = self.handle.insert_source(source, |_, _, app| {
                        let changed = app.config.watcher.as_mut().is_some_and(Watcher::drain);
                        if changed {
                            app.schedule_config_reload();
                        }
                        Ok(PostAction::Continue)
                    });
                    if let Err(err) = result {
                        log::error!("config watch: {err}");
                    }
                }
                Err(err) => log::error!("config watch: {err}"),
            },
            Err(err) => log::error!("config watch: {err}"),
        }
    }

    fn schedule_config_reload(&mut self) {
        if self.config.reload_pending {
            return;
        }
        self.config.reload_pending = true;
        self.add_timer(Duration::from_millis(150), |app| {
            app.config.reload_pending = false;
            app.reload_config();
            None
        });
    }

    pub fn reload_config(&mut self) {
        let next = match load() {
            Ok(config) => config,
            Err(err) => {
                log::error!("config: {err}");
                self.config.pending_error = Some(err);
                self.flush_config_error();
                return;
            }
        };
        let previous = config::current();
        if *previous == next {
            return;
        }
        log::info!("config reloaded");
        let changes = crate::theme::apply_config(&next);
        let walls_moved = previous.general.wallpapers_dir != next.general.wallpapers_dir;
        config::set(next);
        self.theme_config_changed(&previous);
        if changes.fonts {
            self.engine = TextEngine::new(&crate::theme::font_families());
        }
        self.refresh_config_values();
        if changes.scale || changes.fonts {
            self.rebuild_surfaces();
        } else {
            self.retheme();
        }
        if walls_moved {
            self.refresh_wallpapers();
        }
    }

    fn refresh_config_values(&mut self) {
        let config = config::current();
        self.model.clock = format_clock_with(&services::clock::now(), &config.general.clock_format);
        if !self.state.keyboard.name.is_empty() {
            self.state.keyboard.label = config.keyboard.label(&self.state.keyboard.name);
        }
    }

    pub fn flush_config_error(&mut self) {
        let Some(handle) = self.notes.handle.clone() else {
            return;
        };
        if let Some(err) = self.config.pending_error.take() {
            handle.post(
                crate::APP_NAME,
                "Config error",
                &format!("{err}\nThe shell keeps the last working settings."),
                Urgency::Critical,
            );
        }
    }
}
