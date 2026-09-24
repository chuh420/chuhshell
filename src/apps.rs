use std::collections::HashSet;
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

pub fn toggled_hidden(
    apps: &[AppEntry],
    id: &str,
    persisted: HashSet<String>,
) -> Option<HashSet<String>> {
    let target = apps.iter().find(|app| app.id == id)?;
    let known: HashSet<&str> = apps.iter().map(|app| app.id.as_str()).collect();
    let mut hidden: HashSet<String> = apps
        .iter()
        .filter(|app| {
            if app.id == id {
                !target.hidden
            } else {
                app.hidden
            }
        })
        .map(|app| app.id.clone())
        .collect();
    hidden.extend(
        persisted
            .into_iter()
            .filter(|id| !known.contains(id.as_str())),
    );
    Some(hidden)
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

pub fn parse_entry(id: &str, contents: &str, hidden: bool) -> Option<AppEntry> {
    parse_entry_for_locale(id, contents, hidden, None)
}

fn parse_entry_for_locale(
    id: &str,
    contents: &str,
    hidden: bool,
    locale: Option<&str>,
) -> Option<AppEntry> {
    let key_file = glib::KeyFile::new();
    key_file
        .load_from_data(contents, glib::KeyFileFlags::KEEP_TRANSLATIONS)
        .ok()?;
    if !key_file.has_group("Desktop Entry") {
        return None;
    }
    let get_string = |key: &str| {
        key_file
            .locale_string("Desktop Entry", key, locale)
            .ok()
            .map(|value| value.to_string())
    };
    if get_string("Type")
        .as_deref()
        .is_some_and(|kind| kind != "Application")
        || ["Hidden", "NoDisplay"]
            .iter()
            .any(|key| key_file.boolean("Desktop Entry", key).unwrap_or(false))
    {
        return None;
    }
    let name = get_string("Name").unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
    Some(AppEntry {
        id: id.to_owned(),
        name,
        icon: get_string("Icon").unwrap_or_default(),
        comment: get_string("Comment").unwrap_or_default(),
        exec: get_string("Exec").unwrap_or_default(),
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

fn collect_entries(
    entries: impl IntoIterator<Item = (String, String)>,
    hidden: &HashSet<String>,
) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for (id, contents) in entries {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(app) = parse_entry(&id, &contents, hidden.contains(&id)) {
            apps.push(app);
        }
    }
    apps
}

fn load_entry_files() -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for dir in search_dirs() {
        let Ok(files) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<_> = files.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for file in paths {
            if file.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(id) = file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Ok(contents) = fs::read_to_string(&file) else {
                continue;
            };
            entries.push((id.to_owned(), contents));
        }
    }
    entries
}

pub fn load_apps() -> Vec<AppEntry> {
    let mut apps = collect_entries(load_entry_files(), &read_hidden());
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
        assert_eq!(app.exec, "/usr/bin/firefox %u");
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
        let localized = "[Desktop Entry]\nType=Application\nName[ru_RU]=Терминал\n";
        let app = parse_entry_for_locale(
            "org.example.Terminal.desktop",
            localized,
            false,
            Some("ru_RU.UTF-8"),
        )
        .expect("entry");
        assert_eq!(app.name, "Терминал");

        let unnamed = "[Desktop Entry]\nType=Application\n";
        let app = parse_entry("htop.desktop", unnamed, false).expect("entry");
        assert_eq!(app.name, "htop");
    }

    #[test]
    fn locale_uses_unlocalized_name_as_fallback() {
        let app =
            parse_entry_for_locale("firefox.desktop", FIREFOX, false, Some("fr")).expect("entry");
        assert_eq!(app.name, "Firefox");
        assert_eq!(app.comment, "Browse the web");
    }

    #[test]
    fn key_file_unescapes_desktop_values_and_preserves_exec_codes() {
        let desktop =
            "[Desktop Entry]\nType=Application\nName=Line\\nBreak\nExec=/usr/bin/app %F\n";
        let app = parse_entry("example.desktop", desktop, false).expect("entry");

        assert_eq!(app.name, "Line\nBreak");
        assert_eq!(app.exec, "/usr/bin/app %F");
    }

    #[test]
    fn higher_priority_entry_masks_lower_priority_duplicate() {
        let entries = vec![
            ("app.desktop".to_owned(), FIREFOX.to_owned()),
            (
                "app.desktop".to_owned(),
                "[Desktop Entry]\nType=Application\nName=Shadowed\n".to_owned(),
            ),
        ];
        let apps = collect_entries(entries, &HashSet::new());

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Firefox");
    }

    #[test]
    fn higher_priority_hidden_entry_masks_lower_priority_visible_one() {
        let entries = vec![
            (
                "app.desktop".to_owned(),
                "[Desktop Entry]\nType=Application\nName=Hidden One\nHidden=true\n".to_owned(),
            ),
            ("app.desktop".to_owned(), FIREFOX.to_owned()),
        ];
        let apps = collect_entries(entries, &HashSet::new());

        assert!(apps.is_empty());
    }

    #[test]
    fn non_application_entry_also_masks_lower_priority_duplicate() {
        let entries = vec![
            (
                "app.desktop".to_owned(),
                "[Desktop Entry]\nType=Link\nName=Link\n".to_owned(),
            ),
            ("app.desktop".to_owned(), FIREFOX.to_owned()),
        ];
        let apps = collect_entries(entries, &HashSet::new());

        assert!(apps.is_empty());
    }

    #[test]
    fn hidden_list_marks_matching_entry() {
        let hidden: HashSet<String> = ["firefox.desktop".to_owned()].into_iter().collect();
        let apps = collect_entries(
            vec![("firefox.desktop".to_owned(), FIREFOX.to_owned())],
            &hidden,
        );

        assert_eq!(apps.len(), 1);
        assert!(apps[0].hidden);
    }

    #[test]
    fn toggled_hidden_adds_and_removes_the_id() {
        let mut firefox = parse_entry("firefox.desktop", FIREFOX, false).expect("entry");
        let apps = vec![firefox.clone()];

        let hidden = toggled_hidden(&apps, "firefox.desktop", HashSet::new()).expect("toggle");
        assert!(hidden.contains("firefox.desktop"));

        firefox.hidden = true;
        let apps = vec![firefox];
        let hidden = toggled_hidden(
            &apps,
            "firefox.desktop",
            ["firefox.desktop".to_owned()].into_iter().collect(),
        )
        .expect("toggle");
        assert!(!hidden.contains("firefox.desktop"));
    }

    #[test]
    fn toggled_hidden_preserves_unknown_persisted_ids() {
        let firefox = parse_entry("firefox.desktop", FIREFOX, false).expect("entry");
        let apps = vec![firefox];
        let persisted: HashSet<String> = ["removed-app.desktop".to_owned()].into_iter().collect();

        let hidden = toggled_hidden(&apps, "firefox.desktop", persisted).expect("toggle");

        assert!(hidden.contains("removed-app.desktop"));
        assert!(hidden.contains("firefox.desktop"));
    }

    #[test]
    fn toggled_hidden_rejects_unknown_id() {
        let firefox = parse_entry("firefox.desktop", FIREFOX, false).expect("entry");
        let apps = vec![firefox];

        assert!(toggled_hidden(&apps, "missing.desktop", HashSet::new()).is_none());
    }
}
