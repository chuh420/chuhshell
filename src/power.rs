use glib::variant::ToVariant;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const INTERFACE: &str = "org.freedesktop.UPower.PowerProfiles";
const PROFILES: [(&str, &str); 3] = [
    ("Economy", "power-saver"),
    ("Balance", "balanced"),
    ("Performance", "performance"),
];

fn automatic(battery: &crate::modules::BatteryStatus) -> Option<&'static str> {
    let capacity = battery.capacity?;
    Some(if battery.plugged {
        "performance"
    } else if capacity < 20 {
        "power-saver"
    } else {
        "balanced"
    })
}

#[derive(Default)]
struct Policy {
    previous: Option<&'static str>,
    manual: Option<&'static str>,
}

impl Policy {
    fn update(
        &mut self,
        automatic: Option<&'static str>,
        request: Option<Option<&'static str>>,
    ) -> Option<&'static str> {
        if automatic.is_some() && automatic != self.previous {
            self.manual = None;
            self.previous = automatic;
        }
        if let Some(selected) = request {
            self.manual = selected;
        }
        self.manual.or(automatic)
    }
}

fn call(method: &str, args: glib::Variant) -> Result<glib::Variant, String> {
    if cfg!(test) {
        return Err("Power profile backend is isolated during tests".into());
    }
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
        .and_then(|bus| {
            bus.call_sync(
                Some(INTERFACE),
                "/org/freedesktop/UPower/PowerProfiles",
                "org.freedesktop.DBus.Properties",
                method,
                Some(&args),
                None,
                gio::DBusCallFlags::NONE,
                3000,
                gio::Cancellable::NONE,
            )
        })
        .map_err(|error| format!("Power profiles: {error}"))
}

#[derive(Clone, Default)]
struct Snapshot {
    battery: String,
    uptime: String,
    active: String,
    supported: Vec<String>,
    manual: bool,
    error: Option<String>,
}

#[derive(Clone)]
struct View {
    popover: gtk::Popover,
    battery: gtk::Label,
    uptime: gtk::Label,
    status: gtk::Label,
    buttons: Vec<gtk::Button>,
}

pub struct Service {
    requests: std::sync::mpsc::SyncSender<Option<&'static str>>,
    snapshot: RefCell<Snapshot>,
    view: RefCell<Option<View>>,
}

impl Service {
    pub fn new() -> Rc<Self> {
        let (requests, receiver) = std::sync::mpsc::sync_channel(1);
        let (sender, updates) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let mut policy = Policy::default();
            let mut request = None;
            while !crate::process::stopped() {
                let battery = crate::modules::battery_status();
                let desired = policy.update(automatic(&battery), request.take());
                let mut snapshot = Snapshot {
                    battery: battery.tooltip,
                    uptime: uptime(),
                    manual: policy.manual.is_some(),
                    ..Snapshot::default()
                };
                let result = (|| {
                    let properties: HashMap<String, glib::Variant> =
                        call("GetAll", (INTERFACE,).to_variant())?
                            .child_value(0)
                            .get()
                            .ok_or("Invalid power profile response")?;
                    snapshot.active = properties
                        .get("ActiveProfile")
                        .and_then(|v| v.get::<String>())
                        .ok_or("Missing active profile")?;
                    let profiles = properties
                        .get("Profiles")
                        .and_then(|v| v.get::<Vec<HashMap<String, glib::Variant>>>())
                        .ok_or("Missing supported profiles")?;
                    snapshot.supported = profiles
                        .iter()
                        .filter_map(|p| p.get("Profile")?.get::<String>())
                        .collect();
                    if let Some(target) = desired
                        && target != snapshot.active
                    {
                        if !snapshot.supported.iter().any(|p| p == target) {
                            return Err(format!(
                                "Profile {target} is unavailable on this hardware"
                            ));
                        }
                        call(
                            "Set",
                            (INTERFACE, "ActiveProfile", target.to_variant()).to_variant(),
                        )?;
                        snapshot.active = target.into();
                    }
                    Ok::<_, String>(())
                })();
                snapshot.error = result.err();
                if sender.send_blocking(snapshot).is_err() {
                    break;
                }
                match receiver.recv_timeout(Duration::from_secs(5)) {
                    Ok(selected) => request = Some(selected),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        let service = Rc::new(Self {
            requests,
            snapshot: RefCell::new(Snapshot::default()),
            view: RefCell::new(None),
        });
        let weak = Rc::downgrade(&service);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(snapshot) = updates.recv().await {
                let Some(service) = weak.upgrade() else { break };
                *service.snapshot.borrow_mut() = snapshot;
                service.render();
            }
        });
        service
    }

    fn render(&self) {
        let view = self.view.borrow();
        let Some(view) = view.as_ref() else { return };
        let snapshot = self.snapshot.borrow();
        view.battery.set_text(&snapshot.battery);
        view.uptime
            .set_text(&format!("Uptime · {}", snapshot.uptime));
        view.status
            .set_text(snapshot.error.as_deref().unwrap_or(if snapshot.manual {
                "Manual · until power state changes"
            } else {
                "Automatic · follows power and battery level"
            }));
        for (index, button) in view.buttons.iter().enumerate() {
            let selected = if index == 3 {
                !snapshot.manual
            } else {
                button.set_sensitive(snapshot.supported.iter().any(|p| p == PROFILES[index].1));
                snapshot.active == PROFILES[index].1
            };
            if selected {
                button.add_css_class("primary");
            } else {
                button.remove_css_class("primary");
            }
        }
    }

