use std::{collections::BTreeMap, path::PathBuf};

use serde::Deserialize;

pub const DEFAULT_CLOCK: &str = "%a, %-d %b  ·  %H:%M";

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub fonts: Fonts,
    pub bar: Bar,
    pub launcher: Launcher,
    pub keyboard: Keyboard,
    pub theme: Theme,
    pub commands: Commands,
    pub hooks: Hooks,
    pub vpn: Vpn,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub scale: f32,
    pub terminal: Option<String>,
    pub clock_format: String,
    pub wallpapers_dir: Option<PathBuf>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            scale: 1.22,
            terminal: None,
            clock_format: DEFAULT_CLOCK.into(),
            wallpapers_dir: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fonts {
    pub ui: String,
    pub icons: String,
    pub icon_fallback: String,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            ui: "Adwaita Sans".into(),
            icons: "JetBrainsMono Nerd Font Mono".into(),
            icon_fallback: "JetBrainsMonoNL Nerd Font".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bar {
    pub workspaces: u32,
}

impl Default for Bar {
    fn default() -> Self {
        Self { workspaces: 5 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Launcher {
    pub pinned: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    pub labels: BTreeMap<String, String>,
}

impl Default for Keyboard {
    fn default() -> Self {
        let labels = [
            ("English", "EN"),
            ("Russian", "RU"),
            ("Kazakh", "KZ"),
            ("German", "DE"),
            ("French", "FR"),
        ];
        Self {
            labels: labels
                .into_iter()
                .map(|(name, label)| (name.to_string(), label.to_string()))
                .collect(),
        }
    }
}

impl Keyboard {
    pub fn label(&self, layout: &str) -> String {
        self.labels
            .iter()
            .filter(|(name, _)| layout.contains(name.as_str()))
            .max_by_key(|(name, _)| name.len())
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| layout.chars().take(2).collect::<String>().to_uppercase())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub bg: Option<String>,
    pub dim: Option<String>,
    pub surface: Option<String>,
    pub elevated: Option<String>,
    pub line: Option<String>,
    pub text: Option<String>,
    pub muted: Option<String>,
    pub green: Option<String>,
    pub yellow: Option<String>,
    pub red: Option<String>,
    pub blue: Option<String>,
}

impl Colors {
    pub fn entries(&self) -> [(&'static str, Option<&str>); 11] {
        [
            ("bg", self.bg.as_deref()),
            ("dim", self.dim.as_deref()),
            ("surface", self.surface.as_deref()),
            ("elevated", self.elevated.as_deref()),
            ("line", self.line.as_deref()),
            ("text", self.text.as_deref()),
            ("muted", self.muted.as_deref()),
            ("green", self.green.as_deref()),
            ("yellow", self.yellow.as_deref()),
            ("red", self.red.as_deref()),
            ("blue", self.blue.as_deref()),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub name: String,
    pub colors: Colors,
    pub auto: Option<bool>,
    pub scheme: Option<String>,
    pub templates: Option<PathBuf>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "gruvbox".into(),
            colors: Colors::default(),
            auto: None,
            scheme: None,
            templates: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Commands {
    pub lock: Option<String>,
    pub screenshot: Option<String>,
    pub mixer: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hooks {
    pub wallpaper_changed: Option<String>,
    pub theme_changed: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vpn {
    pub home: Option<String>,
    pub exclude: Vec<String>,
}

fn country_code(value: &str) -> bool {
    value.len() == 2 && value.chars().all(|c| c.is_ascii_alphabetic())
}

pub fn parse_hex(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

impl Config {
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let scale = self.general.scale;
        if !(0.5..=4.0).contains(&scale) {
            errors.push(format!("general.scale: {scale} is outside 0.5..4"));
        }
        if self.general.clock_format.trim().is_empty() {
            errors.push("general.clock_format: empty".into());
        }
        if !(1..=20).contains(&self.bar.workspaces) {
            errors.push(format!(
                "bar.workspaces: {} is outside 1..20",
                self.bar.workspaces
            ));
        }
        for (key, value) in [
            ("fonts.ui", &self.fonts.ui),
            ("fonts.icons", &self.fonts.icons),
            ("fonts.icon_fallback", &self.fonts.icon_fallback),
        ] {
            if value.trim().is_empty() {
                errors.push(format!("{key}: empty"));
            }
        }
        for (key, value) in self.theme.colors.entries() {
            if let Some(value) = value
                && parse_hex(value).is_none()
            {
                errors.push(format!(
                    "theme.colors.{key}: \"{value}\" is not a #rrggbb color"
                ));
            }
        }
        for (key, value) in [
            ("general.terminal", &self.general.terminal),
            ("commands.lock", &self.commands.lock),
            ("commands.screenshot", &self.commands.screenshot),
            ("commands.mixer", &self.commands.mixer),
            ("hooks.wallpaper_changed", &self.hooks.wallpaper_changed),
            ("hooks.theme_changed", &self.hooks.theme_changed),
        ] {
            if value
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                errors.push(format!("{key}: empty"));
            }
        }
        if let Some(home) = &self.vpn.home
            && !country_code(home)
        {
            errors.push(format!(
                "vpn.home: \"{home}\" is not a two-letter country code"
            ));
        }
        for code in &self.vpn.exclude {
            if !country_code(code) {
                errors.push(format!(
                    "vpn.exclude: \"{code}\" is not a two-letter country code"
                ));
            }
        }
        errors
    }
}
