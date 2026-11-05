use std::io::Read;

use config::Config;
use services::theme::{self, Mode, Settings};

use crate::{
    app::{App, ServiceEvent},
    theme::{Palette, hex, set_palette},
};

pub struct ThemeCtl {
    pub mode: Mode,
    pub scheme: &'static str,
    pub available: bool,
    running: bool,
    queued: Option<String>,
}

impl ThemeCtl {
    pub fn new(settings: Settings) -> Self {
        Self {
            mode: settings.mode,
            scheme: settings.scheme,
            available: theme::available(),
            running: false,
            queued: None,
        }
    }
}

pub fn palette_for(mode: Mode, config: &Config) -> Palette {
    let base = Palette::configured(&config.theme);
    if mode != Mode::Auto {
        return base;
    }
    theme::read_auto_palette()
        .as_deref()
        .and_then(Palette::from_json)
        .map(|palette| palette.with_overrides(&config.theme.colors))
        .unwrap_or(base)
}

pub fn load_palette(mode: Mode) -> bool {
    set_palette(palette_for(mode, &config::current()))
}

fn settings_from_config(mut settings: Settings, config: &Config, available: bool) -> Settings {
    if let Some(auto) = config.theme.auto {
        settings.mode = if auto { Mode::Auto } else { Mode::Static };
    }
    if let Some(scheme) = config.theme.scheme.as_deref().and_then(theme::parse_scheme) {
        settings.scheme = scheme;
    }
    if !available {
        settings.mode = Mode::Static;
    }
    settings
}

pub fn load_initial() -> Settings {
    match theme::adopt_legacy() {
        Ok(true) => log::info!("copied the theme folder into the chuhshell state folder"),
        Ok(false) => {}
        Err(err) => log::warn!("theme folder migration: {err}"),
    }
    let config = config::current();
    let available = theme::available();
    if !available {
        services::programs::warn_missing("matugen", "wallpaper colors");
    }
    let settings = settings_from_config(theme::read_settings(), &config, available);
    if let Err(err) = theme::link(settings.mode, &config.theme.name) {
        log::warn!("theme link: {err}");
    }
    load_palette(settings.mode);
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

fn reload(mode: Mode) {
    if let Some(command) = theme::reload_command(mode)
        && let Err(err) = run(command)
    {
        log::warn!("theme_changed hook: {err}");
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
        if !self.theme.available {
            return;
        }
        if self.theme.running {
            self.theme.queued = Some(image);
            return;
        }
        self.theme.running = true;
        let sender = self.events.clone();
        let scheme = self.theme.scheme;
        let base = Palette::configured(&config::current().theme);
        let (yellow, red, blue) = (hex(base.yellow), hex(base.red), hex(base.blue));
        std::thread::spawn(move || {
            let result = theme::prepare_matugen(&yellow, &red, &blue)
                .map_err(|err| format!("matugen config: {err}"))
                .and_then(|config_path| run(theme::generate_command(&image, scheme, &config_path)));
            if result.is_ok() {
                reload(Mode::Auto);
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

    pub fn set_theme_mode(&mut self, mode: Mode) -> Result<(), String> {
        if mode == Mode::Auto && !self.theme.available {
            return Err("wallpaper colors need matugen".into());
        }
        self.theme.mode = mode;
        self.save_theme_settings();
        if let Err(err) = theme::link(mode, &config::current().theme.name) {
            log::error!("theme link: {err}");
        }
        let regenerate = mode == Mode::Auto
            && theme::stale(&services::wallpapers::state_path())
            && !self.walls.image.is_empty();
        if regenerate {
            self.theme_wallpaper_changed();
        } else {
            std::thread::spawn(move || reload(mode));
        }
        self.apply_palette();
        if !regenerate {
            self.cursor_palette_changed();
        }
        Ok(())
    }

    pub fn theme_config_changed(&mut self, previous: &Config) {
        let next = config::current();
        let (before, after) = (&previous.theme, &next.theme);
        if before.auto != after.auto
            && let Some(auto) = after.auto
        {
            let mode = if auto { Mode::Auto } else { Mode::Static };
            if let Err(err) = self.set_theme_mode(mode) {
                log::warn!("theme: {err}");
            }
        }
        if before.scheme != after.scheme
            && let Some(scheme) = after.scheme.as_deref().and_then(theme::parse_scheme)
        {
            self.set_theme_scheme(scheme);
        }
        if before.name != after.name || before.colors != after.colors {
            if let Err(err) = theme::link(self.theme.mode, &after.name) {
                log::error!("theme link: {err}");
            }
            if self.theme.mode == Mode::Static {
                let mode = self.theme.mode;
                std::thread::spawn(move || reload(mode));
            }
            self.apply_palette();
            self.cursor_palette_changed();
        }
    }

    fn apply_palette(&mut self) {
        if load_palette(self.theme.mode) {
            self.retheme();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_overrides_saved_settings() {
        let saved = Settings {
            mode: Mode::Auto,
            scheme: "vibrant",
        };
        let mut config = Config::default();
        assert_eq!(settings_from_config(saved, &config, true), saved);
        assert_eq!(
            settings_from_config(saved, &config, false).mode,
            Mode::Static
        );
        config.theme.auto = Some(false);
        config.theme.scheme = Some("neutral".into());
        let settings = settings_from_config(saved, &config, true);
        assert_eq!(settings.mode, Mode::Static);
        assert_eq!(settings.scheme, "neutral");
        config.theme.scheme = Some("bogus".into());
        assert_eq!(settings_from_config(saved, &config, true).scheme, "vibrant");
    }

    #[test]
    fn static_palette_follows_config() {
        let mut config = Config::default();
        config.theme.name = "nord".into();
        assert_eq!(palette_for(Mode::Static, &config), crate::theme::NORD);
    }
}
