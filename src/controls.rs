use crate::{
    modules, niri, notifications,
    services::{Services, SystemState},
};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Layout,
    Brightness,
    Volume,
}

impl Kind {
    fn index(self) -> usize {
        match self {
            Self::Layout => 0,
            Self::Brightness => 1,
            Self::Volume => 2,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Layout => "Keyboard layout",
            Self::Brightness => "Brightness",
            Self::Volume => "Volume",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Layout(usize, Vec<String>),
    Brightness(u8),
    Volume(u8),
}

impl Action {
    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Layout(..) => Kind::Layout,
            Self::Brightness(_) => Kind::Brightness,
            Self::Volume(_) => Kind::Volume,
        }
    }
}

#[derive(Clone)]
pub(crate) enum Applied {
    Layout(niri::KeyboardLayouts),
    Brightness((u8, &'static str)),
    Volume(String),
}

pub(crate) fn execute(action: Action) -> Result<Applied, String> {
    match action {
        Action::Layout(index, names) => {
            let layouts = niri::keyboard_layouts().ok_or("Keyboard layouts unavailable")?;
            if layouts.names != names || index >= names.len() || index > u8::MAX as usize {
                return Err("Keyboard layouts changed. Select a layout again.".into());
            }
            if !niri::command(
                &serde_json::json!({"Action":{"SwitchLayout":{"layout":{"Index":index}}}})
                    .to_string(),
            ) {
                return Err("Could not switch keyboard layout".into());
            }
            niri::keyboard_layouts()
                .map(Applied::Layout)
                .ok_or_else(|| "Could not read keyboard layout".into())
        }
        Action::Brightness(percent) => {
            let device = modules::backlight_device().ok_or("Backlight unavailable")?;
            crate::process::run(
                "brightnessctl",
                &["-d", &device, "set", &format!("{}%", percent.clamp(1, 100))],
            )?;
            modules::brightness_level(&device)
                .map(Applied::Brightness)
                .ok_or_else(|| "Could not read brightness".into())
        }
        Action::Volume(percent) => {
            crate::process::run(
                "wpctl",
                &[
                    "set-volume",
                    "-l",
                    "1.0",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{}%", percent.min(100)),
                ],
            )?;
            crate::process::run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])?;
            crate::process::run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
                .map(Applied::Volume)
        }
    }
}

#[derive(Clone)]
struct View {
    kind: Kind,
    popover: gtk::Popover,
    status: gtk::Label,
    scale: Option<gtk::Scale>,
    list: gtk::Box,
    names: Vec<String>,
    buttons: Vec<gtk::Button>,
}

pub struct Controls {
    services: Weak<Services>,
    snapshot: RefCell<SystemState>,
    view: RefCell<Option<View>>,
    updating: Cell<bool>,
    pending: RefCell<[usize; 3]>,
    requests: async_channel::Sender<notifications::Work>,
    results: async_channel::Sender<(Kind, Result<Applied, String>)>,
}

impl Controls {
    pub fn new(services: &Rc<Services>) -> Rc<Self> {
        let requests = notifications::worker();
        let (sender, results) = async_channel::bounded(32);
        let controls = Rc::new(Self {
            services: Rc::downgrade(services),
            snapshot: RefCell::new(SystemState::default()),
            view: RefCell::new(None),
            updating: Cell::new(false),
            pending: RefCell::new([0; 3]),
            requests,
            results: sender,
        });
        let weak = Rc::downgrade(&controls);
        services.subscribe(move |data, changes| {
            let Some(controls) = weak.upgrade() else {
                return false;
            };
            if changes.audio || changes.brightness || changes.layouts {
                let mut snapshot = controls.snapshot.borrow_mut();
                snapshot.audio.clone_from(&data.audio);
                snapshot.brightness = data.brightness;
                snapshot.niri.layouts.clone_from(&data.niri.layouts);
                drop(snapshot);
                controls.render();
            }
            true
        });
        let weak = Rc::downgrade(&controls);
        glib::MainContext::default().spawn_local(async move {
            while let Ok((completed, result)) = results.recv().await {
                let Some(controls) = weak.upgrade() else {
                    break;
                };
                controls.pending.borrow_mut()[completed.index()] -= 1;
                match result {
                    Ok(applied) => {
                        if let Some(view) = controls
                            .view
                            .borrow()
                            .as_ref()
                            .filter(|v| v.kind == completed)
                        {
                            view.status.remove_css_class("menu-error");
                        }
                        if let Some(services) = controls.services.upgrade() {
                            services.update(|data| match applied {
                                Applied::Volume(value) => data.audio = Some(value),
                                Applied::Brightness(value) => data.brightness = Some(value),
                                Applied::Layout(value) => data.niri.layouts = value,
                            });
                        }
                    }
                    Err(error) => {
                        if let Some(view) = controls
                            .view
                            .borrow()
                            .as_ref()
                            .filter(|v| v.kind == completed)
                        {
                            view.status.set_text(&error);
                            view.status.add_css_class("menu-error");
                        }
                    }
                }
                controls.render();
            }
        });
        controls
    }

