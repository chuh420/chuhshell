use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, TrySendError};
use std::thread;
use std::time::Duration;

use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;

use crate::apps::{self, AppEntry};
use crate::modules;
use crate::niri::{self, WindowProcess};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessIdentity {
    pid: u32,
    start_time: u64,
}

struct ProcessInfo {
    pid: u32,
    parent: u32,
    start_time: u64,
    executable: String,
    flatpak_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BackgroundApp {
    desktop_id: String,
    name: String,
    icon: String,
    targets: Vec<ProcessIdentity>,
}

pub struct BackgroundManager {
    app: gtk::Application,
    label: RefCell<Option<gtk::Label>>,
    entries: RefCell<Vec<BackgroundApp>>,
    drawer: RefCell<Option<gtk::Window>>,
    list: RefCell<Option<gtk::Box>>,
}

impl BackgroundManager {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let manager = Rc::new(Self {
            app: app.clone(),
            label: RefCell::new(None),
            entries: RefCell::new(Vec::new()),
            drawer: RefCell::new(None),
            list: RefCell::new(None),
        });
        manager.start();
        manager
    }

    fn start(self: &Rc<Self>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut desktop_apps = apps::load_apps();
            let mut refresh = 0;
            loop {
                if refresh == 15 {
                    desktop_apps = apps::load_apps();
                    refresh = 0;
                }
                if let Some(entries) = scan(&desktop_apps) {
                    match sender.try_send(entries) {
                        Ok(()) | Err(TrySendError::Full(_)) => {}
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
                refresh += 1;
                thread::sleep(Duration::from_secs(4));
            }
        });
        let manager = Rc::clone(self);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if let Some(entries) = receiver.try_iter().last()
                && *manager.entries.borrow() != entries
            {
                *manager.entries.borrow_mut() = entries;
                manager.refresh();
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn attach_label(&self, label: &gtk::Label) {
        *self.label.borrow_mut() = Some(label.clone());
        self.refresh_label();
    }

    fn refresh_label(&self) {
        if let Some(label) = self.label.borrow().as_ref() {
            let count = self.entries.borrow().len();
            label.set_text(&if count == 0 {
                "󰀻".to_owned()
            } else {
                format!("󰀻 {count}")
            });
        }
    }

    pub fn toggle(self: &Rc<Self>) {
        if let Some(window) = self.drawer.borrow_mut().take() {
            *self.list.borrow_mut() = None;
            window.close();
            return;
        }
        let window = gtk::Window::builder()
            .application(&self.app)
            .title("Background apps")
            .default_width(350)
            .build();
        window.set_widget_name("background-apps-drawer");
        window.init_layer_shell();
        window.set_namespace(Some("chuhshell-background-apps"));
        window.set_layer(layer_shell::Layer::Overlay);
        window.set_anchor(layer_shell::Edge::Top, true);
        window.set_margin(layer_shell::Edge::Top, 40);
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(layer_shell::KeyboardMode::None);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("background-apps-content");
        let heading = gtk::Label::new(Some("Background apps"));
        heading.add_css_class("background-apps-heading");
        heading.set_xalign(0.0);
        root.append(&heading);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(360);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        scroll.set_child(Some(&list));
        root.append(&scroll);
        window.set_child(Some(&root));
        *self.list.borrow_mut() = Some(list);
        *self.drawer.borrow_mut() = Some(window.clone());
        self.refresh();
        window.present();
    }

    fn refresh(&self) {
        self.refresh_label();
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let entries = self.entries.borrow();
        if entries.is_empty() {
            let empty = gtk::Label::new(Some("No background apps"));
            empty.add_css_class("background-apps-empty");
            list.append(&empty);
            return;
        }
        for entry in entries.iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("background-app-row");
            let icon = if Path::new(&entry.icon).is_absolute() {
                gtk::Image::from_file(&entry.icon)
            } else {
                gtk::Image::from_icon_name(if entry.icon.is_empty() {
                    "application-x-executable-symbolic"
                } else {
                    &entry.icon
                })
            };
            icon.set_pixel_size(24);
            row.append(&icon);
            let name = gtk::Label::new(Some(&entry.name));
            name.set_hexpand(true);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&name);
            let open = gtk::Button::with_label("Open");
            open.add_css_class("background-app-action");
            let desktop_id = entry.desktop_id.clone();
            open.connect_clicked(move |_| {
                let app_id = desktop_id.strip_suffix(".desktop").unwrap_or(&desktop_id);
                modules::spawn_detached("gtk-launch", &[app_id]);
            });
            row.append(&open);
            let quit = gtk::Button::with_label("Quit");
            quit.add_css_class("background-app-action");
            let app = entry.clone();
            quit.connect_clicked(move |_| {
                let app = app.clone();
                thread::spawn(move || quit_app(&app));
            });
            row.append(&quit);
            list.append(&row);
        }
    }
}

