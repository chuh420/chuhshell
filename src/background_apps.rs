use crate::{
    apps::{self, AppEntry},
    niri::{self, WindowProcess},
    process,
};
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ProcessIdentity {
    pid: u32,
    start_time: u64,
}
#[derive(Clone)]
struct ProcessInfo {
    identity: ProcessIdentity,
    parent: u32,
    executable: PathBuf,
    desktop_id: Option<String>,
    flatpak_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct BackgroundApp {
    desktop_id: String,
    name: String,
    icon: String,
    targets: Vec<ProcessIdentity>,
}

const PROCESS_LIMIT: usize = 2048;
const DIRECTORY_LIMIT: usize = 16384;
const SCAN_TIME: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    entries: Vec<BackgroundApp>,
    incomplete: bool,
}

struct ScanBudget {
    deadline: Instant,
    remaining: usize,
    bytes_remaining: usize,
    incomplete: bool,
}

impl ScanBudget {
    fn new() -> Self {
        Self {
            deadline: Instant::now() + SCAN_TIME,
            remaining: DIRECTORY_LIMIT,
            bytes_remaining: 16 * 1024 * 1024,
            incomplete: false,
        }
    }

    fn available(&mut self) -> bool {
        if self.remaining == 0 || Instant::now() >= self.deadline || process::stopped() {
            self.incomplete = true;
            false
        } else {
            true
        }
    }

    fn read(&mut self, path: &Path, limit: usize) -> Option<Vec<u8>> {
        if !self.available() || self.bytes_remaining == 0 {
            self.incomplete = true;
            return None;
        }
        let limit = limit.min(self.bytes_remaining);
        match crate::storage::read_limited(path, limit) {
            Ok(bytes) => {
                self.bytes_remaining -= bytes.len();
                Some(bytes)
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::Other {
                    self.incomplete = true;
                }
                None
            }
        }
    }

