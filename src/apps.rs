use gio::prelude::*;
use gtk::gdk::prelude::DisplayExt;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub comment: String,
    pub exec: String,
    pub terminal: bool,
    pub keywords: String,
    pub startup_wm_class: String,
    pub hidden: bool,
}

fn config_path() -> PathBuf {
    crate::paths::config().join("chuhshell/hidden-apps")
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LauncherPreferences {
    pub pinned: std::collections::BTreeSet<String>,
    pub alphabetical: bool,
}

fn load_launcher_preferences() -> LauncherPreferences {
    crate::storage::read_limited(&config_path().with_file_name("launcher.json"), 1024 * 1024)
        .ok()
        .and_then(|contents| serde_json::from_slice(&contents).ok())
        .unwrap_or_default()
}

static PREFERENCES: OnceLock<Mutex<LauncherPreferences>> = OnceLock::new();

pub fn read_launcher_preferences() -> LauncherPreferences {
    PREFERENCES
        .get_or_init(|| Mutex::new(load_launcher_preferences()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub enum PreferenceChange {
    Pin(String),
    Sort,
}

pub fn change_launcher_preferences(
    change: PreferenceChange,
) -> impl std::future::Future<Output = Result<LauncherPreferences, String>> {
    crate::storage::run(move || {
        let mut preferences = PREFERENCES
            .get_or_init(|| Mutex::new(load_launcher_preferences()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut next = preferences.clone();
        match change {
            PreferenceChange::Pin(id) => {
                if !next.pinned.remove(&id) {
                    next.pinned.insert(id);
                }
            }
            PreferenceChange::Sort => next.alphabetical = !next.alphabetical,
        }
        write_launcher_preferences(&next).map_err(|e| e.to_string())?;
        *preferences = next.clone();
        Ok(next)
    })
}

pub fn write_launcher_preferences(preferences: &LauncherPreferences) -> std::io::Result<()> {
    crate::storage::atomic_write(
        &config_path().with_file_name("launcher.json"),
        &serde_json::to_vec(preferences)?,
    )
}

fn launch_counts_path() -> PathBuf {
    crate::paths::state().join("chuhshell/launch-counts.json")
}

pub fn read_launch_counts() -> HashMap<String, u64> {
    crate::storage::read_text(&launch_counts_path(), 1024 * 1024)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

pub fn record_launch(counts: &mut HashMap<String, u64>, id: &str) -> std::io::Result<()> {
    record_launch_at(&launch_counts_path(), counts, id)
}

fn record_launch_at(
    path: &Path,
    counts: &mut HashMap<String, u64>,
    id: &str,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let count = counts.entry(id.to_owned()).or_default();
    *count = count.saturating_add(1);
    let contents = serde_json::to_vec(counts)?;
    crate::storage::atomic_write(path, &contents)
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
    crate::storage::read_text(&config_path(), 1024 * 1024)
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
    crate::storage::atomic_write(&path, body.as_bytes())
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
        terminal: key_file
            .boolean("Desktop Entry", "Terminal")
            .unwrap_or(false),
        startup_wm_class: get_string("StartupWMClass").unwrap_or_default(),
        keywords: [get_string("Keywords"), get_string("GenericName")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" "),
        hidden,
    })
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![crate::paths::data().join("applications")];
    dirs.extend(
        crate::paths::data_dirs()
            .into_iter()
            .map(|path| path.join("applications")),
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
    load_entry_files_at(search_dirs())
}

fn load_entry_files_at(dirs: Vec<PathBuf>) -> Vec<(String, String)> {
    fn collect(
        base: &Path,
        dir: &Path,
        entries: &mut Vec<(String, String)>,
        remaining: &mut usize,
        depth: usize,
        visits: &mut usize,
    ) {
        if entries.len() >= 4096 || *remaining == 0 || depth > 16 || *visits == 0 {
            return;
        }
        let Ok(files) = fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<_> = files.flatten().take(4096).collect();
        files.sort_by_key(|entry| entry.path());
        for entry in files {
            if entries.len() >= 4096 || *remaining == 0 || *visits == 0 {
                break;
            }
            *visits -= 1;
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                collect(base, &path, entries, remaining, depth + 1, visits);
            } else if path.extension().is_some_and(|ext| ext == "desktop") {
                let Ok(relative) = path.strip_prefix(base) else {
                    continue;
                };
                let id = relative.to_string_lossy().replace('/', "-");
                if let Ok(contents) = crate::storage::read_text(&path, 256 * 1024) {
                    if contents.len() > *remaining {
                        break;
                    }
                    *remaining -= contents.len();
                    entries.push((id, contents));
                }
            }
        }
    }
    let mut entries = Vec::new();
    let mut remaining = 16 * 1024 * 1024;
    let mut visits = 16384;
    for dir in dirs {
        collect(&dir, &dir, &mut entries, &mut remaining, 0, &mut visits);
    }
    entries
}

#[derive(Default)]
struct CatalogCache {
    generation: u64,
    entries: Option<Arc<Vec<AppEntry>>>,
}
static CATALOG: OnceLock<Mutex<CatalogCache>> = OnceLock::new();

pub fn invalidate() {
    let mut cache = CATALOG
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache.generation = cache.generation.wrapping_add(1);
    cache.entries = None;
}

pub fn watch() -> gio::AppInfoMonitor {
    let monitor = gio::AppInfoMonitor::get();
    monitor.connect_changed(|_| invalidate());
    monitor
}

pub fn load_apps() -> Vec<AppEntry> {
    let (generation, entries) = {
        let cache = CATALOG
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (cache.generation, cache.entries.clone())
    };
    let entries = entries.unwrap_or_else(|| {
        let visible: HashSet<String> = gio::AppInfo::all()
            .into_iter()
            .filter(|app| app.should_show())
            .filter_map(|app| app.id().map(|id| id.to_string()))
            .collect();
        let mut apps = collect_entries(load_entry_files(), &HashSet::new());
        apps.retain(|app| visible.contains(&app.id));
        apps.sort_by_cached_key(|app| app.name.to_lowercase());
        let apps = Arc::new(apps);
        let mut cache = CATALOG
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.generation == generation {
            cache.entries = Some(apps.clone());
        }
        apps
    });
    let mut apps = entries.as_ref().clone();
    let hidden = read_hidden();
    for app in &mut apps {
        app.hidden = hidden.contains(&app.id);
    }
    apps
}

pub fn app_info(id: &str) -> Option<gio::AppInfo> {
    let entries = gio::AppInfo::all();
    entries
        .into_iter()
        .find(|app| app.id().as_deref() == Some(id))
        .or_else(|| {
            let full_id = format!("{id}.desktop");
            gio::AppInfo::all()
                .into_iter()
                .find(|app| app.id().as_deref() == Some(&full_id))
        })
}

pub async fn launch(id: &str) -> Result<(), String> {
    let info = app_info(id).ok_or_else(|| format!("Application not found: {id}"))?;
    let context = gtk::gdk::Display::default().map(|display| display.app_launch_context());
    info.launch_uris_future(&[], context.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
pub fn profile_catalog() -> usize {
    let dirs = std::env::var_os("CHUHSHELL_PROFILE_CATALOG_DIRS")
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_else(search_dirs);
    collect_entries(load_entry_files_at(dirs), &read_hidden()).len()
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
    fn desktop_reads_reject_oversized_files_and_deep_trees() {
        let root =
            std::env::temp_dir().join(format!("chuhshell-catalog-limits-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("oversized.desktop"), vec![b'x'; 256 * 1024 + 1]).unwrap();
        let deep = (0..17).fold(root.clone(), |path, _| path.join("deep"));
        fs::create_dir_all(&deep).unwrap();
        fs::write(
            deep.join("deep.desktop"),
            "[Desktop Entry]\nName=Deep\nType=Application\nExec=true\n",
        )
        .unwrap();
        fs::write(
            root.join("valid.desktop"),
            "[Desktop Entry]\nName=Valid\nType=Application\nExec=true\n",
        )
        .unwrap();
        let entries = load_entry_files_at(vec![root.clone()]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "valid.desktop");
        fs::remove_dir_all(root).unwrap();
    }

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

    #[test]
    fn launch_counts_survive_writes_and_saturate() {
        let path = std::env::temp_dir().join(format!(
            "chuhshell-launch-counts-{}.json",
            std::process::id()
        ));
        let mut counts = HashMap::from([("firefox.desktop".to_owned(), u64::MAX - 1)]);
        record_launch_at(&path, &mut counts, "firefox.desktop").expect("save launch count");
        record_launch_at(&path, &mut counts, "firefox.desktop").expect("save saturated count");
        record_launch_at(&path, &mut counts, "foot.desktop").expect("save another app");
        let saved: HashMap<String, u64> =
            serde_json::from_slice(&fs::read(&path).expect("read counts")).expect("parse counts");
        assert_eq!(saved.get("firefox.desktop"), Some(&u64::MAX));
        assert_eq!(saved.get("foot.desktop"), Some(&1));
        fs::remove_file(path).expect("remove test counts");
    }
}
