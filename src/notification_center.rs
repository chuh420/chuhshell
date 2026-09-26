use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

#[derive(Clone, PartialEq, Eq)]
struct NotificationView {
    id: u32,
    app: String,
    icon: String,
    summary: String,
    body: String,
    actions: Vec<(String, String)>,
    desktop_id: Option<String>,
    resident: bool,
    transient: bool,
    image: Option<ImageData>,
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
    owned: Cell<bool>,
    buttons: RefCell<Vec<glib::WeakRef<gtk::Button>>>,
    drawer: RefCell<Option<gtk::Popover>>,
    rows: RefCell<HashMap<u32, (NotificationView, bool, gtk::Box)>>,
    list: RefCell<Option<gtk::Box>>,
}

impl NotificationCenter {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        Rc::new(Self {
            app: app.clone(),
            connection: RefCell::new(None),
            notifications: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
            owned: Cell::new(false),
            buttons: RefCell::new(Vec::new()),
            rows: RefCell::new(HashMap::new()),
            drawer: RefCell::new(None),
            list: RefCell::new(None),
        })
    }

    pub fn start(self: &Rc<Self>) {
        let acquired = Rc::downgrade(self);
        let owner = Rc::downgrade(self);
        let lost = Rc::downgrade(self);
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
            move |_, _| {
                if let Some(center) = owner.upgrade() {
                    center.owned.set(true);
                    center.refresh_buttons();
                }
            },
            move |_, _| {
                if let Some(center) = lost.upgrade() {
                    center.owned.set(false);
                    center.refresh_buttons();
                }
                eprintln!("chuhshell: notification service unavailable or owned by another daemon");
            },
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
                let image = hints
                    .lookup_value("image-data", None)
                    .or_else(|| hints.lookup_value("image_data", None))
                    .filter(|value| value.size() <= 1024 * 1024 + 1024)
                    .and_then(|value| value.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>())
                    .and_then(|(w, h, stride, alpha, bits, channels, bytes)| {
                        ImageData::decode(w, h, stride, alpha, bits, channels, &bytes)
                    });
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
                    .take(8)
                    .filter(|pair| pair[0].len() <= 256)
                    .map(|pair| (pair[0].clone(), limited(&pair[1], 128)))
                    .collect();
                let id = self.notify(
                    replaces_id,
                    NotificationView {
                        id: 0,
                        app: limited(&app, 128),
                        icon: limited(&icon, 1024),
                        summary: limited(&summary, 256),
                        body: limited(&body, 4096),
                        actions,
                        desktop_id: hints
                            .lookup::<String>("desktop-entry")
                            .ok()
                            .flatten()
                            .map(|id| limited(&id, 256)),
                        resident: hints
                            .lookup::<bool>("resident")
                            .ok()
                            .flatten()
                            .unwrap_or(false),
                        transient: hints
                            .lookup::<bool>("transient")
                            .ok()
                            .flatten()
                            .unwrap_or(false),
                        image,
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
        let replacing = self
            .notifications
            .borrow()
            .iter()
            .any(|entry| entry.active && entry.view.id == replaces_id);
        if !replacing
            && self.notifications.borrow().len() >= crate::config::get().notification_history_limit
        {
            let victim = {
                let entries = self.notifications.borrow();
                entries
                    .iter()
                    .find(|entry| !entry.active)
                    .or_else(|| entries.first())
                    .map(|entry| entry.view.id)
            };
            if let Some(id) = victim {
                self.remove(id);
            }
        }
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
            let mut id = self.next_id.get().max(1);
            while entries.iter().any(|entry| entry.view.id == id) {
                id = id.wrapping_add(1).max(1);
            }
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
        let existing_popup = self
            .notifications
            .borrow()
            .iter()
            .find(|entry| entry.view.id == id)
            .and_then(|entry| entry.popup.clone());
        if let Some(popup) = existing_popup {
            popup.set_child(Some(&self.notification_content(&view, true, true)));
            self.position_popups();
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
        window.set_monitor(crate::ui::active_monitor().as_ref());
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
        let icon = if let Some(image) = &view.image {
            let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
                &glib::Bytes::from_owned(image.bytes.clone()),
                gtk::gdk_pixbuf::Colorspace::Rgb,
                image.alpha,
                8,
                image.width,
                image.height,
                image.stride,
            );
            gtk::Image::from_pixbuf(Some(&pixbuf))
        } else {
            crate::ui::image(&view.icon, 28)
        };
        icon.set_pixel_size(28);
        header.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let app = gtk::Label::new(Some(&view.app));
        app.set_ellipsize(gtk::pango::EllipsizeMode::End);
        app.set_max_width_chars(30);
        app.add_css_class("app-notification-app");
        app.set_xalign(0.0);
        text.append(&app);
        let summary = gtk::Label::new(Some(&view.summary));
        summary.add_css_class("app-notification-summary");
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        summary.set_max_width_chars(38);
        if compact {
            summary.set_lines(2);
            summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        }
        text.append(&summary);
        header.append(&text);
        let dismiss = gtk::Button::from_icon_name("window-close-symbolic");
        dismiss.add_css_class("notification-action");
        dismiss.set_tooltip_text(Some("Dismiss notification"));
        let weak = Rc::downgrade(self);
        let id = view.id;
        dismiss.connect_clicked(move |_| {
            if let Some(center) = weak.upgrade() {
                center.remove(id);
            }
        });
        header.append(&dismiss);
        root.append(&header);
        if !view.body.is_empty() {
            let body = gtk::Label::new(Some(&view.body));
            body.add_css_class("app-notification-body");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            body.set_max_width_chars(42);
            if compact {
                body.set_lines(3);
                body.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            root.append(&body);
        }
        if active && !view.actions.is_empty() {
            let actions = gtk::FlowBox::new();
            actions.set_selection_mode(gtk::SelectionMode::None);
            actions.set_max_children_per_line(3);
            actions.set_row_spacing(4);
            actions.set_column_spacing(4);
            for (key, label) in &view.actions {
                let button = gtk::Button::with_label(if label.is_empty() && key == "default" {
                    "Open"
                } else {
                    label
                });
                button.add_css_class("notification-action");
                let key = key.clone();
                let id = view.id;
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(center) = weak.upgrade() {
                        center.invoke_action(id, &key);
                    }
                });
                actions.insert(&button, -1);
            }
            root.append(&actions);
        }
        if !active && let Some(id) = view.desktop_id.clone() {
            let open = gtk::Button::with_label("Open application");
            open.add_css_class("notification-action");
            open.connect_clicked(move |button| {
                let id = id.clone();
                let weak = button.downgrade();
                glib::MainContext::default().spawn_local(async move {
                    if let Err(error) = crate::apps::launch(&id).await
                        && let Some(button) = weak.upgrade()
                    {
                        button.set_tooltip_text(Some(&error));
                    }
                });
            });
            root.append(&open);
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
        let resident = self
            .notifications
            .borrow()
            .iter()
            .find(|entry| entry.view.id == id)
            .is_some_and(|entry| entry.view.resident);
        if !resident {
            self.close(id, 2);
        }
    }

    fn remove(self: &Rc<Self>, id: u32) {
        self.close(id, 2);
        self.notifications
            .borrow_mut()
            .retain(|entry| entry.view.id != id);
        self.refresh();
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
        let transient = entry.view.transient;
        if transient || reason == 3 {
            entries.retain(|entry| entry.view.id != id);
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
        let mut offsets: HashMap<Option<String>, i32> = HashMap::new();
        let mut entries = self.notifications.borrow_mut();
        for entry in entries.iter_mut().rev() {
            let Some(popup) = entry.popup.as_ref() else {
                continue;
            };
            let monitor = popup.monitor().or_else(crate::ui::active_monitor);
            let key = monitor
                .as_ref()
                .and_then(|m| m.connector())
                .map(|s| s.to_string());
            let bottom = offsets.entry(key).or_insert(140);
            let height = popup.child().map_or(140, |child| {
                child.measure(gtk::Orientation::Vertical, 360).1
            });
            let available = monitor.map_or(900, |m| m.geometry().height());
            if *bottom + height + 48 > available {
                if let Some(popup) = entry.popup.take() {
                    popup.close();
                }
            } else {
                popup.set_margin(layer_shell::Edge::Bottom, *bottom);
                *bottom += height + 10;
            }
        }
    }

    pub fn attach_button(&self, button: &gtk::Button) {
        self.buttons.borrow_mut().push(button.downgrade());
        self.refresh_buttons();
    }
    fn refresh_buttons(&self) {
        let count = self.notifications.borrow().len();
        self.buttons.borrow_mut().retain(|weak| {
            if let Some(button) = weak.upgrade() {
                button.set_tooltip_text(Some(if self.owned.get() {
                    "Notifications"
                } else {
                    "Notification service unavailable or owned by another daemon"
                }));
                button.set_label(&if count == 0 {
                    "󰂚".into()
                } else {
                    format!("󰂚 {count}")
                });
                true
            } else {
                false
            }
        });
    }
    pub fn toggle_drawer(self: &Rc<Self>) {
        let button = self
            .buttons
            .borrow()
            .iter()
            .filter_map(|b| b.upgrade())
            .find(|b| b.is_visible());
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
        root.add_css_class("notification-drawer-content");
        let title = gtk::Label::new(Some("Notifications"));
        title.add_css_class("notification-drawer-heading");
        root.append(&title);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_min_content_width(340);
        scroll.set_max_content_height(
            crate::ui::widget_monitor(anchor)
                .map_or(420, |m| (m.geometry().height() - 160).clamp(100, 420)),
        );
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
        let popup = crate::ui::popover(anchor, &root);
        *self.list.borrow_mut() = Some(list);
        *self.drawer.borrow_mut() = Some(popup.clone());
        let weak = Rc::downgrade(self);
        popup.connect_closed(move |_| {
            if let Some(center) = weak.upgrade() {
                center.drawer.borrow_mut().take();
                center.list.borrow_mut().take();
                center.rows.borrow_mut().clear();
            }
        });
        self.refresh();
        popup.popup();
    }
    fn refresh(self: &Rc<Self>) {
        self.refresh_buttons();
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        let entries = self.notifications.borrow();
        let mut rows = self.rows.borrow_mut();
        rows.retain(|id, (view, active, row)| {
            if entries
                .iter()
                .any(|entry| entry.view.id == *id && entry.view == *view && entry.active == *active)
            {
                true
            } else {
                list.remove(row);
                false
            }
        });
        if let Some(empty) = list
            .first_child()
            .filter(|widget| widget.widget_name() == "notifications-empty")
        {
            list.remove(&empty);
        }
        let mut previous: Option<gtk::Box> = None;
        for entry in entries.iter().rev() {
            let (_, _, row) = rows.entry(entry.view.id).or_insert_with(|| {
                let row = self.notification_content(&entry.view, entry.active, false);
                list.append(&row);
                (entry.view.clone(), entry.active, row)
            });
            list.reorder_child_after(row, previous.as_ref());
            previous = Some(row.clone());
        }
        if entries.is_empty() {
            let empty = gtk::Label::new(Some("No notifications"));
            empty.set_widget_name("notifications-empty");
            empty.add_css_class("notification-empty");
            list.append(&empty);
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

fn limited(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[derive(Clone, PartialEq, Eq)]
struct ImageData {
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
    bytes: Vec<u8>,
}
impl ImageData {
    fn decode(
        width: i32,
        height: i32,
        stride: i32,
        alpha: bool,
        bits: i32,
        channels: i32,
        bytes: &[u8],
    ) -> Option<Self> {
        if width <= 0
            || height <= 0
            || width > 4096
            || height > 4096
            || bits != 8
            || channels != if alpha { 4 } else { 3 }
            || stride < width.checked_mul(channels)?
        {
            return None;
        }
        let needed = (height as usize - 1)
            .checked_mul(stride as usize)?
            .checked_add((width * channels) as usize)?;
        if needed > bytes.len() || bytes.len() > 1024 * 1024 {
            return None;
        }
        let scale = f64::from(width.max(height)) / 64.0;
        let w = (f64::from(width) / scale.max(1.0)).round().max(1.0) as i32;
        let h = (f64::from(height) / scale.max(1.0)).round().max(1.0) as i32;
        let mut result = Vec::with_capacity((w * h * channels) as usize);
        for y in 0..h {
            for x in 0..w {
                let source = ((y * height / h) * stride + (x * width / w) * channels) as usize;
                result.extend_from_slice(&bytes[source..source + channels as usize]);
            }
        }
        Some(Self {
            width: w,
            height: h,
            stride: w * channels,
            alpha,
            bytes: result,
        })
    }
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application) {
    let center = NotificationCenter::new(app);
    center.start();
    crate::ui_tests::pump(80);
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    let parameters = (
        "Test application",
        0u32,
        "",
        "D-Bus notification",
        "Body",
        Vec::<String>::new(),
        HashMap::<String, glib::Variant>::new(),
        0i32,
    )
        .to_variant();
    let reply = glib::MainContext::default()
        .block_on(connection.call_future(
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            "Notify",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            1000,
        ))
        .unwrap();
    let dbus_id = reply.child_get::<u32>(0);
    assert!(dbus_id > 0);
    assert!(
        center
            .notifications
            .borrow()
            .iter()
            .any(|entry| entry.view.id == dbus_id)
    );
    glib::MainContext::default()
        .block_on(connection.call_future(
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            "CloseNotification",
            Some(&(dbus_id,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
        ))
        .unwrap();
    assert!(
        !center
            .notifications
            .borrow()
            .iter()
            .any(|entry| entry.view.id == dbus_id)
    );
    let view = || NotificationView {
        id: 0,
        app: "Regression test".into(),
        icon: String::new(),
        summary: "Long heading ".repeat(30),
        body: "Long message ".repeat(100),
        actions: vec![("default".into(), "Open".into())],
        desktop_id: None,
        resident: false,
        transient: false,
        image: None,
    };
    let first = center.notify(0, view(), 0);
    assert_eq!(center.notify(first, view(), 0), first);
    assert_eq!(center.notifications.borrow().len(), 1);
    let content = center.notification_content(&view(), true, true);
    assert!(content.measure(gtk::Orientation::Vertical, 360).1 < 350);
    for _ in 0..crate::config::get().notification_history_limit + 5 {
        center.notify(0, view(), 0);
    }
    assert_eq!(
        center.notifications.borrow().len(),
        crate::config::get().notification_history_limit
    );
    assert!(
        center
            .notifications
            .borrow()
            .iter()
            .filter(|entry| entry.popup.is_some())
            .count()
            <= 4
    );
    let timed = center.notify(0, view(), 10);
    crate::ui_tests::pump(80);
    assert!(
        !center
            .notifications
            .borrow()
            .iter()
            .find(|entry| entry.view.id == timed)
            .unwrap()
            .active
    );
    let mut transient = view();
    transient.transient = true;
    let transient = center.notify(0, transient, 10);
    crate::ui_tests::pump(80);
    assert!(
        !center
            .notifications
            .borrow()
            .iter()
            .any(|entry| entry.view.id == transient)
    );
    let mut resident = view();
    resident.resident = true;
    let resident = center.notify(0, resident, 0);
    center.invoke_action(resident, "default");
    assert!(
        center
            .notifications
            .borrow()
            .iter()
            .find(|entry| entry.view.id == resident)
            .unwrap()
            .active
    );
    center.remove(resident);
    assert!(
        !center
            .notifications
            .borrow()
            .iter()
            .any(|entry| entry.view.id == resident)
    );
    center.clear();
    assert!(center.notifications.borrow().is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_data_rejects_invalid_layouts() {
        assert!(ImageData::decode(2, 2, 3, false, 8, 3, &[0; 12]).is_none());
        assert!(ImageData::decode(2, 2, 6, false, 8, 3, &[0; 11]).is_none());
        assert!(ImageData::decode(1, 1, 4, true, 8, 4, &[0; 4]).is_some());
        assert!(ImageData::decode(i32::MAX, 2, 6, false, 8, 3, &[]).is_none());
    }
    #[test]
    fn input_limits_preserve_utf8() {
        assert_eq!(limited("Привет", 3), "П");
        assert_eq!(limited("hello", 3), "hel");
    }
}
