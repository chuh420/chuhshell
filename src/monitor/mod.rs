mod backend;

use backend::{Sampler, Snapshot};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

type Listener = Box<dyn Fn(&Snapshot) -> bool>;

#[derive(Clone)]
struct View {
    popover: gtk::Popover,
    values: Vec<gtk::Label>,
}

pub struct Service {
    snapshot: RefCell<Snapshot>,
    listeners: RefCell<Vec<Listener>>,
    view: RefCell<Option<View>>,
    active: Arc<AtomicBool>,
    wake: std::sync::mpsc::SyncSender<()>,
}

impl Service {
    pub fn new() -> Rc<Self> {
        let active = Arc::new(AtomicBool::new(false));
        let (wake, refresh) = std::sync::mpsc::sync_channel(1);
        let (sender, receiver) = async_channel::bounded(1);
        let enabled = active.clone();
        std::thread::spawn(move || {
            let mut sampler = Sampler::default();
            while !crate::process::stopped() {
                if sender
                    .send_blocking(sampler.sample(enabled.load(Ordering::Relaxed)))
                    .is_err()
                {
                    break;
                }
                if matches!(
                    refresh.recv_timeout(Duration::from_secs(2)),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                ) {
                    break;
                }
            }
        });
        let service = Rc::new(Self {
            snapshot: RefCell::new(Snapshot::default()),
            listeners: RefCell::new(Vec::new()),
            view: RefCell::new(None),
            active,
            wake,
        });
        let weak = Rc::downgrade(&service);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(snapshot) = receiver.recv().await {
                let Some(service) = weak.upgrade() else { break };
                service
                    .listeners
                    .borrow_mut()
                    .retain(|listener| listener(&snapshot));
                *service.snapshot.borrow_mut() = snapshot;
                service.render();
            }
        });
        service
    }

    pub fn subscribe_temperature(&self, listener: impl Fn(Option<i64>) -> bool + 'static) {
        listener(self.snapshot.borrow().cpu_temperature);
        self.listeners
            .borrow_mut()
            .push(Box::new(move |snapshot| listener(snapshot.cpu_temperature)));
    }

    fn render(&self) {
        let view = self.view.borrow();
        let Some(view) = view.as_ref() else { return };
        let snapshot = self.snapshot.borrow();
        let percent =
            |value: Option<f64>| value.map_or_else(|| "Unavailable".into(), |n| format!("{n:.1}%"));
        let temp = |value: Option<i64>| {
            value.map_or_else(
                || "Unavailable".into(),
                |n| format!("{:.1}°C", n as f64 / 1000.0),
            )
        };
        let memory = snapshot.memory.map_or_else(
            || "Unavailable".into(),
            |(used, total)| {
                format!(
                    "{:.1}% · {:.1} / {:.1} GiB",
                    used as f64 * 100.0 / total as f64,
                    used as f64 / 1073741824.0,
                    total as f64 / 1073741824.0
                )
            },
        );
        let gpu = if !snapshot.gpu_sampled {
            "Measuring…".into()
        } else {
            percent(snapshot.gpu)
        };
        let values = [
            percent(snapshot.cpu),
            gpu,
            memory,
            temp(snapshot.cpu_temperature),
            temp(snapshot.gpu_temperature),
        ];
        for (label, value) in view.values.iter().zip(values) {
            label.set_text(&value);
        }
        view.values[1].set_tooltip_text(Some(if snapshot.gpu_partial {
            "Busiest GPU engine across accessible applications; other users and inaccessible processes are excluded"
        } else {
            "GPU usage reported by the driver"
        }));
        view.values[4].set_tooltip_text(Some(
            "A separate GPU temperature requires a GPU sensor exposed by the driver",
        ));
    }

    pub fn toggle(self: &Rc<Self>, anchor: &gtk::Button) {
        let previous = self.view.borrow().as_ref().map(|view| view.popover.clone());
        if let Some(previous) = previous {
            previous.popdown();
            return;
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("network-content");
        let title = gtk::Label::new(Some("System monitor"));
        title.set_xalign(0.0);
        title.add_css_class("network-heading");
        root.append(&title);
        let mut values = Vec::new();
        for name in [
            "CPU usage",
            "GPU usage",
            "RAM usage",
            "CPU temperature",
            "GPU temperature",
        ] {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            let name = gtk::Label::new(Some(name));
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.add_css_class("network-meta");
            let value = gtk::Label::new(None);
            value.set_xalign(1.0);
            value.add_css_class("network-name");
            row.append(&name);
            row.append(&value);
            root.append(&row);
            values.push(value);
        }
        let hint = gtk::Label::new(Some("Updates every 2 seconds"));
        hint.set_xalign(0.0);
        hint.add_css_class("network-meta");
        root.append(&hint);
        let popover = crate::ui::popover(anchor, &root);
        crate::ui::attach_to_bar(&popover, anchor);
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |_| {
            if let Some(service) = weak.upgrade() {
                service.active.store(false, Ordering::Relaxed);
                service.snapshot.borrow_mut().gpu_sampled = false;
                service.view.borrow_mut().take();
            }
        });
        *self.view.borrow_mut() = Some(View {
            popover: popover.clone(),
            values,
        });
        self.active.store(true, Ordering::Relaxed);
        let _ = self.wake.try_send(());
        self.render();
        popover.popup();
    }
}

#[cfg(test)]
pub fn regression_checks(anchor: &gtk::Button) {
    let (wake, _receiver) = std::sync::mpsc::sync_channel(1);
    let service = Rc::new(Service {
        snapshot: RefCell::new(Snapshot {
            cpu: Some(25.0),
            memory: Some((4 * 1073741824, 16 * 1073741824)),
            cpu_temperature: Some(45500),
            gpu: Some(12.0),
            gpu_sampled: true,
            ..Snapshot::default()
        }),
        listeners: RefCell::new(Vec::new()),
        view: RefCell::new(None),
        active: Arc::new(AtomicBool::new(false)),
        wake,
    });
    service.toggle(anchor);
    let view = service.view.borrow().as_ref().unwrap().clone();
    assert!(view.popover.has_css_class("bar-attached"));
    assert_eq!(view.values[0].text(), "25.0%");
    assert_eq!(view.values[1].text(), "12.0%");
    assert_eq!(view.values[2].text(), "25.0% · 4.0 / 16.0 GiB");
    assert_eq!(view.values[3].text(), "45.5°C");
    assert_eq!(view.values[4].text(), "Unavailable");
    service.snapshot.borrow_mut().cpu = Some(50.0);
    service.render();
    assert_eq!(view.values[0].text(), "50.0%");
    assert_eq!(
        service.view.borrow().as_ref().unwrap().values[0],
        view.values[0]
    );
    service.toggle(anchor);
    assert!(service.view.borrow().is_none());
    assert!(!service.active.load(Ordering::Relaxed));
    service.toggle(anchor);
    assert_eq!(
        service.view.borrow().as_ref().unwrap().values[1].text(),
        "Measuring…"
    );
    crate::ui::close_popover();
    assert!(service.view.borrow().is_none());
}
