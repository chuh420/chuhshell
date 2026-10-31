use std::{collections::HashMap, sync::Arc};

use services::tray::{self, MenuEntry, TrayItem};
use ui::tiny_skia::Pixmap;

use crate::{
    app::{App, ServiceEvent},
    icons,
};

const ICON_SIZE: u32 = 21;

#[derive(Default)]
pub struct Tray {
    pub handle: Option<tray::Handle>,
    pub items: Vec<TrayItem>,
    pub icons: HashMap<String, Option<Arc<Pixmap>>>,
    pub pending_icons: Vec<String>,
    pub target: Option<String>,
    pub only_menu: bool,
    pub expanded: bool,
    pub serial: u64,
    pub menu: Option<Vec<MenuEntry>>,
    pub menu_open: Option<String>,
}

impl Tray {
    pub fn find(&self, key: &str) -> Option<&TrayItem> {
        self.items.iter().find(|item| item.key == key)
    }

    pub fn target_item(&self) -> Option<&TrayItem> {
        self.target.as_deref().and_then(|key| self.find(key))
    }

    pub fn title(&self) -> String {
        self.target_item()
            .map(|item| {
                if item.title.is_empty() {
                    item.id.clone()
                } else {
                    item.title.clone()
                }
            })
            .unwrap_or_default()
    }

    pub fn bar_entries(&self) -> Vec<(String, Option<Arc<Pixmap>>)> {
        let mut items: Vec<&TrayItem> = self.items.iter().collect();
        items.sort_by(|a, b| a.id.cmp(&b.id));
        items
            .into_iter()
            .map(|item| {
                (
                    item.key.clone(),
                    self.icons.get(&item.key).cloned().flatten(),
                )
            })
            .collect()
    }
}

fn icon_fields(item: &TrayItem) -> impl PartialEq + '_ {
    (
        &item.status,
        &item.icon_name,
        &item.attention_icon_name,
        &item.icon_theme_path,
        &item.icon_pixmap,
        &item.attention_pixmap,
    )
}

