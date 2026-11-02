use std::io::Read;

use services::theme::{self, Mode};

use crate::{
    app::{App, ServiceEvent},
    theme::{Palette, set_palette},
};

pub struct ThemeCtl {
    pub mode: Mode,
    running: bool,
    queued: Option<String>,
}

impl ThemeCtl {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
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

pub fn load_initial() -> Mode {
    let mode = theme::read_mode();
    if let Err(err) = theme::link(mode) {
        log::warn!("theme link: {err}");
    }
    load_palette();
    mode
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
        std::thread::spawn(move || {
            let result = run(theme::generate_command(&image));
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
        }
    }

    pub fn set_theme_mode(&mut self, mode: Mode) {
        self.theme.mode = mode;
        if let Err(err) = theme::write_mode(mode) {
            log::error!("theme mode: {err}");
        }
        if let Err(err) = theme::link(mode) {
            log::error!("theme link: {err}");
        }
        if mode == Mode::Auto && theme::stale(&services::wallpapers::state_path()) {
            self.theme_wallpaper_changed();
        } else {
            std::thread::spawn(|| {
                if let Err(err) = run(theme::reload_command()) {
                    log::warn!("theme reload: {err}");
                }
            });
        }
        self.apply_palette();
    }

    fn apply_palette(&mut self) {
        if load_palette() {
            self.retheme();
        }
    }
}
