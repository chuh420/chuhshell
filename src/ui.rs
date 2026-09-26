use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;

const EDGES: [layer_shell::Edge; 4] = [
    layer_shell::Edge::Top,
    layer_shell::Edge::Bottom,
    layer_shell::Edge::Left,
    layer_shell::Edge::Right,
];

pub fn set_layer_window(
    window: &impl IsA<gtk::Window>,
    namespace: &str,
    layer: layer_shell::Layer,
    anchors: &[layer_shell::Edge],
    exclusive: i32,
    keyboard: layer_shell::KeyboardMode,
) {
    window.init_layer_shell();
    window.set_namespace(Some(namespace));
    window.set_layer(layer);
    for edge in EDGES {
        window.set_anchor(edge, anchors.contains(&edge));
    }
    window.set_exclusive_zone(exclusive);
    window.set_keyboard_mode(keyboard);
}

thread_local! {
    static ACTIVE_OUTPUT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static OPEN_MENU: std::cell::RefCell<Option<glib::WeakRef<gtk::Popover>>> = const { std::cell::RefCell::new(None) };
}

pub fn set_active_output(output: Option<&str>) {
    ACTIVE_OUTPUT.with(|value| *value.borrow_mut() = output.map(str::to_owned));
}

pub fn active_monitor() -> Option<gtk::gdk::Monitor> {
    let monitors = gtk::gdk::Display::default()?.monitors();
    let output = ACTIVE_OUTPUT.with(|value| value.borrow().clone());
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
        .find(|monitor| monitor.connector().as_deref() == output.as_deref())
        .or_else(|| monitors.item(0).and_downcast())
}

pub fn widget_monitor(widget: &impl IsA<gtk::Widget>) -> Option<gtk::gdk::Monitor> {
    widget
        .native()
        .and_then(|native| native.surface())
        .and_then(|surface| surface.display().monitor_at_surface(&surface))
        .or_else(active_monitor)
}

pub fn popover(anchor: &gtk::Button, content: &impl IsA<gtk::Widget>) -> gtk::Popover {
    let previous =
        OPEN_MENU.with(|value| value.borrow_mut().take().and_then(|weak| weak.upgrade()));
    if let Some(previous) = previous {
        previous.popdown();
    }
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_autohide(true);
    popover.set_parent(anchor);
    popover.set_child(Some(content));
    popover.connect_closed(|popover| popover.unparent());
    OPEN_MENU.with(|value| *value.borrow_mut() = Some(popover.downgrade()));
    popover
}

pub fn image(icon: &str, size: i32) -> gtk::Image {
    let path = if icon.starts_with("file://") {
        gio::File::for_uri(icon).path()
    } else if std::path::Path::new(icon).is_absolute() {
        Some(std::path::PathBuf::from(icon))
    } else {
        None
    };
    if let Some(path) = path {
        if path
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() <= 2 * 1024 * 1024)
            && let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true)
        {
            return gtk::Image::from_pixbuf(Some(&pixbuf));
        }
        return gtk::Image::from_icon_name("application-x-executable-symbolic");
    }
    gtk::Image::from_icon_name(if icon.is_empty() {
        "application-x-executable-symbolic"
    } else {
        icon
    })
}
