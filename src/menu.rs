use crate::app::{AppState, LauncherMode};
use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Copy)]
enum Page {
    Home,
    Launcher,
    Bar,
    Modules,
    Trigger,
    Idle,
    Screensaver,
    System,
    Info,
    Bluetooth,
    Weather,
    Calendar,
    Clipboard,
    Todo,
    Reminder,
    Keybindings,
    Appearance,
    Wallpaper,
}

#[derive(Clone)]
enum Action {
    Page(Page),
    Launch(LauncherMode),
    Toggle(&'static str),
    Configure(bool),
    OpenWallpapersFolder,
    SelectWallpaper(String),
    IdleShow,
    Poweroff,
    Reboot,
}

fn entries(page: Page) -> Vec<(String, String, Action)> {
    match page {
        Page::Home => vec![
            ("App launcher", "", Action::Page(Page::Launcher)),
            ("Bar", "", Action::Page(Page::Bar)),
            ("Trigger", "", Action::Page(Page::Trigger)),
            ("Info", "", Action::Page(Page::Info)),
            ("Appearance", "", Action::Page(Page::Appearance)),
            ("Keybindings", "", Action::Page(Page::Keybindings)),
            ("System", "", Action::Page(Page::System)),
        ],
        Page::System => vec![
            ("Poweroff", "", Action::Poweroff),
            ("Reboot", "", Action::Reboot),
            ("Screensaver", "", Action::IdleShow),
        ],
        Page::Trigger => vec![
            ("Bluetooth", "", Action::Page(Page::Bluetooth)),
            ("Todo", "", Action::Page(Page::Todo)),
            ("Clipboard", "", Action::Page(Page::Clipboard)),
            ("Reminder", "", Action::Page(Page::Reminder)),
            ("Idle", "", Action::Page(Page::Idle)),
        ],
        Page::Idle => vec![("Screensaver", "", Action::Page(Page::Screensaver))],
        Page::Screensaver => Vec::new(),
        Page::Appearance => vec![("Wallpaper", "", Action::Page(Page::Wallpaper))],
        Page::Wallpaper => Vec::new(),
        Page::Info => vec![
            ("Weather", "", Action::Page(Page::Weather)),
            ("Calendar", "", Action::Page(Page::Calendar)),
        ],
        Page::Bluetooth
        | Page::Weather
        | Page::Calendar
        | Page::Clipboard
        | Page::Todo
        | Page::Reminder
        | Page::Keybindings => Vec::new(),
        Page::Launcher => vec![
            (
                "Open app launcher",
                "",
                Action::Launch(LauncherMode::Normal),
            ),
            ("Hide/show apps", "", Action::Launch(LauncherMode::Manage)),
            ("Configure", "", Action::Configure(false)),
        ],
        Page::Bar => vec![
            ("Modules", "", Action::Page(Page::Modules)),
            ("Configure", "", Action::Configure(true)),
        ],
        Page::Modules => crate::bar_settings::MODULES
            .iter()
            .map(|&(id, title)| (title, "", Action::Toggle(id)))
            .collect(),
    }
    .into_iter()
    .map(|(title, hint, action)| (title.to_owned(), hint.to_owned(), action))
    .collect()
}

fn wallpaper_folder() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Pictures/Wallpapers")
}

fn wallpaper_script() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/bin/wallpaper.sh")
}

fn wallpaper_files(folder: &Path) -> Result<Vec<String>, String> {
    let mut files = std::fs::read_dir(folder)
        .map_err(|error| format!("{}: {error}", folder.display()))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            let extension = Path::new(&name).extension()?.to_str()?;
            matches!(extension, "jpg" | "png" | "jpeg" | "webp").then_some(name)
        })
        .collect::<Vec<_>>();
    files.sort_unstable();
    Ok(files)
}

fn wallpaper_view(state: &Rc<AppState>) -> gtk::Box {
    wallpaper_view_with(state, wallpaper_folder())
}