    pub fn toggle(self: &Rc<Self>, anchor: &gtk::Button) {
        let previous = self.view.borrow().as_ref().map(|view| view.popover.clone());
        if let Some(previous) = previous {
            previous.popdown();
            return;
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("network-content");
        let title = gtk::Label::new(Some("Battery & power"));
        title.add_css_class("network-heading");
        title.set_xalign(0.0);
        root.append(&title);
        let battery = gtk::Label::new(None);
        let uptime = gtk::Label::new(None);
        let status = gtk::Label::new(None);
        for label in [&battery, &uptime, &status] {
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_max_width_chars(40);
            label.add_css_class("network-meta");
            root.append(label);
        }
        let mut buttons = Vec::new();
        for (text, profile) in PROFILES
            .into_iter()
            .map(|(label, id)| (label, Some(id)))
            .chain(std::iter::once(("Automatic", None)))
        {
            let button = gtk::Button::with_label(text);
            button.add_css_class("network-action");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(service) = weak.upgrade() {
                    match service.requests.try_send(profile) {
                        Ok(()) => {
                            if let Some(view) = service.view.borrow().as_ref() {
                                view.status.set_text("Applying power profile…");
                            }
                        }
                        Err(_) => {
                            if let Some(view) = service.view.borrow().as_ref() {
                                view.status
                                    .set_text("Power profile request is busy. Try again.");
                            }
                        }
                    }
                }
            });
            root.append(&button);
            buttons.push(button);
        }
        let popover = crate::ui::popover(anchor, &root);
        crate::ui::attach_to_bar(&popover, anchor);
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |_| {
            if let Some(service) = weak.upgrade() {
                service.view.borrow_mut().take();
            }
        });
        *self.view.borrow_mut() = Some(View {
            popover: popover.clone(),
            battery,
            uptime,
            status,
            buttons,
        });
        self.render();
        popover.popup();
    }
}

fn uptime() -> String {
    let mut info = std::mem::MaybeUninit::<libc::sysinfo>::uninit();
    if unsafe { libc::sysinfo(info.as_mut_ptr()) } != 0 {
        return "Unavailable".into();
    }
    let seconds = unsafe { info.assume_init() }.uptime.max(0) as u64;
    format!(
        "{}d {}h {}m",
        seconds / 86400,
        seconds / 3600 % 24,
        seconds / 60 % 60
    )
}

#[cfg(test)]
pub fn regression_checks(anchor: &gtk::Button) {
    let (requests, receiver) = std::sync::mpsc::sync_channel(1);
    let service = Rc::new(Service {
        requests,
        snapshot: RefCell::new(Snapshot {
            battery: "50%".into(),
            uptime: "1d 2h 3m".into(),
            active: "balanced".into(),
            supported: vec!["balanced".into(), "power-saver".into()],
            ..Snapshot::default()
        }),
        view: RefCell::new(None),
    });
    service.toggle(anchor);
    let view = service.view.borrow().as_ref().unwrap().clone();
    assert!(view.popover.has_css_class("bar-attached"));
    assert_eq!(view.uptime.text(), "Uptime · 1d 2h 3m");
    assert!(view.buttons[1].has_css_class("primary"));
    assert!(!view.buttons[2].is_sensitive());
    view.buttons[0].emit_clicked();
    assert_eq!(receiver.try_recv().unwrap(), Some("power-saver"));
    view.buttons[3].emit_clicked();
    assert_eq!(receiver.try_recv().unwrap(), None);
    service.toggle(anchor);
    assert!(service.view.borrow().is_none());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_selection_survives_polling_until_transition_or_automatic() {
        let mut policy = Policy::default();
        assert_eq!(policy.update(Some("balanced"), None), Some("balanced"));
        assert_eq!(
            policy.update(Some("balanced"), Some(Some("performance"))),
            Some("performance")
        );
        assert_eq!(policy.update(Some("balanced"), None), Some("performance"));
        assert_eq!(policy.update(None, None), Some("performance"));
        assert_eq!(
            policy.update(Some("power-saver"), None),
            Some("power-saver")
        );
        assert_eq!(
            policy.update(Some("power-saver"), Some(Some("balanced"))),
            Some("balanced")
        );
        assert_eq!(
            policy.update(Some("power-saver"), Some(None)),
            Some("power-saver")
        );
        assert_eq!(
            policy.update(Some("performance"), None),
            Some("performance")
        );
    }

    #[test]
    fn automatic_profiles_follow_supply_and_threshold() {
        let mut battery = crate::modules::BatteryStatus::default();
        assert_eq!(automatic(&battery), None);
        for (plugged, capacity, expected) in [
            (true, 10, "performance"),
            (true, 100, "performance"),
            (false, 20, "balanced"),
            (false, 19, "power-saver"),
            (false, 0, "power-saver"),
        ] {
            battery.plugged = plugged;
            battery.capacity = Some(capacity);
            assert_eq!(automatic(&battery), Some(expected));
        }
    }
}
