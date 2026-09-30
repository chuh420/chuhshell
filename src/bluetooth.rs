use glib::variant::{FromVariant, ObjectPath, ToVariant};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const AGENT: &str = "/dev/chuh/chuhshell/agent";
const XML: &str = r#"<node><interface name="org.bluez.Agent1">
<method name="Release"/><method name="Cancel"/>
<method name="RequestPinCode"><arg type="o" direction="in"/><arg type="s" direction="out"/></method>
<method name="RequestPasskey"><arg type="o" direction="in"/><arg type="u" direction="out"/></method>
<method name="DisplayPinCode"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
<method name="DisplayPasskey"><arg type="o" direction="in"/><arg type="u" direction="in"/><arg type="q" direction="in"/></method>
<method name="RequestConfirmation"><arg type="o" direction="in"/><arg type="u" direction="in"/></method>
<method name="RequestAuthorization"><arg type="o" direction="in"/></method>
<method name="AuthorizeService"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
</interface></node>"#;
type Properties = HashMap<String, glib::Variant>;
type Objects = HashMap<ObjectPath, HashMap<String, Properties>>;
type CachedRows = HashMap<String, (HashMap<String, Properties>, gtk::Box)>;

fn value<T: FromVariant + Default>(properties: &Properties, name: &str) -> T {
    properties
        .get(name)
        .and_then(T::from_variant)
        .unwrap_or_default()
}

async fn call(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    args: Option<glib::Variant>,
) -> Result<glib::Variant, String> {
    call_owner(bus, "org.bluez", path, interface, method, args).await
}

async fn call_owner(
    bus: &gio::DBusConnection,
    owner: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: Option<glib::Variant>,
) -> Result<glib::Variant, String> {
    bus.call_future(
        Some(owner),
        path,
        interface,
        method,
        args.as_ref(),
        None,
        gio::DBusCallFlags::NONE,
        if method == "Pair" { 90_000 } else { 12_000 },
    )
    .await
    .map_err(|e| e.to_string())
}

struct Panel {
    bus: RefCell<Option<gio::DBusConnection>>,
    registration: RefCell<Option<gio::RegistrationId>>,
    subscription: RefCell<Option<gio::SignalSubscription>>,
    owner: RefCell<String>,
    bus_type: gio::BusType,
    initializing: Cell<bool>,
    agent_registered: Cell<bool>,
    generation: Cell<u64>,
    objects: RefCell<Objects>,
    rows: RefCell<CachedRows>,
    content: glib::WeakRef<gtk::Box>,
    prompt: glib::WeakRef<gtk::Box>,
    status: glib::WeakRef<gtk::Label>,
    pending: RefCell<Option<gio::DBusMethodInvocation>>,
    prompt_generation: Cell<u64>,
    pairing: RefCell<Option<String>>,
    busy: Cell<bool>,
    refreshing: Cell<bool>,
    alive: Cell<bool>,
    scanning: RefCell<HashMap<String, glib::SourceId>>,
    scan_timeout: std::time::Duration,
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.reject();
        self.clear_scanning();
        if let Some(bus) = self.bus.borrow_mut().take() {
            if let Some(id) = self.registration.borrow_mut().take() {
                let _ = bus.unregister_object(id);
            }
            glib::MainContext::default().spawn_local(async move {
                let _ = bus.close_future().await;
            });
        }
    }
}

impl Panel {
    fn status(&self, text: &str) {
        if let Some(label) = self.status.upgrade() {
            label.set_text(text);
        }
    }
    fn reject(&self) {
        self.prompt_generation
            .set(self.prompt_generation.get().wrapping_add(1));
        let pending = self.pending.borrow_mut().take();
        if let Some(pending) = pending {
            pending.return_dbus_error("org.bluez.Error.Rejected", "Pairing was cancelled");
        }
        if let Some(prompt) = self.prompt.upgrade() {
            while let Some(child) = prompt.first_child() {
                prompt.remove(&child);
            }
        }
    }

