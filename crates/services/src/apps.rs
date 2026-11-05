use std::{
    collections::{HashMap, HashSet},
    io,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::fs::PermissionsExt,
    },
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub generic: String,
    pub comment: String,
    pub icon: String,
    pub terminal: bool,
    pub path: PathBuf,
    searchable: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

pub fn casefold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == 'ß' || c == 'ẞ' {
            out.push_str("ss");
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

pub fn application_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{home}/.local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs = vec![PathBuf::from(data_home).join("applications")];
    for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
        let path = PathBuf::from(dir).join("applications");
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    }
    dirs
}

fn language_names() -> Vec<String> {
    let mut names = Vec::new();
    let mut sources = Vec::new();
    if let Ok(language) = std::env::var("LANGUAGE") {
        sources.extend(
            language
                .split(':')
                .filter(|part| !part.is_empty())
                .map(str::to_owned),
        );
    }
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()));
    if let Some(locale) = locale {
        sources.push(locale);
    }
    for source in sources {
        if source == "C" || source == "POSIX" {
            continue;
        }
        let (rest, modifier) = match source.split_once('@') {
            Some((rest, modifier)) => (rest, Some(modifier)),
            None => (source.as_str(), None),
        };
        let rest = rest.split('.').next().unwrap_or(rest);
        let (lang, country) = match rest.split_once('_') {
            Some((lang, country)) => (lang, Some(country)),
            None => (rest, None),
        };
        let mut variants = Vec::new();
        if let (Some(country), Some(modifier)) = (country, modifier) {
            variants.push(format!("{lang}_{country}@{modifier}"));
        }
        if let Some(country) = country {
            variants.push(format!("{lang}_{country}"));
        }
        if let Some(modifier) = modifier {
            variants.push(format!("{lang}@{modifier}"));
        }
        variants.push(lang.to_string());
        for variant in variants {
            if !names.contains(&variant) {
                names.push(variant);
            }
        }
    }
    names
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

struct Entry {
    values: HashMap<String, String>,
}

impl Entry {
    fn parse(text: &str) -> Option<Self> {
        let mut values = HashMap::new();
        let mut in_group = false;
        let mut seen = false;
        for line in text.lines() {
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_group = line.trim_end() == "[Desktop Entry]";
                seen |= in_group;
                continue;
            }
            if !in_group {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                values
                    .entry(key.trim_end().to_string())
                    .or_insert_with(|| unescape(value.trim_start()));
            }
        }
        seen.then_some(Self { values })
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn localized(&self, key: &str, languages: &[String]) -> Option<&str> {
        languages
            .iter()
            .find_map(|lang| self.get(&format!("{key}[{lang}]")))
            .or_else(|| self.get(key))
    }

    fn boolean(&self, key: &str) -> bool {
        self.get(key).is_some_and(|value| value.trim() == "true")
    }

    fn list(&self, key: &str) -> Vec<String> {
        self.get(key)
            .map(|value| {
                value
                    .split(';')
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn find_program(program: &str) -> Option<PathBuf> {
    let executable = |path: &Path| {
        path.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let path = PathBuf::from(program);
        return executable(&path).then_some(path);
    }
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| executable(candidate))
}

pub fn shell_split(command: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        other => current.push(other),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            next @ ('$' | '`' | '"' | '\\') => current.push(next),
                            '\n' => {}
                            next => {
                                current.push('\\');
                                current.push(next);
                            }
                        },
                        other => current.push(other),
                    }
                }
            }
            '\\' => {
                started = true;
                match chars.next() {
                    Some('\n') => {}
                    Some(next) => current.push(next),
                    None => current.push('\\'),
                }
            }
            '#' if !started => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        args.push(current);
    }
    Some(args)
}

fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn load(id: &str, path: &Path, languages: &[String], desktops: &[String]) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let entry = Entry::parse(&text)?;
    if entry.get("Type") != Some("Application") || entry.boolean("Hidden") {
        return None;
    }
    if let Some(try_exec) = entry.get("TryExec").filter(|value| !value.is_empty())
        && find_program(try_exec).is_none()
    {
        return None;
    }
    if let Some(exec) = entry.get("Exec").filter(|value| !value.is_empty()) {
        let argv = shell_split(exec)?;
        find_program(argv.first()?)?;
    }
    if entry.boolean("NoDisplay") {
        return None;
    }
    let only = entry.list("OnlyShowIn");
    let not = entry.list("NotShowIn");
    if desktops.iter().any(|desktop| not.contains(desktop)) {
        return None;
    }
    if !only.is_empty() && !desktops.iter().any(|desktop| only.contains(desktop)) {
        return None;
    }
    let name = entry
        .localized("X-GNOME-FullName", languages)
        .or_else(|| entry.localized("Name", languages))?
        .to_string();
    let generic = entry
        .localized("GenericName", languages)
        .unwrap_or_default()
        .to_string();
    let comment = entry
        .localized("Comment", languages)
        .unwrap_or_default()
        .to_string();
    let mut icon = entry
        .localized("Icon", languages)
        .unwrap_or_default()
        .to_string();
    if !icon.starts_with('/') {
        for ext in [".png", ".xpm", ".svg"] {
            if let Some(stripped) = icon.strip_suffix(ext) {
                icon = stripped.to_string();
                break;
            }
        }
    }
    let searchable = casefold(&format!("{name} {generic} {comment}"));
    Some(AppEntry {
        id: id.to_string(),
        name,
        generic,
        comment,
        icon,
        terminal: entry.boolean("Terminal"),
        path: path.to_path_buf(),
        searchable,
    })
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_dir() {
            collect(&path, &format!("{prefix}{name}-"), out);
        } else if name.ends_with(".desktop") {
            out.push((format!("{prefix}{name}"), path));
        }
    }
}

pub fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(subdirs(&path));
            }
        }
    }
    out
}

fn priority(entry: &AppEntry, pinned: &[String]) -> bool {
    let id = entry.id.to_lowercase();
    !pinned
        .iter()
        .filter(|term| !term.is_empty())
        .any(|term| id.contains(&term.to_lowercase()))
}

pub fn scan() -> Vec<AppEntry> {
    let languages = language_names();
    let desktops = current_desktops();
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in application_dirs() {
        let mut files = Vec::new();
        collect(&dir, "", &mut files);
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(app) = load(&id, &path, &languages, &desktops) {
                apps.push(app);
            }
        }
    }
    sort(&mut apps, &config::current().launcher.pinned);
    apps
}

fn sort(apps: &mut [AppEntry], pinned: &[String]) {
    apps.sort_by_cached_key(|app| (priority(app, pinned), casefold(&app.name)));
}

pub fn search(apps: &[AppEntry], query: &str) -> Vec<AppEntry> {
    let query = casefold(query);
    apps.iter()
        .filter(|app| app.searchable.contains(&query))
        .cloned()
        .collect()
}

pub fn hidden_path() -> PathBuf {
    config::paths().state_file("hidden-applications.json")
}

pub fn parse_hidden(text: &str) -> Vec<String> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(|item| {
                if item.ends_with(".desktop") {
                    item.to_string()
                } else {
                    format!("{item}.desktop")
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn load_hidden() -> Vec<String> {
    std::fs::read_to_string(hidden_path())
        .map(|text| parse_hidden(&text))
        .unwrap_or_default()
}

pub fn format_hidden(hidden: &[String]) -> String {
    let items: Vec<String> = hidden
        .iter()
        .map(|item| serde_json::to_string(item).unwrap_or_default())
        .collect();
    format!("[{}]\n", items.join(", "))
}

pub fn save_hidden(hidden: &[String]) -> io::Result<()> {
    let path = hidden_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, format_hidden(hidden))?;
    std::fs::rename(&temporary, &path)
}

pub fn expand_exec(
    tokens: &[String],
    icon: Option<&str>,
    name: &str,
    filename: &str,
) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut files = 0;
    for token in tokens {
        if token == "%i" {
            if let Some(icon) = icon.filter(|icon| !icon.is_empty()) {
                arguments.push("--icon".to_string());
                arguments.push(icon.to_string());
            }
            continue;
        }
        if token == "%F" || token == "%U" {
            files += 1;
            continue;
        }
        let mut expanded = String::new();
        let mut removed = false;
        let mut chars = token.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                expanded.push(c);
                continue;
            }
            let Some(code) = chars.next() else {
                return Err("Incomplete desktop field code".into());
            };
            match code {
                '%' => expanded.push('%'),
                'c' => expanded.push_str(name),
                'k' => expanded.push_str(filename),
                'f' | 'u' => {
                    files += 1;
                    removed = true;
                }
                'd' | 'D' | 'n' | 'N' | 'v' | 'm' => removed = true,
                other => return Err(format!("Invalid desktop field code: %{other}")),
            }
        }
        if !expanded.is_empty() || !removed {
            arguments.push(expanded);
        }
    }
    if files > 1 {
        return Err("Multiple desktop file field codes".into());
    }
    if arguments.first().is_none_or(|first| first.is_empty()) {
        return Err("Missing desktop executable".into());
    }
    Ok(arguments)
}

