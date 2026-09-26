use crate::app::{AppState, LauncherMode};
use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::rc::Rc;

#[derive(Clone, Copy)]
enum Page {
    Home,
    Launcher,
    Bar,
    Modules,
}

#[derive(Clone, Copy)]
enum Action {
    Page(Page),
    Launch(LauncherMode),
    Toggle(&'static str),
    Wip,
}

fn entries(page: Page) -> Vec<(&'static str, &'static str, Action)> {
    match page {
        Page::Home => vec![
            ("App launcher", "", Action::Page(Page::Launcher)),
            ("Bar", "", Action::Page(Page::Bar)),
            ("Settings", "WIP", Action::Wip),
            ("Appearance", "WIP", Action::Wip),
            ("Keybindings", "WIP", Action::Wip),
            ("System", "WIP", Action::Wip),
        ],
        Page::Launcher => vec![
            (
                "Open app launcher",
                "Mod+D",
                Action::Launch(LauncherMode::Normal),
            ),
            ("Hide/show apps", "", Action::Launch(LauncherMode::Manage)),
            ("Configure", "WIP", Action::Wip),
        ],
        Page::Bar => vec![
            ("Modules", "", Action::Page(Page::Modules)),
            ("Order", "WIP", Action::Wip),
            ("Configure", "WIP", Action::Wip),
        ],
        Page::Modules => crate::bar_settings::MODULES
            .iter()
            .map(|&(id, title)| (title, "", Action::Toggle(id)))
            .collect(),
    }
}

pub fn close(state: &AppState) {
    let old = state.menu.borrow_mut().take();
    if let Some(old) = old {
        old.close();
    }
}

pub fn show(app: &gtk::Application, state: &Rc<AppState>) {
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
    render(&window, state, Page::Home);
    *state.menu.borrow_mut() = Some(window.clone().upcast());
    window.present();
}

fn render(window: &gtk::ApplicationWindow, state: &Rc<AppState>, page: Page) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    outer.add_css_class("menu-content");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let parent = match page {
        Page::Home => None,
        Page::Launcher | Page::Bar => Some(Page::Home),
        Page::Modules => Some(Page::Bar),
    };
    if let Some(parent) = parent {
        let back = gtk::Button::with_label("←");
        back.add_css_class("network-action");
        back.add_css_class("menu-back");
        back.set_tooltip_text(Some("Back · Alt+Left"));
        let window = window.downgrade();
        let state = Rc::downgrade(state);
        back.connect_clicked(move |_| {
            if let (Some(window), Some(state)) = (window.upgrade(), state.upgrade()) {
                render(&window, &state, parent);
            }
        });
        header.append(&back);
    }
    let title = gtk::Label::new(Some(match page {
        Page::Home => "chuh menu",
        Page::Launcher => "App launcher",
        Page::Bar => "Bar",
        Page::Modules => "Modules",
    }));
    title.add_css_class("menu-heading");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    header.append(&title);
    outer.append(&header);
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
    let items = Rc::new(entries(page));
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
            match *action {
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
                Action::Wip => {}
            }
        }
    });
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    key.connect_key_pressed({
        let list = list.downgrade();
        let window = window.downgrade();
        let state = Rc::downgrade(state);
        move |_, key, _, modifiers| {
            let (Some(window), Some(list), Some(state)) =
                (window.upgrade(), list.upgrade(), state.upgrade())
            else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Escape => close(&state),
                gdk::Key::Left if modifiers.contains(gdk::ModifierType::ALT_MASK) => {
                    if let Some(parent) = parent {
                        render(&window, &state, parent);
                    }
                }
                gdk::Key::Down | gdk::Key::Up | gdk::Key::Page_Down | gdk::Key::Page_Up => {
                    let offset = match key {
                        gdk::Key::Up => -1,
                        gdk::Key::Page_Up => -5,
                        gdk::Key::Page_Down => 5,
                        _ => 1,
                    };
                    let rows = crate::launcher::visible_rows(&list);
                    let selected = rows
                        .iter()
                        .position(|row| Some(row) == list.selected_row().as_ref());
                    if let Some(index) =
                        crate::launcher::selection_index(rows.len(), selected, offset)
                    {
                        list.select_row(rows.get(index));
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
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 6);
    press(&window, gdk::Key::Down);
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 3);
    press(&window, gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(crate::launcher::visible_rows(&list(&window)).len(), 10);
    let list = list(&window);
    let row = list.row_at_index(4).unwrap();
    list.select_row(Some(&row));
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
    assert_eq!(list.selected_row().unwrap().index(), 9);
    press(&window, gdk::Key::Page_Up);
    assert_eq!(list.selected_row().unwrap().index(), 4);
    press(&window, gdk::Key::Escape);
    assert!(state.menu.borrow().is_none());
    show(app, state);
    show(app, state);
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