fn wallpaper_view_with(state: &Rc<AppState>, folder_path: PathBuf) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(620)
        .child(&strip)
        .build();
    outer.append(&scrolled);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let previous = gtk::Button::with_label("←");
    let next = gtk::Button::with_label("→");
    let apply = gtk::Button::with_label("Apply");
    let folder = gtk::Button::with_label("Open folder");
    for button in [&previous, &next, &apply, &folder] {
        button.add_css_class("network-action");
        controls.append(button);
    }
    outer.append(&controls);
    let message = gtk::Label::new(Some("Loading wallpapers…"));
    message.add_css_class("menu-hint");
    message.set_wrap(true);
    outer.append(&message);
    let gate = gtk::ListBox::new();
    gate.set_visible(false);
    outer.append(&gate);
    let names = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let cards = Rc::new(std::cell::RefCell::new(
        Vec::<glib::WeakRef<gtk::Button>>::new(),
    ));
    let selected = Rc::new(std::cell::Cell::new(0usize));
    let select: Rc<dyn Fn(usize)> = Rc::new({
        let cards = cards.clone();
        let selected = selected.clone();
        let scrolled = scrolled.downgrade();
        move |index| {
            let cards: Vec<_> = cards
                .borrow()
                .iter()
                .filter_map(|card| card.upgrade())
                .collect();
            if cards.is_empty() {
                return;
            }
            let index = index.min(cards.len() - 1);
            selected.set(index);
            for (i, card) in cards.iter().enumerate() {
                if i == index {
                    card.add_css_class("suggested-action");
                } else {
                    card.remove_css_class("suggested-action");
                }
            }
            cards[index].grab_focus();
            if let Some(scrolled) = scrolled.upgrade() {
                let adjustment = scrolled.hadjustment();
                let Some(bounds) = cards[index]
                    .parent()
                    .and_then(|parent| cards[index].compute_bounds(&parent))
                else {
                    return;
                };
                let left = f64::from(bounds.x());
                let right = left + f64::from(bounds.width());
                if left < adjustment.value() {
                    adjustment.set_value(left);
                } else if right > adjustment.value() + adjustment.page_size() {
                    adjustment.set_value(right - adjustment.page_size());
                }
            }
        }
    });
    for (button, forward) in [(&previous, false), (&next, true)] {
        let select = select.clone();
        let selected = selected.clone();
        button.connect_clicked(move |_| {
            select(if forward {
                selected.get().saturating_add(1)
            } else {
                selected.get().saturating_sub(1)
            });
        });
    }
    apply.connect_clicked({
        let names = names.clone();
        let selected = selected.clone();
        let message = message.downgrade();
        let gate = gate.downgrade();
        let busy = state.wallpaper_busy.clone();
        move |_| {
            if let Some(name) = names.borrow().get(selected.get()) {
                run_wallpaper_action(
                    Action::SelectWallpaper(name.clone()),
                    message.clone(),
                    gate.clone(),
                    busy.clone(),
                );
            }
        }
    });
    folder.connect_clicked({
        let message = message.downgrade();
        let gate = gate.downgrade();
        let busy = state.wallpaper_busy.clone();
        move |_| {
            run_wallpaper_action(
                Action::OpenWallpapersFolder,
                message.clone(),
                gate.clone(),
                busy.clone(),
            )
        }
    });
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    key.connect_key_pressed({
        let select = select.clone();
        let selected = selected.clone();
        let apply = apply.downgrade();
        let cards = cards.clone();
        move |_, key, _, _| {
            match key {
                gdk::Key::Left => select(selected.get().saturating_sub(1)),
                gdk::Key::Right => select(selected.get().saturating_add(1)),
                gdk::Key::Return | gdk::Key::KP_Enter
                    if cards
                        .borrow()
                        .iter()
                        .filter_map(|card| card.upgrade())
                        .any(|card| card.has_focus()) =>
                {
                    if let Some(apply) = apply.upgrade() {
                        apply.emit_clicked();
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    outer.add_controller(key);
    let (sender, receiver) = async_channel::bounded(2);
    std::thread::spawn(move || {
        let files = wallpaper_files(&folder_path);
        let Ok(files) = files else {
            let _ = sender.send_blocking(Err(files.unwrap_err()));
            return;
        };
        for name in files {
            let pixels = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(
                folder_path.join(&name),
                240,
                150,
                true,
            )
            .map_err(|error| error.to_string())
            .map(|p| {
                (
                    p.read_pixel_bytes().as_ref().to_vec(),
                    p.width(),
                    p.height(),
                    p.rowstride(),
                    p.has_alpha(),
                )
            });
            if sender.send_blocking(Ok((name, pixels))).is_err() {
                return;
            }
        }
    });
    glib::MainContext::default().spawn_local({
        let strip = strip.downgrade();
        let message = message.downgrade();
        async move {
            while let Ok(result) = receiver.recv().await {
                let (Some(strip), Some(message)) = (strip.upgrade(), message.upgrade()) else {
                    return;
                };
                match result {
                    Ok((name, pixels)) => {
                        let index = names.borrow().len();
                        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
                        let picture = gtk::Picture::new();
                        picture.set_size_request(240, 150);
                        if let Ok((bytes, width, height, stride, alpha)) = pixels {
                            let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
                                &glib::Bytes::from_owned(bytes),
                                gtk::gdk_pixbuf::Colorspace::Rgb,
                                alpha,
                                8,
                                width,
                                height,
                                stride,
                            );
                            picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
                        } else {
                            picture.set_tooltip_text(Some(&format!(
                                "Preview unavailable: {}",
                                pixels.unwrap_err()
                            )));
                        }
                        content.append(&picture);
                        let label = gtk::Label::new(Some(&name));
                        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                        label.set_max_width_chars(24);
                        content.append(&label);
                        let card = gtk::Button::new();
                        card.set_child(Some(&content));
                        card.set_tooltip_text(Some(&name));
                        card.connect_clicked({
                            let select = select.clone();
                            move |_| select(index)
                        });
                        strip.append(&card);
                        names.borrow_mut().push(name);
                        cards.borrow_mut().push(card.downgrade());
                        if index == 0 {
                            select(0);
                        }
                        message.set_text("← / → Browse · Enter Apply");
                    }
                    Err(error) => {
                        message.set_text(&error);
                        message.add_css_class("menu-error");
                    }
                }
            }
            if names.borrow().is_empty()
                && let Some(message) = message.upgrade()
                && !message.has_css_class("menu-error")
            {
                message.set_text("No wallpapers found");
            }
        }
    });
    outer
}

fn run_wallpaper_command(
    script: &Path,
    argument: Option<String>,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let mut command = std::process::Command::new(script);
    if let Some(argument) = argument {
        command.arg(argument);
    }
    crate::process::run_background_command(&mut command, timeout).map(|_| ())
}

fn run_wallpaper_action(
    action: Action,
    message: glib::WeakRef<gtk::Label>,
    list: glib::WeakRef<gtk::ListBox>,
    busy: Rc<std::cell::Cell<bool>>,
) {
    run_wallpaper_action_with(
        action,
        message,
        list,
        busy,
        wallpaper_script(),
        std::time::Duration::from_secs(30),
    );
}

fn run_wallpaper_action_with(
    action: Action,
    message: glib::WeakRef<gtk::Label>,
    list: glib::WeakRef<gtk::ListBox>,
    busy: Rc<std::cell::Cell<bool>>,
    script: PathBuf,
    timeout: std::time::Duration,
) {
    let Some(active_list) = list.upgrade().filter(|list| list.is_sensitive()) else {
        return;
    };
    if busy.replace(true) {
        if let Some(message) = message.upgrade() {
            message.set_text("A wallpaper operation is already running");
            message.set_visible(true);
        }
        return;
    }
    active_list.set_sensitive(false);
    if matches!(action, Action::OpenWallpapersFolder) {
        let uri = gio::File::for_path(wallpaper_folder()).uri();
        glib::MainContext::default().spawn_local(async move {
            let result =
                gio::AppInfo::launch_default_for_uri_future(&uri, gio::AppLaunchContext::NONE)
                    .await;
            busy.set(false);
            if let Some(list) = list.upgrade() {
                list.set_sensitive(true);
            }
            if let Some(message) = message.upgrade() {
                match result {
                    Ok(()) => {
                        message.remove_css_class("menu-error");
                        message.set_text("Wallpapers folder opened");
                    }
                    Err(error) => {
                        message.add_css_class("menu-error");
                        message.set_text(&format!("Open wallpapers folder: {error}"));
                    }
                }
                message.set_visible(true);
            }
        });
        return;
    }
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match action {
            Action::SelectWallpaper(name) => {
                run_wallpaper_command(&script, Some(name), timeout).map(|()| "Wallpaper selected")
            }
            _ => return,
        };
        let _ = sender.send_blocking(result);
    });
    glib::MainContext::default().spawn_local(async move {
        let result = receiver.recv().await;
        busy.set(false);
        if let Some(list) = list.upgrade() {
            list.set_sensitive(true);
        }
        if let Ok(result) = result
            && let Some(message) = message.upgrade()
        {
            match result {
                Ok(text) => {
                    message.remove_css_class("menu-error");
                    message.set_text(text);
                    message.set_visible(true);
                }
                Err(error) => {
                    message.add_css_class("menu-error");
                    message.set_text(&error);
                    message.set_visible(true);
                }
            }
        }
    });
}

fn run_system_action(
    action: Action,
    message: glib::WeakRef<gtk::Label>,
    list: glib::WeakRef<gtk::ListBox>,
    busy: Rc<std::cell::Cell<bool>>,
    program: PathBuf,
) {
    let (argument, text) = match action {
        Action::Poweroff => ("poweroff", "Poweroff requested"),
        Action::Reboot => ("reboot", "Reboot requested"),
        _ => return,
    };
    let Some(active_list) = list.upgrade().filter(|list| list.is_sensitive()) else {
        return;
    };
    if busy.replace(true) {
        if let Some(message) = message.upgrade() {
            message.set_text("A system operation is already running");
            message.set_visible(true);
        }
        return;
    }
    active_list.set_sensitive(false);
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = crate::process::run_command(
            std::process::Command::new(program).args(["--no-ask-password", argument]),
            std::time::Duration::from_secs(30),
        );
        let _ = sender.send_blocking(result);
    });
    glib::MainContext::default().spawn_local(async move {
        let result = receiver.recv().await;
        busy.set(false);
        if let Some(list) = list.upgrade() {
            list.set_sensitive(true);
        }
        if let Some(message) = message.upgrade() {
            match result {
                Ok(Ok(_)) => {
                    message.remove_css_class("menu-error");
                    message.set_text(text);
                }
                Ok(Err(error)) => {
                    message.add_css_class("menu-error");
                    message.set_text(&error);
                }
                Err(_) => {
                    message.add_css_class("menu-error");
                    message.set_text("System operation failed");
                }
            }
            message.set_visible(true);
        }
    });
}

