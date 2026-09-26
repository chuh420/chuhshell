use crate::app::AppState;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub height: i32,
    #[serde(rename = "bottom", skip_serializing)]
    pub _legacy_bottom: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.0,
            height: 500,
            _legacy_bottom: false,
        }
    }
}

pub struct Layouts {
    pub launcher: Cell<Option<Geometry>>,
}

impl Default for Layouts {
    fn default() -> Self {
        Self {
            launcher: Cell::new(crate::config::get().launcher_layout),
        }
    }
}

impl Geometry {
    fn bounded(self, width: i32, height: i32) -> (Self, i32, i32, i32) {
        let min_height = 180;
        let max_height = height.max(1);
        let h = self
            .height
            .clamp(min_height.min(max_height), max_height.max(1));
        let w = 520.min(width);
        let x = if self.x.is_finite() {
            self.x.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let y = if self.y.is_finite() {
            self.y.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let left = (x * f64::from(width - w)).round() as i32;
        let top = (y * f64::from(height - h)).round() as i32;
        (
            Self {
                x,
                y,
                height: h,
                ..self
            },
            left,
            top,
            w,
        )
    }
}

pub fn apply_launcher(window: &impl IsA<gtk::Window>, geometry: Geometry) {
    let Some(monitor) = window.monitor().or_else(crate::ui::active_monitor) else {
        return;
    };
    let bounds = monitor.geometry();
    let (geometry, x, y, width) = geometry.bounded(bounds.width(), bounds.height());
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Left, x);
    window.set_margin(Edge::Top, y);
    window.set_exclusive_zone(-1);
    window.set_default_size(width, geometry.height);
}

fn save(geometry: Geometry) -> Result<(), String> {
    crate::config::save_value("launcher_layout", serde_json::json!(geometry))
}

pub fn show(app: &gtk::Application, state: &Rc<AppState>, bar: bool) {
    if bar {
        crate::bar_editor::show(app, state);
    } else {
        crate::launcher::configure(app, state);
    }
}

pub fn show_ready(app: &gtk::Application, state: &Rc<AppState>) {
    crate::menu::close(state);
    crate::ui::close_popover();
    let monitor = state
        .launcher
        .borrow()
        .as_ref()
        .and_then(|window| window.monitor());
    let Some(monitor) = monitor else {
        return;
    };
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Configure app launcher")
        .build();
    window.set_widget_name("layout-editor");
    crate::ui::set_layer_window(
        &window,
        "chuhshell-layout-editor",
        Layer::Overlay,
        &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right],
        -1,
        KeyboardMode::Exclusive,
    );
    window.set_monitor(Some(&monitor));
    let overlay = gtk::Overlay::new();
    let canvas = gtk::DrawingArea::new();
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    overlay.set_child(Some(&canvas));
    let initial = {
        let launcher = state.launcher.borrow();
        let Some(launcher) = launcher.as_ref() else {
            return;
        };
        let mut geometry = state.layouts.launcher.get().unwrap_or_default();
        geometry.height = launcher
            .surface()
            .map_or(geometry.height, |surface| surface.height());
        if state.layouts.launcher.get().is_none() {
            let top = state
                .bars
                .borrow()
                .iter()
                .filter(|(output, bar)| output == &monitor && bar.is_anchor(Edge::Top))
                .map(|(_, bar)| bar.exclusive_zone().max(0))
                .max()
                .unwrap_or(0);
            geometry.y =
                f64::from(top) / f64::from((monitor.geometry().height() - geometry.height).max(1));
        }
        if let Some(scrolled) = launcher
            .child()
            .and_then(|outer| outer.last_child())
            .and_downcast::<gtk::ScrolledWindow>()
        {
            scrolled.set_min_content_height(0);
            scrolled.set_propagate_natural_height(false);
            scrolled.set_vexpand(true);
        }
        apply_launcher(launcher, geometry);
        geometry
    };
    let geometry = Rc::new(Cell::new(initial));
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 10);
    controls.add_css_class("menu-content");
    controls.set_halign(gtk::Align::Center);
    controls.set_valign(gtk::Align::Center);
    controls.set_margin_top(12);
    controls.set_margin_bottom(12);
    let title = gtk::Label::new(Some("Configure app launcher"));
    title.add_css_class("menu-heading");
    controls.append(&title);
    let hint = gtk::Label::new(Some("Drag to move · Drag the bottom edge to resize"));
    hint.add_css_class("menu-hint");
    controls.append(&hint);
    let status = gtk::Label::new(None);
    controls.append(&status);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::Center);
    let reset = gtk::Button::with_label("Reset");
    let cancel = gtk::Button::with_label("Cancel");
    let apply = gtk::Button::with_label("Save");
    for button in [&reset, &cancel, &apply] {
        button.add_css_class("network-action");
        buttons.append(button);
    }
    controls.append(&buttons);
    overlay.add_overlay(&controls);
    let update: Rc<dyn Fn(i32, i32)> = Rc::new({
        let state = Rc::downgrade(state);
        let geometry = geometry.clone();
        let controls = controls.downgrade();
        let status = status.downgrade();
        move |width, height| {
            let (g, x, y, w) = geometry.get().bounded(width, height);
            if let Some(state) = state.upgrade()
                && let Some(window) = state.launcher.borrow().as_ref()
            {
                apply_launcher(window, g);
            }
            if let Some(controls) = controls.upgrade() {
                controls.set_valign(if y + g.height / 2 < height / 2 {
                    gtk::Align::End
                } else {
                    gtk::Align::Start
                });
            }
            if let Some(status) = status.upgrade() {
                status.set_text(&format!("{} × {} · x {} · y {}", w, g.height, x, y));
            }
        }
    });
    canvas.connect_resize({
        let update = update.clone();
        move |_, width, height| update(width, height)
    });
    canvas.set_draw_func({
        let geometry = geometry.clone();
        move |canvas, cr, width, height| {
            let (g, x, y, w) = geometry.get().bounded(width, height);
            let color = |name: &str| {
                let rgba = canvas
                    .style_context()
                    .lookup_color(name)
                    .unwrap_or(gtk::gdk::RGBA::BLACK);
                cr.set_source_rgba(
                    f64::from(rgba.red()),
                    f64::from(rgba.green()),
                    f64::from(rgba.blue()),
                    f64::from(rgba.alpha()),
                );
            };
            cr.rectangle(
                f64::from(x) + 1.0,
                f64::from(y) + 1.0,
                f64::from(w) - 2.0,
                f64::from(g.height) - 2.0,
            );
            color("shell_accent");
            cr.set_line_width(2.0);
            let _ = cr.stroke();
            let edge = y + g.height - 5;
            color("shell_handle");
            cr.set_line_width(4.0);
            cr.move_to(f64::from(x + w / 2 - 32), f64::from(edge));
            cr.line_to(f64::from(x + w / 2 + 32), f64::from(edge));
            let _ = cr.stroke();
        }
    });
    let drag = gtk::GestureDrag::new();
    let start = Rc::new(Cell::new(None));
    drag.connect_drag_begin({
        let geometry = geometry.clone();
        let start = start.clone();
        let canvas = canvas.downgrade();
        move |gesture, px, py| {
            let Some(canvas) = canvas.upgrade() else {
                return;
            };
            let (g, x, y, w) = geometry.get().bounded(canvas.width(), canvas.height());
            if px < f64::from(x)
                || px > f64::from(x + w)
                || py < f64::from(y)
                || py > f64::from(y + g.height)
            {
                start.set(None);
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            let edge = y + g.height;
            start.set(Some((g, x, y, (py - f64::from(edge)).abs() <= 12.0)));
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    drag.connect_drag_update({
        let update = update.clone();
        let geometry = geometry.clone();
        let canvas = canvas.downgrade();
        move |_, dx, dy| {
            let (Some(canvas), Some((mut g, x, y, resize))) = (canvas.upgrade(), start.get())
            else {
                return;
            };
            let width = canvas.width();
            let height = canvas.height();
            if resize {
                g.height =
                    (g.height + dy.round() as i32).clamp(180.min(height), (height - y).max(1));
                g.y = f64::from(y) / f64::from((height - g.height).max(1));
            } else {
                g.x = (f64::from(x) + dx) / f64::from((width - 520).max(1));
                g.y = (f64::from(y) + dy) / f64::from((height - g.height).max(1));
            }
            geometry.set(g.bounded(width, height).0);
            update(width, height);
            canvas.queue_draw();
        }
    });
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion({
        let geometry = geometry.clone();
        let canvas = canvas.downgrade();
        move |_, px, py| {
            let Some(canvas) = canvas.upgrade() else {
                return;
            };
            let (g, x, y, w) = geometry.get().bounded(canvas.width(), canvas.height());
            let inside = px >= f64::from(x)
                && px <= f64::from(x + w)
                && py >= f64::from(y)
                && py <= f64::from(y + g.height);
            let edge = y + g.height;
            canvas.set_cursor_from_name(Some(if !inside {
                "default"
            } else if (py - f64::from(edge)).abs() <= 12.0 {
                "ns-resize"
            } else {
                "grab"
            }));
        }
    });
    canvas.add_controller(motion);
    canvas.add_controller(drag);
    reset.connect_clicked({
        let update = update.clone();
        let geometry = geometry.clone();
        let canvas = canvas.downgrade();
        move |_| {
            geometry.set(Geometry {
                height: 500,
                ..Default::default()
            });
            if let Some(canvas) = canvas.upgrade() {
                update(canvas.width(), canvas.height());
                canvas.queue_draw();
            }
        }
    });
    let error = gtk::Label::new(None);
    error.add_css_class("menu-error");
    error.set_visible(false);
    controls.append(&error);
    apply.connect_clicked({
        let state = Rc::downgrade(state);
        let window = window.downgrade();
        let canvas = canvas.downgrade();
        move |_| {
            let (Some(state), Some(window), Some(canvas)) =
                (state.upgrade(), window.upgrade(), canvas.upgrade())
            else {
                return;
            };
            let g = geometry.get().bounded(canvas.width(), canvas.height()).0;
            if let Err(message) = save(g) {
                error.set_text(&message);
                error.set_visible(true);
                return;
            }
            state.layouts.launcher.set(Some(g));
            window.close();
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
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let window = window.downgrade();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
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
                let launcher = state.launcher.borrow_mut().take();
                if let Some(window) = launcher {
                    window.close();
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
    fn button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(found) = button(&widget, label) {
                return Some(found);
            }
        }
        None
    }
    fn editor(state: &AppState) -> (gtk::Window, gtk::DrawingArea, gtk::GestureDrag) {
        crate::ui_tests::pump(150);
        let window = state.menu.borrow().as_ref().unwrap().clone();
        let canvas = window
            .child()
            .unwrap()
            .downcast::<gtk::Overlay>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::DrawingArea>()
            .unwrap();
        let controllers = canvas.observe_controllers();
        let drag = (0..controllers.n_items())
            .find_map(|i| controllers.item(i).and_downcast::<gtk::GestureDrag>())
            .unwrap();
        (window, canvas, drag)
    }
    show(app, state, false);
    let (window, canvas, drag) = editor(state);
    let launcher = state.launcher.borrow().as_ref().unwrap().clone();
    assert_eq!(launcher.widget_name(), "launcher");
    let outer = launcher.child().unwrap();
    assert!(outer.first_child().unwrap().is::<gtk::SearchEntry>());
    let initial_height = launcher.surface().unwrap().height();
    let initial_y = launcher.margin(Edge::Top);
    let expected_height = (initial_height - 80).max(180);
    let (_, x, _, _) = Geometry {
        height: initial_height,
        ..Default::default()
    }
    .bounded(canvas.width(), canvas.height());
    drag.emit_by_name::<()>(
        "drag-begin",
        &[&(f64::from(x) + 20.0), &f64::from(initial_y + 30)],
    );
    drag.emit_by_name::<()>("drag-update", &[&(-100.0f64), &100.0f64]);
    drag.emit_by_name::<()>("drag-end", &[&(-100.0f64), &100.0f64]);
    crate::ui_tests::pump(100);
    assert_eq!(launcher.margin(Edge::Top), initial_y + 100);
    drag.emit_by_name::<()>(
        "drag-begin",
        &[
            &(f64::from(x) - 80.0),
            &f64::from(initial_y + 100 + initial_height - 5),
        ],
    );
    drag.emit_by_name::<()>("drag-update", &[&0.0f64, &(-80.0f64)]);
    drag.emit_by_name::<()>("drag-end", &[&0.0f64, &(-80.0f64)]);
    crate::ui_tests::pump(100);
    assert_eq!(launcher.surface().unwrap().height(), expected_height);
    button(window.upcast_ref(), "Save").unwrap().emit_clicked();
    let saved = state.layouts.launcher.get().unwrap();
    assert_eq!(saved.height, expected_height);
    assert!(saved.x < 0.5);
    assert!(saved.y > 0.0);
    assert_eq!(
        crate::config::read()
            .unwrap()
            .launcher_layout
            .unwrap()
            .height,
        expected_height
    );
    assert_eq!(
        crate::config::read().unwrap().notification_history_limit,
        10
    );
    crate::launcher::show(app, state, crate::app::LauncherMode::Normal);
    crate::ui_tests::pump(350);
    let launcher = state.launcher.borrow().as_ref().unwrap().clone();
    assert_eq!(launcher.surface().unwrap().height(), expected_height);
    assert_eq!(launcher.margin(Edge::Top), initial_y + 100);
    assert_eq!(launcher.surface().unwrap().width(), 520);
    launcher.close();
    show(app, state, false);
    let (window, _, _) = editor(state);
    button(window.upcast_ref(), "Reset").unwrap().emit_clicked();
    button(window.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    assert_eq!(
        state.layouts.launcher.get().unwrap().height,
        expected_height
    );
}