    fn agent(
        self: &Rc<Self>,
        method: &str,
        args: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        if !self.alive.get() {
            invocation.return_dbus_error("org.bluez.Error.Rejected", "Bluetooth menu is closed");
            return;
        }
        self.reject();
        if matches!(method, "Cancel" | "Release") {
            invocation.return_value(Some(&().to_variant()));
            return;
        }
        let Some(prompt) = self.prompt.upgrade() else {
            invocation.return_dbus_error("org.bluez.Error.Rejected", "No pairing UI");
            return;
        };
        let path = args.child_get::<ObjectPath>(0);
        let name = self
            .objects
            .borrow()
            .get(&path)
            .and_then(|interfaces| interfaces.get(DEVICE))
            .map(|props| value::<String>(props, "Alias"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.to_string());
        let description = match method {
            "RequestConfirmation" => format!(
                "Pair with {name}?\nConfirm code {:06} matches the device.",
                args.child_get::<u32>(1)
            ),
            "DisplayPasskey" => format!(
                "Type {:06} on {name}, then press Enter.\n{} digits entered",
                args.child_get::<u32>(1),
                args.child_get::<u16>(2)
            ),
            "DisplayPinCode" => format!(
                "Type {} on {name}, then press Enter.",
                args.child_get::<String>(1)
            ),
            "RequestPinCode" => format!("Enter the PIN for {name}"),
            "RequestPasskey" => format!("Enter the six-digit passkey for {name}"),
            "RequestAuthorization" => format!("Allow pairing with {name}?"),
            "AuthorizeService" => format!(
                "Allow {name} to use service {}?",
                args.child_get::<String>(1)
            ),
            _ => {
                invocation
                    .return_dbus_error("org.bluez.Error.Rejected", "Unsupported pairing request");
                return;
            }
        };
        prompt.append(&crate::info::label(&description, "menu-title"));
        if matches!(method, "DisplayPasskey" | "DisplayPinCode") {
            invocation.return_value(Some(&().to_variant()));
            return;
        }
        let entry = gtk::Entry::new();
        entry.add_css_class("search");
        entry.set_max_length(if method == "RequestPasskey" { 6 } else { 16 });
        let input = matches!(method, "RequestPinCode" | "RequestPasskey");
        if input {
            prompt.append(&entry);
        }
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let accept = crate::info::button("Confirm");
        let cancel = crate::info::button("Cancel");
        controls.append(&accept);
        controls.append(&cancel);
        prompt.append(&controls);
        *self.pending.borrow_mut() = Some(invocation);
        let method = method.to_string();
        accept.connect_clicked({
            let panel = Rc::downgrade(self);
            let entry = entry.clone();
            move |_| {
                let Some(panel) = panel.upgrade() else {
                    return;
                };
                let reply = match method.as_str() {
                    "RequestPasskey" => match entry.text().parse::<u32>() {
                        Ok(key) if key <= 999999 => (key,).to_variant(),
                        _ => {
                            panel.status("Enter a number from 000000 to 999999");
                            return;
                        }
                    },
                    "RequestPinCode" => {
                        let pin = entry.text();
                        if pin.is_empty() || pin.len() > 16 {
                            panel.status("PIN must contain 1–16 bytes");
                            return;
                        }
                        (pin.as_str(),).to_variant()
                    }
                    _ => ().to_variant(),
                };
                let pending = panel.pending.borrow_mut().take();
                if let Some(pending) = pending {
                    pending.return_value(Some(&reply));
                }
                panel.reject();
            }
        });
        cancel.connect_clicked({
            let panel = Rc::downgrade(self);
            move |_| {
                if let Some(panel) = panel.upgrade() {
                    panel.reject();
                }
            }
        });
        let generation = self.prompt_generation.get();
        let panel = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_secs(85), move || {
            if let Some(panel) = panel.upgrade()
                && panel.prompt_generation.get() == generation
            {
                panel.reject();
            }
        });
        if input {
            entry.grab_focus();
        } else {
            accept.grab_focus();
        }
    }

