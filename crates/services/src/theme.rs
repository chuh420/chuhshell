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

pub const SHELL_TEMPLATE: &str = "chuhshell.json";

pub fn root() -> PathBuf {
    config::paths().state.join("theme")
}

pub fn available() -> bool {
    crate::programs::exists("matugen")
}

fn copy_dir(source: &Path, target: &Path) -> io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)?.flatten() {
        let path = entry.path();
        if path.is_file() {
            std::fs::copy(&path, target.join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub fn adopt_legacy() -> io::Result<bool> {
    let root = root();
    if root.exists() {
        return Ok(false);
    }
    let legacy = config::paths().home.join(".local/state/rice/theme");
    let staging = root.with_extension("migrating");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    if legacy.is_dir() {
        for entry in std::fs::read_dir(&legacy)?.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                copy_dir(&path, &staging.join(entry.file_name()))?;
            }
        }
    }
    std::fs::rename(&staging, &root)?;
    Ok(legacy.is_dir())
}

fn folder(mode: Mode, static_name: &str) -> String {
    match mode {
        Mode::Auto => "auto".into(),
        Mode::Static => static_name.into(),
    }
}

fn mode_path() -> PathBuf {
    config::paths().state_file("theme.json")
}

fn palette_path(folder: &str) -> PathBuf {
    root().join(folder).join("chuhshell.json")
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

pub fn link(mode: Mode, static_name: &str) -> io::Result<()> {
    let root = root();
    let folder = folder(mode, static_name);
    std::fs::create_dir_all(root.join(&folder))?;
    let current = root.join("current");
    if std::fs::read_link(&current).is_ok_and(|target| target == Path::new(&folder)) {
        return Ok(());
    }
    let temporary = root.join("current.tmp");
    let _ = std::fs::remove_file(&temporary);
    std::os::unix::fs::symlink(&folder, &temporary)?;
    std::fs::rename(&temporary, &current)
}

pub fn read_auto_palette() -> Option<String> {
    std::fs::read_to_string(palette_path("auto")).ok()
}

pub fn shell_template(yellow: &str, red: &str, blue: &str) -> String {
    let harmonize = |hex: &str| {
        format!(
            "{{{{ \"{hex}\" | to_color | harmonize: {{{{ colors.source_color.default.hex | to_color }}}} }}}}"
        )
    };
    format!(
        "{{\"bg\": \"{{{{colors.surface_container_low.default.hex}}}}\", \"dim\": \"{{{{colors.surface.default.hex}}}}\", \"surface\": \"{{{{colors.surface_container.default.hex}}}}\", \"elevated\": \"{{{{colors.surface_container_high.default.hex}}}}\", \"line\": \"{{{{colors.outline_variant.default.hex}}}}\", \"text\": \"{{{{colors.on_surface.default.hex}}}}\", \"muted\": \"{{{{colors.outline.default.hex}}}}\", \"green\": \"{{{{colors.primary.default.hex}}}}\", \"yellow\": \"{}\", \"red\": \"{}\", \"blue\": \"{}\"}}\n",
        harmonize(yellow),
        harmonize(red),
        harmonize(blue)
    )
}

fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn matugen_config(shell_template: &Path, templates: &[PathBuf], output: &Path) -> String {
    let mut text = String::from("[config]\n");
    let mut entry = |name: &str, input: &Path| {
        text.push_str(&format!(
            "\n[templates.{}]\ninput_path = {}\noutput_path = {}\n",
            toml_string(name),
            toml_string(&input.to_string_lossy()),
            toml_string(&output.join(name).to_string_lossy())
        ));
    };
    entry(SHELL_TEMPLATE, shell_template);
    for template in templates {
        let Some(name) = template
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name != SHELL_TEMPLATE && !name.starts_with('.') {
            entry(&name, template);
        }
    }
    text
}

pub fn user_templates(dir: &Path) -> Vec<PathBuf> {
    let mut templates: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default();
    templates.sort();
    templates
}

pub fn prepare_matugen(yellow: &str, red: &str, blue: &str) -> io::Result<PathBuf> {
    let root = root();
    let auto = root.join("auto");
    std::fs::create_dir_all(&auto)?;
    let template = root.join("chuhshell.template.json");
    write_atomic(&template, &shell_template(yellow, red, blue))?;
    let config_path = root.join("matugen.toml");
    let templates = user_templates(&config::templates_dir());
    write_atomic(&config_path, &matugen_config(&template, &templates, &auto))?;
    Ok(config_path)
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

pub fn generate_command(image: &str, scheme: &str, config_path: &Path) -> Command {
    let mut command = Command::new("matugen");
    let scheme = format!("scheme-{scheme}");
    command
        .arg("-c")
        .arg(config_path)
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

pub fn reload_command(mode: Mode) -> Option<Command> {
    let hook = config::current().hooks.theme_changed.clone()?;
    Some(crate::session::hook_command(&hook, &[mode.name()]))
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
        assert_eq!(folder(Mode::Auto, "nord"), "auto");
        assert_eq!(folder(Mode::Static, "nord"), "nord");
        assert_eq!(Mode::parse(Mode::Auto.name()), Some(Mode::Auto));
    }

    #[test]
    fn shell_template_matches_matugen_syntax() {
        let template = shell_template("#d8a657", "#ea6962", "#7daea3");
        assert!(template.starts_with("{\"bg\": \"{{colors.surface_container_low.default.hex}}\""));
        assert!(template.contains(
            "\"yellow\": \"{{ \"#d8a657\" | to_color | harmonize: {{ colors.source_color.default.hex | to_color }} }}\""
        ));
        assert!(template.contains("\"green\": \"{{colors.primary.default.hex}}\""));
    }

    #[test]
    fn matugen_config_lists_templates() {
        let text = matugen_config(
            Path::new("/s/chuhshell.template.json"),
            &[
                PathBuf::from("/t/kitty.conf"),
                PathBuf::from("/t/chuhshell.json"),
                PathBuf::from("/t/.hidden"),
                PathBuf::from("/t/we\"ird"),
            ],
            Path::new("/s/auto"),
        );
        let parsed: toml::Table = toml::from_str(&text).expect("valid toml");
        let templates = parsed["templates"].as_table().expect("templates");
        assert_eq!(templates.len(), 3);
        assert_eq!(
            templates["chuhshell.json"]["output_path"].as_str(),
            Some("/s/auto/chuhshell.json")
        );
        assert_eq!(
            templates["kitty.conf"]["input_path"].as_str(),
            Some("/t/kitty.conf")
        );
        assert_eq!(
            templates["we\"ird"]["output_path"].as_str(),
            Some("/s/auto/we\"ird")
        );
    }
}
