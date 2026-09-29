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

pub fn close_popover() {
    let previous =
        OPEN_MENU.with(|value| value.borrow_mut().take().and_then(|weak| weak.upgrade()));
    if let Some(previous) = previous {
        previous.popdown();
    }
}

pub fn popover(anchor: &gtk::Button, content: &impl IsA<gtk::Widget>) -> gtk::Popover {
    close_popover();
    let popover = gtk::Popover::new();
    popover.add_css_class("shell-popover");
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_autohide(true);
    popover.set_parent(anchor);
    popover.set_child(Some(content));
    anchor.add_css_class("popup-open");
    let weak_anchor = anchor.downgrade();
    popover.connect_closed(move |popover| {
        if let Some(anchor) = weak_anchor.upgrade() {
            anchor.remove_css_class("popup-open");
        }
        popover.unparent();
    });
    OPEN_MENU.with(|value| *value.borrow_mut() = Some(popover.downgrade()));
    popover
}

pub fn attach_to_bar(popover: &gtk::Popover, anchor: &gtk::Button) {
    popover.add_css_class("bar-attached");
    if let Some(root) = anchor.root().and_downcast::<gtk::Window>()
        && let Some(origin) = anchor.compute_point(&root, &gtk::graphene::Point::new(0.0, 0.0))
    {
        let height = (root.height() as f32 - origin.y()).ceil() as i32;
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            0,
            0,
            anchor.width(),
            height.max(anchor.height()),
        )));
        popover.set_offset(0, -1);
    }
}

pub fn animate_close(window: &impl IsA<gtk::Window>) {
    let window = window.as_ref();
    window.connect_close_request(|window| {
        if !window.is_mapped()
            || !gtk::Settings::default().is_some_and(|settings| settings.is_gtk_enable_animations())
        {
            return glib::Propagation::Proceed;
        }
        if window.has_css_class("shell-closing") {
            return glib::Propagation::Stop;
        }
        window.add_css_class("shell-closing");
        window.set_can_target(false);
        if window.is_layer_window() {
            window.set_keyboard_mode(layer_shell::KeyboardMode::None);
        }
        let window = window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            window.destroy();
        });
        glib::Propagation::Stop
    });
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
        let image = gtk::Image::from_icon_name("application-x-executable-symbolic");
        image.set_pixel_size(size);
        let (reply, result) = async_channel::bounded(1);
        if icon_worker()
            .try_send(IconRequest { path, size, reply })
            .is_ok()
        {
            let weak = image.downgrade();
            glib::MainContext::default().spawn_local(async move {
                if let Ok(Some(pixels)) = result.recv().await
                    && let Some(image) = weak.upgrade()
                {
                    let bytes = glib::Bytes::from(pixels.bytes.as_slice());
                    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
                        &bytes,
                        gtk::gdk_pixbuf::Colorspace::Rgb,
                        pixels.alpha,
                        8,
                        pixels.width,
                        pixels.height,
                        pixels.stride,
                    );
                    image.set_from_pixbuf(Some(&pixbuf));
                }
            });
        }
        return image;
    }
    gtk::Image::from_icon_name(if icon.is_empty() {
        "application-x-executable-symbolic"
    } else {
        icon
    })
}

struct IconPixels {
    bytes: Vec<u8>,
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
}
struct IconRequest {
    path: std::path::PathBuf,
    size: i32,
    reply: async_channel::Sender<Option<std::sync::Arc<IconPixels>>>,
}
fn icon_worker() -> &'static async_channel::Sender<IconRequest> {
    static WORKER: std::sync::OnceLock<async_channel::Sender<IconRequest>> =
        std::sync::OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = async_channel::bounded::<IconRequest>(128);
        std::thread::spawn(move || {
            let mut cache = std::collections::VecDeque::new();
            while let Ok(request) = rx.recv_blocking() {
                let key = (request.path.clone(), request.size);
                let cached = cache
                    .iter()
                    .find(|(old, time, _): &&(_, std::time::Instant, _)| {
                        old == &key && time.elapsed().as_secs() < 60
                    })
                    .map(|(_, _, pixels)| std::sync::Arc::clone(pixels));
                let pixels = cached.or_else(|| {
                    let metadata = request.path.metadata().ok()?;
                    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
                        return None;
                    }
                    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(
                        &request.path,
                        request.size,
                        request.size,
                        true,
                    )
                    .ok()?;
                    let pixels = std::sync::Arc::new(IconPixels {
                        bytes: pixbuf.read_pixel_bytes().as_ref().to_vec(),
                        width: pixbuf.width(),
                        height: pixbuf.height(),
                        stride: pixbuf.rowstride(),
                        alpha: pixbuf.has_alpha(),
                    });
                    cache.retain(|(old, _, _)| old != &key);
                    if cache.len() >= 128 {
                        cache.pop_front();
                    }
                    cache.push_back((key, std::time::Instant::now(), pixels.clone()));
                    Some(pixels)
                });
                let _ = request.reply.try_send(pixels);
            }
        });
        tx
    })
}

pub fn active_button(buttons: &[glib::WeakRef<gtk::Button>]) -> Option<gtk::Button> {
    let active = active_monitor();
    let visible: Vec<_> = buttons
        .iter()
        .filter_map(|b| b.upgrade())
        .filter(|b| b.is_visible() && b.is_mapped())
        .collect();
    visible
        .iter()
        .find(|button| widget_monitor(*button) == active)
        .cloned()
        .or_else(|| visible.first().cloned())
}