    fn initialize(self: &Rc<Self>) {
        if !self.alive.get() || self.initializing.replace(true) {
            return;
        }
        let self_bus_type = self.bus_type;
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                let address =
                    gio::dbus_address_get_for_bus_sync(self_bus_type, gio::Cancellable::NONE)
                        .map_err(|e| e.to_string())?;
                let bus = gio::DBusConnection::for_address_future(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
                bus.set_exit_on_close(false);
                let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
                    let _ = bus.close_future().await;
                    return Ok(());
                };
                let info = gio::DBusNodeInfo::for_xml(XML)
                    .unwrap()
                    .lookup_interface("org.bluez.Agent1")
                    .unwrap();
                let agent = Rc::downgrade(&panel);
                let registration = bus
                    .register_object(AGENT, &info)
                    .method_call(move |_, sender, _, _, method, args, invocation| {
                        if let Some(panel) = agent
                            .upgrade()
                            .filter(|p| sender == Some(p.owner.borrow().as_str()))
                        {
                            panel.agent(method, &args, invocation);
                        } else {
                            invocation.return_dbus_error(
                                "org.bluez.Error.Rejected",
                                "Unrecognized Bluetooth service",
                            );
                        }
                    })
                    .build()
                    .map_err(|e| e.to_string())?;
                *panel.registration.borrow_mut() = Some(registration);
                *panel.bus.borrow_mut() = Some(bus.clone());
                let observer = Rc::downgrade(&panel);
                *panel.subscription.borrow_mut() = Some(bus.subscribe_to_signal(
                    Some("org.freedesktop.DBus"),
                    Some("org.freedesktop.DBus"),
                    Some("NameOwnerChanged"),
                    Some("/org/freedesktop/DBus"),
                    Some("org.bluez"),
                    gio::DBusSignalFlags::NONE,
                    move |signal| {
                        if let Some(panel) = observer.upgrade().filter(|p| p.alive.get()) {
                            panel.set_owner(signal.parameters.child_get::<String>(2));
                            panel.render();
                            panel.refresh();
                        }
                    },
                ));
                panel.refresh();
                Ok::<_, String>(())
            }
            .await;
            if let Some(panel) = weak.upgrade() {
                panel.initializing.set(false);
                if let Err(error) = result {
                    panel.status(&error);
                }
            }
        });
    }

    fn clear_scanning(&self) {
        for (_, timer) in self.scanning.borrow_mut().drain() {
            timer.remove();
        }
    }

    fn set_owner(&self, owner: String) {
        if *self.owner.borrow() == owner {
            return;
        }
        *self.owner.borrow_mut() = owner;
        self.generation.set(self.generation.get().wrapping_add(1));
        self.agent_registered.set(false);
        self.reject();
        self.pairing.borrow_mut().take();
        self.clear_scanning();
        self.objects.borrow_mut().clear();
        self.rows.borrow_mut().clear();
        self.busy.set(false);
        if let Some(content) = self.content.upgrade() {
            content.set_sensitive(true);
        }
    }

    fn refresh(self: &Rc<Self>) {
        if !self.alive.get() || self.refreshing.replace(true) {
            return;
        }
        if self
            .bus
            .borrow()
            .as_ref()
            .is_some_and(|bus| bus.is_closed())
        {
            self.bus.borrow_mut().take();
            self.registration.borrow_mut().take();
            self.subscription.borrow_mut().take();
            self.set_owner(String::new());
        }
        let Some(bus) = self.bus.borrow().clone() else {
            self.refreshing.set(false);
            self.initialize();
            return;
        };
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                let owner = bus
                    .call_future(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "GetNameOwner",
                        Some(&("org.bluez",).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        5000,
                    )
                    .await
                    .map_err(|_| {
                        "BlueZ is not running. Install bluez and enable bluetooth.service."
                            .to_string()
                    })?
                    .child_get::<String>(0);
                let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
                    return Err("Bluetooth menu is closed".into());
                };
                if panel.generation.get() != generation {
                    return Err("Bluetooth service changed; retrying".into());
                }
                panel.set_owner(owner.clone());
                if !panel.agent_registered.get() {
                    bus.call_future(
                        Some(&owner),
                        "/org/bluez",
                        "org.bluez.AgentManager1",
                        "RegisterAgent",
                        Some(
                            &(ObjectPath::try_from(AGENT).unwrap(), "KeyboardDisplay").to_variant(),
                        ),
                        None,
                        gio::DBusCallFlags::NONE,
                        12000,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    if *panel.owner.borrow() != owner {
                        return Err("Bluetooth service changed; retrying".into());
                    }
                    panel.agent_registered.set(true);
                    panel.status("Ready · Start scanning to find devices");
                }
                let generation = panel.generation.get();
                let result = call_owner(
                    &bus,
                    &owner,
                    "/",
                    "org.freedesktop.DBus.ObjectManager",
                    "GetManagedObjects",
                    None,
                )
                .await?;
                if panel.generation.get() != generation {
                    return Err("Bluetooth service changed; retrying".into());
                }
                Ok::<_, String>(result)
            }
            .await;
            let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
                return;
            };
            panel.refreshing.set(false);
            match result {
                Ok(result) => {
                    if let Some((mut objects,)) = result.get::<(Objects,)>() {
                        for interfaces in objects.values_mut() {
                            if let Some(device) = interfaces.get_mut(DEVICE) {
                                device.remove("RSSI");
                                device.remove("TxPower");
                            }
                        }
                        let changed = *panel.objects.borrow() != objects;
                        *panel.objects.borrow_mut() = objects;
                        if changed
                            || panel
                                .content
                                .upgrade()
                                .is_some_and(|content| content.first_child().is_none())
                        {
                            panel.render();
                        }
                    }
                }
                Err(error) => panel.status(&format!("Bluetooth unavailable: {error}")),
            }
        });
    }

    fn action(
        self: &Rc<Self>,
        path: String,
        interface: &'static str,
        method: &'static str,
        args: Option<glib::Variant>,
        pair: bool,
    ) {
        if !self.alive.get() || !self.agent_registered.get() || self.busy.replace(true) {
            return;
        }
        let Some(bus) = self.bus.borrow().clone() else {
            self.busy.set(false);
            return;
        };
        if pair {
            *self.pairing.borrow_mut() = Some(path.clone());
        }
        self.status(if pair {
            "Pairing… Check the device and confirm the code below."
        } else {
            "Working…"
        });
        if let Some(content) = self.content.upgrade() {
            content.set_sensitive(false);
        }
        let args_power_off = args
            .as_ref()
            .and_then(|args| args.get::<(String, String, glib::Variant)>())
            .is_some_and(|(interface, property, value)| {
                interface == ADAPTER && property == "Powered" && value.get::<bool>() == Some(false)
            });
        let owner = self.owner.borrow().clone();
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let mut result = call_owner(&bus, &owner, &path, interface, method, args).await;
            if pair
                && result.is_ok()
                && weak
                    .upgrade()
                    .is_some_and(|panel| panel.alive.get() && panel.generation.get() == generation)
            {
                result = call_owner(
                    &bus,
                    &owner,
                    &path,
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    Some((DEVICE, "Trusted", true.to_variant()).to_variant()),
                )
                .await;
                if result.is_ok()
                    && weak.upgrade().is_some_and(|panel| {
                        panel.alive.get() && panel.generation.get() == generation
                    })
                {
                    result = call_owner(&bus, &owner, &path, DEVICE, "Connect", None).await;
                }
            }
            let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
                return;
            };
            if panel.generation.get() != generation {
                return;
            }
            panel.busy.set(false);
            panel.pairing.borrow_mut().take();
            panel.reject();
            if let Some(content) = panel.content.upgrade() {
                content.set_sensitive(true);
            }
            match result {
                Ok(_) => {
                    if method == "StartDiscovery" {
                        if let Some(timer) = panel.scanning.borrow_mut().remove(&path) {
                            timer.remove();
                        }
                        let scan_path = path.clone();
                        let weak = Rc::downgrade(&panel);
                        let bus = bus.clone();
                        let timer = glib::timeout_add_local_once(panel.scan_timeout, move || {
                            if let Some(panel) = weak.upgrade().filter(|panel| {
                                panel.alive.get()
                                    && panel.generation.get() == generation
                                    && panel.scanning.borrow().contains_key(&path)
                            }) {
                                panel.scanning.borrow_mut().remove(&path);
                                glib::MainContext::default().spawn_local(async move {
                                    let _ = call_owner(
                                        &bus,
                                        &owner,
                                        &path,
                                        ADAPTER,
                                        "StopDiscovery",
                                        None,
                                    )
                                    .await;
                                });
                                panel.render();
                            }
                        });
                        panel.scanning.borrow_mut().insert(scan_path, timer);
                    } else if (method == "StopDiscovery"
                        || (method == "Set"
                            && interface == "org.freedesktop.DBus.Properties"
                            && args_power_off))
                        && let Some(timer) = panel.scanning.borrow_mut().remove(&path)
                    {
                        timer.remove();
                    }
                    panel.status("Ready");
                }
                Err(error) => panel.status(&error),
            }
            panel.render();
            panel.refresh();
        });
    }

    fn control(
        self: &Rc<Self>,
        title: &str,
        path: &str,
        interface: &'static str,
        method: &'static str,
        args: Option<glib::Variant>,
        pair: bool,
    ) -> gtk::Button {
        let button = crate::info::button(title);
        let panel = Rc::downgrade(self);
        let path = path.to_string();
        button.connect_clicked(move |_| {
            if let Some(panel) = panel.upgrade() {
                panel.action(path.clone(), interface, method, args.clone(), pair);
            }
        });
        button
    }

    fn cached_row(&self, path: &str, signature: &HashMap<String, Properties>) -> Option<gtk::Box> {
        self.rows
            .borrow()
            .get(path)
            .filter(|(old, _)| old == signature)
            .map(|(_, row)| row.clone())
    }

    fn remember_row(
        &self,
        path: &str,
        signature: HashMap<String, Properties>,
        new: gtk::Box,
    ) -> gtk::Box {
        let existing = self.rows.borrow().get(path).map(|(_, row)| row.clone());
        let row = if let Some(row) = existing {
            let mut children = Vec::new();
            while let Some(child) = new.first_child() {
                new.remove(&child);
                children.push(child);
            }
            crate::ui::reconcile_box(&row, &children);
            row
        } else {
            new
        };
        self.rows
            .borrow_mut()
            .insert(path.to_owned(), (signature, row.clone()));
        row
    }

    fn render(self: &Rc<Self>) {
        let Some(content) = self.content.upgrade() else {
            return;
        };
        let mut widgets = Vec::<gtk::Widget>::new();
        let mut retained = std::collections::HashSet::new();
        let objects = self.objects.borrow().clone();
        let mut adapters: Vec<_> = objects
            .iter()
            .filter_map(|(path, interfaces)| interfaces.get(ADAPTER).map(|props| (path, props)))
            .collect();
        adapters.sort_by_key(|(path, _)| path.to_string());
        if adapters.is_empty() {
            widgets.push(
                crate::info::label(
                    "No Bluetooth adapter found. Check the adapter or airplane mode.",
                    "menu-hint",
                )
                .upcast(),
            );
        }
        for (path, props) in adapters {
            let name: String = value(props, "Alias");
            let powered: bool = value(props, "Powered");
            let scanning = self.scanning.borrow().contains_key(path.as_str());
            let mut signature = HashMap::from([(ADAPTER.to_owned(), props.clone())]);
            signature
                .get_mut(ADAPTER)
                .unwrap()
                .insert("scanning".into(), scanning.to_variant());
            retained.insert(path.to_string());
            let adapter_row = if let Some(row) = self.cached_row(path, &signature) {
                row
            } else {
                let row = gtk::Box::new(gtk::Orientation::Vertical, 12);
                row.append(&crate::info::label(&name, "menu-title"));
                let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                controls.append(&self.control(
                    if powered { "Turn off" } else { "Turn on" },
                    path,
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    Some((ADAPTER, "Powered", (!powered).to_variant()).to_variant()),
                    false,
                ));
                let scan = self.control(
                    if scanning { "Stop scanning" } else { "Scan" },
                    path,
                    ADAPTER,
                    if scanning {
                        "StopDiscovery"
                    } else {
                        "StartDiscovery"
                    },
                    None,
                    false,
                );
                scan.set_sensitive(powered);
                controls.append(&scan);
                row.append(&controls);
                self.remember_row(path, signature, row)
            };
            widgets.push(adapter_row.upcast());
            if !powered {
                widgets.push(crate::info::label("Bluetooth is off", "menu-hint").upcast());
                continue;
            }
            let mut devices: Vec<_> = objects
                .iter()
                .filter_map(|(path, interfaces)| {
                    interfaces
                        .get(DEVICE)
                        .map(|props| (path, props, interfaces))
                })
                .filter(|(_, props, _)| {
                    props
                        .get("Adapter")
                        .and_then(ObjectPath::from_variant)
                        .as_ref()
                        == Some(path)
                })
                .collect();
            devices.sort_by_key(|(_, props, _)| {
                (
                    !value::<bool>(props, "Connected"),
                    !value::<bool>(props, "Paired"),
                    value::<String>(props, "Alias").to_lowercase(),
                )
            });
            if devices.is_empty() {
                widgets.push(
                    crate::info::label(
                        "No devices yet. Start scanning to find nearby devices.",
                        "menu-hint",
                    )
                    .upcast(),
                );
            }
            for (device, props, interfaces) in devices.into_iter().take(100) {
                retained.insert(device.to_string());
                if let Some(row) = self.cached_row(device, interfaces) {
                    widgets.push(row.upcast());
                    continue;
                }
                let row = gtk::Box::new(gtk::Orientation::Vertical, 7);
                row.add_css_class("bluetooth-device");
                let connected: bool = value(props, "Connected");
                let paired: bool = value(props, "Paired");
                let trusted: bool = value(props, "Trusted");
                let name: String = value(props, "Alias");
                let address: String = value(props, "Address");
                row.append(&crate::info::label(
                    if name.is_empty() { &address } else { &name },
                    "menu-title",
                ));
                let battery = interfaces
                    .get("org.bluez.Battery1")
                    .and_then(|props| props.get("Percentage"))
                    .and_then(u8::from_variant)
                    .map(|n| format!(" · Battery {n}%"))
                    .unwrap_or_default();
                row.append(&crate::info::label(
                    &format!(
                        "{} · {address}{battery}",
                        if connected {
                            "Connected"
                        } else if paired {
                            "Paired"
                        } else {
                            "Available"
                        }
                    ),
                    "menu-hint",
                ));
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                actions.append(&self.control(
                    if connected {
                        "Disconnect"
                    } else if paired {
                        "Connect"
                    } else {
                        "Pair"
                    },
                    device,
                    DEVICE,
                    if connected {
                        "Disconnect"
                    } else if paired {
                        "Connect"
                    } else {
                        "Pair"
                    },
                    None,
                    !paired && !connected,
                ));
                if paired {
                    actions.append(&self.control(
                        if trusted { "Untrust" } else { "Trust" },
                        device,
                        "org.freedesktop.DBus.Properties",
                        "Set",
                        Some((DEVICE, "Trusted", (!trusted).to_variant()).to_variant()),
                        false,
                    ));
                    let forget = crate::info::button("Forget");
                    let panel = Rc::downgrade(self);
                    let path = path.to_string();
                    let device = device.clone();
                    let confirm = Cell::new(false);
                    forget.connect_clicked(move |button| {
                        if !confirm.replace(true) {
                            button.set_label("Confirm forget");
                            return;
                        }
                        if let Some(panel) = panel.upgrade() {
                            panel.action(
                                path.clone(),
                                ADAPTER,
                                "RemoveDevice",
                                Some((&device,).to_variant()),
                                false,
                            );
                        }
                    });
                    actions.append(&forget);
                }
                row.append(&actions);
                widgets.push(self.remember_row(device, interfaces.clone(), row).upcast());
            }
        }
        self.rows
            .borrow_mut()
            .retain(|path, _| retained.contains(path));
        crate::ui::reconcile_box(&content, &widgets);
    }
}