    fn visit(&mut self) -> bool {
        if !self.available() {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

pub struct BackgroundManager {
    buttons: RefCell<Vec<glib::WeakRef<gtk::Button>>>,
    entries: RefCell<Vec<BackgroundApp>>,
    drawer: RefCell<Option<gtk::Popover>>,
    list: RefCell<Option<gtk::Box>>,
    rows: RefCell<HashMap<String, (BackgroundApp, gtk::Box)>>,
    active: Arc<AtomicBool>,
    status: RefCell<Option<String>>,
    wake: std::sync::mpsc::SyncSender<()>,
}

impl BackgroundManager {
    pub fn new(_app: &gtk::Application) -> Rc<Self> {
        let (wake, refresh) = std::sync::mpsc::sync_channel(1);
        let active = Arc::new(AtomicBool::new(false));
        let manager = Rc::new(Self {
            active: active.clone(),
            status: RefCell::new(None),
            buttons: RefCell::new(Vec::new()),
            entries: RefCell::new(Vec::new()),
            drawer: RefCell::new(None),
            list: RefCell::new(None),
            rows: RefCell::new(HashMap::new()),
            wake,
        });
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let mut previous = None;
            let mut delay = 4;
            while !process::stopped() {
                if !active.load(Ordering::Relaxed) {
                    if matches!(
                        refresh.recv_timeout(Duration::from_secs(30)),
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                    ) {
                        return;
                    }
                    continue;
                }
                let result = niri::window_processes()
                    .map(|windows| scan(&apps::load_apps(), &windows))
                    .ok_or("Niri is unavailable");
                if previous.as_ref() == Some(&result) {
                    delay = (delay * 2).min(30);
                } else {
                    delay = if result.as_ref().is_ok_and(|snapshot| !snapshot.incomplete) {
                        4
                    } else {
                        30
                    };
                }
                previous = Some(result.clone());
                if tx.send_blocking(result).is_err() {
                    return;
                }
                if matches!(
                    refresh.recv_timeout(Duration::from_secs(delay)),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                ) {
                    return;
                }
            }
        });
        let weak = Rc::downgrade(&manager);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                let Some(manager) = weak.upgrade() else {
                    break;
                };
                match result {
                    Ok(snapshot) => {
                        *manager.status.borrow_mut() = snapshot
                            .incomplete
                            .then(|| "Incomplete process scan; try refreshing later".into());
                        if *manager.entries.borrow() != snapshot.entries {
                            *manager.entries.borrow_mut() = snapshot.entries;
                        }
                        manager.refresh();
                    }
                    Err(error) => {
                        *manager.status.borrow_mut() = Some(error.into());
                        manager.entries.borrow_mut().clear();
                        manager.refresh();
                        for button in manager.buttons.borrow().iter().filter_map(|w| w.upgrade()) {
                            button.set_label("󰀻 --");
                            button.set_tooltip_text(Some(error));
                        }
                    }
                }
            }
        });
        manager
    }

    pub fn request_refresh(&self) {
        let _ = self.wake.try_send(());
    }

    pub fn attach_button(self: &Rc<Self>, button: &gtk::Button) {
        let weak = Rc::downgrade(self);
        button.connect_map(move |_| {
            if let Some(manager) = weak.upgrade() {
                manager.refresh();
                manager.request_refresh();
            }
        });
        let weak = Rc::downgrade(self);
        button.connect_unmap(move |_| {
            if let Some(manager) = weak.upgrade() {
                manager.refresh();
            }
        });
        self.buttons.borrow_mut().push(button.downgrade());
        self.refresh();
    }
    pub fn toggle(self: &Rc<Self>) {
        let button = crate::ui::active_button(&self.buttons.borrow());
        if let Some(button) = button {
            self.toggle_at(&button);
        }
    }
    pub fn toggle_at(self: &Rc<Self>, anchor: &gtk::Button) {
        let old = self.drawer.borrow_mut().take();
        if let Some(old) = old {
            old.popdown();
            return;
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("background-apps-content");
        let title = gtk::Label::new(Some("Background apps"));
        title.add_css_class("background-apps-heading");
        root.append(&title);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(
            crate::ui::widget_monitor(anchor)
                .map_or(360, |m| (m.geometry().height() - 120).clamp(100, 360)),
        );
        scroll.set_min_content_width(300);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        scroll.set_child(Some(&list));
        root.append(&scroll);
        let popup = crate::ui::popover(anchor, &root);
        *self.list.borrow_mut() = Some(list);
        *self.drawer.borrow_mut() = Some(popup.clone());
        let weak = Rc::downgrade(self);
        popup.connect_closed(move |_| {
            if let Some(manager) = weak.upgrade() {
                manager.drawer.borrow_mut().take();
                manager.list.borrow_mut().take();
                manager.rows.borrow_mut().clear();
                manager.refresh();
            }
        });
        self.refresh();
        popup.popup();
        let _ = self.wake.try_send(());
    }
    fn refresh(&self) {
        self.active.store(
            self.drawer.borrow().is_some()
                || self
                    .buttons
                    .borrow()
                    .iter()
                    .filter_map(|w| w.upgrade())
                    .any(|b| b.is_mapped()),
            Ordering::Relaxed,
        );
        let entries = self.entries.borrow();
        let status = self.status.borrow();
        self.buttons.borrow_mut().retain(|weak| {
            if let Some(button) = weak.upgrade() {
                button.set_label(&if entries.is_empty() {
                    "󰀻".into()
                } else {
                    format!("󰀻 {}", entries.len())
                });
                button.set_tooltip_text(Some(status.as_deref().unwrap_or("Background apps")));
                true
            } else {
                false
            }
        });
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        let mut rows = self.rows.borrow_mut();
        for (_, row) in rows.values() {
            if let Some(quit) = row.last_child() {
                quit.set_sensitive(status.is_none());
            }
        }
        let mut child = list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if matches!(
                widget.widget_name().as_str(),
                "background-status" | "background-empty"
            ) {
                list.remove(&widget);
            }
        }
        if let Some(message) = status.as_deref() {
            let label = gtk::Label::new(Some(message));
            label.set_widget_name("background-status");
            label.set_wrap(true);
            list.append(&label);
        }
        let mut previous: Option<gtk::Box> = None;
        for entry in entries.iter() {
            let (_, row) = rows.entry(entry.desktop_id.clone()).or_insert_with(|| {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.add_css_class("background-app-row");
                let icon = crate::ui::image(&entry.icon, 24);
                icon.set_pixel_size(24);
                row.append(&icon);
                let name = gtk::Label::new(Some(&entry.name));
                name.set_hexpand(true);
                name.set_xalign(0.0);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row.append(&name);
                let open = gtk::Button::with_label("Open");
                open.add_css_class("background-app-action");
                let id = entry.desktop_id.clone();
                open.connect_clicked(move |button| {
                    let id = id.clone();
                    let weak = button.downgrade();
                    button.set_sensitive(false);
                    glib::MainContext::default().spawn_local(async move {
                        let result = apps::launch(&id).await;
                        if let Some(button) = weak.upgrade() {
                            button.set_sensitive(true);
                            if let Err(error) = result {
                                button.set_tooltip_text(Some(&error));
                                eprintln!("chuhshell: {error}");
                            }
                        }
                    });
                });
                row.append(&open);
                let quit = gtk::Button::with_label("Quit");
                quit.set_sensitive(status.is_none());
                quit.add_css_class("background-app-action");
                quit.set_tooltip_text(Some(
                    "Send a termination request to this background application",
                ));
                let app = entry.clone();
                let wake = self.wake.clone();
                quit.connect_clicked(move |button| {
                    button.set_sensitive(false);
                    let weak = button.downgrade();
                    let app = app.clone();
                    let wake = wake.clone();
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let _ = tx.send_blocking(quit_app(&app));
                        let _ = wake.try_send(());
                    });
                    glib::MainContext::default().spawn_local(async move {
                        if let Ok(result) = rx.recv().await
                            && let Some(button) = weak.upgrade()
                        {
                            button.set_sensitive(true);
                            if let Err(error) = result {
                                button.set_tooltip_text(Some(&error));
                                eprintln!("chuhshell: {error}");
                            }
                        }
                    });
                });
                row.append(&quit);
                list.append(&row);
                (entry.clone(), row)
            });
            list.reorder_child_after(row, previous.as_ref());
            previous = Some(row.clone());
        }
        if entries.is_empty() {
            let empty = gtk::Label::new(Some("No background apps"));
            empty.set_widget_name("background-empty");
            empty.add_css_class("background-apps-empty");
            list.append(&empty);
        }
    }
}

