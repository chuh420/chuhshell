use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Static,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Static => "static",
        }
    }

    pub fn parse(value: &str) -> Option<Mode> {
        match value {
            "auto" => Some(Mode::Auto),
            "static" => Some(Mode::Static),
            _ => None,
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Static => "gruvbox",
        }
    }
}

pub const SCHEMES: &[&str] = &[
    "tonal-spot",
    "content",
    "fidelity",
    "vibrant",
    "rainbow",
    "neutral",
];

pub const DEFAULT_SCHEME: &str = "tonal-spot";

pub fn parse_scheme(value: &str) -> Option<&'static str> {
    let value = value.strip_prefix("scheme-").unwrap_or(value);
    SCHEMES.iter().copied().find(|scheme| *scheme == value)
}

pub fn next_scheme(current: &str) -> &'static str {
    let index = SCHEMES
        .iter()
        .position(|scheme| *scheme == current)
        .map_or(0, |index| (index + 1) % SCHEMES.len());
    SCHEMES[index]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub scheme: &'static str,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Static,
            scheme: DEFAULT_SCHEME,
        }
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

pub fn root() -> PathBuf {
    home().join(".local/state/rice/theme")
}

fn mode_path() -> PathBuf {
    home().join(".local/state/rice/theme.json")
}

fn palette_path(folder: &str) -> PathBuf {
    root().join(folder).join("idk-yet.json")
}

fn parse_settings(text: &str) -> Settings {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Settings::default();
    };
    let field = |key: &str| value.get(key).and_then(serde_json::Value::as_str);
    Settings {
        mode: field("mode").and_then(Mode::parse).unwrap_or(Mode::Static),
        scheme: field("scheme")
            .and_then(parse_scheme)
            .unwrap_or(DEFAULT_SCHEME),
    }
}

pub fn read_settings() -> Settings {
    std::fs::read_to_string(mode_path())
        .map(|text| parse_settings(&text))
        .unwrap_or_default()
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

pub fn write_settings(settings: Settings) -> io::Result<()> {
    let path = mode_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::json!({ "mode": settings.mode.name(), "scheme": settings.scheme })
        .to_string()
        + "\n";
    write_atomic(&path, &text)
}

pub fn link(mode: Mode) -> io::Result<()> {
    let root = root();
    std::fs::create_dir_all(root.join(mode.folder()))?;
    let current = root.join("current");
    if std::fs::read_link(&current).is_ok_and(|target| target == Path::new(mode.folder())) {
        return Ok(());
    }
    let temporary = root.join("current.tmp");
    let _ = std::fs::remove_file(&temporary);
    std::os::unix::fs::symlink(mode.folder(), &temporary)?;
    std::fs::rename(&temporary, &current)
}

pub fn read_palette() -> Option<String> {
    std::fs::read_to_string(palette_path("current")).ok()
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

pub fn stale(wallpaper_state: &Path) -> bool {
    match (modified(&palette_path("auto")), modified(wallpaper_state)) {
        (Some(palette), Some(state)) => state > palette,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

pub fn generate_command(image: &str, scheme: &str) -> Command {
    let mut command = Command::new("matugen");
    let scheme = format!("scheme-{scheme}");
    command
        .args([
            "image",
            image,
            "-m",
            "dark",
            "-t",
            &scheme,
            "--source-color-index",
            "0",
            "-q",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    crate::wallpapers::bind_to_parent(&mut command);
    command
}

pub fn reload_command() -> Command {
    let mut command = Command::new(home().join(".config/rice/theme-reload"));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_file_parses() {
        let auto = parse_settings(r#"{"mode": "auto"}"#);
        assert_eq!(auto.mode, Mode::Auto);
        assert_eq!(auto.scheme, DEFAULT_SCHEME);
        let vibrant = parse_settings(r#"{"mode": "static", "scheme": "vibrant"}"#);
        assert_eq!(vibrant.mode, Mode::Static);
        assert_eq!(vibrant.scheme, "vibrant");
        assert_eq!(
            parse_settings(r#"{"mode": "other", "scheme": "x"}"#),
            Settings::default()
        );
        assert_eq!(parse_settings("nope"), Settings::default());
    }

    #[test]
    fn schemes_cycle() {
        assert_eq!(next_scheme("tonal-spot"), "content");
        assert_eq!(next_scheme("neutral"), "tonal-spot");
        assert_eq!(next_scheme("unknown"), "tonal-spot");
        assert_eq!(parse_scheme("scheme-vibrant"), Some("vibrant"));
        assert_eq!(parse_scheme("expressive"), None);
    }

    #[test]
    fn modes_map_to_folders() {
        assert_eq!(Mode::Auto.folder(), "auto");
        assert_eq!(Mode::Static.folder(), "gruvbox");
        assert_eq!(Mode::parse(Mode::Auto.name()), Some(Mode::Auto));
    }
}
