use std::io::Read;

use services::theme::{self, Mode, Settings};

use crate::{
    app::{App, ServiceEvent},
    theme::{Palette, set_palette},
};

pub struct ThemeCtl {
    pub mode: Mode,
    pub scheme: &'static str,
    running: bool,
    queued: Option<String>,
}

impl ThemeCtl {
    pub fn new(settings: Settings) -> Self {
        Self {
            mode: settings.mode,
            scheme: settings.scheme,
            running: false,
            queued: None,
        }
    }
}

pub fn load_palette() -> bool {
    match theme::read_palette()
        .as_deref()
        .and_then(Palette::from_json)
    {
        Some(palette) => set_palette(palette),
        None => set_palette(crate::theme::GRUVBOX),
    }
}

pub fn load_initial() -> Settings {
    let settings = theme::read_settings();
    if let Err(err) = theme::link(settings.mode) {
        log::warn!("theme link: {err}");
    }
    load_palette();
    settings
}

fn run(mut command: std::process::Command) -> Result<(), String> {
    match command.spawn() {
        Ok(mut child) => {
            let mut stderr = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut stderr);
            }
            match child.wait() {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(format!("{status}: {}", stderr.trim())),
                Err(err) => Err(err.to_string()),
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

impl App {
    pub fn start_theme(&mut self) {
        if self.theme.mode == Mode::Auto
            && theme::stale(&services::wallpapers::state_path())
            && !self.walls.image.is_empty()
        {
            let image = self.walls.image.clone();
            self.generate_theme(image);
        }
    }

    pub fn theme_wallpaper_changed(&mut self) {
        if self.theme.mode == Mode::Auto && !self.walls.image.is_empty() {
            let image = self.walls.image.clone();
            self.generate_theme(image);
        }
    }

    fn generate_theme(&mut self, image: String) {
        if self.theme.running {
            self.theme.queued = Some(image);
            return;
        }
        self.theme.running = true;
        let sender = self.events.clone();
        let scheme = self.theme.scheme;
        std::thread::spawn(move || {
            let result = run(theme::generate_command(&image, scheme));
            if result.is_ok()
                && let Err(err) = run(theme::reload_command())
            {
                log::warn!("theme reload: {err}");
            }
            let _ = sender.send(ServiceEvent::ThemeGenerated(result));
        });
    }

    pub fn theme_generated(&mut self, result: Result<(), String>) {
        self.theme.running = false;
        if let Err(err) = result {
            log::error!("matugen: {err}");
        }
        if let Some(image) = self.theme.queued.take() {
            self.generate_theme(image);
            return;
        }
        if self.theme.mode == Mode::Auto {
            self.apply_palette();
            self.cursor_palette_changed();
        }
    }

    fn save_theme_settings(&self) {
        let settings = Settings {
            mode: self.theme.mode,
            scheme: self.theme.scheme,
        };
        if let Err(err) = theme::write_settings(settings) {
            log::error!("theme settings: {err}");
        }
    }

    pub fn set_theme_scheme(&mut self, scheme: &'static str) {
        if self.theme.scheme == scheme {
            return;
        }
        self.theme.scheme = scheme;
        self.save_theme_settings();
        self.theme_wallpaper_changed();
        self.sync();
    }

    pub fn set_theme_mode(&mut self, mode: Mode) {
        self.theme.mode = mode;
        self.save_theme_settings();
        if let Err(err) = theme::link(mode) {
            log::error!("theme link: {err}");
        }
        let regenerate = mode == Mode::Auto
            && theme::stale(&services::wallpapers::state_path())
            && !self.walls.image.is_empty();
        if regenerate {
            self.theme_wallpaper_changed();
        } else {
            std::thread::spawn(|| {
                if let Err(err) = run(theme::reload_command()) {
                    log::warn!("theme reload: {err}");
                }
            });
        }
        self.apply_palette();
        if !regenerate {
            self.cursor_palette_changed();
        }
    }

    fn apply_palette(&mut self) {
        if load_palette() {
            self.retheme();
        }
    }
}
