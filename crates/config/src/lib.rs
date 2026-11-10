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

static APP: OnceLock<String> = OnceLock::new();

pub fn set_app(name: &str) -> bool {
    APP.set(name.to_string()).is_ok() && PATHS.get().is_none()
}

pub fn app() -> &'static str {
    APP.get().map(String::as_str).unwrap_or(paths::DEFAULT_APP)
}

pub fn paths() -> &'static Paths {
    PATHS.get_or_init(|| Paths::from_env_for(app()))
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

pub fn templates_dir() -> PathBuf {
    let paths = paths();
    match current().theme.templates.as_deref() {
        Some(dir) => expand_home(dir, &paths.home),
        None => paths.home.join(".config/matugen/templates"),
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
    fn vpn_codes_are_validated() {
        let config = parse("[vpn]\nhome = \"KZ\"\nexclude = [\"RU\", \"by\"]\n");
        let Ok(config) = config else {
            panic!("vpn section rejected");
        };
        assert_eq!(config.vpn.home.as_deref(), Some("KZ"));
        assert_eq!(config.vpn.exclude, vec!["RU", "by"]);
        assert!(parse("[vpn]\nexclude = [\"Russia\"]\n").is_err());
        assert!(parse("[vpn]\nhome = \"K1\"\n").is_err());
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

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn any_text_parses_or_reports(text in "\\PC*") {
            let _ = parse(&text);
        }

        #[test]
        fn toml_like_text_parses_or_reports(
            section in "\\[(general|fonts|bar|theme|theme\\.colors|keyboard\\.labels|commands|hooks|launcher|x)\\]",
            key in "[a-z_]{1,12}",
            value in prop_oneof![
                "-?[0-9]{1,6}(\\.[0-9]{1,3})?",
                "\"[^\"\\\\\n]{0,20}\"",
                "(true|false)",
                "\\[\\]",
            ],
        ) {
            let _ = parse(&format!("{section}\n{key} = {value}\n"));
        }

        #[test]
        fn accepted_configs_are_valid(scale in proptest::num::f64::ANY, workspaces in any::<i64>()) {
            if let Ok(config) = parse(&format!("[general]\nscale = {scale:?}\n[bar]\nworkspaces = {workspaces}\n")) {
                prop_assert!((0.5..=4.0).contains(&config.general.scale));
                prop_assert!((1..=20).contains(&config.bar.workspaces));
            }
        }

        #[test]
        fn hex_colors(value in "\\PC{0,9}") {
            if let Some(color) = parse_hex(&value) {
                prop_assert!(color <= 0xff_ffff);
                prop_assert_eq!(format!("#{color:06x}"), value.to_ascii_lowercase());
            }
        }

        #[test]
        fn keyboard_labels_never_panic(layout in "\\PC{0,40}") {
            let keyboard = Keyboard::default();
            let label = keyboard.label(&layout);
            prop_assert!(label.chars().count() <= 6 || keyboard.labels.values().any(|value| *value == label));
        }
    }
}
