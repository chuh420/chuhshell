use super::backend::{self, Snapshot};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

type Listener = Box<dyn Fn(&Result<Snapshot, String>) -> bool>;

pub struct Service {
    latest: RefCell<Option<Result<Snapshot, String>>>,
    listeners: RefCell<Vec<Listener>>,
    preferred: Arc<Mutex<Option<String>>>,
    wake: mpsc::SyncSender<()>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
}

impl Service {
    pub fn new() -> Rc<Self> {
        let (wake, refresh) = mpsc::sync_channel(1);
        let preferred = Arc::new(Mutex::new(crate::config::get().wifi.clone()));
        let service = Rc::new(Self {
            latest: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            preferred: preferred.clone(),
            wake,
            subscriptions: RefCell::new(Vec::new()),
        });
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            while !crate::process::stopped() && !tx.is_closed() {
                let start = Instant::now();
                let selected = preferred.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let result = backend::snapshot(selected.as_deref());
                if tx.send_blocking((selected, result)).is_err() {
                    break;
                }
                match refresh.recv_timeout(Duration::from_secs(60)) {
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => {
                        if !crate::process::pause(
                            Duration::from_millis(500).saturating_sub(start.elapsed()),
                        ) {
                            break;
                        }
                        while refresh.try_recv().is_ok() {}
                    }
                }
            }
        });
        let weak = Rc::downgrade(&service);
        glib::MainContext::default().spawn_local(async move {
            while let Ok((selected, result)) = rx.recv().await {
                let Some(service) = weak.upgrade() else {
                    break;
                };
                if *service.preferred.lock().unwrap_or_else(|e| e.into_inner()) != selected {
                    service.refresh();
                    continue;
                }
                if service.latest.borrow().as_ref() != Some(&result) {
                    *service.latest.borrow_mut() = Some(result.clone());
                    service
                        .listeners
                        .borrow_mut()
                        .retain(|listener| listener(&result));
                }
            }
        });
        let weak = Rc::downgrade(&service);
        glib::MainContext::default().spawn_local(async move {
            let Ok(bus) = gio::bus_get_future(gio::BusType::System).await else {
                return;
            };
            let Some(service) = weak.upgrade() else {
                return;
            };
            let wake = service.wake.clone();
            let id = bus.subscribe_to_signal(
                Some("org.freedesktop.NetworkManager"),
                None,
                None,
                None,
                None,
                gio::DBusSignalFlags::NONE,
                move |_| {
                    let _ = wake.try_send(());
                },
            );
            service.subscriptions.borrow_mut().push(id);
            let wake = service.wake.clone();
            let id = bus.subscribe_to_signal(
                Some("org.freedesktop.DBus"),
                Some("org.freedesktop.DBus"),
                Some("NameOwnerChanged"),
                Some("/org/freedesktop/DBus"),
                Some("org.freedesktop.NetworkManager"),
                gio::DBusSignalFlags::NONE,
                move |_| {
                    let _ = wake.try_send(());
                },
            );
            service.subscriptions.borrow_mut().push(id);
            service.refresh();
        });
        service
    }
    pub fn refresh(&self) {
        let _ = self.wake.try_send(());
    }
    pub fn select(&self, name: Option<String>) {
        *self.preferred.lock().unwrap_or_else(|e| e.into_inner()) = name;
        self.refresh();
    }
    pub fn subscribe(&self, listener: impl Fn(&Result<Snapshot, String>) -> bool + 'static) {
        if let Some(value) = self.latest.borrow().as_ref()
            && !listener(value)
        {
            return;
        }
        self.listeners.borrow_mut().push(Box::new(listener));
    }
}

pub fn panel_info(result: &Result<Snapshot, String>) -> crate::modules::NetworkInfo {
    let unavailable = |status: &str| crate::modules::NetworkInfo {
        text: "󰖪".into(),
        tooltip: format!("Wi-Fi: {status}"),
        status: status.into(),
        ssid: None,
    };
    let Ok(snapshot) = result else {
        return unavailable("unavailable");
    };
    if !snapshot.enabled || !snapshot.hardware_enabled {
        return unavailable("disabled");
    }
    if snapshot.device.is_none() {
        return unavailable("unavailable");
    }
    let Some(network) = snapshot.networks.iter().find(|n| n.active) else {
        return unavailable("disconnected");
    };
    let signal = network.strength.unwrap_or(0);
    crate::modules::NetworkInfo {
        text: crate::modules::signal_icon(signal).into(),
        tooltip: format!("{}\n{signal}% • {}", network.name, snapshot.address),
        status: "connected".into(),
        ssid: Some(network.name.clone()),
    }
}
