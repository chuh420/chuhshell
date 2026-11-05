use std::{
    collections::BTreeMap,
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use sha2::{Digest, Sha256};

use crate::cursor_colors::{self, Recolor};

pub const DEFAULT_THEME: &str = "Capitaine Cursors (Gruvbox)";
pub const DEFAULT_SIZE: u32 = 24;
pub const SIZES: [u32; 5] = [24, 32, 40, 48, 64];
pub const SLOTS: [&str; 2] = [".chuhshell-colors-a", ".chuhshell-colors-b"];
pub const PREVIEW_SHAPES: [&[&str]; 5] = [
    &["default", "left_ptr"],
    &["pointer", "hand2"],
    &["text", "xterm"],
    &["progress", "left_ptr_watch", "wait", "watch"],
    &["not-allowed", "crossed_circle", "forbidden"],
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorTheme {
    pub name: String,
    pub path: String,
    pub revision: String,
    pub animated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub theme: String,
    pub size: u32,
    pub animation: bool,
    pub sizes: BTreeMap<String, u32>,
    pub colors: BTreeMap<String, bool>,
    pub applied: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME.into(),
            size: DEFAULT_SIZE,
            animation: true,
            sizes: BTreeMap::new(),
            colors: BTreeMap::new(),
            applied: String::new(),
        }
    }
}

impl State {
    pub fn size_for(&self, theme: &str) -> u32 {
        self.sizes.get(theme).copied().unwrap_or(DEFAULT_SIZE)
    }

    pub fn effective(&self) -> &str {
        if self.applied.is_empty() {
            &self.theme
        } else {
            &self.applied
        }
    }

    pub fn colors_wanted(&self) -> bool {
        self.colors.get(&self.theme) != Some(&false)
    }
}

pub fn is_slot(name: &str) -> bool {
    SLOTS.contains(&name)
}

pub fn next_size(size: u32) -> u32 {
    SIZES
        .iter()
        .copied()
        .find(|&candidate| candidate > size)
        .unwrap_or(SIZES[0])
}

fn valid_size(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .filter(|size| (8..=256).contains(size))
        .map(|size| size as u32)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub hotspot: (u32, u32),
    pub delay: u32,
    pub pixels: Vec<u32>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

pub fn folder() -> PathBuf {
    crate::wallpapers::folder().join("cursors")
}

pub fn state_path() -> PathBuf {
    config::paths().state_file("cursor.json")
}

pub fn lua_path() -> PathBuf {
    home().join(".local/state/rice/cursor.lua")
}

pub fn icons_dir() -> PathBuf {
    home().join(".local/share/icons")
}

pub fn preview_cache() -> PathBuf {
    config::paths().cache_dir("cursors")
}

fn revision(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", meta.mtime(), meta.mtime_nsec()))
}

fn has_frames(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    parse(&bytes).is_some_and(|images| {
        let first = images.first().map(|image| image.width);
        images
            .iter()
            .filter(|image| Some(image.width) == first)
            .count()
            > 1
    })
}

pub fn scan(root: &Path) -> Vec<CursorTheme> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut themes: Vec<CursorTheme> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.join("cursors").is_dir()
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .filter_map(|path| {
            let cursors = path.join("cursors");
            let revision = revision(&cursors)?;
            let animated = ["progress", "left_ptr_watch", "wait", "pointer"]
                .iter()
                .any(|name| has_frames(&cursors.join(name)));
            Some(CursorTheme {
                name: path.file_name()?.to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                revision,
                animated,
            })
        })
        .collect();
    themes.sort_by_cached_key(|theme| crate::apps::casefold(&theme.name));
    themes
}

pub fn list() -> Vec<CursorTheme> {
    scan(&folder())
}

pub fn parse_state(text: &str) -> Option<State> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let theme = value
        .get("theme")?
        .as_str()
        .filter(|theme| !theme.is_empty())?;
    let size = value
        .get("size")
        .and_then(valid_size)
        .unwrap_or(DEFAULT_SIZE);
    let animation = value
        .get("animation")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let mut sizes: BTreeMap<String, u32> = value
        .get("sizes")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(name, size)| Some((name.clone(), valid_size(size)?)))
                .collect()
        })
        .unwrap_or_default();
    sizes.entry(theme.to_string()).or_insert(size);
    let colors = value
        .get("colors")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(name, on)| Some((name.clone(), on.as_bool()?)))
                .collect()
        })
        .unwrap_or_default();
    let applied = value
        .get("applied")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(State {
        theme: theme.to_string(),
        size,
        animation,
        sizes,
        colors,
        applied,
    })
}