fn command_name(exec: &str) -> Option<String> {
    let argv = glib::shell_parse_argv(exec).ok()?;
    let args: Vec<_> = argv.iter().filter_map(|part| part.to_str()).collect();
    let mut index = 0;
    if Path::new(*args.first()?).file_name()?.to_str()? == "env" {
        index = 1;
        while args
            .get(index)
            .is_some_and(|arg| arg.starts_with('-') || arg.contains('='))
        {
            index += 1;
        }
    }
    let name = Path::new(*args.get(index)?)
        .file_name()?
        .to_str()?
        .to_ascii_lowercase();
    if [
        "sh",
        "bash",
        "zsh",
        "java",
        "env",
        "flatpak",
        "niri",
        "chuhshell",
    ]
    .contains(&name.as_str())
        || name.starts_with("python")
        || name.starts_with("node")
    {
        None
    } else {
        Some(name)
    }
}

fn process_stat(pid: u32) -> Option<(u32, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(") ")?.1;
    let fields: Vec<_> = rest.split_whitespace().collect();
    if fields.first().is_some_and(|state| *state == "Z") {
        return None;
    }
    Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

fn processes() -> HashMap<u32, ProcessInfo> {
    let mut result = HashMap::new();
    let Ok(uid) = fs::metadata("/proc/self").map(|metadata| metadata.uid()) else {
        return result;
    };
    let Ok(entries) = fs::read_dir("/proc") else {
        return result;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if !entry.metadata().is_ok_and(|metadata| metadata.uid() == uid) {
            continue;
        }
        let Some((parent, start_time)) = process_stat(pid) else {
            continue;
        };
        let Some(executable) = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_ascii_lowercase())
            })
        else {
            continue;
        };
        result.insert(
            pid,
            ProcessInfo {
                pid,
                parent,
                start_time,
                executable,
                flatpak_id: fs::read_to_string(format!("/proc/{pid}/root/.flatpak-info"))
                    .ok()
                    .and_then(|info| {
                        info.lines()
                            .find_map(|line| line.strip_prefix("name="))
                            .map(str::to_owned)
                    }),
            },
        );
    }
    result
}

fn has_ancestor(pid: u32, root: u32, processes: &HashMap<u32, ProcessInfo>) -> bool {
    let mut current = pid;
    for _ in 0..64 {
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

fn same_executable_ancestor(process: &ProcessInfo, processes: &HashMap<u32, ProcessInfo>) -> bool {
    let mut parent = process.parent;
    for _ in 0..64 {
        let Some(ancestor) = processes.get(&parent) else {
            return false;
        };
        if ancestor.executable == process.executable {
            return true;
        }
        parent = ancestor.parent;
    }
    false
}

fn is_visible(
    app: &AppEntry,
    pid: u32,
    windows: &[WindowProcess],
    processes: &HashMap<u32, ProcessInfo>,
) -> bool {
    let app_id = app.id.strip_suffix(".desktop").unwrap_or(&app.id);
    windows.iter().any(|window| {
        window.app_id.as_deref() == Some(app_id)
            || window
                .pid
                .is_some_and(|window_pid| has_ancestor(window_pid, pid, processes))
    })
}

fn scan(apps: &[AppEntry]) -> Option<Vec<BackgroundApp>> {
    let windows = niri::window_processes()?;
    let processes = processes();
    let mut by_command = HashMap::new();
    let mut by_id = HashMap::new();
    for app in apps {
        if app.terminal
            || app.id.contains("url-handler")
            || app.id.contains("protocol")
            || app.exec.is_empty()
        {
            continue;
        }
        let app_id = app.id.strip_suffix(".desktop").unwrap_or(&app.id);
        by_id.entry(app_id).or_insert(app);
        if let Some(name) = command_name(&app.exec) {
            by_command.entry(name).or_insert(app);
        }
    }
    let mut grouped: HashMap<String, BackgroundApp> = HashMap::new();
    for process in processes.values() {
        let Some(app) = process
            .flatpak_id
            .as_deref()
            .and_then(|app_id| by_id.get(app_id))
            .or_else(|| by_command.get(&process.executable))
        else {
            continue;
        };
        if same_executable_ancestor(process, &processes)
            || is_visible(app, process.pid, &windows, &processes)
        {
            continue;
        }
        let entry = grouped
            .entry(app.id.clone())
            .or_insert_with(|| BackgroundApp {
                desktop_id: app.id.clone(),
                name: app.name.clone(),
                icon: app.icon.clone(),
                targets: Vec::new(),
            });
        entry.targets.push(ProcessIdentity {
            pid: process.pid,
            start_time: process.start_time,
        });
    }
    let mut result: Vec<_> = grouped.into_values().collect();
    for entry in &mut result {
        entry.targets.sort_by_key(|target| target.pid);
    }
    result.sort_by_cached_key(|entry| entry.name.to_lowercase());
    Some(result)
}

fn quit_app(app: &BackgroundApp) {
    let Some(windows) = niri::window_processes() else {
        return;
    };
    let app_id = app
        .desktop_id
        .strip_suffix(".desktop")
        .unwrap_or(&app.desktop_id);
    let processes = processes();
    if windows.iter().any(|window| {
        window.app_id.as_deref() == Some(app_id)
            || window.pid.is_some_and(|window_pid| {
                app.targets.iter().any(|target| {
                    processes.get(&target.pid).is_some_and(|process| {
                        process.start_time == target.start_time
                            && has_ancestor(window_pid, target.pid, &processes)
                    })
                })
            })
    }) {
        return;
    }
    for target in &app.targets {
        if process_stat(target.pid).is_some_and(|(_, start_time)| start_time == target.start_time) {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(target.pid.to_string())
                .status();
        }
    }
}
