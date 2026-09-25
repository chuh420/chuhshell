use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use glib::variant::ToVariant;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const XML: &str = r#"<node>
<interface name="org.freedesktop.Notifications">
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="Notify">
<arg type="s" direction="in"/><arg type="u" direction="in"/>
<arg type="s" direction="in"/><arg type="s" direction="in"/>
<arg type="s" direction="in"/><arg type="as" direction="in"/>
<arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
<arg type="u" direction="out"/>
</method>
<method name="CloseNotification"><arg type="u" direction="in"/></method>
<method name="GetServerInformation">
<arg type="s" direction="out"/><arg type="s" direction="out"/>
<arg type="s" direction="out"/><arg type="s" direction="out"/>
</method>
<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
</interface>
</node>"#;

#[derive(Clone)]
struct NotificationView {
    id: u32,
    app: String,
    icon: String,
    summary: String,
    body: String,
    actions: Vec<(String, String)>,
}

struct Notification {
    view: NotificationView,
    active: bool,
    popup: Option<gtk::Window>,
    timer: Option<glib::SourceId>,
}

pub struct NotificationCenter {
    app: gtk::Application,
    connection: RefCell<Option<gio::DBusConnection>>,
    notifications: RefCell<Vec<Notification>>,
    next_id: Cell<u32>,
    label: RefCell<Option<gtk::Label>>,
    drawer: RefCell<Option<gtk::Window>>,
    list: RefCell<Option<gtk::Box>>,
}