fn process_stat(pid: u32) -> Option<(u32, u64)> {
    let stat =
        crate::storage::read_text(Path::new(&format!("/proc/{pid}/stat")), 16 * 1024).ok()?;
    parse_stat(&stat)
}
fn parse_stat(stat: &str) -> Option<(u32, u64)> {
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    if fields
        .first()
        .is_some_and(|state| matches!(*state, "Z" | "X"))
    {
        return None;
    }
    Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}
fn processes(budget: &mut ScanBudget) -> HashMap<u32, ProcessInfo> {
    static CACHE: OnceLock<Mutex<HashMap<ProcessIdentity, ProcessInfo>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut result = HashMap::new();
    let Ok(uid) = fs::metadata("/proc/self").map(|m| m.uid()) else {
        budget.incomplete = true;
        return result;
    };
    let directory = match fs::read_dir("/proc") {
        Ok(directory) => directory,
        Err(_) => {
            budget.incomplete = true;
            return result;
        }
    };
    for entry in directory {
        if !budget.visit() || result.len() >= PROCESS_LIMIT {
            budget.incomplete = true;
            break;
        }
        let Ok(entry) = entry else {
            budget.incomplete = true;
            continue;
        };
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        if !entry.metadata().is_ok_and(|m| m.uid() == uid) {
            continue;
        }
        let stat = budget
            .read(Path::new(&format!("/proc/{pid}/stat")), 16 * 1024)
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let Some((parent, start_time)) = stat.as_deref().and_then(parse_stat) else {
            continue;
        };
        let Ok(executable) = fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        let identity = ProcessIdentity { pid, start_time };
        if let Some(cached) = cache
            .get(&identity)
            .filter(|p| p.executable == executable && p.parent == parent)
        {
            result.insert(pid, cached.clone());
            continue;
        }
        let flatpak_id = budget
            .read(
                Path::new(&format!("/proc/{pid}/root/.flatpak-info")),
                64 * 1024,
            )
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| {
                let key = glib::KeyFile::new();
                key.load_from_data(&text, glib::KeyFileFlags::NONE).ok()?;
                key.string("Application", "name")
                    .ok()
                    .map(|v| v.to_string())
            });
        let desktop_id = budget
            .read(Path::new(&format!("/proc/{pid}/environ")), 128 * 1024)
            .and_then(|bytes| {
                let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
                let launched_pid = fields.iter().find_map(|field| {
                    field
                        .strip_prefix(b"GIO_LAUNCHED_DESKTOP_FILE_PID=")
                        .and_then(|value| std::str::from_utf8(value).ok())
                        .and_then(|value| value.parse::<u32>().ok())
                });
                if launched_pid != Some(pid) {
                    return None;
                }
                fields.iter().find_map(|field| {
                    field
                        .strip_prefix(b"GIO_LAUNCHED_DESKTOP_FILE=")
                        .and_then(|path| std::str::from_utf8(path).ok())
                        .and_then(|path| Path::new(path).file_name())
                        .map(|s| s.to_string_lossy().into_owned())
                })
            });
        result.insert(
            pid,
            ProcessInfo {
                identity: ProcessIdentity { pid, start_time },
                parent,
                executable,
                desktop_id,
                flatpak_id,
            },
        );
    }
    *cache = result
        .values()
        .map(|p| (p.identity.clone(), p.clone()))
        .collect();
    result
}
fn command_path(exec: &str) -> Option<PathBuf> {
    type Entry = (std::time::Instant, Option<PathBuf>);
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((time, path)) = cache
        .get(exec)
        .filter(|(time, _)| time.elapsed().as_secs() < 60)
    {
        let _ = time;
        return path.clone();
    }
    let path = resolve_command_path(exec);
    if cache.len() >= 2048 {
        cache.clear();
    }
    cache.insert(exec.to_owned(), (std::time::Instant::now(), path.clone()));
    path
}
fn resolve_command_path(exec: &str) -> Option<PathBuf> {
    let args = glib::shell_parse_argv(exec).ok()?;
    let args: Vec<_> = args.iter().filter_map(|arg| arg.to_str()).collect();
    let mut index = 0;
    if Path::new(args.first()?).file_name()? == "env" {
        index = 1;
        while args
            .get(index)
            .is_some_and(|arg| arg.contains('=') || arg.starts_with('-'))
        {
            index += 1;
        }
    }
    let command = Path::new(args.get(index)?);
    let name = command.file_name()?.to_str()?;
    if [
        "sh",
        "bash",
        "zsh",
        "env",
        "flatpak",
        "java",
        "electron",
        "niri",
        "chuhshell",
    ]
    .contains(&name)
        || name.starts_with("python")
        || name.starts_with("node")
    {
        return None;
    }
    if command.is_absolute() {
        return fs::canonicalize(command).ok();
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .find_map(|dir| fs::canonicalize(dir.join(command)).ok())
}
fn has_ancestor(pid: u32, root: u32, processes: &HashMap<u32, ProcessInfo>) -> bool {
    let mut current = pid;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        if current == root {
            return true;
        }
        let Some(process) = processes.get(&current) else {
            return false;
        };
        current = process.parent;
    }
    false
}
fn matches_window(app: &AppEntry, window: &WindowProcess) -> bool {
    window.app_id.as_deref().is_some_and(|id| {
        id.eq_ignore_ascii_case(app.id.strip_suffix(".desktop").unwrap_or(&app.id))
            || (!app.startup_wm_class.is_empty() && id.eq_ignore_ascii_case(&app.startup_wm_class))
    })
}
fn scan(apps: &[AppEntry], windows: &[WindowProcess]) -> Snapshot {
    static KNOWN: OnceLock<Mutex<HashMap<ProcessIdentity, String>>> = OnceLock::new();
    let mut budget = ScanBudget::new();
    let processes = processes(&mut budget);
    let mut known = KNOWN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    known.retain(|identity, _| {
        processes
            .get(&identity.pid)
            .is_some_and(|p| p.identity == *identity)
    });
    for window in windows {
        if !budget.available() {
            break;
        }
        if let Some(process) = window.pid.and_then(|pid| processes.get(&pid)) {
            let candidates: Vec<_> = apps
                .iter()
                .filter(|app| matches_window(app, window))
                .collect();
            if candidates.len() == 1 {
                known.insert(process.identity.clone(), candidates[0].id.clone());
            }
        }
    }
    let entries = classify_bounded(apps, windows, &processes, &known, &mut budget);
    Snapshot {
        entries,
        incomplete: budget.incomplete,
    }
}

