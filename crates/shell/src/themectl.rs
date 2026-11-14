use std::io::Read;

use config::Config;
use services::theme::{self, Mode, Settings};

use crate::{
    app::{App, ServiceEvent},
    theme::{GRUVBOX_NAME, Palette, builtin, hex, palette_json, set_palette, static_colors},
};

pub struct ThemeCtl {
    pub mode: Mode,
    pub scheme: &'static str,
    pub palette: String,
    pub auto_preview: Option<Palette>,
    pub available: bool,
    running: bool,
    queued: Option<String>,
}

impl ThemeCtl {
    pub fn new(settings: Settings, palette: String) -> Self {
        Self {
            mode: settings.mode,
            scheme: settings.scheme,
            palette,
            auto_preview: auto_palette(&config::current()),
            available: theme::available(),
            running: false,
            queued: None,
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            mode: self.mode,
            scheme: self.scheme,
            palette: Some(self.palette.clone()),
        }
    }
}

pub fn static_palette(name: &str, config: &Config) -> Palette {
    if name == config.theme.name {
        Palette::configured(&config.theme)
    } else {
        builtin(name).unwrap_or(crate::theme::GRUVBOX)
    }
}

fn auto_palette(config: &Config) -> Option<Palette> {
    theme::read_auto_palette()
        .as_deref()
        .and_then(Palette::from_json)
        .map(|palette| palette.with_overrides(&config.theme.colors))
}

pub fn palette_for(mode: Mode, name: &str, config: &Config) -> Palette {
    let base = static_palette(name, config);
    if mode != Mode::Auto {
        return base;
    }
    auto_palette(config).unwrap_or(base)
}

pub fn load_palette(mode: Mode, name: &str) -> bool {
    set_palette(palette_for(mode, name, &config::current()))
}

fn initial_palette(saved: Option<&str>, config: &Config) -> String {
    saved
        .filter(|name| builtin(name).is_some())
        .or_else(|| builtin(&config.theme.name).map(|_| config.theme.name.as_str()))
        .unwrap_or(GRUVBOX_NAME)
        .to_string()
}

fn write_static_files(name: &str, palette: &Palette, force: bool) {
    if name == GRUVBOX_NAME && theme::has_palette(name) {
        return;
    }
    if !force && theme::has_palette(name) {
        return;
    }
    if let Err(err) =
        theme::write_static(name, &static_colors(name, palette), &palette_json(palette))
    {
        log::warn!("theme files for {name}: {err}");
    }
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

pub fn load_initial() -> (Settings, String) {
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
    let palette = initial_palette(settings.palette.as_deref(), &config);
    write_static_files(&palette, &static_palette(&palette, &config), false);
    if let Err(err) = theme::link(settings.mode, &palette) {
        log::warn!("theme link: {err}");
    }
    load_palette(settings.mode, &palette);
    (settings, palette)
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
        let base = static_palette(&self.theme.palette, &config::current());
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
        self.theme.auto_preview = auto_palette(&config::current());
        if self.theme.mode == Mode::Auto {
            self.apply_palette();
            self.cursor_palette_changed();
        } else {
            self.sync();
        }
    }

    fn save_theme_settings(&self) {
        if let Err(err) = theme::write_settings(&self.theme.settings()) {
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
        if let Err(err) = theme::link(mode, &self.theme.palette) {
            log::error!("theme link: {err}");
        }
        let regenerate = mode == Mode::Auto
            && theme::stale(&services::wallpapers::state_path())
            && !self.walls.image.is_empty();
        if regenerate {
            self.theme_wallpaper_changed();
        } else {
            let name = self.theme.palette.clone();
            let palette = static_palette(&name, &config::current());
            std::thread::spawn(move || {
                if mode == Mode::Static {
                    write_static_files(&name, &palette, false);
                }
                reload(mode)
            });
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
        if before.name != after.name && builtin(&after.name).is_some() {
            if let Err(err) = self.set_theme_palette(&after.name) {
                log::warn!("theme: {err}");
            }
        } else if before.colors != after.colors && self.theme.palette == after.name {
            self.refresh_static_palette(true);
        }
    }

    pub fn set_theme_palette(&mut self, name: &str) -> Result<(), String> {
        if builtin(name).is_none() {
            let names: Vec<&str> = crate::theme::BUILTIN
                .iter()
                .map(|(name, _)| *name)
                .collect();
            return Err(format!(
                "unknown palette: {name} (one of {})",
                names.join(", ")
            ));
        }
        let changed = self.theme.palette != name || self.theme.mode != Mode::Static;
        self.theme.palette = name.to_string();
        self.theme.mode = Mode::Static;
        self.save_theme_settings();
        if changed {
            self.refresh_static_palette(true);
        }
        Ok(())
    }

    fn refresh_static_palette(&mut self, force: bool) {
        let name = self.theme.palette.clone();
        if let Err(err) = theme::link(self.theme.mode, &name) {
            log::error!("theme link: {err}");
        }
        if self.theme.mode == Mode::Static {
            let palette = static_palette(&name, &config::current());
            std::thread::spawn(move || {
                write_static_files(&name, &palette, force);
                reload(Mode::Static);
            });
        }
        self.apply_palette();
        self.cursor_palette_changed();
        self.sync();
    }

    fn apply_palette(&mut self) {
        if load_palette(self.theme.mode, &self.theme.palette) {
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
            palette: None,
        };
        let mut config = Config::default();
        assert_eq!(settings_from_config(saved.clone(), &config, true), saved);
        assert_eq!(
            settings_from_config(saved.clone(), &config, false).mode,
            Mode::Static
        );
        config.theme.auto = Some(false);
        config.theme.scheme = Some("neutral".into());
        let settings = settings_from_config(saved.clone(), &config, true);
        assert_eq!(settings.mode, Mode::Static);
        assert_eq!(settings.scheme, "neutral");
        config.theme.scheme = Some("bogus".into());
        assert_eq!(settings_from_config(saved, &config, true).scheme, "vibrant");
    }

    #[test]
    fn static_palette_follows_config() {
        let mut config = Config::default();
        config.theme.name = "nord".into();
        assert_eq!(
            palette_for(Mode::Static, "nord", &config),
            crate::theme::NORD
        );
        assert_eq!(
            palette_for(Mode::Static, "ember", &config),
            crate::theme::EMBER
        );
        config.theme.colors.green = Some("#123456".into());
        assert_eq!(
            crate::theme::hex(palette_for(Mode::Static, "nord", &config).green),
            "#123456"
        );
        assert_eq!(
            palette_for(Mode::Static, "glacier", &config),
            crate::theme::GLACIER
        );
    }

    #[test]
    fn saved_palette_wins_over_config() {
        let mut config = Config::default();
        assert_eq!(initial_palette(None, &config), "gruvbox");
        assert_eq!(initial_palette(Some("lavender"), &config), "lavender");
        assert_eq!(initial_palette(Some("bogus"), &config), "gruvbox");
        config.theme.name = "nord".into();
        assert_eq!(initial_palette(None, &config), "nord");
        config.theme.name = "bogus".into();
        assert_eq!(initial_palette(None, &config), "gruvbox");
    }
}
