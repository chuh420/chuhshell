use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub comment: String,
    pub exec: String,
    pub hidden: bool,
}

fn config_path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    config_home.join("chuhshell/hidden-apps")
}

pub fn read_hidden() -> HashSet<String> {
    fs::read_to_string(config_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

pub fn write_hidden(hidden: &HashSet<String>) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut ids: Vec<_> = hidden.iter().collect();
    ids.sort();
    let mut body = ids.into_iter().cloned().collect::<Vec<_>>().join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, body)?;
    fs::rename(temp, path)
}

fn desktop_value(section: &str, key: &str) -> Option<String> {
    let exact = format!("{key}=");
    let localized = format!("{key}[");
    let mut fallback = None;
    for line in section.lines() {
        if let Some(value) = line.strip_prefix(&exact)
            && !value.starts_with('[')
        {
            return Some(value.to_owned());
        }
        if fallback.is_none()
            && let Some(rest) = line.strip_prefix(&localized)
            && let Some((_, value)) = rest.split_once('=')
        {
            fallback = Some(value.to_owned());
        }
    }
    fallback
}

fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|argument| !argument.starts_with('%'))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse_entry(id: &str, contents: &str, hidden: bool) -> Option<AppEntry> {
    let section = contents.split("[Desktop Entry]").nth(1)?;
    let section = section.split("\n[").next().unwrap_or(section);
    if desktop_value(section, "Type")
        .as_deref()
        .is_some_and(|kind| kind != "Application")
    {
        return None;
    }
    if ["Hidden", "NoDisplay"].iter().any(|key| {
        desktop_value(section, key).is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }) {
        return None;
    }
    let name = desktop_value(section, "Name")
        .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
    Some(AppEntry {
        id: id.to_owned(),
        name,
        icon: desktop_value(section, "Icon").unwrap_or_default(),
        comment: desktop_value(section, "Comment").unwrap_or_default(),
        exec: strip_field_codes(&desktop_value(section, "Exec").unwrap_or_default()),
        hidden,
    })
}

fn search_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    let mut dirs = vec![data_home.join("applications")];
    dirs.extend(
        data_dirs
            .split(':')
            .filter(|part| !part.is_empty())
            .map(|part| Path::new(part).join("applications")),
    );
    dirs
}

pub fn load_apps() -> Vec<AppEntry> {
    let hidden = read_hidden();
    let mut entries: HashMap<String, AppEntry> = HashMap::new();
    for dir in search_dirs() {
        let Ok(files) = fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = files.flatten().map(|entry| entry.path()).collect();
        files.sort();
        for file in files {
            if file.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(id) = file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if entries.contains_key(id) {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&file) else {
                continue;
            };
            if let Some(app) = parse_entry(id, &contents, hidden.contains(id)) {
                entries.insert(id.to_owned(), app);
            }
        }
    }
    let mut apps: Vec<_> = entries.into_values().collect();
    apps.sort_by_cached_key(|app| app.name.to_lowercase());
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
[Desktop Entry]
Type=Application
Name=Firefox
Name[ru]=Firefox RU
Comment=Browse the web
Icon=firefox
Exec=/usr/bin/firefox %u
";

    #[test]
    fn parses_application_entry() {
        let app = parse_entry("firefox.desktop", FIREFOX, false).expect("valid entry");
        assert_eq!(app.name, "Firefox");
        assert_eq!(app.icon, "firefox");
        assert_eq!(app.exec, "/usr/bin/firefox");
        assert!(!app.hidden);
    }

    #[test]
    fn rejects_non_application_and_hidden_entries() {
        let link = "[Desktop Entry]\nType=Link\nName=Example\n";
        assert!(parse_entry("example.desktop", link, false).is_none());

        let hidden = "[Desktop Entry]\nType=Application\nName=Secret\nHidden=true\n";
        assert!(parse_entry("secret.desktop", hidden, false).is_none());

        let no_display = "[Desktop Entry]\nType=Application\nName=Secret\nNoDisplay=true\n";
        assert!(parse_entry("secret.desktop", no_display, false).is_none());
    }

    #[test]
    fn falls_back_to_localized_name_and_default_id() {
        let localized = "[Desktop Entry]\nType=Application\nName[ru]=Терминал\n";
        let app = parse_entry("org.example.Terminal.desktop", localized, false).expect("entry");
        assert_eq!(app.name, "Терминал");

        let unnamed = "[Desktop Entry]\nType=Application\n";
        let app = parse_entry("htop.desktop", unnamed, false).expect("entry");
        assert_eq!(app.name, "htop");
    }

    #[test]
    fn prefers_unlocalized_value_over_localized() {
        assert_eq!(desktop_value(FIREFOX, "Name").as_deref(), Some("Firefox"));
        assert_eq!(
            desktop_value(FIREFOX, "Comment").as_deref(),
            Some("Browse the web")
        );
    }

    #[test]
    fn strips_exec_field_codes() {
        assert_eq!(
            strip_field_codes("/usr/bin/firefox %u %F"),
            "/usr/bin/firefox"
        );
        assert_eq!(strip_field_codes(""), "");
    }
}
