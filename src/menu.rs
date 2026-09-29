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
    Settings,
    Info,
    Bluetooth,
    Weather,
    Calendar,
    Clipboard,
    Keybindings,
    Appearance,
    Wallpaper,
    SelectWallpaper,
}

#[derive(Clone)]
enum Action {
    Page(Page),
    Launch(LauncherMode),
    Toggle(&'static str),
    Configure(bool),
    NextWallpaper,
    OpenWallpapersFolder,
    SelectWallpaper(String),
    Notice,
    Wip,
}

fn entries(page: Page) -> Vec<(String, String, Action)> {
    match page {
        Page::Home => vec![
            ("App launcher", "", Action::Page(Page::Launcher)),
            ("Bar", "", Action::Page(Page::Bar)),
            ("Settings", "", Action::Page(Page::Settings)),
            ("Info", "", Action::Page(Page::Info)),
            ("Appearance", "", Action::Page(Page::Appearance)),
            ("Keybindings", "", Action::Page(Page::Keybindings)),
            ("System", "WIP", Action::Wip),
        ],
        Page::Settings => vec![("Bluetooth", "", Action::Page(Page::Bluetooth))],
        Page::Appearance => vec![("Wallpaper", "", Action::Page(Page::Wallpaper))],
        Page::Wallpaper => vec![
            ("Next wallpaper", "", Action::NextWallpaper),
            ("Select wallpaper", "", Action::Page(Page::SelectWallpaper)),
            ("Open wallpapers folder", "", Action::OpenWallpapersFolder),
        ],
        Page::Info => vec![
            ("Weather", "", Action::Page(Page::Weather)),
            ("Calendar", "", Action::Page(Page::Calendar)),
            ("Clipboard", "", Action::Page(Page::Clipboard)),
        ],
        Page::Bluetooth
        | Page::Weather
        | Page::Calendar
        | Page::Clipboard
        | Page::Keybindings
        | Page::SelectWallpaper => Vec::new(),
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

fn run_wallpaper_command(argument: Option<String>) -> Result<(), String> {
    let mut command = std::process::Command::new(wallpaper_script());
    if let Some(argument) = argument {
        command.arg(argument);
    }
    let status = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("Wallpaper script: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Wallpaper script exited with {status}"))
    }
}

fn run_wallpaper_action(
    action: Action,
    message: glib::WeakRef<gtk::Label>,
    list: glib::WeakRef<gtk::ListBox>,
) {
    if let Some(list) = list.upgrade() {
        list.set_sensitive(false);
    }
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match action {
            Action::NextWallpaper => {
                run_wallpaper_command(Some("--next".to_owned())).map(|()| "Wallpaper changed")
            }
            Action::SelectWallpaper(name) => {
                run_wallpaper_command(Some(name)).map(|()| "Wallpaper selected")
            }
            Action::OpenWallpapersFolder => std::process::Command::new("xdg-open")
                .arg(wallpaper_folder())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|error| format!("Open wallpapers folder: {error}"))
                .and_then(|status| {
                    status
                        .success()
                        .then_some("Wallpapers folder opened")
                        .ok_or_else(|| format!("Open wallpapers folder: {status}"))
                }),
            _ => return,
        };
        let _ = sender.send_blocking(result);
    });
    glib::MainContext::default().spawn_local(async move {
        let result = receiver.recv().await;
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
    if matches!(page, Page::SelectWallpaper) {
        render_entries(
            window,
            state,
            page,
            vec![("Loading wallpapers…".into(), String::new(), Action::Notice)],
        );
        let current = window.child().map(|child| child.downgrade());
        let weak_window = window.downgrade();
        let weak_state = Rc::downgrade(state);
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(wallpaper_files(&wallpaper_folder()));
        });
        glib::MainContext::default().spawn_local(async move {
            let (Ok(result), Some(window), Some(state), Some(current)) = (
                receiver.recv().await,
                weak_window.upgrade(),
                weak_state.upgrade(),
                current.and_then(|child| child.upgrade()),
            ) else {
                return;
            };
            if window.child().as_ref() != Some(&current) {
                return;
            }
            match result {
                Ok(files) if !files.is_empty() => render_entries(
                    &window,
                    &state,
                    page,
                    files
                        .into_iter()
                        .map(|name| (name.clone(), String::new(), Action::SelectWallpaper(name)))
                        .collect(),
                ),
                Ok(_) => render_entries(
                    &window,
                    &state,
                    page,
                    vec![("No wallpapers found".into(), String::new(), Action::Notice)],
                ),
                Err(error) => render_entries(
                    &window,
                    &state,
                    page,
                    vec![(error, String::new(), Action::Notice)],
                ),
            }
        });
    } else {
        render_entries(window, state, page, entries(page));
    }
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
        | Page::Settings
        | Page::Info
        | Page::Keybindings
        | Page::Appearance => Some(Page::Home),
        Page::Wallpaper => Some(Page::Appearance),
        Page::SelectWallpaper => Some(Page::Wallpaper),
        Page::Bluetooth => Some(Page::Settings),
        Page::Weather | Page::Calendar | Page::Clipboard => Some(Page::Info),
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
    let title = gtk::Label::new(Some(match page {
        Page::Keybindings => "Keybindings",
        Page::Settings => "Settings",
        Page::Info => "Info",
        Page::Bluetooth => "Bluetooth",
        Page::Weather => "Weather",
        Page::Calendar => "Calendar",
        Page::Clipboard => "Clipboard",
        Page::Home => "chuh menu",
        Page::Launcher => "App launcher",
        Page::Bar => "Bar",
        Page::Modules => "Modules",
        Page::Appearance => "Appearance",
        Page::Wallpaper => "Wallpaper",
        Page::SelectWallpaper => "Select wallpaper",
    }));
    window.set_title(Some(&title.text()));
    title.add_css_class("menu-heading");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    header.append(&title);
    outer.append(&header);
    let leaf = match page {
        Page::Keybindings => Some(crate::keybindings::view()),
        Page::Bluetooth => Some(crate::bluetooth::view()),
        Page::Weather => Some(crate::weather::view()),
        Page::Calendar => Some(crate::info::calendar()),
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
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_window = window.downgrade();
        let weak_state = Rc::downgrade(state);
        let weak_leaf = leaf.downgrade();
        let weak_back = back.as_ref().map(|back| back.downgrade());
        key.connect_key_pressed(move |_, raw_key, _, modifiers| {
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
        if !hint.is_empty() && !matches!(action, Action::Wip) {
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
            Action::Wip => "WIP",
            Action::Notice => "",
            _ => "",
        }));
        status.add_css_class("menu-state");
        if matches!(action, Action::Wip) {
            row.add_css_class("menu-wip");
            status.add_css_class("menu-hint");
        }
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
        move |_, row| {
            if !row.is_visible() || !row.is_child_visible() {
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
                Action::Toggle(id) => match state.bar_modules.toggle(id) {
                    Ok(enabled) => {
                        if let Some(status) = statuses[row.index() as usize].upgrade() {
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
                },
                Action::Configure(bar) => {
                    if let Some(app) = window.application() {
                        close(&state);
                        crate::layout::show(&app, &state, bar);
                    }
                }
                Action::Wip | Action::Notice => {}
                action @ (Action::NextWallpaper
                | Action::OpenWallpapersFolder
                | Action::SelectWallpaper(_)) => {
                    if let Some(message) = message.upgrade() {
                        message.set_visible(false);
                        run_wallpaper_action(action, message.downgrade(), weak_list.clone());
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
            let default_key = crate::keybindings::is_default("menu", raw_key);
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
            child = widget.next_sibling();
            if let Some(found) = find(&widget, class) {
                return Some(found);
            }
        }
        None
    }
    show(app, state);
    crate::ui_tests::pump(100);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    assert!(!window.is_anchor(gtk4_layer_shell::Edge::Top));
    crate::ui_tests::capture("menu");
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 7);
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
    assert_eq!(window.title().as_deref(), Some("Settings"));
    press(&window, gdk::Key::Left);
    assert_eq!(window.title().as_deref(), Some("Settings"));
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
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 3);
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
