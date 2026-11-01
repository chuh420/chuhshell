use std::{
    io,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::fs::MetadataExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use sha2::{Digest, Sha256};

const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "avif", "bmp"];
const LIVE_EXTENSIONS: [&str; 4] = ["mp4", "webm", "mkv", "mov"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallItem {
    pub path: String,
    pub name: String,
    pub live: bool,
    pub revision: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub current: String,
    pub image: String,
    pub live: bool,
    pub items: Vec<WallItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub path: String,
    pub image: String,
    pub live: bool,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

pub fn folder() -> PathBuf {
    home().join("Pictures/Wallpapers")
}

pub fn state_path() -> PathBuf {
    home().join(".local/state/rice/wallpaper.json")
}

pub fn default_wallpaper() -> PathBuf {
    home().join(".config/rice/wallpaper.png")
}

pub fn preview_cache() -> PathBuf {
    home().join(".cache/quickshell/material/wallpapers")
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn is_live(path: &Path) -> bool {
    LIVE_EXTENSIONS.contains(&extension(path).as_str())
}

fn state_value() -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(state_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn current() -> PathBuf {
    state_value()
        .and_then(|value| value.get("path")?.as_str().map(PathBuf::from))
        .filter(|path| path.is_file())
        .unwrap_or_else(default_wallpaper)
}

fn image() -> PathBuf {
    state_value()
        .and_then(|value| value.get("image")?.as_str().map(PathBuf::from))
        .filter(|path| path.is_file())
        .unwrap_or_else(current)
}

pub fn parse_state(text: &str) -> Option<State> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let path = value
        .get("path")?
        .as_str()
        .filter(|path| !path.is_empty())?;
    let image = value
        .get("image")
        .and_then(|image| image.as_str())
        .filter(|image| !image.is_empty())
        .unwrap_or(path);
    let live = match value.get("live") {
        Some(serde_json::Value::Bool(live)) => *live,
        Some(serde_json::Value::Null) | None => false,
        Some(serde_json::Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(serde_json::Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    };
    Some(State {
        path: path.to_string(),
        image: image.to_string(),
        live,
    })
}

fn revision(meta: &std::fs::Metadata) -> (i128, u64) {
    let nanos = meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128;
    (nanos, meta.size())
}

fn scan(root: &Path, allowed: &[&str]) -> Vec<WallItem> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
                && allowed.contains(&extension(path).as_str())
        })
        .collect();
    files.sort_by_cached_key(|path| {
        crate::apps::casefold(&path.file_name().unwrap_or_default().to_string_lossy())
    });
    files
        .into_iter()
        .filter_map(|path| {
            let resolved = std::fs::canonicalize(&path).ok()?;
            let meta = std::fs::metadata(&path).ok()?;
            let (mtime, size) = revision(&meta);
            Some(WallItem {
                path: resolved.to_string_lossy().into_owned(),
                name: path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                live: is_live(&path),
                revision: format!("{mtime}:{size}"),
            })
        })
        .collect()
}

pub fn list() -> Listing {
    let folder = folder();
    let mut items = scan(&folder, &EXTENSIONS);
    items.extend(scan(&folder.join("live wallpapers"), &LIVE_EXTENSIONS));
    let current = current();
    Listing {
        live: is_live(&current),
        current: current.to_string_lossy().into_owned(),
        image: image().to_string_lossy().into_owned(),
        items,
    }
}

pub fn preview_key(path: &str, mtime_ns: i128, size: u64) -> String {
    let digest = Sha256::digest(format!("640x360:{path}:{mtime_ns}:{size}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn preview_target(path: &str) -> io::Result<PathBuf> {
    let meta = std::fs::metadata(path)?;
    let (mtime, size) = revision(&meta);
    Ok(preview_cache().join(format!("{}.png", preview_key(path, mtime, size))))
}

pub fn preview_command(source: &Path, temporary: &Path) -> Command {
    let mut command = if is_live(source) {
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-y",
                "-threads",
                "1",
                "-i",
            ])
            .arg(source)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=640:360:force_original_aspect_ratio=increase,crop=640:360",
            ])
            .arg(temporary);
        command
    } else {
        let mut command = Command::new("magick");
        let mut input = source.as_os_str().to_owned();
        input.push("[0]");
        command
            .args(["-limit", "thread", "1", "-limit", "memory", "256MiB"])
            .arg(input)
            .args([
                "-auto-orient",
                "-thumbnail",
                "640x360^",
                "-gravity",
                "center",
                "-extent",
                "640x360",
            ])
            .arg(temporary);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    bind_to_parent(&mut command);
    command
}

unsafe extern "C" {
    fn prctl(option: std::ffi::c_int, arg: std::ffi::c_ulong, ...) -> std::ffi::c_int;
    fn getppid() -> std::ffi::c_int;
}

fn bind_to_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    let parent = std::process::id() as std::ffi::c_int;
    unsafe {
        command.pre_exec(move || {
            if prctl(1, 15) != 0 {
                return Err(io::Error::last_os_error());
            }
            if getppid() != parent {
                return Err(io::Error::other("preview converter lost its parent"));
            }
            Ok(())
        });
    }
}

pub fn apply_command(path: &str) -> Command {
    let mut command = Command::new("python");
    command
        .arg(home().join(".config/rice/wallpapers.py"))
        .arg("apply")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

pub struct StateWatcher {
    inotify: inotify::Inotify,
}

impl StateWatcher {
    pub fn new() -> io::Result<Self> {
        let path = state_path();
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        std::fs::create_dir_all(&dir)?;
        let inotify = inotify::Inotify::init()?;
        use inotify::WatchMask;
        inotify.watches().add(
            &dir,
            WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::DELETE,
        )?;
        Ok(Self { inotify })
    }

    pub fn drain(&mut self) -> bool {
        let mut buffer = [0u8; 4096];
        let mut changed = false;
        loop {
            match self.inotify.read_events(&mut buffer) {
                Ok(events) => {
                    for event in events {
                        changed |= event.name.is_some_and(|name| name == "wallpaper.json");
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => {
                    log::warn!("wallpaper state watch: {err}");
                    break;
                }
            }
        }
        changed
    }
}

impl AsFd for StateWatcher {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }
}

pub fn read_state() -> Option<State> {
    parse_state(&std::fs::read_to_string(state_path()).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_key_matches_python() {
        assert_eq!(
            preview_key("/a/b.png", 1_791_353_006_870_810_310, 1234),
            "3daa5c95205423b5512e7a0f34f6fd73726a7edd3c836b8002911d7d742220f4"
        );
    }

    #[test]
    fn state_follows_qml_rules() {
        let state = parse_state(r#"{"path": "/w/a.jpg", "image": "", "live": 1}"#);
        assert_eq!(
            state,
            Some(State {
                path: "/w/a.jpg".into(),
                image: "/w/a.jpg".into(),
                live: true
            })
        );
        assert_eq!(parse_state(r#"{"path": ""}"#), None);
        assert_eq!(parse_state("nope"), None);
    }

    #[test]
    fn live_extensions() {
        assert!(is_live(Path::new("/x/a.MP4")));
        assert!(!is_live(Path::new("/x/a.png")));
    }
}