impl NotificationCenter {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        Rc::new(Self {
            app: app.clone(),
            connection: RefCell::new(None),
            notifications: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
            label: RefCell::new(None),
            drawer: RefCell::new(None),
            list: RefCell::new(None),
        })
    }

    pub fn start(self: &Rc<Self>) {
        let acquired = Rc::downgrade(self);
        gio::bus_own_name(
            gio::BusType::Session,
            BUS_NAME,
            gio::BusNameOwnerFlags::NONE,
            move |connection, _| {
                let Some(center) = acquired.upgrade() else {
                    return;
                };
                let info = gio::DBusNodeInfo::for_xml(XML)
                    .expect("valid notification interface")
                    .lookup_interface(INTERFACE)
                    .expect("notification interface exists");
                let weak = Rc::downgrade(&center);
                match connection
                    .register_object(OBJECT_PATH, &info)
                    .method_call(move |_, _, _, _, method, parameters, invocation| {
                        if let Some(center) = weak.upgrade() {
                            center.handle_method(method, &parameters, invocation);
                        } else {
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.Failed",
                                "Notification service stopped",
                            );
                        }
                    })
                    .build()
                {
                    Ok(_) => *center.connection.borrow_mut() = Some(connection),
                    Err(error) => eprintln!("Could not register notification service: {error}"),
                }
            },
            |_, _| {},
            |_, _| eprintln!("Could not own org.freedesktop.Notifications"),
        );
    }

    fn handle_method(
        self: &Rc<Self>,
        method: &str,
        parameters: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        match method {
            "GetCapabilities" => {
                invocation.return_value(Some(
                    &(vec!["actions", "body", "icon-static"],).to_variant(),
                ));
            }
            "GetServerInformation" => {
                invocation.return_value(Some(
                    &("chuhshell", "chuh", env!("CARGO_PKG_VERSION"), "1.2").to_variant(),
                ));
            }
            "Notify" => {
                let app = parameters.child_get::<String>(0);
                let app = if app.is_empty() {
                    "Application".to_owned()
                } else {
                    app
                };
                let replaces_id = parameters.child_get::<u32>(1);
                let mut icon = parameters.child_get::<String>(2);
                let summary = parameters.child_get::<String>(3);
                let body = parameters.child_get::<String>(4);
                let raw_actions = parameters.child_get::<Vec<String>>(5);
                let hints = glib::VariantDict::new(Some(&parameters.child_value(6)));
                let urgency = hints.lookup::<u8>("urgency").ok().flatten().unwrap_or(1);
                if icon.is_empty()
                    && let Some(path) = hints.lookup::<String>("image-path").ok().flatten()
                {
                    icon = path;
                }
                let timeout = if urgency == 2 {
                    0
                } else {
                    parameters.child_get::<i32>(7)
                };
                let actions = raw_actions
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| (pair[0].clone(), pair[1].clone()))
                    .collect();
                let id = self.notify(
                    replaces_id,
                    NotificationView {
                        id: 0,
                        app,
                        icon,
                        summary,
                        body,
                        actions,
                    },
                    timeout,
                );
                invocation.return_value(Some(&(id,).to_variant()));
            }
            "CloseNotification" => {
                if self.close(parameters.child_get::<u32>(0), 3) {
                    invocation.return_value(Some(&().to_variant()));
                } else {
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "");
                }
            }
            _ => invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "Unknown notification method",
            ),
        }
    }

    fn notify(self: &Rc<Self>, replaces_id: u32, mut view: NotificationView, timeout: i32) -> u32 {
        let mut entries = self.notifications.borrow_mut();
        let existing = entries
            .iter_mut()
            .find(|entry| entry.active && entry.view.id == replaces_id);
        let id = if let Some(entry) = existing {
            if let Some(timer) = entry.timer.take() {
                timer.remove();
            }
            view.id = entry.view.id;
            entry.view = view;
            entry.view.id
        } else {
            let id = self.next_id.get();
            self.next_id.set(id.wrapping_add(1).max(1));
            view.id = id;
            entries.push(Notification {
                view,
                active: true,
                popup: None,
                timer: None,
            });
            id
        };
        drop(entries);
        self.show_popup(id);
        if timeout != 0 {
            let duration = if timeout < 0 {
                Duration::from_secs(6)
            } else {
                Duration::from_millis(timeout as u64)
            };
            let weak = Rc::downgrade(self);
            let timer = glib::timeout_add_local_once(duration, move || {
                if let Some(center) = weak.upgrade() {
                    center.close(id, 1);
                }
            });
            if let Some(entry) = self
                .notifications
                .borrow_mut()
                .iter_mut()
                .find(|e| e.view.id == id)
            {
                entry.timer = Some(timer);
            }
        }
        self.refresh();
        id
    }

    fn show_popup(self: &Rc<Self>, id: u32) {
        let Some(view) = self
            .notifications
            .borrow()
            .iter()
            .find(|entry| entry.view.id == id)
            .map(|entry| entry.view.clone())
        else {
            return;
        };
        if let Some(popup) = self
            .notifications
            .borrow()
            .iter()
            .find(|entry| entry.view.id == id)
            .and_then(|entry| entry.popup.clone())
        {
            popup.set_child(Some(&self.notification_content(&view, true, true)));
            return;
        }
        if self
            .notifications
            .borrow()
            .iter()
            .filter(|entry| entry.popup.is_some())
            .count()
            >= 4
        {
            let oldest = self
                .notifications
                .borrow_mut()
                .iter_mut()
                .filter_map(|entry| entry.popup.take())
                .next();
            if let Some(oldest) = oldest {
                oldest.close();
            }
        }
        let window = gtk::Window::builder()
            .application(&self.app)
            .title("Notification")
            .default_width(360)
            .build();
        window.set_widget_name("app-notification");
        window.init_layer_shell();
        window.set_namespace(Some("chuhshell-app-notification"));
        window.set_layer(layer_shell::Layer::Overlay);
        window.set_anchor(layer_shell::Edge::Bottom, true);
        window.set_anchor(layer_shell::Edge::Right, true);
        window.set_margin(layer_shell::Edge::Right, 24);
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(layer_shell::KeyboardMode::None);
        window.set_child(Some(&self.notification_content(&view, true, true)));
        if let Some(entry) = self
            .notifications
            .borrow_mut()
            .iter_mut()
            .find(|entry| entry.view.id == id)
        {
            entry.popup = Some(window.clone());
        }
        window.present();
        self.position_popups();
    }

    fn notification_content(
        self: &Rc<Self>,
        view: &NotificationView,
        active: bool,
        compact: bool,
    ) -> gtk::Box {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("app-notification-content");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = if view.icon.starts_with("file://") {
            gtk::Image::from_file(gio::File::for_uri(&view.icon).path().unwrap_or_default())
        } else if Path::new(&view.icon).is_absolute() {
            gtk::Image::from_file(&view.icon)
        } else {
            gtk::Image::from_icon_name(if view.icon.is_empty() {
                "dialog-information-symbolic"
            } else {
                &view.icon
            })
        };
        icon.set_pixel_size(28);
        header.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let app = gtk::Label::new(Some(&view.app));
        app.add_css_class("app-notification-app");
        app.set_xalign(0.0);
        text.append(&app);
        let summary = gtk::Label::new(Some(&view.summary));
        summary.add_css_class("app-notification-summary");
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        if compact {
            summary.set_lines(2);
        }
        text.append(&summary);
        header.append(&text);
        root.append(&header);
        if !view.body.is_empty() {
            let body = gtk::Label::new(Some(&view.body));
            body.add_css_class("app-notification-body");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_max_width_chars(42);
            if compact {
                body.set_lines(3);
            }
            root.append(&body);
        }
        if active && !view.actions.is_empty() {
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for (key, label) in &view.actions {
                let button = gtk::Button::with_label(label);
                button.add_css_class("notification-action");
                let key = key.clone();
                let id = view.id;
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(center) = weak.upgrade() {
                        center.invoke_action(id, &key);
                    }
                });
                actions.append(&button);
            }
            root.append(&actions);
        }
        root
    }

    fn invoke_action(self: &Rc<Self>, id: u32, key: &str) {
        if !self
            .notifications
            .borrow()
            .iter()
            .any(|entry| entry.view.id == id && entry.active)
        {
            return;
        }
        if let Some(connection) = self.connection.borrow().as_ref() {
            let _ = connection.emit_signal(
                None,
                OBJECT_PATH,
                INTERFACE,
                "ActionInvoked",
                Some(&(id, key).to_variant()),
            );
        }
        self.close(id, 2);
    }

    fn close(self: &Rc<Self>, id: u32, reason: u32) -> bool {
        let mut entries = self.notifications.borrow_mut();
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.view.id == id && entry.active)
        else {
            return false;
        };
        entry.active = false;
        if let Some(timer) = entry.timer.take()
            && reason != 1
        {
            timer.remove();
        }
        if let Some(popup) = entry.popup.take() {
            popup.close();
        }
        drop(entries);
        if let Some(connection) = self.connection.borrow().as_ref() {
            let _ = connection.emit_signal(
                None,
                OBJECT_PATH,
                INTERFACE,
                "NotificationClosed",
                Some(&(id, reason).to_variant()),
            );
        }
        self.position_popups();
        self.refresh();
        true
    }

    fn position_popups(&self) {
        for (index, entry) in self
            .notifications
            .borrow()
            .iter()
            .filter(|entry| entry.popup.is_some())
            .enumerate()
        {
            if let Some(popup) = &entry.popup {
                popup.set_margin(layer_shell::Edge::Bottom, 140 + index as i32 * 150);
            }
        }
    }

    pub fn attach_label(&self, label: &gtk::Label) {
        *self.label.borrow_mut() = Some(label.clone());
        self.refresh_label();
    }

    fn refresh_label(&self) {
        if let Some(label) = self.label.borrow().as_ref() {
            let count = self.notifications.borrow().len();
            label.set_text(&if count == 0 {
                "󰂚".to_owned()
            } else {
                format!("󰂚 {count}")
            });
        }
    }

    pub fn toggle_drawer(self: &Rc<Self>) {
        if let Some(window) = self.drawer.borrow_mut().take() {
            *self.list.borrow_mut() = None;
            window.close();
            return;
        }
        let window = gtk::Window::builder()
            .application(&self.app)
            .title("Notifications")
            .default_width(390)
            .build();
        window.set_widget_name("notification-drawer");
        window.init_layer_shell();
        window.set_namespace(Some("chuhshell-notification-drawer"));
        window.set_layer(layer_shell::Layer::Overlay);
        window.set_anchor(layer_shell::Edge::Top, true);
        window.set_anchor(layer_shell::Edge::Right, true);
        window.set_margin(layer_shell::Edge::Top, 40);
        window.set_margin(layer_shell::Edge::Right, 8);
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(layer_shell::KeyboardMode::None);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("notification-drawer-content");
        let heading = gtk::Label::new(Some("Notifications"));
        heading.add_css_class("notification-drawer-heading");
        heading.set_xalign(0.0);
        root.append(&heading);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(420);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        scroll.set_child(Some(&list));
        root.append(&scroll);
        let clear = gtk::Button::with_label("Clear notifications");
        clear.add_css_class("notification-clear");
        let weak = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(center) = weak.upgrade() {
                center.clear();
            }
        });
        root.append(&clear);
        window.set_child(Some(&root));
        *self.list.borrow_mut() = Some(list);
        *self.drawer.borrow_mut() = Some(window.clone());
        self.refresh();
        window.present();
    }

    fn refresh(self: &Rc<Self>) {
        self.refresh_label();
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let entries = self.notifications.borrow();
        if entries.is_empty() {
            let empty = gtk::Label::new(Some("No notifications"));
            empty.add_css_class("notification-empty");
            list.append(&empty);
        } else {
            for entry in entries.iter().rev() {
                list.append(&self.notification_content(&entry.view, entry.active, false));
            }
        }
    }

    fn clear(self: &Rc<Self>) {
        let entries = std::mem::take(&mut *self.notifications.borrow_mut());
        for mut entry in entries {
            if let Some(timer) = entry.timer.take() {
                timer.remove();
            }
            if let Some(popup) = entry.popup.take() {
                popup.close();
            }
            if entry.active
                && let Some(connection) = self.connection.borrow().as_ref()
            {
                let _ = connection.emit_signal(
                    None,
                    OBJECT_PATH,
                    INTERFACE,
                    "NotificationClosed",
                    Some(&(entry.view.id, 2_u32).to_variant()),
                );
            }
        }
        self.refresh();
    }
}
