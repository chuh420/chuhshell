mod backend;
pub mod service;

use backend::{Action, Network, Security, Snapshot};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

pub struct NetworkMenu {
    service: Rc<service::Service>,
    view: RefCell<Option<View>>,
    snapshot: RefCell<Option<Snapshot>>,
    service_error: RefCell<Option<String>>,
    busy: Cell<bool>,
    updating: Cell<bool>,
    switching: Cell<bool>,
    generation: Cell<u64>,
    detail_network: RefCell<Option<Network>>,
    rows: RefCell<std::collections::HashMap<String, gtk::Button>>,
    #[cfg(test)]
    captured_actions: RefCell<Option<Vec<Action>>>,
}

fn network_id(network: &Network) -> String {
    let source = if network.strength.is_some() {
        "access-point"
    } else {
        network.profile.as_deref().unwrap_or(&network.access_point)
    };
    format!("{:?}:{:?}:{source}", network.ssid, network.security)
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
    pub fn new(service: Rc<service::Service>) -> Rc<Self> {
        let menu = Rc::new(Self {
            service: service.clone(),
            view: RefCell::new(None),
            snapshot: RefCell::new(None),
            service_error: RefCell::new(None),
            busy: Cell::new(false),
            updating: Cell::new(false),
            switching: Cell::new(false),
            generation: Cell::new(0),
            detail_network: RefCell::new(None),
            rows: RefCell::new(Default::default()),
            #[cfg(test)]
            captured_actions: RefCell::new(None),
        });
        let weak = Rc::downgrade(&menu);
        service.subscribe(move |result| {
            let Some(menu) = weak.upgrade() else {
                return false;
            };
            menu.receive(result);
            true
        });
        menu
    }

    fn receive(self: &Rc<Self>, result: &Result<Snapshot, String>) {
        match result {
            Ok(snapshot) => {
                let initial = self.snapshot.borrow().is_none();
                let stale = self.detail_network.borrow().as_ref().is_some_and(|target| {
                    !snapshot.networks.iter().any(|network| {
                        network.ssid == target.ssid
                            && network.security == target.security
                            && network.access_point == target.access_point
                            && network.profile == target.profile
                            && network.active == target.active
                    })
                });
                let changed = self.device() != snapshot.device.as_ref().map(|a| a.path.clone());
                self.switching.set(false);
                *self.snapshot.borrow_mut() = Some(snapshot.clone());
                if changed || stale {
                    self.show_list();
                }
                let recovered = self.service_error.borrow_mut().take().is_some();
                self.update_header();
                self.set_busy(self.busy.get());
                self.render_list();
                if initial || recovered {
                    self.message("Select a network to manage it", false);
                }
            }
            Err(error) => {
                *self.service_error.borrow_mut() = Some(error.clone());
                self.switching.set(false);
                self.show_list();
                self.set_busy(self.busy.get());
                self.message(error, true);
            }
        }
    }

    pub fn toggle(self: &Rc<Self>, anchor: &gtk::Button) {
        let previous = self.view.borrow().as_ref().map(|v| v.popover.clone());
        if let Some(previous) = previous {
            previous.popdown();
            return;
        }
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
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(200);
        stack.set_interpolate_size(true);
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
        crate::ui::attach_to_bar(&popover, anchor);
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
                menu.detail_network.borrow_mut().take();
                menu.view.borrow_mut().take();
                menu.rows.borrow_mut().clear();
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
                menu.switching.set(true);
                menu.show_list();
                menu.set_busy(menu.busy.get());
                menu.service.select(name);
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
        } else if let Some(error) = self.service_error.borrow().as_ref() {
            self.message(error, true);
        } else if self.snapshot.borrow().is_some() {
            self.message("Select a network to manage it", false);
        }
        popover.popup();
        view.search.grab_focus();
        self.refresh();
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
            view.controls.set_sensitive(
                !busy && !self.switching.get() && self.service_error.borrow().is_none(),
            );
            view.spinner.set_visible(busy);
            view.spinner.set_spinning(busy);
        }
    }

    fn refresh(&self) {
        self.service.refresh();
    }

    fn run(self: &Rc<Self>, action: Action, message: &str) {
        if self.busy.get() || self.switching.get() || self.service_error.borrow().is_some() {
            return;
        }
        #[cfg(test)]
        if let Some(actions) = self.captured_actions.borrow_mut().as_mut() {
            actions.push(action);
            return;
        }
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
        let same =
            view.adapter
                .model()
                .and_downcast::<gtk::StringList>()
                .is_some_and(|model| {
                    model.n_items() as usize == names.len()
                        && names.iter().enumerate().all(|(index, name)| {
                            model.string(index as u32).as_deref() == Some(*name)
                        })
                });
        if !same {
            view.adapter.set_model(Some(&gtk::StringList::new(&names)));
        }
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
        let mut widgets = Vec::<gtk::Widget>::new();
        let mut retained = std::collections::HashSet::new();
        let Some(snapshot) = snapshot else {
            crate::ui::reconcile_box(
                &view.list,
                &[label("Reading network status…", "network-empty").upcast()],
            );
            return;
        };
        retained.extend(snapshot.networks.iter().map(network_id));
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
            widgets.push(label(text, "network-empty").upcast());
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
                widgets.push(label(heading, "network-section").upcast());
                section = heading;
            }
            let key = network_id(network);
            retained.insert(key.clone());
            let existing = self.rows.borrow().get(&key).cloned();
            let row = if let Some(row) = existing {
                row
            } else {
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
                let signal = label(
                    &network
                        .strength
                        .map_or_else(|| "—".into(), |s| format!("{s}%")),
                    "network-signal",
                );
                signal.set_wrap(false);
                content.append(&signal);
                row.set_child(Some(&content));
                row.set_tooltip_text(Some(&network.name));
                let weak = Rc::downgrade(self);
                let id = key.clone();
                row.connect_clicked(move |_| {
                    if let Some(menu) = weak.upgrade() {
                        let network = menu.snapshot.borrow().as_ref().and_then(|snapshot| {
                            snapshot
                                .networks
                                .iter()
                                .find(|network| network_id(network) == id)
                                .cloned()
                        });
                        if let Some(network) = network {
                            menu.network_form(network);
                        }
                    }
                });
                self.rows.borrow_mut().insert(key, row.clone());
                row
            };
            let content = row.child().unwrap().downcast::<gtk::Box>().unwrap();
            let icon = content
                .first_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap();
            icon.set_text(if network.active {
                "󰤨"
            } else {
                match network.strength.unwrap_or(0) {
                    75.. => "󰤨",
                    50.. => "󰤥",
                    25.. => "󰤢",
                    _ => "󰤟",
                }
            });
            let text = icon.next_sibling().unwrap().downcast::<gtk::Box>().unwrap();
            text.first_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .set_text(&network.name);
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
            text.last_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .set_text(&detail);
            content
                .last_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .set_text(
                    &network
                        .strength
                        .map_or_else(|| "—".into(), |strength| format!("{strength}%")),
                );
            row.set_tooltip_text(Some(&network.name));
            if network.active {
                row.add_css_class("connected");
            } else {
                row.remove_css_class("connected");
            }
            widgets.push(row.upcast());
            count += 1;
        }
        if count == 0 && empty.is_none() {
            widgets.push(
                label(
                    if query.is_empty() {
                        "No networks found. Try scanning again."
                    } else {
                        "No matching networks"
                    },
                    "network-empty",
                )
                .upcast(),
            );
        }
        self.rows
            .borrow_mut()
            .retain(|key, _| retained.contains(key));
        crate::ui::reconcile_box(&view.list, &widgets);
    }

    fn show_list(self: &Rc<Self>) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.detail_network.borrow_mut().take();
        if let Some(view) = self.view.borrow().as_ref() {
            view.stack.set_visible_child_name("networks");
            while let Some(child) = view.detail.first_child() {
                view.detail.remove(&child);
            }
        }
        self.render_list();
    }

    fn form(self: &Rc<Self>, title: &str) -> Option<View> {
        if self.busy.get() || self.switching.get() || self.service_error.borrow().is_some() {
            return None;
        }
        self.detail_network.borrow_mut().take();
        let view = self.view.borrow().clone()?;
        self.generation.set(self.generation.get().wrapping_add(1));
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

    fn form_device(&self, device: &str, generation: u64) -> Option<String> {
        if self.generation.get() != generation
            || self.switching.get()
            || self.service_error.borrow().is_some()
        {
            return None;
        }
        self.device().filter(|current| current == device)
    }

    fn network_form(self: &Rc<Self>, network: Network) {
        let Some(view) = self.form(&network.name) else {
            return;
        };
        let Some(device_path) = self.device() else {
            return;
        };
        let generation = self.generation.get();
        *self.detail_network.borrow_mut() = Some(network.clone());
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
            let device_path = device_path.clone();
            disconnect.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade()
                    && let Some(device) = menu.form_device(&device_path, generation)
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
            let device_path = device_path.clone();
            connect.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade()
                    && let Some(device) = menu.form_device(&device_path, generation)
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
                if let Some(menu) = weak.upgrade()
                    && menu.form_device(&device_path, generation).is_some()
                {
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
        let Some(device_path) = self.device() else {
            return;
        };
        let generation = self.generation.get();
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
                && let Some(device) = menu.form_device(&device_path, generation)
            {
                let name = ssid.text().to_string();
                if name.is_empty() || name.len() > 32 {
                    menu.message("Network names must contain 1–32 bytes", true);
                    return;
                }
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

#[cfg(test)]
pub fn regression_checks(anchor: &gtk::Button) {
    backend::regression_checks();
    let menu = NetworkMenu::new(service::Service::new());
    *menu.captured_actions.borrow_mut() = Some(Vec::new());
    menu.toggle(anchor);
    let adapters = ["A", "B"].map(|name| backend::Adapter {
        path: format!("/adapter/{name}"),
        name: name.into(),
    });
    let network = Network {
        ssid: b"Example".to_vec(),
        name: "Example".into(),
        access_point: "/ap/A".into(),
        profile: None,
        security: Security::Open,
        strength: Some(80),
        frequency: 2400,
        active: true,
        hidden: false,
    };
    let mut snapshot = Snapshot {
        enabled: true,
        hardware_enabled: true,
        adapters: adapters.to_vec(),
        device: Some(adapters[0].clone()),
        networks: vec![network.clone()],
        address: String::new(),
    };
    menu.receive(&Ok(snapshot.clone()));
    let key = network_id(&network);
    let stable = menu.rows.borrow()[&key].clone();
    let model = menu
        .view
        .borrow()
        .as_ref()
        .unwrap()
        .adapter
        .model()
        .unwrap();
    for strength in 0..100 {
        snapshot.networks[0].strength = Some(strength);
        menu.receive(&Ok(snapshot.clone()));
        assert_eq!(menu.rows.borrow()[&key], stable);
        assert_eq!(menu.rows.borrow().len(), 1);
        assert_eq!(
            menu.view
                .borrow()
                .as_ref()
                .unwrap()
                .adapter
                .model()
                .unwrap(),
            model
        );
    }
    let search = menu.view.borrow().as_ref().unwrap().search.clone();
    search.set_text("missing");
    search.emit_by_name::<()>("search-changed", &[]);
    assert_eq!(menu.rows.borrow()[&key], stable);
    assert!(stable.parent().is_none());
    search.set_text("");
    search.emit_by_name::<()>("search-changed", &[]);
    assert_eq!(menu.rows.borrow()[&key], stable);
    assert!(stable.parent().is_some());
    for active in [true, false] {
        snapshot.device = Some(adapters[0].clone());
        snapshot.networks[0].active = active;
        menu.receive(&Ok(snapshot.clone()));
        menu.network_form(snapshot.networks[0].clone());
        let view = menu.view.borrow().clone().unwrap();
        let old_button = view
            .detail
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        old_button.emit_clicked();
        let actions = menu
            .captured_actions
            .borrow_mut()
            .as_mut()
            .unwrap()
            .pop()
            .unwrap();
        match actions {
            Action::Disconnect(device) | Action::Connect { device, .. } => {
                assert_eq!(device, "/adapter/A")
            }
            _ => panic!("Unexpected network operation"),
        }
        view.adapter.set_selected(1);
        assert_eq!(view.stack.visible_child_name().as_deref(), Some("networks"));
        assert!(!view.controls.is_sensitive());
        old_button.emit_clicked();
        assert!(menu.captured_actions.borrow().as_ref().unwrap().is_empty());
        snapshot.device = Some(adapters[1].clone());
        menu.receive(&Ok(snapshot.clone()));
        old_button.emit_clicked();
        assert!(menu.captured_actions.borrow().as_ref().unwrap().is_empty());
        assert!(view.controls.is_sensitive());
    }
    let mut duplicates = snapshot.clone();
    duplicates.networks[0].strength = None;
    duplicates.networks[0].profile = Some("/profile/one".into());
    let mut second = duplicates.networks[0].clone();
    second.profile = Some("/profile/two".into());
    duplicates.networks.push(second);
    menu.receive(&Ok(duplicates));
    assert_eq!(menu.rows.borrow().len(), 2);
    let rows = menu.rows.borrow().values().cloned().collect::<Vec<_>>();
    assert_ne!(rows[0], rows[1]);
    menu.receive(&Ok(snapshot.clone()));
    menu.network_form(snapshot.networks[0].clone());
    let view = menu.view.borrow().clone().unwrap();
    let old_button = view
        .detail
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    menu.receive(&Err("Unavailable".into()));
    old_button.emit_clicked();
    assert!(menu.captured_actions.borrow().as_ref().unwrap().is_empty());
    assert!(!view.controls.is_sensitive());
    menu.receive(&Ok(snapshot));
    assert!(view.controls.is_sensitive());
    view.popover.popdown();
}