#[cfg(test)]
fn classify(
    apps: &[AppEntry],
    windows: &[WindowProcess],
    processes: &HashMap<u32, ProcessInfo>,
    known: &HashMap<ProcessIdentity, String>,
) -> Vec<BackgroundApp> {
    classify_bounded(apps, windows, processes, known, &mut ScanBudget::new())
}

fn classify_bounded(
    apps: &[AppEntry],
    windows: &[WindowProcess],
    processes: &HashMap<u32, ProcessInfo>,
    known: &HashMap<ProcessIdentity, String>,
    budget: &mut ScanBudget,
) -> Vec<BackgroundApp> {
    let mut paths: HashMap<PathBuf, Vec<&AppEntry>> = HashMap::new();
    for app in apps.iter().filter(|app| !app.terminal) {
        if !budget.available() {
            break;
        }
        if let Some(path) = command_path(&app.exec) {
            paths.entry(path).or_default().push(app);
        }
    }
    let mut matches: HashMap<u32, &AppEntry> = HashMap::new();
    for process in processes.values() {
        if !budget.available() {
            break;
        }
        let id = process
            .flatpak_id
            .as_ref()
            .map(|id| format!("{id}.desktop"))
            .or_else(|| process.desktop_id.clone())
            .or_else(|| known.get(&process.identity).cloned());
        let app = id
            .as_deref()
            .and_then(|id| apps.iter().find(|app| app.id == id && !app.terminal))
            .or_else(|| {
                paths
                    .get(&process.executable)
                    .filter(|apps| apps.len() == 1)
                    .map(|apps| apps[0])
            });
        if let Some(app) = app {
            matches.insert(process.identity.pid, app);
        }
    }
    let mut result = Vec::new();
    for app in apps {
        if !budget.available() {
            break;
        }
        let candidates: Vec<_> = matches
            .iter()
            .filter(|(_, a)| a.id == app.id)
            .map(|(pid, _)| *pid)
            .collect();
        if candidates.is_empty()
            || windows.iter().any(|w| {
                matches_window(app, w)
                    || w.pid.is_some_and(|pid| {
                        candidates.iter().any(|root| {
                            if !budget.visit() {
                                return true;
                            }
                            has_ancestor(pid, *root, processes)
                        })
                    })
            })
        {
            continue;
        }
        let mut targets: Vec<_> = candidates
            .iter()
            .filter(|pid| {
                if !budget.visit() {
                    return false;
                }
                !candidates.iter().any(|ancestor| {
                    if !budget.visit() {
                        return true;
                    }
                    *pid != ancestor && has_ancestor(**pid, *ancestor, processes)
                })
            })
            .filter_map(|pid| processes.get(pid).map(|p| p.identity.clone()))
            .collect();
        targets.sort_by_key(|p| p.pid);
        if !targets.is_empty() {
            result.push(BackgroundApp {
                desktop_id: app.id.clone(),
                name: app.name.clone(),
                icon: app.icon.clone(),
                targets,
            });
        }
    }
    result.sort_by_cached_key(|app| app.name.to_lowercase());
    result
}
fn quit_app(app: &BackgroundApp) -> Result<(), String> {
    let windows =
        niri::window_processes().ok_or("Niri is unavailable; application was not terminated")?;
    let snapshot = scan(&apps::load_apps(), &windows);
    if snapshot.incomplete {
        return Err("Incomplete process scan; application was not terminated".into());
    }
    let current = snapshot
        .entries
        .into_iter()
        .find(|a| a.desktop_id == app.desktop_id)
        .ok_or("Application changed or has an open window")?;
    for target in &app.targets {
        if !current.targets.contains(target) {
            return Err("Application processes changed; refresh and try again".into());
        }
    }
    let mut handles = Vec::new();
    for target in &app.targets {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, target.pid, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        if !process_stat(target.pid).is_some_and(|(_, time)| time == target.start_time) {
            return Err("Application exited or restarted".into());
        }
        handles.push(fd);
    }
    for fd in handles {
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(id: &str) -> AppEntry {
        apps::parse_entry(
            id,
            "[Desktop Entry]\nType=Application\nName=Example\nExec=/usr/bin/sleep\n",
            false,
        )
        .unwrap()
    }
    fn process(pid: u32, parent: u32) -> ProcessInfo {
        ProcessInfo {
            identity: ProcessIdentity {
                pid,
                start_time: 10,
            },
            parent,
            executable: fs::canonicalize("/usr/bin/sleep").unwrap(),
            desktop_id: None,
            flatpak_id: None,
        }
    }
    #[test]
    fn process_scan_budgets_reject_oversized_reads_and_expired_work() {
        let path =
            std::env::temp_dir().join(format!("chuhshell-process-budget-{}", std::process::id()));
        fs::write(&path, vec![b'a'; 129 * 1024]).unwrap();
        let mut budget = ScanBudget::new();
        assert!(budget.read(&path, 128 * 1024).is_none());
        assert!(budget.incomplete);
        fs::write(&path, b"small").unwrap();
        let mut budget = ScanBudget::new();
        budget.bytes_remaining = 5;
        assert_eq!(budget.read(&path, 16), Some(b"small".to_vec()));
        assert!(budget.read(&path, 16).is_none());
        assert!(budget.incomplete);
        let mut budget = ScanBudget::new();
        budget.deadline = Instant::now();
        assert!(
            classify_bounded(
                &[app("one.desktop")],
                &[],
                &HashMap::new(),
                &HashMap::new(),
                &mut budget
            )
            .is_empty()
        );
        assert!(budget.incomplete);
        let mut budget = ScanBudget::new();
        budget.remaining = 1;
        assert!(budget.visit());
        assert!(!budget.visit());
        assert!(budget.incomplete);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn ambiguous_executable_is_not_assigned_to_an_app() {
        let processes = HashMap::from([(10, process(10, 1))]);
        assert!(
            classify(
                &[app("one.desktop"), app("two.desktop")],
                &[],
                &processes,
                &HashMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn background_targets_only_roots_and_excludes_open_apps() {
        let processes = HashMap::from([(10, process(10, 1)), (11, process(11, 10))]);
        let apps = [app("one.desktop")];
        let result = classify(&apps, &[], &processes, &HashMap::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].targets.len(), 1);
        assert_eq!(result[0].targets[0].pid, 10);
        let windows = [WindowProcess {
            pid: Some(11),
            app_id: None,
        }];
        assert!(classify(&apps, &windows, &processes, &HashMap::new()).is_empty());
    }
    #[test]
    fn flatpak_helpers_are_grouped_under_the_sandbox_root() {
        let mut first = process(10, 1);
        first.flatpak_id = Some("org.example.App".into());
        let mut helper = process(11, 10);
        helper.flatpak_id = first.flatpak_id.clone();
        helper.executable = PathBuf::from("/app/helper");
        let processes = HashMap::from([(10, first), (11, helper)]);
        let result = classify(
            &[app("org.example.App.desktop")],
            &[],
            &processes,
            &HashMap::new(),
        );
        assert_eq!(result[0].targets.len(), 1);
        assert_eq!(result[0].targets[0].pid, 10);
    }
    #[test]
    fn interpreter_commands_are_not_application_identity() {
        for command in [
            "python3 app.py",
            "env A=B node app.js",
            "electron app",
            "bash -c foo",
        ] {
            assert!(command_path(command).is_none());
        }
    }
    #[test]
    fn ancestry_handles_cycles_and_siblings() {
        let make = |pid, parent| {
            (
                pid,
                ProcessInfo {
                    identity: ProcessIdentity { pid, start_time: 1 },
                    parent,
                    executable: PathBuf::new(),
                    desktop_id: None,
                    flatpak_id: None,
                },
            )
        };
        let processes = HashMap::from([make(1, 1), make(2, 1), make(3, 1)]);
        assert!(has_ancestor(2, 1, &processes));
        assert!(!has_ancestor(2, 3, &processes));
    }
}