fn object_path(app_id: &str) -> String {
    let mut path = String::from("/");
    for c in app_id.chars() {
        match c {
            '.' => path.push('/'),
            '-' => path.push('_'),
            other => path.push(other),
        }
    }
    path
}

pub fn launch_plan(app: &AppEntry) -> Result<LaunchPlan, String> {
    let text = std::fs::read_to_string(&app.path)
        .map_err(|_| "Application no longer exists".to_string())?;
    let entry = Entry::parse(&text).ok_or("Application no longer exists")?;
    let languages = language_names();
    let cwd = entry
        .get("Path")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let filename = app.path.to_string_lossy().into_owned();
    let name = entry
        .localized("Name", &languages)
        .unwrap_or_default()
        .to_string();
    let icon = entry.get("Icon");
    let tokens = || {
        shell_split(entry.get("Exec").unwrap_or_default())
            .ok_or_else(|| "Invalid Exec line".to_string())
    };
    if entry.boolean("Terminal") {
        let mut argv =
            crate::session::terminal().ok_or_else(|| "No terminal emulator found".to_string())?;
        argv.extend(expand_exec(&tokens()?, icon, &name, &filename)?);
        return Ok(LaunchPlan {
            argv,
            cwd,
            env: Vec::new(),
        });
    }
    let env = vec![("GIO_LAUNCHED_DESKTOP_FILE".to_string(), filename.clone())];
    if entry.boolean("DBusActivatable") {
        let app_id = app.id.strip_suffix(".desktop").unwrap_or(&app.id);
        return Ok(LaunchPlan {
            argv: vec![
                "gdbus".into(),
                "call".into(),
                "--session".into(),
                "--dest".into(),
                app_id.into(),
                "--object-path".into(),
                object_path(app_id),
                "--method".into(),
                "org.freedesktop.Application.Activate".into(),
                "{}".into(),
            ],
            cwd: None,
            env,
        });
    }
    Ok(LaunchPlan {
        argv: expand_exec(&tokens()?, icon, &name, &filename)?,
        cwd,
        env,
    })
}

pub fn launch_argv(plan: &LaunchPlan, scope: bool) -> Vec<String> {
    let mut argv: Vec<String> = Vec::new();
    if scope {
        argv.extend(
            [
                "systemd-run",
                "--user",
                "--scope",
                "--slice=app.slice",
                "--collect",
                "--quiet",
                "--expand-environment=no",
                "--",
            ]
            .map(String::from),
        );
    }
    if plan
        .env
        .iter()
        .any(|(key, _)| key == "GIO_LAUNCHED_DESKTOP_FILE")
    {
        argv.extend(
            [
                "sh",
                "-c",
                "export GIO_LAUNCHED_DESKTOP_FILE_PID=$$; exec \"$@\"",
                "sh",
            ]
            .map(String::from),
        );
    }
    argv.extend(plan.argv.iter().cloned());
    argv
}

