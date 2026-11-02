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

fn parse_mode(text: &str) -> Option<Mode> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    Mode::parse(value.get("mode")?.as_str()?)
}

pub fn read_mode() -> Mode {
    std::fs::read_to_string(mode_path())
        .ok()
        .and_then(|text| parse_mode(&text))
        .unwrap_or(Mode::Static)
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

pub fn write_mode(mode: Mode) -> io::Result<()> {
    let path = mode_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::json!({ "mode": mode.name() }).to_string() + "\n";
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

pub fn generate_command(image: &str) -> Command {
    let mut command = Command::new("matugen");
    command
        .args([
            "image",
            image,
            "-m",
            "dark",
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
        assert_eq!(parse_mode(r#"{"mode": "auto"}"#), Some(Mode::Auto));
        assert_eq!(parse_mode(r#"{"mode": "static"}"#), Some(Mode::Static));
        assert_eq!(parse_mode(r#"{"mode": "other"}"#), None);
        assert_eq!(parse_mode("nope"), None);
    }

    #[test]
    fn modes_map_to_folders() {
        assert_eq!(Mode::Auto.folder(), "auto");
        assert_eq!(Mode::Static.folder(), "gruvbox");
        assert_eq!(Mode::parse(Mode::Auto.name()), Some(Mode::Auto));
    }
}
