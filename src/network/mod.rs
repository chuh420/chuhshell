mod backend;

use backend::{Action, Network, Security, Snapshot};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone)]
struct View {
    popover: gtk::Popover,
    controls: gtk::Box,
    radio: gtk::Button,
    scan: gtk::Button,
    adapter: gtk::DropDown,
    search: gtk::SearchEntry,
    list: gtk::Box,
    stack: gtk::Stack,
    detail: gtk::Box,
    status: gtk::Label,
    spinner: gtk::Spinner,
}

#[derive(Default)]
pub struct NetworkMenu {
    view: RefCell<Option<View>>,
    snapshot: RefCell<Option<Snapshot>>,
    preferred: RefCell<Option<String>>,
    busy: Cell<bool>,
    loading: Cell<bool>,
    updating: Cell<bool>,
    generation: Cell<u64>,
    revision: Cell<u64>,
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(40);
    label
}

fn button(text: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.add_css_class("network-action");
    button
}

impl NetworkMenu {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            preferred: RefCell::new(crate::config::get().wifi.clone()),
            ..Self::default()
        })
    }

    pub fn toggle(self: &Rc<Self>, anchor: &gtk::Button) {
        let previous = self.view.borrow().as_ref().map(|v| v.popover.clone());
        if let Some(previous) = previous {
            previous.popdown();
            return;
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("network-content");
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("Wi-Fi", "network-heading");
        title.set_hexpand(true);
        header.append(&title);
        let scan = button("󰑐");
        scan.set_tooltip_text(Some("Scan for networks"));
        let radio = button("Wi-Fi");
        header.append(&scan);
        header.append(&radio);
        controls.append(&header);
        let adapter = gtk::DropDown::from_strings(&[]);
        adapter.add_css_class("network-adapter");
        adapter.set_tooltip_text(Some("Wi-Fi adapter"));
        controls.append(&adapter);
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Find a network…"));
        search.add_css_class("network-search");
        page.append(&search);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_min_content_width(340);
        scroll.set_max_content_height(
            crate::ui::widget_monitor(anchor)
                .map_or(360, |m| (m.geometry().height() - 260).clamp(120, 360)),
        );
        scroll.set_child(Some(&list));
        page.append(&scroll);
        let hidden = button("Join hidden network…");
        page.append(&hidden);
        stack.add_named(&page, Some("networks"));
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 10);
        stack.add_named(&detail, Some("detail"));
        controls.append(&stack);
        root.append(&controls);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let status = label("Loading networks…", "network-status");
        status.set_hexpand(true);
        footer.append(&spinner);
        footer.append(&status);
        root.append(&footer);
        let popover = crate::ui::popover(anchor, &root);
        let view = View {
            popover: popover.clone(),
            controls,
            radio,
            scan,
            adapter,
            search,
            list,
            stack,
            detail,
            status,
            spinner,
        };
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.generation.set(menu.generation.get().wrapping_add(1));
                menu.view.borrow_mut().take();
            }
        });
        let weak = Rc::downgrade(self);
        view.search.connect_search_changed(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.render_list();
            }
        });
        let weak = Rc::downgrade(self);
        view.radio.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                let enabled = menu.snapshot.borrow().as_ref().map(|s| s.enabled);
                if let Some(enabled) = enabled {
                    menu.run(Action::Radio(!enabled), "Updating Wi-Fi…");
                }
            }
        });
        let weak = Rc::downgrade(self);
        view.scan.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                if let Some(device) = menu.device() {
                    menu.run(Action::Scan(device), "Scanning…");
                } else {
                    menu.refresh();
                }
            }
        });
        let weak = Rc::downgrade(self);
        view.adapter.connect_selected_notify(move |dropdown| {
            if let Some(menu) = weak.upgrade() {
                if menu.updating.get() {
                    return;
                }
                let name = menu
                    .snapshot
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.adapters.get(dropdown.selected() as usize))
                    .map(|a| a.name.clone());
                *menu.preferred.borrow_mut() = name;
                menu.revision.set(menu.revision.get().wrapping_add(1));
                menu.refresh();
            }
        });
        let weak = Rc::downgrade(self);
        hidden.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.hidden_form();
            }
        });
        *self.view.borrow_mut() = Some(view.clone());
        self.update_header();
        self.render_list();
        self.set_busy(self.busy.get());
        if self.busy.get() {
            self.message("Network operation in progress…", false);
        } else if self.snapshot.borrow().is_some() {
            self.message("Select a network to manage it", false);
        }
        popover.popup();
        view.search.grab_focus();
        self.refresh();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(4), move || {
            let Some(menu) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if menu.view.borrow().is_none() || menu.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            menu.refresh();
            glib::ControlFlow::Continue
        });
    }

    fn device(&self) -> Option<String> {
        self.snapshot
            .borrow()
            .as_ref()
            .and_then(|s| s.device.as_ref())
            .map(|a| a.path.clone())
    }

    fn message(&self, text: &str, error: bool) {
        if let Some(view) = self.view.borrow().as_ref() {
            view.status.set_text(text);
            if error {
                view.status.add_css_class("error");
            } else {
                view.status.remove_css_class("error");
            }
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        if let Some(view) = self.view.borrow().as_ref() {
            view.controls.set_sensitive(!busy);
            view.spinner.set_visible(busy);
            view.spinner.set_spinning(busy);
        }
    }

    fn refresh(self: &Rc<Self>) {
        if self.busy.get() || self.loading.replace(true) {
            return;
        }
        let preferred = self.preferred.borrow().clone();
        let revision = self.revision.get();
        let generation = self.generation.get();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(backend::snapshot(preferred.as_deref()));
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = rx.recv().await;
            let Some(menu) = weak.upgrade() else {
                return;
            };
            menu.loading.set(false);
            if revision != menu.revision.get() || generation != menu.generation.get() {
                if menu.view.borrow().is_some() {
                    menu.refresh();
                }
                return;
            }
            match result {
                Ok(Ok(snapshot)) => {
                    let initial = menu.snapshot.borrow().is_none();
                    let changed = menu.snapshot.borrow().as_ref() != Some(&snapshot);
                    *menu.snapshot.borrow_mut() = Some(snapshot);
                    if changed {
                        menu.update_header();
                        menu.render_list();
                    }
                    if initial {
                        menu.message("Select a network to manage it", false);
                    }
                }
                Ok(Err(error)) => menu.message(&error, true),
                Err(_) => menu.message("Network service unavailable", true),
            }
        });
    }

    fn run(self: &Rc<Self>, action: Action, message: &str) {
        if self.busy.get() {
            return;
        }
        self.revision.set(self.revision.get().wrapping_add(1));
        self.set_busy(true);
        self.message(message, false);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(backend::perform(action));
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = rx.recv().await;
            let Some(menu) = weak.upgrade() else {
                return;
            };
            menu.set_busy(false);
            match result {
                Ok(Ok(message)) => {
                    menu.message(&message, false);
                    menu.show_list();
                }
                Ok(Err(error)) => menu.message(&error, true),
                Err(_) => menu.message("Network operation interrupted", true),
            }
            menu.refresh();
        });
    }

    fn update_header(&self) {
        let view = self.view.borrow().clone();
        let snapshot = self.snapshot.borrow().clone();
        let (Some(view), Some(snapshot)) = (view, snapshot) else {
            return;
        };
        self.updating.set(true);
        view.radio
            .set_label(if snapshot.enabled { "On" } else { "Off" });
        view.radio
            .set_tooltip_text(Some("Enable or disable Wi-Fi on all adapters"));
        view.radio
            .set_sensitive(snapshot.hardware_enabled && !snapshot.adapters.is_empty());
        if snapshot.enabled {
            view.radio.add_css_class("enabled");
        } else {
            view.radio.remove_css_class("enabled");
        }
        view.scan.set_sensitive(
            snapshot.enabled && snapshot.hardware_enabled && snapshot.device.is_some(),
        );
        let names: Vec<_> = snapshot.adapters.iter().map(|a| a.name.as_str()).collect();
        view.adapter.set_model(Some(&gtk::StringList::new(&names)));
        view.adapter.set_visible(names.len() > 1);
        if let Some(index) = snapshot
            .adapters
            .iter()
            .position(|a| Some(a) == snapshot.device.as_ref())
        {
            view.adapter.set_selected(index as u32);
        }
        self.updating.set(false);
    }

    fn render_list(self: &Rc<Self>) {
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        let snapshot = self.snapshot.borrow().clone();
        while let Some(child) = view.list.first_child() {
            view.list.remove(&child);
        }
        let Some(snapshot) = snapshot else {
            view.list
                .append(&label("Reading network status…", "network-empty"));
            return;
        };
        let empty = if snapshot.adapters.is_empty() {
            Some("No Wi-Fi adapter found")
        } else if !snapshot.hardware_enabled {
            Some("Wi-Fi is blocked by the hardware switch")
        } else if !snapshot.enabled {
            Some("Wi-Fi is off")
        } else {
            None
        };
        if let Some(text) = empty {
            view.list.append(&label(text, "network-empty"));
        }
        let query = view.search.text().to_lowercase();
        let mut count = 0;
        let mut section = "";
        for network in snapshot
            .networks
            .iter()
            .filter(|n| n.name.to_lowercase().contains(&query))
        {
            let heading = if network.active {
                "CONNECTED"
            } else if network.strength.is_some() {
                "AVAILABLE NETWORKS"
            } else {
                "SAVED NETWORKS"
            };
            if heading != section {
                view.list.append(&label(heading, "network-section"));
                section = heading;
            }
            let row = gtk::Button::new();
            row.add_css_class("network-row");
            if network.active {
                row.add_css_class("connected");
            }
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let icon = label(
                if network.active {
                    "󰤨"
                } else {
                    match network.strength.unwrap_or(0) {
                        75.. => "󰤨",
                        50.. => "󰤥",
                        25.. => "󰤢",
                        _ => "󰤟",
                    }
                },
                "network-icon",
            );
            content.append(&icon);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
            text.set_hexpand(true);
            let name = gtk::Label::new(Some(&network.name));
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_max_width_chars(25);
            name.add_css_class("network-name");
            text.append(&name);
            let detail = if network.active {
                if snapshot.address.is_empty() {
                    "Connected".into()
                } else {
                    snapshot.address.clone()
                }
            } else if network.profile.is_some() {
                format!("Saved · {}", network.security.label())
            } else {
                network.security.label().into()
            };
            text.append(&label(&detail, "network-meta"));
            content.append(&text);
            content.append(&label(
                &network
                    .strength
                    .map_or_else(|| "—".into(), |s| format!("{s}%")),
                "network-signal",
            ));
            row.set_child(Some(&content));
            row.set_tooltip_text(Some(&network.name));
            let network = network.clone();
            let weak = Rc::downgrade(self);
            row.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade() {
                    menu.network_form(network.clone());
                }
            });
            view.list.append(&row);
            count += 1;
        }
        if count == 0 && empty.is_none() {
            view.list.append(&label(
                if query.is_empty() {
                    "No networks found. Try scanning again."
                } else {
                    "No matching networks"
                },
                "network-empty",
            ));
        }
    }

    fn show_list(self: &Rc<Self>) {
        if let Some(view) = self.view.borrow().as_ref() {
            view.stack.set_visible_child_name("networks");
            while let Some(child) = view.detail.first_child() {
                view.detail.remove(&child);
            }
        }
        self.render_list();
    }

    fn form(self: &Rc<Self>, title: &str) -> Option<View> {
        let view = self.view.borrow().clone()?;
        while let Some(child) = view.detail.first_child() {
            view.detail.remove(&child);
        }
        let back = button("‹  Networks");
        back.set_halign(gtk::Align::Start);
        let weak = Rc::downgrade(self);
        back.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.show_list();
                menu.message("Select a network to manage it", false);
            }
        });
        view.detail.append(&back);
        view.detail.append(&label(title, "network-heading"));
        view.stack.set_visible_child_name("detail");
        Some(view)
    }

    fn network_form(self: &Rc<Self>, network: Network) {
        let Some(view) = self.form(&network.name) else {
            return;
        };
        let mut details = network.security.label().to_string();
        if let Some(strength) = network.strength {
            details.push_str(&format!(" · {strength}%"));
            if network.frequency != 0 {
                details.push_str(&format!(" · {:.1} GHz", network.frequency as f64 / 1000.0));
            }
        }
        view.detail.append(&label(&details, "network-meta"));
        let online = self
            .snapshot
            .borrow()
            .as_ref()
            .is_some_and(|s| s.enabled && s.hardware_enabled);
        if network.active {
            let disconnect = button("Disconnect");
            let weak = Rc::downgrade(self);
            disconnect.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade()
                    && let Some(device) = menu.device()
                {
                    menu.run(Action::Disconnect(device), "Disconnecting…");
                }
            });
            view.detail.append(&disconnect);
        } else if network.security.supported() || network.profile.is_some() {
            let password = gtk::PasswordEntry::new();
            password.set_show_peek_icon(true);
            password.add_css_class("network-password");
            password.set_placeholder_text(Some(if network.profile.is_some() {
                "Saved password · type to replace"
            } else {
                "Password"
            }));
            if network.security.password() {
                view.detail.append(&password);
            }
            let connect = button("Connect");
            connect.add_css_class("primary");
            connect.set_sensitive(online);
            let weak = Rc::downgrade(self);
            let target = network.clone();
            let input = password.clone();
            connect.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade()
                    && let Some(device) = menu.device()
                {
                    let password = input.text().to_string();
                    menu.run(
                        Action::Connect {
                            device,
                            network: target.clone(),
                            password,
                        },
                        "Connecting…",
                    );
                    input.set_text("");
                }
            });
            let weak_button = connect.downgrade();
            password.connect_activate(move |_| {
                if let Some(button) = weak_button.upgrade()
                    && button.is_sensitive()
                {
                    button.emit_clicked();
                }
            });
            view.detail.append(&connect);
            if network.security.password() {
                password.grab_focus();
            }
            if !online {
                view.detail
                    .append(&label("Turn on Wi-Fi to connect", "network-meta"));
            }
        } else {
            view.detail.append(&label("This network requires a configured Enterprise or legacy profile. Existing profiles can be connected here.", "network-meta"));
        }
        if let Some(profile) = network.profile {
            let forget = button("Forget network…");
            forget.add_css_class("danger");
            let weak = Rc::downgrade(self);
            let confirmed = Cell::new(false);
            forget.connect_clicked(move |button| {
                if let Some(menu) = weak.upgrade() {
                    if confirmed.replace(true) {
                        menu.run(Action::Forget(profile.clone()), "Removing saved network…");
                    } else {
                        button.set_label("Confirm forget");
                        menu.message("Remove this saved profile and its password?", false);
                    }
                }
            });
            view.detail.append(&forget);
        }
    }

    fn hidden_form(self: &Rc<Self>) {
        let Some(view) = self.form("Hidden network") else {
            return;
        };
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("Network name (SSID)"));
        name.set_max_length(32);
        name.add_css_class("network-search");
        view.detail.append(&name);
        let security =
            gtk::DropDown::from_strings(&["WPA / WPA2 Personal", "WPA3 Personal", "Open network"]);
        view.detail.append(&security);
        let password = gtk::PasswordEntry::new();
        password.set_placeholder_text(Some("Password"));
        password.set_show_peek_icon(true);
        password.add_css_class("network-password");
        view.detail.append(&password);
        let input = password.downgrade();
        security.connect_selected_notify(move |dropdown| {
            if let Some(input) = input.upgrade() {
                input.set_visible(dropdown.selected() != 2);
            }
        });
        let connect = button("Join network");
        connect.add_css_class("primary");
        connect.set_sensitive(
            self.snapshot
                .borrow()
                .as_ref()
                .is_some_and(|s| s.enabled && s.hardware_enabled && s.device.is_some()),
        );
        let weak = Rc::downgrade(self);
        let ssid = name.clone();
        let input = password.clone();
        connect.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade()
                && let Some(device) = menu.device()
            {
                let name = ssid.text().to_string();
                let security = match security.selected() {
                    1 => Security::Sae,
                    2 => Security::Open,
                    _ => Security::Personal,
                };
                let network = Network {
                    ssid: name.as_bytes().to_vec(),
                    name,
                    security,
                    access_point: "/".into(),
                    profile: None,
                    strength: None,
                    frequency: 0,
                    active: false,
                    hidden: true,
                };
                menu.run(
                    Action::Connect {
                        device,
                        network,
                        password: input.text().to_string(),
                    },
                    "Connecting…",
                );
                input.set_text("");
            }
        });
        let weak_button = connect.downgrade();
        password.connect_activate(move |_| {
            if let Some(button) = weak_button.upgrade()
                && button.is_sensitive()
            {
                button.emit_clicked();
            }
        });
        view.detail.append(&connect);
        name.grab_focus();
    }
}
