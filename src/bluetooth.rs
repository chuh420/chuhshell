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
    bus.call_future(
        Some("org.bluez"),
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
    owner: RefCell<String>,
    objects: RefCell<Objects>,
    content: glib::WeakRef<gtk::Box>,
    prompt: glib::WeakRef<gtk::Box>,
    status: glib::WeakRef<gtk::Label>,
    pending: RefCell<Option<gio::DBusMethodInvocation>>,
    prompt_generation: Cell<u64>,
    pairing: RefCell<Option<String>>,
    busy: Cell<bool>,
    refreshing: Cell<bool>,
    alive: Cell<bool>,
    scanning: RefCell<Vec<String>>,
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.reject();
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

    fn refresh(self: &Rc<Self>) {
        if !self.alive.get() || self.refreshing.replace(true) {
            return;
        }
        let Some(bus) = self.bus.borrow().clone() else {
            self.refreshing.set(false);
            return;
        };
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = call(
                &bus,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "GetManagedObjects",
                None,
            )
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
        if self.busy.replace(true) {
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
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let mut result = call(&bus, &path, interface, method, args).await;
            if pair && result.is_ok() && weak.upgrade().is_some_and(|panel| panel.alive.get()) {
                result = call(
                    &bus,
                    &path,
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    Some((DEVICE, "Trusted", true.to_variant()).to_variant()),
                )
                .await;
                if result.is_ok() {
                    result = call(&bus, &path, DEVICE, "Connect", None).await;
                }
            }
            let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
                return;
            };
            panel.busy.set(false);
            panel.pairing.borrow_mut().take();
            panel.reject();
            if let Some(content) = panel.content.upgrade() {
                content.set_sensitive(true);
            }
            match result {
                Ok(_) => {
                    if method == "StartDiscovery" {
                        panel.scanning.borrow_mut().push(path.clone());
                        let weak = Rc::downgrade(&panel);
                        let bus = bus.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_secs(30),
                            move || {
                                if let Some(panel) = weak.upgrade().filter(|panel| {
                                    panel.alive.get() && panel.scanning.borrow().contains(&path)
                                }) {
                                    panel.scanning.borrow_mut().retain(|old| old != &path);
                                    glib::MainContext::default().spawn_local(async move {
                                        let _ =
                                            call(&bus, &path, ADAPTER, "StopDiscovery", None).await;
                                    });
                                    panel.render();
                                }
                            },
                        );
                    } else if method == "StopDiscovery" {
                        panel.scanning.borrow_mut().retain(|old| old != &path);
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

    fn render(self: &Rc<Self>) {
        let Some(content) = self.content.upgrade() else {
            return;
        };
        while let Some(child) = content.first_child() {
            content.remove(&child);
        }
        let objects = self.objects.borrow().clone();
        let mut adapters: Vec<_> = objects
            .iter()
            .filter_map(|(path, interfaces)| interfaces.get(ADAPTER).map(|props| (path, props)))
            .collect();
        adapters.sort_by_key(|(path, _)| path.to_string());
        if adapters.is_empty() {
            content.append(&crate::info::label(
                "No Bluetooth adapter found. Check the adapter or airplane mode.",
                "menu-hint",
            ));
        }
        for (path, props) in adapters {
            let name: String = value(props, "Alias");
            let powered: bool = value(props, "Powered");
            content.append(&crate::info::label(&name, "menu-title"));
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            controls.append(&self.control(
                if powered { "Turn off" } else { "Turn on" },
                path,
                "org.freedesktop.DBus.Properties",
                "Set",
                Some((ADAPTER, "Powered", (!powered).to_variant()).to_variant()),
                false,
            ));
            let scanning = self.scanning.borrow().iter().any(|s| s == path.as_str());
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
            content.append(&controls);
            if !powered {
                content.append(&crate::info::label("Bluetooth is off", "menu-hint"));
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
                content.append(&crate::info::label(
                    "No devices yet. Start scanning to find nearby devices.",
                    "menu-hint",
                ));
            }
            for (device, props, interfaces) in devices.into_iter().take(100) {
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
                content.append(&row);
            }
        }
    }
}

pub fn view() -> gtk::Box {
    view_on_bus(gio::BusType::System)
}

fn view_on_bus(bus_type: gio::BusType) -> gtk::Box {
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
        owner: RefCell::new(String::new()),
        objects: RefCell::new(HashMap::new()),
        content: content.downgrade(),
        prompt: prompt.downgrade(),
        status: status.downgrade(),
        pending: RefCell::new(None),
        prompt_generation: Cell::new(0),
        pairing: RefCell::new(None),
        busy: Cell::new(false),
        refreshing: Cell::new(false),
        alive: Cell::new(true),
        scanning: RefCell::new(Vec::new()),
    });
    outer.connect_unmap({
        let panel = panel.clone();
        move |_| {
            panel.alive.set(false);
            panel.reject();
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
        if !panel.busy.get() {
            panel.refresh();
        }
        glib::ControlFlow::Continue
    });
    let weak = Rc::downgrade(&panel);
    glib::MainContext::default().spawn_local(async move {
        let result = async {
            let address = gio::dbus_address_get_for_bus_sync(bus_type, gio::Cancellable::NONE)
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
                    "BlueZ is not running. Install bluez and enable bluetooth.service.".to_string()
                })?
                .child_get::<String>(0);
            let Some(panel) = weak.upgrade().filter(|p| p.alive.get()) else {
                let _ = bus.close_future().await;
                return Ok(());
            };
            *panel.owner.borrow_mut() = owner;
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
            call(
                &bus,
                "/org/bluez",
                "org.bluez.AgentManager1",
                "RegisterAgent",
                Some((ObjectPath::try_from(AGENT).unwrap(), "KeyboardDisplay").to_variant()),
            )
            .await?;
            if panel.alive.get() {
                panel.status("Ready · Start scanning to find devices");
                panel.refresh();
            }
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = result
            && let Some(panel) = weak.upgrade()
        {
            panel.status(&error);
        }
    });
    outer
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
    let agent = Rc::new(RefCell::new(None::<(String, String)>));
    let calls = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut registrations = Vec::new();
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
        registrations.push(
            bus.register_object(path, &info)
                .method_call(move |bus, sender, _, _, method, args, invocation| {
                    calls.borrow_mut().push(method.into());
                    match method {
                        "GetManagedObjects" => {
                            invocation.return_value(Some(&(objects.clone(),).to_variant()))
                        }
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
                                    Err(_) => invocation
                                        .return_dbus_error("org.bluez.Error.Rejected", "Rejected"),
                                }
                            });
                        }
                        _ => invocation.return_value(Some(&().to_variant())),
                    }
                })
                .build()
                .unwrap(),
        );
    }
    let view = view_on_bus(gio::BusType::Session);
    view.add_css_class("menu-content");
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .child(&view)
        .build();
    window.present();
    crate::ui_tests::pump(300);
    find(&view, "Scan")
        .expect("Bluetooth controls loaded")
        .emit_clicked();
    crate::ui_tests::pump(150);
    assert!(calls.borrow().iter().any(|call| call == "StartDiscovery"));
    find(&view, "Stop scanning").unwrap().emit_clicked();
    crate::ui_tests::pump(150);
    assert!(calls.borrow().iter().any(|call| call == "StopDiscovery"));
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
    window.close();
    crate::ui_tests::pump(100);
    for registration in registrations {
        bus.unregister_object(registration).unwrap();
    }
    bus.call_sync(
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
