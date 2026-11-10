use std::{
    collections::{HashMap, HashSet},
    os::fd::AsFd,
    sync::Arc,
    time::Duration,
};

use calloop::generic::Generic;
use calloop::{Interest, Mode, PostAction};
use services::apps::{self, AppEntry, Watcher};

use crate::{
    app::{App, ServiceEvent, reap},
    icons,
    panels::AppIcon,
};

#[derive(Default)]
pub struct Apps {
    pub catalog: Vec<AppEntry>,
    pub results: Arc<Vec<AppEntry>>,
    pub hidden: Vec<String>,
    pub pinned: Vec<String>,
    pub query: String,
    pub icons: HashMap<String, AppIcon>,
    pending: HashSet<String>,
    reload_queued: bool,
    watcher: Option<Watcher>,
}

impl App {
    pub fn start_apps(&mut self) {
        self.apps.hidden = apps::load_hidden();
        self.apps.pinned = apps::load_pinned();
        self.scan_apps();
        let watcher = match Watcher::new() {
            Ok(watcher) => watcher,
            Err(err) => {
                log::error!("applications watch: {err}");
                return;
            }
        };
        let fd = match watcher.as_fd().try_clone_to_owned() {
            Ok(fd) => fd,
            Err(err) => {
                log::error!("applications watch: {err}");
                return;
            }
        };
        self.apps.watcher = Some(watcher);
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        let result = self.handle.insert_source(source, |_, _, app| {
            let changed = app.apps.watcher.as_mut().is_some_and(Watcher::drain);
            if changed {
                app.queue_app_reload();
            }
            Ok(PostAction::Continue)
        });
        if let Err(err) = result {
            log::error!("applications watch: {err}");
        }
    }

    pub(crate) fn queue_app_reload(&mut self) {
        if self.apps.reload_queued {
            return;
        }
        self.apps.reload_queued = true;
        self.add_timer(Duration::from_millis(100), |app| {
            app.apps.reload_queued = false;
            app.scan_apps();
            None
        });
    }

    fn scan_apps(&self) {
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let catalog = apps::scan();
            let _ = sender.send(ServiceEvent::Apps(catalog));
        });
    }

    pub fn apps_loaded(&mut self, catalog: Vec<AppEntry>) {
        if catalog != self.apps.catalog {
            self.apps.icons.clear();
            self.apps.pending.clear();
        }
        self.apps.catalog = catalog;
        self.reload_app_lists();
        self.respond_apps();
    }

    fn reload_app_lists(&mut self) {
        self.apps.pinned = apps::load_pinned();
        self.apps.hidden = apps::load_hidden();
    }

    fn respond_apps(&mut self) {
        let mut results = apps::search(&self.apps.catalog, &self.apps.query);
        apps::pin_first(&mut results, &self.apps.pinned);
        if *self.apps.results != results {
            self.apps.results = Arc::new(results);
        }
        self.sync();
        self.request_app_icons();
    }

    pub fn search_applications(&mut self, query: &str) {
        self.apps.query = query.to_string();
        self.respond_apps();
    }

    pub fn toggle_application_hidden(&mut self, id: &str) {
        if self.apps.hidden.iter().any(|item| item == id) {
            self.apps.hidden.retain(|item| item != id);
        } else {
            self.apps.hidden.push(id.to_string());
        }
        if let Err(err) = apps::save_hidden(&self.apps.hidden) {
            log::warn!("hidden applications: {err}");
        }
        self.respond_apps();
    }

    pub fn toggle_application_pinned(&mut self, id: &str) {
        if self.apps.pinned.iter().any(|item| item == id) {
            self.apps.pinned.retain(|item| item != id);
        } else {
            self.apps.pinned.push(id.to_string());
        }
        if let Err(err) = apps::save_pinned(&self.apps.pinned) {
            log::warn!("pinned applications: {err}");
        }
        self.respond_apps();
    }

    pub fn launch_application(&mut self, id: &str) {
        let Some(app) = self.apps.catalog.iter().find(|app| app.id == id) else {
            log::warn!("launch {id}: Application no longer exists");
            return;
        };
        let plan = match apps::launch_plan(app) {
            Ok(plan) => plan,
            Err(err) => {
                log::warn!("launch {id}: {err}");
                return;
            }
        };
        match apps::scope_command(&plan).spawn() {
            Ok(child) => reap(child),
            Err(err) => log::warn!("launch {id}: {err}"),
        }
    }

    pub fn apps_panel_changed(&mut self, panel: &str) {
        if panel == "launcher" {
            self.reload_app_lists();
            self.search_applications("");
        } else {
            self.apps.icons.clear();
            self.apps.pending.clear();
        }
    }

    pub fn request_app_icons(&mut self) {
        for (id, icon) in self.overlay.app_icon_requests() {
            if self.apps.icons.contains_key(&id) || !self.apps.pending.insert(id.clone()) {
                continue;
            }
            let sender = self.events.clone();
            std::thread::spawn(move || {
                let result = match icons::app_source(&icon) {
                    Some(source) => AppIcon::Image(icons::load(&source, 56).map(Arc::new)),
                    None => AppIcon::Letter,
                };
                let _ = sender.send(ServiceEvent::AppIcon(id, result));
            });
        }
    }

    pub fn app_icon_loaded(&mut self, id: String, icon: AppIcon) {
        if !self.apps.pending.remove(&id) {
            return;
        }
        self.apps.icons.insert(id, icon);
        self.overlay.set_app_icons(&self.apps.icons);
        self.overlay.schedule(&mut self.platform);
    }
}