pub fn read_state() -> State {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|text| parse_state(&text))
        .unwrap_or_default()
}

fn lua_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\{}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub fn lua_state(state: &State) -> String {
    format!(
        "return {{ theme = {}, size = {}, animation = {} }}\n",
        lua_string(state.effective()),
        state.size,
        state.animation
    )
}

fn atomic_write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.{}", std::process::id()));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

pub fn write_state(state: &State) -> io::Result<()> {
    let json = serde_json::json!({
        "theme": state.theme,
        "size": state.size,
        "animation": state.animation,
        "sizes": state.sizes,
        "colors": state.colors,
        "applied": state.effective(),
    });
    atomic_write(&state_path(), &format!("{json:#}\n"))?;
    atomic_write(&lua_path(), &lua_state(state))
}

pub fn replace_setting(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let matches = trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='));
            if matches {
                found = true;
                format!("{key} = {value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        let section = lines.iter().position(|line| line.trim() == "[Settings]");
        match section {
            Some(index) => lines.insert(index + 1, format!("{key} = {value}")),
            None => {
                lines.insert(0, "[Settings]".into());
                lines.insert(1, format!("{key} = {value}"));
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

pub fn dot_icons() -> PathBuf {
    home().join(".icons")
}

fn link_theme(name: &str, path: &Path) -> io::Result<()> {
    let visible = [icons_dir(), dot_icons()]
        .iter()
        .any(|dir| dir.join(name).join("cursors").is_dir());
    if visible {
        return Ok(());
    }
    let icons = dot_icons();
    match std::fs::symlink_metadata(&icons) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            std::os::unix::fs::symlink(folder(), &icons)
        }
        Ok(meta) if meta.is_dir() => std::os::unix::fs::symlink(path, icons.join(name)),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a folder",
            icons.display()
        ))),
        Err(err) => Err(err),
    }
}

fn update_gtk(theme: &str, size: u32) -> io::Result<()> {
    for version in ["gtk-3.0", "gtk-4.0"] {
        let path = home().join(".config").join(version).join("settings.ini");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let text = replace_setting(&text, "gtk-cursor-theme-name", theme);
        let text = replace_setting(&text, "gtk-cursor-theme-size", &size.to_string());
        atomic_write(&path, &text)?;
    }
    atomic_write(
        &icons_dir().join("default/index.theme"),
        &format!("[Icon Theme]\nInherits={theme}\n"),
    )
}

fn run(command: &mut Command) -> Result<(), String> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }
    if !stdout.is_empty() && stdout != "ok" {
        return Err(stdout);
    }
    Ok(())
}

pub fn apply(
    theme: &CursorTheme,
    state: &State,
    palette: Option<Recolor>,
) -> Result<State, String> {
    link_theme(&theme.name, Path::new(&theme.path))
        .map_err(|err| format!("Could not link cursor theme: {err}"))?;
    let mut next = State {
        theme: theme.name.clone(),
        size: state.size_for(&theme.name),
        ..state.clone()
    };
    next.sizes.insert(next.theme.clone(), next.size);
    let source = Path::new(&theme.path);
    let enabled = *next
        .colors
        .entry(theme.name.clone())
        .or_insert_with(|| !cursor_colors::theme_drawn(source));
    render(next, source, palette.filter(|_| enabled))
}

pub fn set_colors(
    theme: &CursorTheme,
    state: &State,
    enabled: bool,
    palette: Option<Recolor>,
) -> Result<State, String> {
    let mut next = state.clone();
    next.colors.insert(theme.name.clone(), enabled);
    apply(theme, &next, palette)
}

fn remove_slots(keep: &str) {
    for slot in SLOTS.iter().filter(|slot| **slot != keep) {
        let path = folder().join(slot);
        if let Err(err) = std::fs::remove_dir_all(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            log::warn!("cursor colors {}: {err}", path.display());
        }
    }
}