pub fn view() -> gtk::Box {
    view_on_bus(gio::BusType::System)
}

fn view_on_bus(bus_type: gio::BusType) -> gtk::Box {
    view_with_timeout(bus_type, std::time::Duration::from_secs(30))
}

fn view_with_timeout(bus_type: gio::BusType, scan_timeout: std::time::Duration) -> gtk::Box {
    panel_view(bus_type, scan_timeout).0
}

fn panel_view(bus_type: gio::BusType, scan_timeout: std::time::Duration) -> (gtk::Box, Rc<Panel>) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let status = crate::info::label("Connecting to Bluetooth…", "menu-hint");
    outer.append(&status);
    let prompt = gtk::Box::new(gtk::Orientation::Vertical, 8);
    prompt.add_css_class("bluetooth-prompt");
    outer.append(&prompt);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(220)
        .max_content_height(380)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    outer.append(&scroll);
    let panel = Rc::new(Panel {
        bus: RefCell::new(None),
        registration: RefCell::new(None),
        subscription: RefCell::new(None),
        owner: RefCell::new(String::new()),
        bus_type,
        initializing: Cell::new(false),
        agent_registered: Cell::new(false),
        generation: Cell::new(0),
        objects: RefCell::new(HashMap::new()),
        rows: RefCell::new(HashMap::new()),
        content: content.downgrade(),
        prompt: prompt.downgrade(),
        status: status.downgrade(),
        pending: RefCell::new(None),
        prompt_generation: Cell::new(0),
        pairing: RefCell::new(None),
        busy: Cell::new(false),
        refreshing: Cell::new(false),
        alive: Cell::new(true),
        scanning: RefCell::new(HashMap::new()),
        scan_timeout,
    });
    outer.connect_unmap({
        let panel = panel.clone();
        move |_| {
            panel.alive.set(false);
            panel.reject();
            panel.clear_scanning();
            if let Some(bus) = panel.bus.borrow_mut().take() {
                let pairing = panel.pairing.borrow_mut().take();
                glib::MainContext::default().spawn_local(async move {
                    if let Some(path) = pairing {
                        let _ = call(&bus, &path, DEVICE, "CancelPairing", None).await;
                    }
                    let _ = bus.close_future().await;
                });
            }
        }
    });
    let weak = Rc::downgrade(&panel);
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
            return glib::ControlFlow::Break;
        };
        if !panel.busy.get()
            || panel
                .bus
                .borrow()
                .as_ref()
                .is_some_and(|bus| bus.is_closed())
        {
            panel.refresh();
        }
        glib::ControlFlow::Continue
    });
    panel.initialize();
    (outer, panel)
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application) {
    fn find(widget: &impl IsA<gtk::Widget>, text: &str) -> Option<gtk::Button> {
        let widget = widget.as_ref();
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(text)
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(button) = find(&widget, text) {
                return Some(button);
            }
        }
        None
    }
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    let address =
        gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    let replacement = gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    let adapter = "/org/bluez/hci0";
    let device = "/org/bluez/hci0/dev_00_11_22_33_44_55";
    let objects = Objects::from([
        (
            ObjectPath::try_from(adapter).unwrap(),
            HashMap::from([(
                ADAPTER.into(),
                Properties::from([
                    ("Alias".into(), "Test adapter".to_variant()),
                    ("Powered".into(), true.to_variant()),
                ]),
            )]),
        ),
        (
            ObjectPath::try_from(device).unwrap(),
            HashMap::from([(
                DEVICE.into(),
                Properties::from([
                    ("Alias".into(), "Test headphones".to_variant()),
                    (
                        "Adapter".into(),
                        ObjectPath::try_from(adapter).unwrap().to_variant(),
                    ),
                    ("Paired".into(), false.to_variant()),
                ]),
            )]),
        ),
    ]);
    let objects = Rc::new(RefCell::new(objects));
    let agent = Rc::new(RefCell::new(None::<(String, String)>));
    let calls = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut registrations = Vec::new();
    for service in [&bus, &replacement] {
        for (path, interface, methods) in [
            (
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "<method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method>",
            ),
            (
                "/org/bluez",
                "org.bluez.AgentManager1",
                "<method name='RegisterAgent'><arg type='o' direction='in'/><arg type='s' direction='in'/></method>",
            ),
            (
                adapter,
                ADAPTER,
                "<method name='StartDiscovery'/><method name='StopDiscovery'/>",
            ),
            (
                device,
                DEVICE,
                "<method name='Pair'/><method name='Connect'/><method name='CancelPairing'/>",
            ),
            (
                device,
                "org.freedesktop.DBus.Properties",
                "<method name='Set'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='v' direction='in'/></method>",
            ),
        ] {
            let xml = format!("<node><interface name='{interface}'>{methods}</interface></node>");
            let info = gio::DBusNodeInfo::for_xml(&xml)
                .unwrap()
                .lookup_interface(interface)
                .unwrap();
            let objects = objects.clone();
            let agent = agent.clone();
            let calls = calls.clone();
            registrations.push((
                service.clone(),
                service
                    .register_object(path, &info)
                    .method_call(move |bus, sender, _, _, method, args, invocation| {
                        calls.borrow_mut().push(method.into());
                        match method {
                            "GetManagedObjects" => invocation
                                .return_value(Some(&(objects.borrow().clone(),).to_variant())),
                            "RegisterAgent" => {
                                *agent.borrow_mut() = Some((
                                    sender.unwrap().into(),
                                    args.child_get::<ObjectPath>(0).to_string(),
                                ));
                                invocation.return_value(Some(&().to_variant()));
                            }
                            "Pair" => {
                                let (owner, path) = agent.borrow().clone().unwrap();
                                glib::MainContext::default().spawn_local(async move {
                                    let result = bus
                                        .call_future(
                                            Some(&owner),
                                            &path,
                                            "org.bluez.Agent1",
                                            "RequestConfirmation",
                                            Some(
                                                &(ObjectPath::try_from(device).unwrap(), 123456u32)
                                                    .to_variant(),
                                            ),
                                            None,
                                            gio::DBusCallFlags::NONE,
                                            3000,
                                        )
                                        .await;
                                    match result {
                                        Ok(_) => invocation.return_value(Some(&().to_variant())),
                                        Err(_) => invocation.return_dbus_error(
                                            "org.bluez.Error.Rejected",
                                            "Rejected",
                                        ),
                                    }
                                });
                            }
                            _ => invocation.return_value(Some(&().to_variant())),
                        }
                    })
                    .build()
                    .unwrap(),
            ));
        }
    }
    let (view, panel) = panel_view(gio::BusType::Session, std::time::Duration::from_millis(700));
    view.add_css_class("menu-content");
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .child(&view)
        .build();
    window.present();
    crate::ui_tests::pump(100);
    assert!(agent.borrow().is_none());
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&("org.bluez", 0u32).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    )
    .unwrap();
    crate::ui_tests::pump(3100);
    let stable = panel.rows.borrow()[device].1.clone();
    objects
        .borrow_mut()
        .get_mut(&ObjectPath::try_from(device).unwrap())
        .unwrap()
        .get_mut(DEVICE)
        .unwrap()
        .insert("Alias".into(), "Renamed headphones".to_variant());
    panel.refresh();
    crate::ui_tests::pump(100);
    assert_eq!(panel.rows.borrow()[device].1, stable);
    assert_eq!(
        stable
            .first_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap()
            .text(),
        "Renamed headphones"
    );
    find(&view, "Scan")
        .expect("Bluetooth controls loaded")
        .emit_clicked();
    crate::ui_tests::pump(150);
    assert!(calls.borrow().iter().any(|call| call == "StartDiscovery"));
    find(&view, "Stop scanning").unwrap().emit_clicked();
    crate::ui_tests::pump(150);
    assert!(calls.borrow().iter().any(|call| call == "StopDiscovery"));
    find(&view, "Scan").unwrap().emit_clicked();
    crate::ui_tests::pump(500);
    assert!(find(&view, "Stop scanning").is_some());
    assert_eq!(
        calls
            .borrow()
            .iter()
            .filter(|call| call.as_str() == "StopDiscovery")
            .count(),
        1
    );
    find(&view, "Stop scanning").unwrap().emit_clicked();
    crate::ui_tests::pump(100);
    find(&view, "Pair").unwrap().emit_clicked();
    crate::ui_tests::pump(150);
    crate::ui_tests::capture("bluetooth");
    find(&view, "Confirm")
        .expect("Pairing confirmation shown")
        .emit_clicked();
    crate::ui_tests::pump(200);
    assert!(calls.borrow().iter().any(|call| call == "Set"));
    assert!(calls.borrow().iter().any(|call| call == "Connect"));
    find(&view, "Pair").unwrap().emit_clicked();
    crate::ui_tests::pump(100);
    find(&view, "Cancel")
        .expect("Second prompt shown")
        .emit_clicked();
    crate::ui_tests::pump(100);
    assert_eq!(
        calls
            .borrow()
            .iter()
            .filter(|call| call.as_str() == "Connect")
            .count(),
        1
    );
    let release = |service: &gio::DBusConnection| {
        service
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ReleaseName",
                Some(&("org.bluez",).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            )
            .unwrap();
    };
    find(&view, "Scan").unwrap().emit_clicked();
    crate::ui_tests::pump(100);
    find(&view, "Pair").unwrap().emit_clicked();
    crate::ui_tests::pump(100);
    assert!(find(&view, "Confirm").is_some());
    release(&bus);
    crate::ui_tests::pump(100);
    assert!(find(&view, "Confirm").is_none());
    replacement
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&("org.bluez", 0u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    crate::ui_tests::pump(3100);
    assert_eq!(
        calls
            .borrow()
            .iter()
            .filter(|call| call.as_str() == "RegisterAgent")
            .count(),
        2
    );
    assert!(find(&view, "Scan").is_some());
    assert!(find(&view, "Stop scanning").is_none());
    let (destination, agent_path) = agent.borrow().clone().unwrap();
    let rejected = Rc::new(Cell::new(false));
    let result = rejected.clone();
    glib::MainContext::default().spawn_local(async move {
        let response = bus
            .call_future(
                Some(&destination),
                &agent_path,
                "org.bluez.Agent1",
                "RequestConfirmation",
                Some(&(ObjectPath::try_from(device).unwrap(), 123456u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                1000,
            )
            .await;
        result.set(response.is_err());
    });
    crate::ui_tests::pump(100);
    assert!(rejected.get());
    assert!(find(&view, "Confirm").is_none());
    find(&view, "Pair").unwrap().emit_clicked();
    crate::ui_tests::pump(150);
    find(&view, "Confirm")
        .expect("New BlueZ owner can pair")
        .emit_clicked();
    crate::ui_tests::pump(150);
    release(&replacement);
    window.close();
    crate::ui_tests::pump(100);
    for (service, registration) in registrations {
        service.unregister_object(registration).unwrap();
    }
    replacement
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ReleaseName",
            Some(&("org.bluez",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        )
        .unwrap();
}