    fn request(self: &Rc<Self>, action: Action) {
        let kind = action.kind();
        if let Some(view) = self.view.borrow().as_ref().filter(|view| view.kind == kind) {
            view.status.remove_css_class("menu-error");
        }
        let request = notifications::Work::Control(action, self.results.clone());
        if self.requests.try_send(request).is_ok() {
            self.pending.borrow_mut()[kind.index()] += 1;
        } else if let Some(view) = self.view.borrow().as_ref().filter(|view| view.kind == kind) {
            view.status.set_text("Too many pending system commands");
            view.status.add_css_class("menu-error");
        }
    }

    fn render(self: &Rc<Self>) {
        let Some(mut view) = self.view.borrow().clone() else {
            return;
        };
        let snapshot = self.snapshot.borrow();
        self.updating.set(true);
        if view.kind == Kind::Layout {
            let layouts = &snapshot.niri.layouts;
            if view.names != layouts.names {
                while let Some(child) = view.list.first_child() {
                    view.list.remove(&child);
                }
                view.buttons.clear();
                view.names.clone_from(&layouts.names);
                for (index, name) in layouts.names.iter().take(256).enumerate() {
                    let button = gtk::Button::with_label(name);
                    button.add_css_class("network-action");
                    let weak = Rc::downgrade(self);
                    let names = layouts.names.clone();
                    button.connect_clicked(move |_| {
                        if let Some(controls) = weak.upgrade()
                            && controls.snapshot.borrow().niri.layouts.names == names
                        {
                            controls.request(Action::Layout(index, names.clone()));
                        }
                    });
                    view.list.append(&button);
                    view.buttons.push(button);
                }
                *self.view.borrow_mut() = Some(view.clone());
            }
            for (index, button) in view.buttons.iter().enumerate() {
                if index == layouts.current_idx {
                    button.add_css_class("primary");
                } else {
                    button.remove_css_class("primary");
                }
            }
            if !view.status.has_css_class("menu-error") {
                view.status.set_text(if layouts.names.is_empty() {
                    "Keyboard layouts unavailable"
                } else {
                    "Select a keyboard layout"
                });
            }
        } else if let Some(scale) = &view.scale {
            let audio = snapshot
                .audio
                .as_deref()
                .and_then(notifications::parse_wpctl_volume);
            let value = if view.kind == Kind::Volume {
                audio.map(|(percent, _)| percent)
            } else {
                snapshot.brightness.map(|(percent, _)| percent)
            };
            scale.set_sensitive(value.is_some());
            let changing = self.pending.borrow()[view.kind.index()] > 0;
            if !changing && let Some(value) = value {
                scale.set_value(f64::from(value));
            }
            if !changing && !view.status.has_css_class("menu-error") {
                view.status.set_text(if value.is_none() {
                    "Unavailable"
                } else if view.kind == Kind::Volume && audio.is_some_and(|(_, muted)| muted) {
                    "Muted · move the slider to unmute"
                } else {
                    "Drag to adjust"
                });
            }
        }
        self.updating.set(false);
    }

    pub fn toggle(self: &Rc<Self>, kind: Kind, anchor: &gtk::Button) {
        let previous = self
            .view
            .borrow()
            .as_ref()
            .map(|view| (view.kind, view.popover.clone()));
        if let Some((previous_kind, previous)) = previous {
            previous.popdown();
            if previous_kind == kind {
                return;
            }
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("network-content");
        let title = gtk::Label::new(Some(kind.title()));
        title.set_xalign(0.0);
        title.add_css_class("network-heading");
        root.append(&title);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let scale = if kind == Kind::Layout {
            let scrolled = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .max_content_height(320)
                .propagate_natural_height(true)
                .child(&list)
                .build();
            root.append(&scrolled);
            None
        } else {
            let scale = gtk::Scale::with_range(
                gtk::Orientation::Horizontal,
                if kind == Kind::Brightness { 1.0 } else { 0.0 },
                100.0,
                1.0,
            );
            scale.set_draw_value(true);
            scale.set_digits(0);
            scale.set_value_pos(gtk::PositionType::Top);
            scale.set_size_request(280, -1);
            scale.add_css_class("control-scale");
            scale.set_format_value_func(|_, value| format!("{value:.0}%"));
            let snapshot = self.snapshot.borrow();
            let value = if kind == Kind::Volume {
                snapshot
                    .audio
                    .as_deref()
                    .and_then(notifications::parse_wpctl_volume)
                    .map(|(percent, _)| percent)
            } else {
                snapshot.brightness.map(|(percent, _)| percent)
            };
            if let Some(value) = value {
                scale.set_value(f64::from(value));
            }
            drop(snapshot);
            let weak = Rc::downgrade(self);
            scale.connect_value_changed(move |scale| {
                if let Some(controls) = weak.upgrade()
                    && !controls.updating.get()
                {
                    let percent = scale.value().round() as u8;
                    controls.request(if kind == Kind::Volume {
                        Action::Volume(percent)
                    } else {
                        Action::Brightness(percent)
                    });
                }
            });
            root.append(&scale);
            Some(scale)
        };
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_max_width_chars(40);
        status.add_css_class("network-meta");
        root.append(&status);
        let popover = crate::ui::popover(anchor, &root);
        crate::ui::attach_to_bar(&popover, anchor);
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |_| {
            if let Some(controls) = weak.upgrade() {
                controls.view.borrow_mut().take();
            }
        });
        *self.view.borrow_mut() = Some(View {
            kind,
            popover: popover.clone(),
            status,
            scale,
            list,
            names: Vec::new(),
            buttons: Vec::new(),
        });
        self.render();
        popover.popup();
    }
}