fn first_focusable(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if widget.is_focusable() && widget.is_visible() && widget.is_sensitive() {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = first_focusable(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn focus_is_within(focus: &gtk::Widget, widget: &gtk::Widget) -> bool {
    focus == widget || focus.is_ancestor(widget)
}

fn back_is_focused(window: &impl IsA<gtk::Window>, back: &impl IsA<gtk::Widget>) -> bool {
    gtk::prelude::GtkWindowExt::focus(window.as_ref())
        .is_some_and(|focus| focus_is_within(&focus, back.as_ref()))
}

pub fn close(state: &AppState) {
    let old = state.menu.borrow_mut().take();
    if let Some(old) = old {
        old.close();
    }
}

pub fn show(app: &gtk::Application, state: &Rc<AppState>) {
    show_page(app, state, Page::Home);
}

pub fn show_wallpaper(app: &gtk::Application, state: &Rc<AppState>) {
    if state
        .menu
        .borrow()
        .as_ref()
        .is_some_and(|window| window.title().as_deref() != Some("Wallpaper"))
    {
        close(state);
    }
    show_page(app, state, Page::Wallpaper);
}

pub fn show_clipboard(app: &gtk::Application, state: &Rc<AppState>) {
    let other_page = state
        .menu
        .borrow()
        .as_ref()
        .is_some_and(|window| window.title().as_deref() != Some("Clipboard"));
    if other_page {
        close(state);
    }
    show_page(app, state, Page::Clipboard);
}

fn show_page(app: &gtk::Application, state: &Rc<AppState>, page: Page) {
    if state.menu.borrow().is_some() {
        close(state);
        return;
    }
    crate::ui::close_popover();
    state
        .launcher_generation
        .set(state.launcher_generation.get().wrapping_add(1));
    let launcher = state.launcher.borrow_mut().take();
    if let Some(launcher) = launcher {
        launcher.close();
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("chuh menu")
        .default_width(460)
        .build();
    window.set_widget_name("chuh-menu");
    crate::ui::set_layer_window(
        &window,
        "chuhshell-menu",
        Layer::Overlay,
        &[],
        0,
        KeyboardMode::OnDemand,
    );
    window.set_monitor(crate::ui::active_monitor().as_ref());
    let state_close = Rc::downgrade(state);
    window.connect_close_request(move |_| {
        if let Some(state) = state_close.upgrade() {
            state.menu.borrow_mut().take();
        }
        glib::Propagation::Proceed
    });
    render(&window, state, page);
    *state.menu.borrow_mut() = Some(window.clone().upcast());
    crate::ui::animate_close(&window);
    window.present();
}

fn render(window: &gtk::ApplicationWindow, state: &Rc<AppState>, page: Page) {
    let mut items = entries(page);
    if matches!(page, Page::Idle) {
        for (_, hint, _) in &mut items {
            let timer = crate::idle::timer(state);
            *hint = format!(
                "{} · {} min",
                if timer.enabled { "On" } else { "Off" },
                timer.minutes
            );
        }
    }
    render_entries(window, state, page, items);
}

fn render_entries(
    window: &gtk::ApplicationWindow,
    state: &Rc<AppState>,
    page: Page,
    page_entries: Vec<(String, String, Action)>,
) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("menu-content");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("menu-header");
    let parent = match page {
        Page::Home => None,
        Page::Launcher
        | Page::Bar
        | Page::Trigger
        | Page::System
        | Page::Info
        | Page::Keybindings
        | Page::Appearance => Some(Page::Home),
        Page::Idle => Some(Page::Trigger),
        Page::Screensaver => Some(Page::Idle),
        Page::Wallpaper => Some(Page::Appearance),
        Page::Bluetooth | Page::Clipboard | Page::Todo | Page::Reminder => Some(Page::Trigger),
        Page::Weather | Page::Calendar => Some(Page::Info),
        Page::Modules => Some(Page::Bar),
    };
    let back = parent.map(|parent| {
        let back = gtk::Button::with_label("←");
        back.add_css_class("network-action");
        back.add_css_class("menu-back");
        back.set_tooltip_text(Some("Back"));
        let window = window.downgrade();
        let state = Rc::downgrade(state);
        back.connect_clicked(move |_| {
            if let (Some(window), Some(state)) = (window.upgrade(), state.upgrade()) {
                render(&window, &state, parent);
            }
        });
        header.append(&back);
        back
    });
    if matches!(page, Page::Home) {
        let logo = gtk::DrawingArea::new();
        logo.set_content_width(32);
        logo.set_content_height(32);
        logo.set_valign(gtk::Align::Center);
        logo.set_draw_func(|_, context, width, height| {
            context.translate(f64::from(width) / 2.0, f64::from(height) / 2.0);
            context.set_source_rgb(1.0, 1.0, 1.0);
            context.arc(0.0, 0.0, 15.0, 0.0, std::f64::consts::TAU);
            let _ = context.fill();
            context.set_source_rgb(0.0, 0.0, 0.0);
            context.arc(0.0, 0.0, 13.5, 0.0, std::f64::consts::TAU);
            let _ = context.fill();
            context.set_source_rgb(1.0, 1.0, 1.0);
            context.set_line_width(8.0);
            context.set_line_cap(gtk::cairo::LineCap::Round);
            context.move_to(-3.0, -6.0);
            context.line_to(3.0, -6.0);
            context.line_to(3.0, 3.0);
            context.line_to(-3.0, 3.0);
            context.close_path();
            let _ = context.stroke();
            context.set_source_rgb(0.0, 0.0, 0.0);
            context.rectangle(-5.0, -7.0, 10.0, 6.0);
            let _ = context.fill();
            for x in [-4.0, 4.0] {
                context.arc(x, 3.0, 1.5, 0.0, std::f64::consts::TAU);
                let _ = context.fill();
            }
            context.set_source_rgb(1.0, 1.0, 1.0);
            context.set_line_width(1.5);
            context.move_to(-4.0, 7.0);
            context.line_to(-7.0, 11.0);
            context.move_to(4.0, 7.0);
            context.line_to(7.0, 11.0);
            context.move_to(-5.5, 9.0);
            context.line_to(5.5, 9.0);
            let _ = context.stroke();
        });
        header.append(&logo);
    }
    let title = gtk::Label::new(Some(match page {
        Page::Keybindings => "Keybindings",
        Page::Idle => "Idle",
        Page::Screensaver => "Screensaver",
        Page::Trigger => "Trigger",
        Page::System => "System",
        Page::Info => "Info",
        Page::Bluetooth => "Bluetooth",
        Page::Weather => "Weather",
        Page::Calendar => "Calendar",
        Page::Clipboard => "Clipboard",
        Page::Todo => "Todo",
        Page::Reminder => "Reminder",
        Page::Home => "chuh menu",
        Page::Launcher => "App launcher",
        Page::Bar => "Bar",
        Page::Modules => "Modules",
        Page::Appearance => "Appearance",
        Page::Wallpaper => "Wallpaper",
    }));
    window.set_title(Some(&title.text()));
    title.add_css_class("menu-heading");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    header.append(&title);
    outer.append(&header);
    let keybindings = matches!(page, Page::Keybindings).then(crate::keybindings::view);
    let leaf = match page {
        Page::Screensaver => Some(crate::idle::view(state)),
        Page::Wallpaper => Some(wallpaper_view(state)),
        Page::Keybindings => keybindings.as_ref().map(|view| view.widget.clone()),
        Page::Bluetooth => Some(crate::bluetooth::view()),
        Page::Weather => Some(crate::weather::view()),
        Page::Calendar => Some(crate::info::calendar()),
        Page::Todo => Some(crate::todo::view()),
        Page::Reminder => Some(crate::reminder::view(state)),
        Page::Clipboard => {
            let weak = Rc::downgrade(state);
            Some(state.clipboard.view(move || {
                if let Some(state) = weak.upgrade() {
                    close(&state);
                }
            }))
        }
        _ => None,
    };
    if let Some(leaf) = leaf {
        outer.append(&leaf);
        let key = gtk::EventControllerKey::new();
        key.set_name(Some("menu-leaf-navigation"));
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_window = window.downgrade();
        let weak_state = Rc::downgrade(state);
        let weak_leaf = leaf.downgrade();
        let weak_back = back.as_ref().map(|back| back.downgrade());
        key.connect_key_pressed(move |_, raw_key, keycode, modifiers| {
            if let Some(view) = keybindings.as_ref()
                && view.handle_key(raw_key, keycode, modifiers) == glib::Propagation::Stop
            {
                return glib::Propagation::Stop;
            }
            let key = crate::keybindings::remap("menu", raw_key, modifiers);
            let (Some(window), Some(state)) = (weak_window.upgrade(), weak_state.upgrade()) else {
                return glib::Propagation::Proceed;
            };
            if raw_key == gdk::Key::Escape || key == gdk::Key::Escape {
                close(&state);
                return glib::Propagation::Stop;
            }
            let Some(back) = weak_back.as_ref().and_then(|back| back.upgrade()) else {
                return glib::Propagation::Proceed;
            };
            if key == gdk::Key::Home && raw_key != gdk::Key::Left && raw_key != gdk::Key::Right {
                back.grab_focus();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Up
                && let Some(leaf) = weak_leaf.upgrade()
                && let Some(first) = first_focusable(leaf.upcast_ref())
                && gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|focus| focus_is_within(&focus, &first))
            {
                back.grab_focus();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Down && back_is_focused(&window, &back) {
                if let Some(leaf) = weak_leaf.upgrade()
                    && let Some(first) = first_focusable(leaf.upcast_ref())
                {
                    first.grab_focus();
                }
                return glib::Propagation::Stop;
            }
            if (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
                && back_is_focused(&window, &back)
            {
                back.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        outer.add_controller(key);
        window.set_child(Some(&outer));
        leaf.child_focus(gtk::DirectionType::TabForward);
        return;
    }
    let max_height =
        crate::ui::active_monitor().map_or(420, |m| (m.geometry().height() - 230).clamp(100, 420));
    let scrolled = gtk::ScrolledWindow::builder()
        .min_content_height(0)
        .max_content_height(max_height)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let list = gtk::ListBox::new();
    list.add_css_class("menu-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let items = Rc::new(page_entries);
    let mut statuses = Vec::new();
    for (title, hint, action) in items.iter() {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let label = gtk::Label::new(Some(title));
        label.add_css_class("menu-title");
        label.set_xalign(0.0);
        text.append(&label);
        if !hint.is_empty() {
            let hint = gtk::Label::new(Some(hint));
            hint.add_css_class("menu-hint");
            hint.set_xalign(0.0);
            text.append(&hint);
        }
        content.append(&text);
        let status = gtk::Label::new(Some(match action {
            Action::Toggle(id) => {
                if state.bar_modules.enabled(id) {
                    "On"
                } else {
                    "Off"
                }
            }
            Action::Page(_) => "›",
            _ => "",
        }));
        status.add_css_class("menu-state");
        if matches!(action, Action::Toggle(id) if state.bar_modules.enabled(id)) {
            status.add_css_class("enabled");
        }
        content.append(&status);
        statuses.push(status.downgrade());
        row.set_child(Some(&content));
        list.append(&row);
    }
    list.connect_selected_rows_changed({
        let scrolled = scrolled.downgrade();
        move |list| {
            if let Some(scrolled) = scrolled.upgrade() {
                crate::launcher::scroll_selected_row_into_view(list, &scrolled);
            }
        }
    });
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);
    let message = gtk::Label::new(None);
    message.set_visible(false);
    message.add_css_class("menu-hint");
    message.set_wrap(true);
    outer.append(&message);
    let weak_list = list.downgrade();
    list.connect_row_activated({
        let window = window.downgrade();
        let state = Rc::downgrade(state);
        let message = message.downgrade();
        move |list, row| {
            if !list.is_sensitive()
                || !row.is_sensitive()
                || !row.is_visible()
                || !row.is_child_visible()
            {
                return;
            }
            let (Some(window), Some(state)) = (window.upgrade(), state.upgrade()) else {
                return;
            };
            let Some((_, _, action)) = items.get(row.index() as usize) else {
                return;
            };
            match action.clone() {
                Action::Page(page) => render(&window, &state, page),
                Action::Launch(mode) => {
                    if let Some(app) = window.application() {
                        close(&state);
                        crate::launcher::show(&app, &state, mode);
                    }
                }
                Action::Toggle(id) => {
                    if !row.is_sensitive() {
                        return;
                    }
                    row.set_sensitive(false);
                    let row = row.downgrade();
                    let status = statuses[row.upgrade().unwrap().index() as usize].clone();
                    let message = message.clone();
                    glib::MainContext::default().spawn_local(async move {
                        let result = state.bar_modules.toggle(id).await;
                        if let Some(row) = row.upgrade() {
                            row.set_sensitive(true);
                        }
                        match result {
                            Ok(enabled) => {
                                if let Some(status) = status.upgrade() {
                                    status.set_text(if enabled { "On" } else { "Off" });
                                    if enabled {
                                        status.add_css_class("enabled");
                                    } else {
                                        status.remove_css_class("enabled");
                                    }
                                }
                                if let Some(message) = message.upgrade() {
                                    message.set_visible(false);
                                }
                            }
                            Err(error) => {
                                if let Some(message) = message.upgrade() {
                                    message.add_css_class("menu-error");
                                    message.set_text(&error);
                                    message.set_visible(true);
                                }
                            }
                        }
                    });
                }
                Action::Configure(bar) => {
                    if let Some(app) = window.application() {
                        close(&state);
                        crate::layout::show(&app, &state, bar);
                    }
                }
                Action::IdleShow => match crate::idle::show(&state) {
                    Ok(()) => close(&state),
                    Err(error) => {
                        if let Some(message) = message.upgrade() {
                            message.set_text(&error);
                            message.set_visible(true);
                        }
                    }
                },
                action @ (Action::Poweroff | Action::Reboot) => {
                    if let Some(message) = message.upgrade() {
                        message.set_visible(false);
                        run_system_action(
                            action,
                            message.downgrade(),
                            weak_list.clone(),
                            state.system_busy.clone(),
                            PathBuf::from("systemctl"),
                        );
                    }
                }
                action @ (Action::OpenWallpapersFolder | Action::SelectWallpaper(_)) => {
                    if let Some(message) = message.upgrade() {
                        message.set_visible(false);
                        run_wallpaper_action(
                            action,
                            message.downgrade(),
                            weak_list.clone(),
                            state.wallpaper_busy.clone(),
                        );
                    }
                }
            }
        }
    });
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    key.connect_key_pressed({
        let list = list.downgrade();
        let window = window.downgrade();
        let state = Rc::downgrade(state);
        let back = back.as_ref().map(|back| back.downgrade());
        move |_, raw_key, _, modifiers| {
            let default_key = crate::keybindings::is_default("menu", raw_key, modifiers);
            let key = crate::keybindings::remap("menu", raw_key, modifiers);
            let (Some(window), Some(list), Some(state)) =
                (window.upgrade(), list.upgrade(), state.upgrade())
            else {
                return glib::Propagation::Proceed;
            };
            if raw_key == gdk::Key::Escape || key == gdk::Key::Escape {
                close(&state);
                return glib::Propagation::Stop;
            }
            let back = back.as_ref().and_then(|back| back.upgrade());
            if back
                .as_ref()
                .is_some_and(|back| back_is_focused(&window, back))
            {
                match key {
                    gdk::Key::Down => {
                        let rows = crate::launcher::visible_rows(&list);
                        list.select_row(rows.first());
                        list.grab_focus();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        if let Some(back) = back {
                            back.emit_clicked();
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Left | gdk::Key::Right | gdk::Key::Up => {
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            if raw_key == gdk::Key::Left || raw_key == gdk::Key::Right {
                return glib::Propagation::Stop;
            }
            match key {
                gdk::Key::Home => {
                    if let Some(back) = back {
                        list.unselect_all();
                        back.grab_focus();
                    }
                }
                gdk::Key::Up | gdk::Key::Down | gdk::Key::Page_Down | gdk::Key::Page_Up => {
                    let rows = crate::launcher::visible_rows(&list);
                    let selected = rows
                        .iter()
                        .position(|row| Some(row) == list.selected_row().as_ref());
                    if key == gdk::Key::Up
                        && selected == Some(0)
                        && let Some(back) = back
                    {
                        list.unselect_all();
                        back.grab_focus();
                    } else {
                        let offset = match key {
                            gdk::Key::Up => -1,
                            gdk::Key::Page_Up => -5,
                            gdk::Key::Page_Down => 5,
                            _ => 1,
                        };
                        if let Some(index) =
                            crate::launcher::selection_index(rows.len(), selected, offset)
                        {
                            list.select_row(rows.get(index));
                        }
                    }
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if let Some(row) = list
                        .selected_row()
                        .filter(|row| row.is_child_visible() && row.is_visible())
                    {
                        list.emit_by_name::<()>("row-activated", &[&row]);
                    }
                }
                _ if default_key => return glib::Propagation::Stop,
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    outer.add_controller(key);
    window.set_child(Some(&outer));
    list.select_row(crate::launcher::visible_rows(&list).first());
    list.grab_focus();
    let list = list.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(list) = list.upgrade() {
            list.select_row(crate::launcher::visible_rows(&list).first());
            list.grab_focus();
        }
    });
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application, state: &Rc<AppState>) {
    use std::os::unix::fs::PermissionsExt;
    let system_script =
        std::env::temp_dir().join(format!("chuhshell-system-{}", std::process::id()));
    std::fs::write(
        &system_script,
        "#!/bin/sh\nsleep 0.1\n[ \"$#\" = 2 ] && [ \"$1\" = --no-ask-password ] || exit 9\ncase \"$2\" in\npoweroff) printf 'poweroff test failure' >&2; exit 7;;\nreboot) exit 0;;\n*) exit 9;;\nesac\n",
    )
    .unwrap();
    std::fs::set_permissions(&system_script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let system_list = gtk::ListBox::new();
    let system_message = gtk::Label::new(None);
    run_system_action(
        Action::Poweroff,
        system_message.downgrade(),
        system_list.downgrade(),
        state.system_busy.clone(),
        system_script.clone(),
    );
    assert!(!system_list.is_sensitive());
    let other_system_list = gtk::ListBox::new();
    run_system_action(
        Action::Reboot,
        system_message.downgrade(),
        other_system_list.downgrade(),
        state.system_busy.clone(),
        system_script.clone(),
    );
    assert!(other_system_list.is_sensitive());
    assert_eq!(
        system_message.text(),
        "A system operation is already running"
    );
    crate::ui_tests::pump(300);
    assert!(system_list.is_sensitive());
    assert!(!state.system_busy.get());
    assert!(system_message.text().contains("poweroff test failure"));
    assert!(system_message.has_css_class("menu-error"));
    run_system_action(
        Action::Reboot,
        system_message.downgrade(),
        system_list.downgrade(),
        state.system_busy.clone(),
        system_script.clone(),
    );
    crate::ui_tests::pump(300);
    assert_eq!(system_message.text(), "Reboot requested");
    assert!(!system_message.has_css_class("menu-error"));
    assert!(system_list.is_sensitive());
    std::fs::remove_file(system_script).unwrap();
    let script = std::env::temp_dir().join(format!("chuhshell-wallpaper-{}", std::process::id()));
    std::fs::write(
        &script,
        "#!/bin/sh\nsleep 0.1\nprintf 'wallpaper test failure' >&2\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let wallpaper_list = gtk::ListBox::new();
    let wallpaper_message = gtk::Label::new(None);
    run_wallpaper_action_with(
        Action::SelectWallpaper("test.png".into()),
        wallpaper_message.downgrade(),
        wallpaper_list.downgrade(),
        state.wallpaper_busy.clone(),
        script.clone(),
        std::time::Duration::from_secs(1),
    );
    assert!(!wallpaper_list.is_sensitive());
    run_wallpaper_action_with(
        Action::SelectWallpaper("test.png".into()),
        wallpaper_message.downgrade(),
        wallpaper_list.downgrade(),
        state.wallpaper_busy.clone(),
        script.clone(),
        std::time::Duration::from_secs(1),
    );
    let other_list = gtk::ListBox::new();
    run_wallpaper_action_with(
        Action::SelectWallpaper("test.png".into()),
        wallpaper_message.downgrade(),
        other_list.downgrade(),
        state.wallpaper_busy.clone(),
        script.clone(),
        std::time::Duration::from_secs(1),
    );
    assert!(other_list.is_sensitive());
    assert!(wallpaper_message.text().contains("already running"));
    crate::ui_tests::pump(300);
    assert!(wallpaper_list.is_sensitive());
    assert!(wallpaper_message.text().contains("wallpaper test failure"));
    std::fs::write(&script, "#!/bin/sh\nsleep 300 &\nwait\n").unwrap();
    run_wallpaper_action_with(
        Action::SelectWallpaper("test.png".into()),
        wallpaper_message.downgrade(),
        wallpaper_list.downgrade(),
        state.wallpaper_busy.clone(),
        script.clone(),
        std::time::Duration::from_millis(50),
    );
    crate::ui_tests::pump(200);
    assert!(wallpaper_list.is_sensitive());
    assert!(wallpaper_message.text().contains("timed out"));
    std::fs::remove_file(script).unwrap();

    fn list(window: &gtk::Window) -> gtk::ListBox {
        let outer = window.child().unwrap();
        outer
            .first_child()
            .unwrap()
            .next_sibling()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::Viewport>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::ListBox>()
            .unwrap()
    }
    fn press(window: &gtk::Window, key: gdk::Key) {
        let controllers = window.child().unwrap().observe_controllers();
        for i in 0..controllers.n_items() {
            if let Some(controller) = controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
            {
                assert_eq!(
                    controller.propagation_phase(),
                    gtk::PropagationPhase::Capture
                );
                controller.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0u32, &gdk::ModifierType::empty()],
                );
                return;
            }
        }
        panic!("Missing menu keyboard controller");
    }
    fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
        if widget.has_css_class(class) {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = find(&widget, class) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }
    let previews = std::env::temp_dir().join(format!("chuhshell-previews-{}", std::process::id()));
    std::fs::create_dir_all(&previews).unwrap();
    let pixbuf =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 32, 20).unwrap();
    pixbuf.fill(0x667788ff);
    for name in ["a.png", "b.png", "c.png"] {
        pixbuf.savev(previews.join(name), "png", &[]).unwrap();
    }
    let view = wallpaper_view_with(state, previews.clone());
    let preview_window = gtk::ApplicationWindow::builder()
        .application(app)
        .child(&view)
        .build();
    preview_window.present();
    crate::ui_tests::pump(200);
    let scroll = view
        .first_child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let strip = scroll
        .child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Box>()
        .unwrap();
    let first = strip
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    assert!(first.has_css_class("suggested-action"));
    let picture = first
        .child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Picture>()
        .unwrap();
    assert!(
        picture.paintable().is_some(),
        "{:?}",
        picture.tooltip_text()
    );
    let controller = view
        .observe_controllers()
        .item(0)
        .unwrap()
        .downcast::<gtk::EventControllerKey>()
        .unwrap();
    controller.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Right, &0u32, &gdk::ModifierType::empty()],
    );
    let second = first.next_sibling().unwrap();
    assert!(second.has_css_class("suggested-action"));
    assert!(!first.has_css_class("suggested-action"));
    controller.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Left, &0u32, &gdk::ModifierType::empty()],
    );
    assert!(first.has_css_class("suggested-action"));
    let weak_first = first.downgrade();
    preview_window.close();
    gtk::prelude::GtkWindowExt::set_focus(&preview_window, gtk::Widget::NONE);
    preview_window.set_child(gtk::Widget::NONE);
    drop(preview_window);
    drop(controller);
    drop(picture);
    drop(first);
    drop(second);
    drop(strip);
    drop(scroll);
    drop(view);
    crate::ui_tests::pump(350);
    assert!(weak_first.upgrade().is_none());
    std::fs::remove_dir_all(previews).unwrap();
    show(app, state);
    crate::ui_tests::pump(100);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    assert!(!window.is_anchor(gtk4_layer_shell::Edge::Top));
    crate::ui_tests::capture("menu");
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 7);
    let home = list(&window);
    home.select_row(home.row_at_index(6).as_ref());
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(window.title().as_deref(), Some("System"));
    let system = list(&window);
    assert_eq!(crate::launcher::visible_rows(&system).len(), 3);
    let row = system.row_at_index(2).unwrap();
    assert!(!row.has_css_class("menu-wip"));
    assert!(matches!(entries(Page::System)[2].2, Action::IdleShow));
    crate::ui_tests::capture("system");
    press(&window, gdk::Key::Home);
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(window.title().as_deref(), Some("chuh menu"));
    press(&window, gdk::Key::Right);
    assert_eq!(window.title().as_deref(), Some("chuh menu"));
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(window.title().as_deref(), Some("App launcher"));
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 3);
    press(&window, gdk::Key::Left);
    assert_eq!(window.title().as_deref(), Some("App launcher"));
    press(&window, gdk::Key::Up);
    let back = find(window.upcast_ref(), "menu-back").unwrap();
    assert!(back_is_focused(&window, &back));
    press(&window, gdk::Key::Down);
    assert_eq!(list(&window).selected_row().unwrap().index(), 0);
    press(&window, gdk::Key::Up);
    assert!(back_is_focused(&window, &back));
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(window.title().as_deref(), Some("chuh menu"));
    press(&window, gdk::Key::Down);
    assert_eq!(list(&window).selected_row().unwrap().index(), 1);
    press(&window, gdk::Key::Right);
    assert_eq!(window.title().as_deref(), Some("chuh menu"));
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 2);
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 10);
    let modules = list(&window);
    let row = modules.row_at_index(4).unwrap();
    modules.select_row(Some(&row));
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert!(!state.bar_modules.enabled("audio"));
    assert!(
        crate::config::read()
            .unwrap()
            .disabled_modules
            .contains(&"audio".into())
    );
    assert_eq!(
        crate::config::read().unwrap().notification_history_limit,
        10
    );
    for (_, bar) in state.bars.borrow().iter() {
        let audio = find(bar.upcast_ref(), "audio").unwrap();
        assert!(!audio.parent().unwrap().is_visible());
        audio.set_visible(false);
        audio.set_visible(true);
        assert!(!audio.parent().unwrap().is_visible());
    }
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert!(state.bar_modules.enabled("audio"));
    for (_, bar) in state.bars.borrow().iter() {
        let audio = find(bar.upcast_ref(), "audio").unwrap();
        assert!(audio.parent().unwrap().is_visible());
    }
    press(&window, gdk::Key::Page_Down);
    assert_eq!(modules.selected_row().unwrap().index(), 9);
    press(&window, gdk::Key::Page_Up);
    assert_eq!(modules.selected_row().unwrap().index(), 4);
    press(&window, gdk::Key::Escape);
    assert!(state.menu.borrow().is_none());
    show(app, state);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    let menu = list(&window);
    menu.select_row(menu.row_at_index(2).as_ref());
    press(&window, gdk::Key::Return);
    assert_eq!(list(&window).row_at_index(0).unwrap().index(), 0);
    assert_eq!(window.title().as_deref(), Some("Trigger"));
    let trigger = list(&window);
    trigger.select_row(trigger.row_at_index(4).as_ref());
    press(&window, gdk::Key::Return);
    assert_eq!(window.title().as_deref(), Some("Idle"));
    {
        let (index, title) = (0, "Screensaver");
        let idle = list(&window);
        idle.select_row(idle.row_at_index(index).as_ref());
        press(&window, gdk::Key::Return);
        assert_eq!(window.title().as_deref(), Some(title));
        crate::ui_tests::capture("idle-screensaver-settings");
        find(window.upcast_ref(), "menu-back")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert_eq!(window.title().as_deref(), Some("Idle"));
    }
    find(window.upcast_ref(), "menu-back")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    list(&window).select_row(list(&window).row_at_index(3).as_ref());
    press(&window, gdk::Key::Return);
    assert_eq!(window.title().as_deref(), Some("Reminder"));
    crate::reminder::regression_checks(&window, state);
    find(window.upcast_ref(), "menu-back")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert_eq!(window.title().as_deref(), Some("Trigger"));
    list(&window).select_row(list(&window).row_at_index(0).as_ref());
    press(&window, gdk::Key::Left);
    assert_eq!(window.title().as_deref(), Some("Trigger"));
    press(&window, gdk::Key::Up);
    assert!(back_is_focused(
        &window,
        &find(window.upcast_ref(), "menu-back").unwrap()
    ));
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    let menu = list(&window);
    menu.select_row(menu.row_at_index(3).as_ref());
    press(&window, gdk::Key::Return);
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 2);
    let menu = list(&window);
    menu.select_row(menu.row_at_index(1).as_ref());
    press(&window, gdk::Key::Return);
    assert!(find(window.upcast_ref(), "calendar").is_some());
    crate::ui_tests::pump(100);
    crate::ui_tests::capture("calendar");
    press(&window, gdk::Key::Left);
    assert_eq!(window.title().as_deref(), Some("Calendar"));
    let leaf = window.child().unwrap().last_child().unwrap();
    first_focusable(&leaf).unwrap().grab_focus();
    press(&window, gdk::Key::Up);
    assert!(back_is_focused(
        &window,
        &find(window.upcast_ref(), "menu-back").unwrap()
    ));
    press(&window, gdk::Key::Return);
    assert_eq!(window.title().as_deref(), Some("Info"));
    find(window.upcast_ref(), "menu-back")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    let home = list(&window);
    home.select_row(home.row_at_index(2).as_ref());
    press(&window, gdk::Key::Return);
    let trigger = list(&window);
    trigger.select_row(trigger.row_at_index(1).as_ref());
    press(&window, gdk::Key::Return);
    assert_eq!(window.title().as_deref(), Some("Todo"));
    crate::ui_tests::pump(100);
    let entry = find(window.upcast_ref(), "todo-entry")
        .unwrap()
        .downcast::<gtk::Entry>()
        .unwrap();
    entry.set_text("Test task");
    find(window.upcast_ref(), "todo-add")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    crate::ui_tests::pump(100);
    assert_eq!(crate::todo::load(&crate::todo::path()).unwrap().len(), 1);
    let check = find(window.upcast_ref(), "todo-check")
        .unwrap()
        .downcast::<gtk::CheckButton>()
        .unwrap();
    check.set_active(true);
    crate::ui_tests::pump(100);
    assert!(crate::todo::load(&crate::todo::path()).unwrap()[0].done);
    find(window.upcast_ref(), "todo-delete")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    crate::ui_tests::pump(100);
    assert!(crate::todo::load(&crate::todo::path()).unwrap().is_empty());
    let back = find(window.upcast_ref(), "menu-back").unwrap();
    back.downcast::<gtk::Button>().unwrap().emit_clicked();
    assert_eq!(window.title().as_deref(), Some("Trigger"));
    show_clipboard(app, state);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    assert_eq!(window.title().as_deref(), Some("Clipboard"));
    let leaf = window.child().unwrap().last_child().unwrap();
    first_focusable(&leaf).unwrap().grab_focus();
    press(&window, gdk::Key::Up);
    assert!(back_is_focused(
        &window,
        &find(window.upcast_ref(), "menu-back").unwrap()
    ));
    press(&window, gdk::Key::Down);
    assert!(!back_is_focused(
        &window,
        &find(window.upcast_ref(), "menu-back").unwrap()
    ));
    press(&window, gdk::Key::Escape);
    assert!(state.menu.borrow().is_none());
    show_clipboard(app, state);
    assert_eq!(
        state.menu.borrow().as_ref().unwrap().title().as_deref(),
        Some("Clipboard")
    );
    show_clipboard(app, state);
    assert!(state.menu.borrow().is_none());
    show(app, state);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    let menu = list(&window);
    menu.select_row(menu.row_at_index(5).as_ref());
    press(&window, gdk::Key::Return);
    assert_eq!(window.title().as_deref(), Some("Keybindings"));
    assert!(find(window.upcast_ref(), "keybinding-editor").is_some());
    let leaf = window.child().unwrap().last_child().unwrap();
    first_focusable(&leaf).unwrap().grab_focus();
    press(&window, gdk::Key::Up);
    assert!(back_is_focused(
        &window,
        &find(window.upcast_ref(), "menu-back").unwrap()
    ));
    press(&window, gdk::Key::Down);
    assert!(!back_is_focused(
        &window,
        &find(window.upcast_ref(), "menu-back").unwrap()
    ));
    crate::ui_tests::pump(200);
    let rows = find(window.upcast_ref(), "keybinding-row").unwrap();
    let edit = rows
        .last_child()
        .unwrap()
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    edit.emit_clicked();
    let editor = find(window.upcast_ref(), "keybinding-editor").unwrap();
    assert!(editor.is_visible());
    let input = editor
        .first_child()
        .unwrap()
        .next_sibling()
        .unwrap()
        .downcast::<gtk::Entry>()
        .unwrap();
    let record = editor
        .last_child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    record.emit_clicked();
    press(&window, gdk::Key::Escape);
    assert!(state.menu.borrow().is_some());
    assert!(editor.is_visible());
    record.emit_clicked();
    press(&window, gdk::Key::Home);
    assert_eq!(input.text(), "Home");
    assert!(state.menu.borrow().is_some());
    record.emit_clicked();
    press(&window, gdk::Key::Escape);
    assert!(editor.is_visible());
    press(&window, gdk::Key::Escape);
    assert!(state.menu.borrow().is_some());
    assert!(!editor.is_visible());
    press(&window, gdk::Key::Escape);
    assert!(state.menu.borrow().is_none());
    crate::launcher::show(app, state, LauncherMode::Manage);
    crate::ui_tests::pump(350);
    let launcher = state.launcher.borrow().as_ref().unwrap().clone();
    assert!(!launcher.is_anchor(gtk4_layer_shell::Edge::Top));
    launcher.close();
    crate::ui_tests::pump(100);
    for (_, bar) in state.bars.borrow().iter() {
        let audio = find(bar.upcast_ref(), "audio").unwrap();
        for class in ["clock", "background-apps-toggle", "notification-toggle"] {
            let button = find(bar.upcast_ref(), class).unwrap();
            assert_eq!(
                button.height(),
                audio.height(),
                "Unequal module heights: {class}"
            );
            let a = button.compute_bounds(bar).unwrap();
            let b = audio.compute_bounds(bar).unwrap();
            assert!((a.y() + a.height() / 2.0 - b.y() - b.height() / 2.0).abs() < 1.0);
        }
    }
}