pub fn scope_command(plan: &LaunchPlan) -> std::process::Command {
    let argv = launch_argv(plan, crate::programs::exists("systemd-run"));
    let mut command =
        std::process::Command::new(argv.first().map(String::as_str).unwrap_or("true"));
    command.args(argv.iter().skip(1));
    if let Some(cwd) = &plan.cwd {
        command.current_dir(cwd);
    }
    command.envs(plan.env.iter().map(|(key, value)| (key, value)));
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

pub struct Watcher {
    inotify: inotify::Inotify,
}

impl Watcher {
    pub fn new() -> io::Result<Self> {
        let mut watcher = Self {
            inotify: inotify::Inotify::init()?,
        };
        watcher.watch_all();
        Ok(watcher)
    }

    fn watch_all(&mut self) {
        use inotify::WatchMask;
        let mask = WatchMask::CREATE
            | WatchMask::DELETE
            | WatchMask::MODIFY
            | WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_FROM
            | WatchMask::MOVED_TO
            | WatchMask::ATTRIB
            | WatchMask::DELETE_SELF;
        for dir in application_dirs() {
            for path in subdirs(&dir) {
                if path.is_dir()
                    && let Err(err) = self.inotify.watches().add(&path, mask)
                {
                    log::debug!("watch {}: {err}", path.display());
                }
            }
        }
    }

    pub fn drain(&mut self) -> bool {
        let mut buffer = [0u8; 4096];
        let mut changed = false;
        loop {
            match self.inotify.read_events(&mut buffer) {
                Ok(events) => changed |= events.count() > 0,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => {
                    log::warn!("applications watch: {err}");
                    break;
                }
            }
        }
        if changed {
            self.watch_all();
        }
        changed
    }
}

impl AsFd for Watcher {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, generic: &str, comment: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: name.into(),
            generic: generic.into(),
            comment: comment.into(),
            icon: String::new(),
            terminal: false,
            path: PathBuf::new(),
            searchable: casefold(&format!("{name} {generic} {comment}")),
        }
    }

    #[test]
    fn launch_without_systemd_run() {
        let plan = LaunchPlan {
            argv: vec!["zen".into(), "--new-window".into()],
            cwd: None,
            env: vec![("GIO_LAUNCHED_DESKTOP_FILE".into(), "/a/zen.desktop".into())],
        };
        let scoped = launch_argv(&plan, true);
        assert_eq!(scoped[0], "systemd-run");
        assert_eq!(&scoped[scoped.len() - 2..], ["zen", "--new-window"]);
        let plain = launch_argv(&plan, false);
        assert_eq!(&plain[..2], ["sh", "-c"]);
        assert_eq!(&plain[4..], ["zen", "--new-window"]);
    }

    #[test]
    fn sorts_preferred_first() {
        let mut apps = vec![
            entry("blender.desktop", "Blender", "", ""),
            entry("zen.desktop", "Zen Browser", "", ""),
            entry("audacity.desktop", "Audacity", "", ""),
            entry("kitty.desktop", "kitty", "", ""),
        ];
        let pinned = ["kitty", "zen", "dolphin", "pavucontrol"].map(String::from);
        sort(&mut apps, &pinned);
        let names: Vec<_> = apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["kitty", "Zen Browser", "Audacity", "Blender"]);
        sort(&mut apps, &[]);
        let names: Vec<_> = apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["Audacity", "Blender", "kitty", "Zen Browser"]);
    }

    #[test]
    fn searches_name_generic_comment() {
        let apps = vec![
            entry("a.desktop", "Alpha", "Web Browser", ""),
            entry("b.desktop", "Beta", "", "Edit STRASSE"),
        ];
        assert_eq!(search(&apps, "browser").len(), 1);
        assert_eq!(search(&apps, "straße").len(), 1);
        assert_eq!(search(&apps, "").len(), 2);
    }

    #[test]
    fn parses_desktop_group_only() {
        let text = "[Desktop Entry]\nName=Code\nName[de]=Kode\nExec=code %F\n\n[Desktop Action new]\nExec=code --new\n";
        let entry = Entry::parse(text);
        assert!(entry.is_some());
        if let Some(entry) = entry {
            assert_eq!(entry.get("Exec"), Some("code %F"));
            assert_eq!(entry.localized("Name", &["de".into()]), Some("Kode"));
            assert_eq!(entry.localized("Name", &["fr".into()]), Some("Code"));
        }
    }

    #[test]
    fn splits_like_glib() {
        assert_eq!(
            shell_split(r#"sh -c "echo \"hi\"" 'a b' c\ d"#),
            Some(vec![
                "sh".into(),
                "-c".into(),
                "echo \"hi\"".into(),
                "a b".into(),
                "c d".into()
            ])
        );
    }

    #[test]
    fn expands_field_codes() {
        let tokens: Vec<String> = ["app", "%U", "--name=%c", "%i", "%f"]
            .iter()
            .map(|token| token.to_string())
            .collect();
        assert_eq!(
            expand_exec(&tokens, Some("icon"), "App", "/x.desktop"),
            Err("Multiple desktop file field codes".into())
        );
        let tokens: Vec<String> = ["app", "--name=%c", "%i", "%k", "100%%"]
            .iter()
            .map(|token| token.to_string())
            .collect();
        assert_eq!(
            expand_exec(&tokens, Some("icon"), "App", "/x.desktop"),
            Ok(vec![
                "app".into(),
                "--name=App".into(),
                "--icon".into(),
                "icon".into(),
                "/x.desktop".into(),
                "100%".into()
            ])
        );
    }

    #[test]
    fn hidden_file_format_matches_python() {
        assert_eq!(
            format_hidden(&["a.desktop".into(), "b.desktop".into()]),
            "[\"a.desktop\", \"b.desktop\"]\n"
        );
        assert_eq!(
            parse_hidden("[\"a\", \"b.desktop\", 3]"),
            vec!["a.desktop".to_string(), "b.desktop".to_string()]
        );
        assert!(parse_hidden("{}").is_empty());
    }

    #[test]
    fn dbus_object_path() {
        assert_eq!(object_path("org.telegram.desktop"), "/org/telegram/desktop");
        assert_eq!(object_path("org.gnome.Zenity"), "/org/gnome/Zenity");
    }
}
