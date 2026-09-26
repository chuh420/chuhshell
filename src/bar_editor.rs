use crate::app::AppState;
use crate::bar_settings::{MODULES, ModuleOrder};
use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Editor {
    state: std::rc::Weak<AppState>,
    slots: glib::WeakRef<gtk::Box>,
    selected: RefCell<Option<String>>,
    status: glib::WeakRef<gtk::Label>,
}

impl Editor {
    fn select(&self, id: &str) {
        *self.selected.borrow_mut() = Some(id.to_owned());
        if let Some(status) = self.status.upgrade() {
            let title = MODULES
                .iter()
                .find(|(name, _)| *name == id)
                .map_or(id, |(_, title)| title);
            status.set_text(&format!("{title} · Drop or select an insertion point"));
        }
    }

    fn place(self: &Rc<Self>, id: &str, zone: usize, index: usize) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        if !MODULES.iter().any(|(name, _)| *name == id) {
            return false;
        }
        let mut order = state.bar_modules.order();
        order.place(id, zone, index);
        crate::ui::close_popover();
        state.bar_modules.preview(order);
        self.select(id);
        let editor = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(editor) = editor.upgrade() {
                editor.render();
            }
        });
        true
    }

    fn render(self: &Rc<Self>) {
        let (Some(state), Some(slots)) = (self.state.upgrade(), self.slots.upgrade()) else {
            return;
        };
        while let Some(child) = slots.first_child() {
            slots.remove(&child);
        }
        let order = state.bar_modules.order();
        for (zone, (title, ids)) in ["Left", "Center", "Right"]
            .into_iter()
            .zip(order.groups())
            .enumerate()
        {
            let group = gtk::Box::new(gtk::Orientation::Vertical, 6);
            group.add_css_class("bar-edit-zone");
            let title = gtk::Label::new(Some(title));
            title.add_css_class("menu-heading");
            group.append(&title);
            let points = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            points.set_halign(match zone {
                0 => gtk::Align::Start,
                1 => gtk::Align::Center,
                _ => gtk::Align::End,
            });
            for index in 0..=ids.len() {
                let point = gtk::Button::with_label("+");
                point.add_css_class("bar-insert");
                point.set_tooltip_text(Some("Insert module here"));
                point.set_widget_name(&format!("bar-slot-{zone}-{index}"));
                let editor = Rc::downgrade(self);
                point.connect_clicked(move |_| {
                    if let Some(editor) = editor.upgrade() {
                        let selected = editor.selected.borrow().clone();
                        if let Some(id) = selected {
                            editor.place(&id, zone, index);
                        }
                    }
                });
                let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
                target.connect_drop({
                    let editor = Rc::downgrade(self);
                    move |_, value, _, _| {
                        let (Some(editor), Ok(id)) = (editor.upgrade(), value.get::<String>())
                        else {
                            return false;
                        };
                        editor.place(&id, zone, index)
                    }
                });
                point.add_controller(target);
                points.append(&point);
                if let Some(id) = ids.get(index) {
                    let name = MODULES
                        .iter()
                        .find(|(name, _)| name == id)
                        .map_or(id.as_str(), |(_, title)| title);
                    let label = gtk::Label::new(Some(name));
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_max_width_chars(8);
                    label.set_tooltip_text(Some(name));
                    if !state.bar_modules.enabled(id) {
                        label.add_css_class("menu-hint");
                    }
                    points.append(&label);
                }
            }
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .min_content_height(48)
                .build();
            scroll.set_child(Some(&points));
            group.append(&scroll);
            slots.append(&group);
        }
    }
}