#[cfg(test)]
pub fn regression_checks(anchor: &gtk::Button) {
    let (requests, receiver) = async_channel::bounded(32);
    let (results, _) = async_channel::bounded(32);
    let controls = Rc::new(Controls {
        services: Weak::new(),
        snapshot: RefCell::new(SystemState {
            audio: Some("Volume: 0.40".into()),
            brightness: Some((75, "󰃟")),
            niri: niri::Snapshot {
                layouts: niri::KeyboardLayouts {
                    names: vec!["English (US)".into(), "Russian".into()],
                    current_idx: 0,
                },
                ..niri::Snapshot::default()
            },
            ..SystemState::default()
        }),
        view: RefCell::new(None),
        updating: Cell::new(false),
        pending: RefCell::new([0; 3]),
        requests,
        results,
    });
    controls.toggle(Kind::Volume, anchor);
    let view = controls.view.borrow().as_ref().unwrap().clone();
    assert!(view.popover.has_css_class("bar-attached"));
    let scale = view.scale.unwrap();
    assert_eq!(scale.value(), 40.0);
    assert!(receiver.is_empty());
    scale.set_value(45.0);
    scale.set_value(55.0);
    controls.snapshot.borrow_mut().audio = Some("Volume: 0.20".into());
    controls.render();
    assert_eq!(scale.value(), 55.0);
    for expected in [45, 55] {
        let notifications::Work::Control(action, _) = receiver.try_recv().unwrap() else {
            panic!("Expected slider request");
        };
        assert_eq!(action, Action::Volume(expected));
    }
    assert!(receiver.is_empty());
    *controls.pending.borrow_mut() = [0; 3];
    controls.toggle(Kind::Brightness, anchor);
    assert!(view.popover.parent().is_none());
    let view = controls.view.borrow().as_ref().unwrap().clone();
    let scale = view.scale.unwrap();
    assert_eq!(scale.value(), 75.0);
    scale.set_value(35.0);
    let notifications::Work::Control(action, _) = receiver.try_recv().unwrap() else {
        panic!("Expected brightness request");
    };
    assert_eq!(action, Action::Brightness(35));
    *controls.pending.borrow_mut() = [0; 3];
    controls.toggle(Kind::Layout, anchor);
    let view = controls.view.borrow().as_ref().unwrap().clone();
    assert_eq!(view.buttons.len(), 2);
    assert!(view.buttons[0].has_css_class("primary"));
    let stable = view.buttons[1].clone();
    stable.emit_clicked();
    let notifications::Work::Control(action, _) = receiver.try_recv().unwrap() else {
        panic!("Expected layout request");
    };
    assert_eq!(
        action,
        Action::Layout(1, vec!["English (US)".into(), "Russian".into()])
    );
    *controls.pending.borrow_mut() = [0; 3];
    controls.snapshot.borrow_mut().niri.layouts.current_idx = 1;
    controls.render();
    assert_eq!(controls.view.borrow().as_ref().unwrap().buttons[1], stable);
    assert!(stable.has_css_class("primary"));
    controls.snapshot.borrow_mut().niri.layouts.names = vec!["German".into()];
    controls.render();
    stable.emit_clicked();
    assert!(receiver.is_empty());
    controls.toggle(Kind::Volume, anchor);
    controls.snapshot.borrow_mut().audio = None;
    controls.render();
    assert!(
        !controls
            .view
            .borrow()
            .as_ref()
            .unwrap()
            .scale
            .as_ref()
            .unwrap()
            .is_sensitive()
    );
    controls.toggle(Kind::Volume, anchor);
    assert!(controls.view.borrow().is_none());
}
