mod migrate;
mod paths;
mod schema;

use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};

pub use migrate::{legacy_files, run as migrate};
pub use paths::{Paths, expand_home, pictures_dir};
pub use schema::*;

pub const DEFAULT_TEXT: &str = include_str!("default.toml");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

static PATHS: OnceLock<Paths> = OnceLock::new();
static CURRENT: RwLock<Option<Arc<Config>>> = RwLock::new(None);

pub fn paths() -> &'static Paths {
    PATHS.get_or_init(Paths::from_env)
}

pub fn current() -> Arc<Config> {
    if let Ok(guard) = CURRENT.read()
        && let Some(config) = guard.as_ref()
    {
        return config.clone();
    }
    Arc::new(Config::default())
}

pub fn set(config: Config) {
    if let Ok(mut guard) = CURRENT.write() {
        *guard = Some(Arc::new(config));
    }
}

pub fn parse(text: &str) -> Result<Config, Error> {
    let config: Config =
        toml::from_str(text).map_err(|err| Error(err.to_string().trim().to_string()))?;
    let errors = config.validate();
    if errors.is_empty() {
        Ok(config)
    } else {
        Err(Error(errors.join("\n")))
    }
}

pub fn load() -> Result<Config, Error> {
    let path = paths().config_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|err| Error(format!("{}: {err}", path.display()))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(err) => Err(Error(format!("{}: {err}", path.display()))),
    }
}

pub fn wallpapers_dir() -> PathBuf {
    let paths = paths();
    match current().general.wallpapers_dir.as_deref() {
        Some(dir) => expand_home(dir, &paths.home),
        None => paths.wallpapers.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_default() {
        assert_eq!(parse(""), Ok(Config::default()));
    }

    #[test]
    fn sample_matches_defaults() {
        assert_eq!(parse(DEFAULT_TEXT), Ok(Config::default()));
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config = parse("[general]\nscale = 1.0\n[bar]\nworkspaces = 9\n");
        let Ok(config) = config else {
            panic!("partial config failed: {config:?}");
        };
        assert_eq!(config.general.scale, 1.0);
        assert_eq!(config.general.clock_format, DEFAULT_CLOCK);
        assert_eq!(config.bar.workspaces, 9);
        assert_eq!(config.fonts, Fonts::default());
        assert_eq!(config.theme.name, "gruvbox");
    }

    #[test]
    fn syntax_errors_are_reported() {
        assert!(parse("[general\nscale = 1").is_err());
        assert!(parse("[general]\nscale = \"big\"\n").is_err());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let Err(err) = parse("[general]\nscael = 1.0\n") else {
            panic!("typo accepted");
        };
        assert!(err.0.contains("scael"), "{err}");
    }

    #[test]
    fn values_are_validated() {
        let Err(err) =
            parse("[general]\nscale = 9.0\n[bar]\nworkspaces = 0\n[theme.colors]\nbg = \"red\"\n")
        else {
            panic!("invalid values accepted");
        };
        assert!(err.0.contains("general.scale"));
        assert!(err.0.contains("bar.workspaces"));
        assert!(err.0.contains("theme.colors.bg"));
    }

    #[test]
    fn theme_colors_parse() {
        let config =
            parse("[theme]\nname = \"nord\"\nauto = true\n[theme.colors]\nbg = \"#101010\"\n");
        let Ok(config) = config else {
            panic!("theme config failed: {config:?}");
        };
        assert_eq!(config.theme.name, "nord");
        assert_eq!(config.theme.auto, Some(true));
        assert_eq!(config.theme.colors.bg.as_deref(), Some("#101010"));
        assert_eq!(parse_hex("#101010"), Some(0x101010));
        assert_eq!(parse_hex("101010"), None);
        assert_eq!(parse_hex("#10101g"), None);
    }

    #[test]
    fn keyboard_labels_prefer_longest_match() {
        let config = parse("[keyboard.labels]\nEnglish = \"EN\"\n\"English (US)\" = \"US\"\n");
        let Ok(config) = config else {
            panic!("labels failed: {config:?}");
        };
        assert_eq!(config.keyboard.label("English (US)"), "US");
        assert_eq!(config.keyboard.label("English (UK)"), "EN");
        assert_eq!(config.keyboard.label("Ukrainian"), "UK");
        let defaults = Keyboard::default();
        assert_eq!(defaults.label("Kazakh"), "KZ");
        assert_eq!(defaults.label("German (no dead keys)"), "DE");
    }
}