impl App {
    pub fn start_tray(&mut self) {
        let sender = self.events.clone();
        let task = async move {
            let callback_sender = sender.clone();
            match tray::serve(move |event| {
                let _ = callback_sender.send(ServiceEvent::Tray(event));
            })
            .await
            {
                Ok(handle) => {
                    let _ = sender.send(ServiceEvent::TrayServer(handle));
                }
                Err(err) => log::error!("tray watcher: {err}"),
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("tray watcher: {err}");
        }
    }

    fn spawn(&self, task: impl std::future::Future<Output = ()> + 'static) {
        if let Err(err) = self.scheduler.schedule(task) {
            log::warn!("tray task: {err}");
        }
    }

    pub fn tray_server(&mut self, handle: tray::Handle) {
        let sender = self.events.clone();
        let watcher = handle.clone();
        self.spawn(async move {
            if let Err(err) = watcher
                .watch(move |event| {
                    let _ = sender.send(ServiceEvent::Tray(event));
                })
                .await
            {
                log::error!("tray signals: {err}");
            }
        });
        self.tray.handle = Some(handle);
    }

    fn load_tray_item(&self, key: String) {
        let Some(handle) = self.tray.handle.clone() else {
            return;
        };
        let sender = self.events.clone();
        self.spawn(async move {
            let item = match handle.load(&key).await {
                Ok(item) => Some(item),
                Err(err) => {
                    log::warn!("tray item {key}: {err}");
                    None
                }
            };
            let _ = sender.send(ServiceEvent::TrayItem(key, item));
        });
    }

    pub fn tray_event(&mut self, event: tray::Event) {
        match event {
            tray::Event::Registered(key) => self.load_tray_item(key),
            tray::Event::NameLost(name) => {
                let gone: Vec<String> = self
                    .tray
                    .items
                    .iter()
                    .filter(|item| item.service == name || item.owner == name)
                    .map(|item| item.key.clone())
                    .collect();
                if gone.is_empty() {
                    return;
                }
                for key in &gone {
                    self.tray.icons.remove(key);
                    if self.tray.menu_open.as_deref() == Some(key.as_str()) {
                        self.tray.menu_open = None;
                    }
                    if let Some(handle) = self.tray.handle.clone() {
                        let key = key.clone();
                        self.spawn(async move { handle.unregister(&key).await });
                    }
                }
                self.tray.items.retain(|item| !gone.contains(&item.key));
                if self
                    .tray
                    .target
                    .as_ref()
                    .is_some_and(|target| gone.contains(target))
                {
                    self.tray.target = None;
                    self.tray.menu = None;
                }
                self.sync();
            }
            tray::Event::ItemChanged(owner) => {
                let keys: Vec<String> = self
                    .tray
                    .items
                    .iter()
                    .filter(|item| item.owner == owner || item.service == owner)
                    .map(|item| item.key.clone())
                    .collect();
                for key in keys {
                    self.load_tray_item(key);
                }
            }
            tray::Event::MenuChanged(owner) => {
                let reload = self.tray.menu_open.as_ref().and_then(|key| {
                    let item = self.tray.find(key)?;
                    (item.owner == owner || item.service == owner).then(|| item.clone())
                });
                if let Some(item) = reload {
                    self.fetch_menu(item, false);
                }
            }
        }
    }

    pub fn tray_item_loaded(&mut self, key: String, item: Option<TrayItem>) {
        let Some(item) = item else {
            return;
        };
        match self.tray.items.iter_mut().find(|old| old.key == key) {
            Some(old) => {
                if icon_fields(old) != icon_fields(&item) {
                    self.tray.icons.remove(&key);
                }
                *old = item;
            }
            None => self.tray.items.push(item),
        }
        self.request_tray_icons();
        self.sync();
    }

    fn request_tray_icons(&mut self) {
        for item in &self.tray.items {
            if self.tray.icons.contains_key(&item.key)
                || self.tray.pending_icons.contains(&item.key)
            {
                continue;
            }
            self.tray.pending_icons.push(item.key.clone());
            let sender = self.events.clone();
            let item = item.clone();
            std::thread::spawn(move || {
                let pixmap =
                    icons::tray_source(&item).and_then(|source| icons::load(&source, ICON_SIZE));
                let _ = sender.send(ServiceEvent::TrayIcon(item.key.clone(), pixmap));
            });
        }
    }

    pub fn tray_icon_loaded(&mut self, key: String, pixmap: Option<Pixmap>) {
        self.tray.pending_icons.retain(|pending| *pending != key);
        if self.tray.find(&key).is_none() {
            return;
        }
        self.tray.icons.insert(key, pixmap.map(Arc::new));
        self.request_tray_icons();
        self.sync();
    }

    fn fetch_menu(&self, item: TrayItem, opening: bool) {
        let Some(handle) = self.tray.handle.clone() else {
            return;
        };
        let sender = self.events.clone();
        self.spawn(async move {
            let entries = handle.menu(&item).await;
            let _ = sender.send(ServiceEvent::TrayMenu(item.key.clone(), entries));
            if opening {
                handle.event(&item, 0, "opened").await;
            }
        });
    }

    pub fn tray_menu_loaded(&mut self, key: String, entries: Vec<MenuEntry>) {
        if self.tray.target.as_deref() != Some(key.as_str()) {
            return;
        }
        self.tray.menu = Some(entries);
        self.sync();
    }

    fn close_tray_menu(&mut self) {
        let Some(key) = self.tray.menu_open.take() else {
            return;
        };
        self.tray.menu = None;
        if let (Some(handle), Some(item)) =
            (self.tray.handle.clone(), self.tray.find(&key).cloned())
        {
            self.spawn(async move { handle.event(&item, 0, "closed").await });
        }
    }

    pub fn open_tray(&mut self, key: &str, only_menu: bool, anchor: Option<f64>, screen: &str) {
        if self.tray.target.as_deref() != Some(key) {
            self.close_tray_menu();
        }
        match anchor {
            Some(x) => {
                self.state.panel_screen = screen.to_string();
                self.state.panel_anchor = x;
            }
            None => {
                self.state.panel_screen = self.state.focused_monitor.clone().unwrap_or_default();
                self.state.panel_anchor = 0.0;
            }
        }
        self.tray.only_menu = only_menu;
        self.tray.expanded = false;
        self.tray.target = Some(key.to_string());
        self.tray.serial += 1;
        self.state.panel = "tray".into();
        self.sync();
    }

    pub fn tray_ipc(&mut self, id: &str) {
        let Some(item) = self.tray.items.iter().find(|item| item.id == id).cloned() else {
            return;
        };
        if self.tray.target.as_deref() != Some(item.key.as_str()) {
            self.close_tray_menu();
        }
        self.tray.only_menu = item.item_is_menu;
        self.tray.expanded = false;
        self.tray.target = Some(item.key.clone());
        self.tray.serial += 1;
        self.state.toggle("tray", false);
        self.sync();
    }

    pub fn tray_click(&mut self, key: &str, right: bool, anchor: Option<f64>, screen: &str) {
        let Some(item) = self.tray.find(key).cloned() else {
            return;
        };
        if right || item.item_is_menu {
            self.open_tray(key, !right, anchor, screen);
        } else if let Some(handle) = self.tray.handle.clone() {
            self.spawn(async move { handle.activate(&item).await });
        }
    }

    pub fn tray_scroll(&mut self, key: &str, delta: i32) {
        let (Some(item), Some(handle)) = (self.tray.find(key).cloned(), self.tray.handle.clone())
        else {
            return;
        };
        self.spawn(async move { handle.scroll(&item, delta).await });
    }

    pub fn tray_action(&mut self, action: crate::panels::PanelAction) {
        use crate::panels::PanelAction;
        let target = self.tray.target_item().cloned();
        let handle = self.tray.handle.clone();
        match action {
            PanelAction::TrayExpanded(expanded) => {
                self.tray.expanded = expanded;
                self.sync();
            }
            PanelAction::TrayLoadMenu => {
                if let Some(item) = target {
                    self.tray.menu = None;
                    self.tray.menu_open = Some(item.key.clone());
                    self.fetch_menu(item, true);
                    self.sync();
                }
            }
            PanelAction::TrayActivate => {
                if let (Some(item), Some(handle)) = (target, handle) {
                    self.spawn(async move { handle.activate(&item).await });
                }
            }
            PanelAction::TrayQuit => {
                if let (Some(item), Some(handle)) = (target, handle) {
                    let items = self.tray.items.clone();
                    self.spawn(async move { handle.quit(&items, &item).await });
                }
            }
            PanelAction::TrayEvent(id, name) => {
                if id == 0 && name == "closed" {
                    self.close_tray_menu();
                    self.sync();
                } else if let (Some(item), Some(handle)) = (target, handle) {
                    self.spawn(async move { handle.event(&item, id, name).await });
                }
            }
            _ => {}
        }
    }
}