pub fn show(app: &gtk::Application, state: &Rc<AppState>) {
    crate::menu::close(state);
    crate::ui::close_popover();
    state
        .launcher_generation
        .set(state.launcher_generation.get().wrapping_add(1));
    let launcher = state.launcher.borrow_mut().take();
    if let Some(window) = launcher {
        window.close();
    }
    let active = crate::ui::active_monitor();
    let target = {
        let bars = state.bars.borrow();
        bars.iter()
            .find(|(monitor, _)| Some(monitor) == active.as_ref())
            .or_else(|| bars.first())
            .cloned()
    };
    let Some((monitor, bar)) = target else {
        return;
    };
    let original = state.bar_modules.order();
    let committed = Rc::new(Cell::new(false));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Configure bar modules")
        .build();
    window.set_widget_name("layout-editor");
    crate::ui::set_layer_window(
        &window,
        "chuhshell-bar-editor",
        Layer::Overlay,
        &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right],
        -1,
        KeyboardMode::Exclusive,
    );
    window.set_monitor(Some(&monitor));
    let overlay = gtk::Overlay::new();
    let capture = gtk::Box::new(gtk::Orientation::Vertical, 0);
    capture.set_hexpand(true);
    capture.set_vexpand(true);
    overlay.set_child(Some(&capture));
    let slots = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    slots.set_homogeneous(true);
    slots.set_halign(gtk::Align::Fill);
    slots.set_valign(gtk::Align::Start);
    slots.set_margin_top(bar.surface().map_or(37, |surface| surface.height()) + 8);
    slots.set_margin_start(8);
    slots.set_margin_end(8);
    overlay.add_overlay(&slots);
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 10);
    controls.add_css_class("menu-content");
    controls.set_halign(gtk::Align::Center);
    controls.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Arrange bar modules"));
    title.add_css_class("menu-heading");
    controls.append(&title);
    let hint = gtk::Label::new(Some(
        "Drag a module from the bar to a + slot.\nYou can also select a module below, then select a slot.",
    ));
    controls.append(&hint);
    let status = gtk::Label::new(Some("Panel size and position are fixed"));
    status.add_css_class("menu-hint");
    controls.append(&status);
    let editor = Rc::new(Editor {
        state: Rc::downgrade(state),
        slots: slots.downgrade(),
        selected: RefCell::new(None),
        status: status.downgrade(),
    });
    editor.render();
    let modules = gtk::Grid::new();
    modules.set_column_spacing(6);
    modules.set_row_spacing(6);
    for (index, (id, title)) in MODULES.iter().enumerate() {
        let button = gtk::Button::with_label(title);
        button.set_widget_name(&format!("bar-module-{id}"));
        button.add_css_class("network-action");
        if !state.bar_modules.enabled(id) {
            button.set_tooltip_text(Some("Hidden module"));
        }
        button.connect_clicked({
            let editor = editor.clone();
            move |_| editor.select(id)
        });
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        source.connect_prepare({
            let editor = editor.clone();
            move |_, _, _| {
                editor.select(id);
                Some(gdk::ContentProvider::for_value(&id.to_value()))
            }
        });
        button.add_controller(source);
        modules.attach(&button, index as i32 % 2, index as i32 / 2, 1, 1);
    }
    controls.append(&modules);
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    source.connect_prepare({
        let state = Rc::downgrade(state);
        let bar = bar.downgrade();
        let editor = editor.clone();
        move |source, x, y| {
            let (Some(state), Some(bar)) = (state.upgrade(), bar.upgrade()) else {
                return None;
            };
            let (id, module) = state.bar_modules.module_at(&bar, x, y)?;
            editor.select(&id);
            source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&module))), 0, 0);
            Some(gdk::ContentProvider::for_value(&id.to_value()))
        }
    });
    capture.add_controller(source);
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion({
        let state = Rc::downgrade(state);
        let bar = bar.downgrade();
        let capture = capture.downgrade();
        move |_, x, y| {
            if let (Some(state), Some(bar), Some(capture)) =
                (state.upgrade(), bar.upgrade(), capture.upgrade())
            {
                capture.set_cursor_from_name(Some(
                    if state.bar_modules.module_at(&bar, x, y).is_some() {
                        "grab"
                    } else {
                        "default"
                    },
                ));
            }
        }
    });
    capture.add_controller(motion);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::Center);
    let reset = gtk::Button::with_label("Reset");
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    for button in [&reset, &cancel, &save] {
        button.add_css_class("network-action");
        buttons.append(button);
    }
    reset.connect_clicked({
        let editor = editor.clone();
        move |_| {
            if let Some(state) = editor.state.upgrade() {
                state.bar_modules.preview(ModuleOrder::default());
                editor.render();
            }
        }
    });
    cancel.connect_clicked({
        let window = window.downgrade();
        move |_| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        }
    });
    save.connect_clicked({
        let state = Rc::downgrade(state);
        let window = window.downgrade();
        let status = status.downgrade();
        let committed = committed.clone();
        move |_| {
            if let (Some(state), Some(window)) = (state.upgrade(), window.upgrade()) {
                match state.bar_modules.save_order() {
                    Ok(()) => {
                        committed.set(true);
                        window.close();
                    }
                    Err(error) => {
                        if let Some(status) = status.upgrade() {
                            status.set_text(&error);
                            status.add_css_class("menu-error");
                        }
                    }
                }
            }
        }
    });
    controls.append(&buttons);
    overlay.add_overlay(&controls);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let window = window.downgrade();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(window) = window.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    window.add_controller(keys);
    window.connect_close_request({
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.menu.borrow_mut().take();
                if !committed.get() {
                    state.bar_modules.preview(original.clone());
                }
            }
            glib::Propagation::Proceed
        }
    });
    window.set_child(Some(&overlay));
    *state.menu.borrow_mut() = Some(window.clone().upcast());
    window.present();
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application, state: &Rc<AppState>) {
    fn find(widget: &gtk::Widget, name: &str) -> gtk::Widget {
        fn search(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
            if widget.widget_name() == name
                || widget
                    .downcast_ref::<gtk::Button>()
                    .is_some_and(|button| button.label().as_deref() == Some(name))
            {
                return Some(widget.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if let Some(found) = search(&widget, name) {
                    return Some(found);
                }
            }
            None
        }
        search(widget, name).unwrap_or_else(|| panic!("Missing {name}"))
    }
    fn click(window: &gtk::Window, name: &str) {
        find(window.upcast_ref(), name)
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        crate::ui_tests::pump(100);
    }
    let snapshots: Vec<_> = state
        .bars
        .borrow()
        .iter()
        .map(|(_, bar)| {
            (
                bar.clone(),
                bar.child().unwrap(),
                bar.surface().unwrap().width(),
                bar.surface().unwrap().height(),
                bar.exclusive_zone(),
            )
        })
        .collect();
    let original = state.bar_modules.order();
    show(app, state);
    crate::ui_tests::pump(100);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    click(&window, "bar-module-audio");
    click(&window, "bar-slot-0-0");
    assert_eq!(state.bar_modules.order().left.first().unwrap(), "audio");
    assert!(!state.bar_modules.order().right.contains(&"audio".into()));
    for (bar, child, width, height, zone) in &snapshots {
        assert_eq!(bar.child().as_ref(), Some(child));
        assert_eq!(bar.surface().unwrap().width(), *width);
        assert_eq!(bar.surface().unwrap().height(), *height);
        assert_eq!(bar.exclusive_zone(), *zone);
        assert!(bar.is_anchor(Edge::Top));
        assert!(!bar.is_anchor(Edge::Bottom));
        let layout = child.clone().downcast::<gtk::CenterBox>().unwrap();
        let scroll = layout
            .start_widget()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let group = scroll
            .child()
            .unwrap()
            .downcast::<gtk::Viewport>()
            .unwrap()
            .child()
            .unwrap();
        let audio = group.first_child().unwrap();
        assert!(audio.first_child().unwrap().has_css_class("audio"));
    }
    let capture = window
        .child()
        .unwrap()
        .downcast::<gtk::Overlay>()
        .unwrap()
        .child()
        .unwrap();
    let controllers = capture.observe_controllers();
    let source = (0..controllers.n_items())
        .find_map(|i| controllers.item(i).and_downcast::<gtk::DragSource>())
        .unwrap();
    let bar = &snapshots[0].0;
    let layout = bar.child().unwrap().downcast::<gtk::CenterBox>().unwrap();
    let scroll = layout
        .start_widget()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let group = scroll
        .child()
        .unwrap()
        .downcast::<gtk::Viewport>()
        .unwrap()
        .child()
        .unwrap();
    let audio = group.first_child().unwrap();
    let bounds = audio.compute_bounds(bar).unwrap();
    let provider = source.emit_by_name::<Option<gdk::ContentProvider>>(
        "prepare",
        &[
            &f64::from(bounds.x() + bounds.width() / 2.0),
            &f64::from(bounds.y() + bounds.height() / 2.0),
        ],
    );
    assert!(
        provider
            .unwrap()
            .formats()
            .contains_type(String::static_type())
    );
    assert!(
        source
            .emit_by_name::<Option<gdk::ContentProvider>>("prepare", &[&0.0f64, &300.0f64])
            .is_none()
    );
    click(&window, "Save");
    assert_eq!(
        crate::config::read()
            .unwrap()
            .bar_order
            .left
            .first()
            .unwrap(),
        "audio"
    );
    assert_eq!(
        crate::config::read().unwrap().notification_history_limit,
        10
    );
    let saved = state.bar_modules.order();
    show(app, state);
    crate::ui_tests::pump(100);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    click(&window, "Reset");
    assert_eq!(state.bar_modules.order(), original);
    click(&window, "Cancel");
    assert_eq!(state.bar_modules.order(), saved);
    show(app, state);
    crate::ui_tests::pump(100);
    let window = state.menu.borrow().as_ref().unwrap().clone();
    let slot = find(window.upcast_ref(), "bar-slot-1-0");
    let controllers = slot.observe_controllers();
    let target = (0..controllers.n_items())
        .find_map(|i| controllers.item(i).and_downcast::<gtk::DropTarget>())
        .unwrap();
    assert!(target.emit_by_name::<bool>(
        "drop",
        &[&glib::BoxedValue("audio".to_value()), &0.0f64, &0.0f64]
    ));
    crate::ui_tests::pump(100);
    assert_eq!(state.bar_modules.order().center.first().unwrap(), "audio");
    assert!(!target.emit_by_name::<bool>(
        "drop",
        &[
            &glib::BoxedValue("invalid-module".to_value()),
            &0.0f64,
            &0.0f64
        ]
    ));
    let controllers = window.observe_controllers();
    let keys = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .unwrap();
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Escape, &0u32, &gdk::ModifierType::empty()],
    );
    crate::ui_tests::pump(100);
    assert!(state.menu.borrow().is_none());
    assert_eq!(state.bar_modules.order(), saved);
    state.bar_modules.toggle("audio").unwrap();
    let mut order = saved.clone();
    order.place("audio", 1, 0);
    state.bar_modules.preview(order);
    crate::ui_tests::pump(100);
    for (bar, _, _, _, _) in &snapshots {
        let layout = bar.child().unwrap().downcast::<gtk::CenterBox>().unwrap();
        let scroll = layout
            .center_widget()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let group = scroll
            .child()
            .unwrap()
            .downcast::<gtk::Viewport>()
            .unwrap()
            .child()
            .unwrap();
        let audio = group.first_child().unwrap();
        assert!(!audio.get_visible());
    }
    state.bar_modules.toggle("audio").unwrap();
    state.bar_modules.preview(saved);
}