fn render(mut next: State, source: &Path, palette: Option<Recolor>) -> Result<State, String> {
    let previous = next.effective().to_string();
    next.applied = match palette {
        Some(palette) => {
            let slot = if previous == SLOTS[0] {
                SLOTS[1]
            } else {
                SLOTS[0]
            };
            let mapping = cursor_colors::theme_mapping(source, palette)
                .ok_or_else(|| "This cursor theme cannot be recolored".to_string())?;
            let target = folder().join(slot);
            cursor_colors::write_theme(source, &target, slot, &mapping)
                .map_err(|err| format!("Could not recolor cursor theme: {err}"))?;
            link_theme(slot, &target)
                .map_err(|err| format!("Could not link cursor theme: {err}"))?;
            slot.to_string()
        }
        None => next.theme.clone(),
    };
    let result = activate(next);
    match &result {
        Ok(state) => remove_slots(state.effective()),
        Err(_) => remove_slots(&previous),
    }
    result
}

pub fn set_size(state: &State, size: u32) -> Result<State, String> {
    let mut next = State {
        size,
        ..state.clone()
    };
    next.sizes.insert(next.theme.clone(), size);
    activate(next)
}

fn activate(next: State) -> Result<State, String> {
    run(Command::new("hyprctl")
        .arg("setcursor")
        .arg(next.effective())
        .arg(next.size.to_string()))?;
    if let Err(err) = update_gtk(next.effective(), next.size) {
        log::warn!("cursor gtk settings: {err}");
    }
    if let Err(err) = run(Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "cursor-theme"])
        .arg(next.effective()))
    {
        log::warn!("cursor gsettings: {err}");
    }
    if let Err(err) = run(Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "cursor-size"])
        .arg(next.size.to_string()))
    {
        log::warn!("cursor gsettings: {err}");
    }
    write_state(&next).map_err(|err| format!("Could not save cursor state: {err}"))?;
    Ok(next)
}

pub fn animation_script(enabled: bool) -> String {
    format!(
        "if hl.plugin.dynamic_cursors then hl.config({{ plugin = {{ dynamic_cursors = {{ enabled = {enabled} }} }} }}) end"
    )
}

pub fn set_animation(state: &State, enabled: bool) -> Result<State, String> {
    run(Command::new("hyprctl")
        .arg("eval")
        .arg(animation_script(enabled)))?;
    let next = State {
        animation: enabled,
        ..state.clone()
    };
    write_state(&next).map_err(|err| format!("Could not save cursor state: {err}"))?;
    Ok(next)
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

pub fn parse(bytes: &[u8]) -> Option<Vec<Image>> {
    if bytes.get(0..4)? != b"Xcur" {
        return None;
    }
    let header = u32_at(bytes, 4)? as usize;
    let count = u32_at(bytes, 12)? as usize;
    let mut images = Vec::new();
    for index in 0..count.min(4096) {
        let entry = header + index * 12;
        if u32_at(bytes, entry)? != 0xfffd_0002 {
            continue;
        }
        let pos = u32_at(bytes, entry + 8)? as usize;
        let width = u32_at(bytes, pos + 16)?;
        let height = u32_at(bytes, pos + 20)?;
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            continue;
        }
        let hotspot = (u32_at(bytes, pos + 24)?, u32_at(bytes, pos + 28)?);
        let delay = u32_at(bytes, pos + 32)?;
        let start = pos + 36;
        let len = (width * height) as usize;
        let data = bytes.get(start..start + len * 4)?;
        let pixels = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| u32::from_le_bytes(*px))
            .collect();
        images.push(Image {
            width,
            height,
            hotspot,
            delay,
            pixels,
        });
    }
    (!images.is_empty()).then_some(images)
}

pub fn best_image(images: Vec<Image>, size: u32) -> Option<Image> {
    let mut best: Option<Image> = None;
    for image in images {
        let better = match &best {
            None => true,
            Some(current) => {
                let a = image.width.abs_diff(size);
                let b = current.width.abs_diff(size);
                a < b || (a == b && image.width > current.width)
            }
        };
        if better {
            best = Some(image);
        }
    }
    best
}

pub fn load_shape(theme: &Path, names: &[&str], size: u32) -> Option<Image> {
    names.iter().find_map(|name| {
        let bytes = std::fs::read(theme.join("cursors").join(name)).ok()?;
        best_image(parse(&bytes)?, size)
    })
}

pub fn preview_target(theme: &CursorTheme) -> PathBuf {
    let digest = Sha256::digest(format!("cursor:{}:{}", theme.path, theme.revision).as_bytes());
    let key: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    preview_cache().join(format!("{key}.png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xcursor(sizes: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"Xcur");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&0x1_0000u32.to_le_bytes());
        out.extend_from_slice(&(sizes.len() as u32).to_le_bytes());
        let mut pos = 16 + 12 * sizes.len();
        let mut chunks = Vec::new();
        for &size in sizes {
            out.extend_from_slice(&0xfffd_0002u32.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(pos as u32).to_le_bytes());
            let mut chunk = Vec::new();
            for value in [36u32, 0xfffd_0002, size, 1, size, size, 1, 2, 50] {
                chunk.extend_from_slice(&value.to_le_bytes());
            }
            for _ in 0..size * size {
                chunk.extend_from_slice(&0xff00_00ffu32.to_le_bytes());
            }
            pos += chunk.len();
            chunks.push(chunk);
        }
        for chunk in chunks {
            out.extend_from_slice(&chunk);
        }
        out
    }

    #[test]
    fn parses_xcursor_and_picks_size() {
        let images = parse(&xcursor(&[24, 48, 96])).unwrap_or_default();
        assert_eq!(images.len(), 3);
        assert_eq!(images[1].hotspot, (1, 2));
        assert_eq!(images[1].pixels[0], 0xff00_00ff);
        assert_eq!(best_image(images, 40).map(|image| image.width), Some(48));
        assert!(parse(b"nope").is_none());
    }

    #[test]
    fn state_round_trip() {
        let state = parse_state(r#"{"theme": "Hu Tao", "size": 32, "animation": false}"#);
        assert_eq!(
            state,
            Some(State {
                theme: "Hu Tao".into(),
                size: 32,
                animation: false,
                sizes: BTreeMap::from([("Hu Tao".into(), 32)]),
                colors: BTreeMap::new(),
                applied: String::new(),
            })
        );
        assert_eq!(
            parse_state(r#"{"theme": "x", "size": 0}"#).map(|state| state.size),
            Some(DEFAULT_SIZE)
        );
        assert_eq!(parse_state(r#"{"theme": ""}"#), None);
        assert_eq!(
            lua_state(&State {
                theme: "A \"b\"".into(),
                size: 24,
                animation: true,
                sizes: BTreeMap::new(),
                colors: BTreeMap::new(),
                applied: String::new(),
            }),
            "return { theme = \"A \\\"b\\\"\", size = 24, animation = true }\n"
        );
        let colored = parse_state(
            r#"{"theme": "a", "colors": {"a": true, "b": false}, "applied": ".chuhshell-colors-b"}"#,
        )
        .unwrap_or_default();
        assert_eq!(colored.effective(), ".chuhshell-colors-b");
        assert!(colored.colors_wanted());
        assert_eq!(colored.colors.get("b"), Some(&false));
        assert!(lua_state(&colored).contains("theme = \".chuhshell-colors-b\""));
    }

    #[test]
    fn sizes_cycle_and_persist_per_theme() {
        assert_eq!(next_size(24), 32);
        assert_eq!(next_size(64), 24);
        assert_eq!(next_size(30), 32);
        let state = parse_state(r#"{"theme": "a", "size": 48, "sizes": {"b": 64, "c": 0}}"#)
            .unwrap_or_default();
        assert_eq!(state.size_for("a"), 48);
        assert_eq!(state.size_for("b"), 64);
        assert_eq!(state.size_for("c"), DEFAULT_SIZE);
    }

    #[test]
    fn gtk_setting_replace() {
        let text = "[Settings]\ngtk-cursor-theme-name = Old\ngtk-theme-name = Adwaita\n";
        assert_eq!(
            replace_setting(text, "gtk-cursor-theme-name", "New"),
            "[Settings]\ngtk-cursor-theme-name = New\ngtk-theme-name = Adwaita\n"
        );
        assert_eq!(
            replace_setting("[Settings]\n", "gtk-cursor-theme-size", "24"),
            "[Settings]\ngtk-cursor-theme-size = 24\n"
        );
    }
}
