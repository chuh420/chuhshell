use crate::{app::AppState, modules, niri, notifications, process};
use gtk::prelude::*;
use notifications::{ConnectionNotice, Notice, NoticeKind, PowerNotice};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SystemState {
    pub niri: niri::Snapshot,
    pub audio: Option<String>,
    pub network: Option<modules::NetworkInfo>,
    pub temperature: Option<i64>,
    pub brightness: Option<(u8, &'static str)>,
    pub battery: modules::BatteryStatus,
}

type Listener = Box<dyn Fn(&SystemState) -> bool>;

pub struct Services {
    data: RefCell<SystemState>,
    listeners: RefCell<Vec<Listener>>,
    _catalog: gio::AppInfoMonitor,
}

impl Services {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let services = Rc::new(Self {
            data: RefCell::new(SystemState::default()),
            listeners: RefCell::new(Vec::new()),
            _catalog: crate::apps::watch(),
        });
        let (tx, rx) = async_channel::bounded(16);
        niri::spawn_poller(tx);
        let weak = Rc::downgrade(&services);
        let state = Rc::downgrade(state);
        let osd_state = state.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(snapshot) = rx.recv().await {
                let (Some(services), Some(state)) = (weak.upgrade(), state.upgrade()) else {
                    break;
                };
                let old = services.data.borrow().niri.clone();
                *state.workspaces.borrow_mut() = snapshot.workspaces.clone();
                *state.layout_names.borrow_mut() = snapshot.layouts.names.clone();
                state.current_layout.set(snapshot.layouts.current_idx);
                if !old.layouts.names.is_empty()
                    && old.layouts.current_idx != snapshot.layouts.current_idx
                    && let Some(name) = snapshot.layouts.names.get(snapshot.layouts.current_idx)
                {
                    notifications::show(
                        &state,
                        Notice::transient(NoticeKind::Keyboard, notifications::layout_label(name)),
                    );
                }
                if let Some(initial) = state.launcher_focus_window.get()
                    && initial != state.focused_window_id()
                {
                    let window = state.launcher.borrow().clone();
                    if let Some(window) = window {
                        window.close();
                    }
                }
                crate::ui::set_active_output(
                    snapshot
                        .workspaces
                        .iter()
                        .find(|w| w.is_focused)
                        .and_then(|w| w.output.as_deref()),
                );
                services.update(|data| data.niri = snapshot);
            }
        });
        let (tx, rx) = async_channel::bounded(1);
        modules::spawn_audio_poller(tx);
        Self::consume(&services, rx, |data, value| data.audio = value);
        let (tx, rx) = async_channel::bounded(1);
        modules::spawn_temperature_poller(tx);
        Self::consume(&services, rx, |data, value| data.temperature = value);
        let (tx, rx) = async_channel::bounded(1);
        modules::spawn_network_poller(tx);
        let weak = Rc::downgrade(&services);
        let state = osd_state.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(info) = rx.recv().await {
                let (Some(services), Some(state)) = (weak.upgrade(), state.upgrade()) else {
                    break;
                };
                let old = services.data.borrow().network.clone();
                let previous = old
                    .filter(|v| v.status != "unavailable")
                    .map(|v| (v.status == "connected", v.ssid));
                if info.status != "unavailable" {
                    let notice = match notifications::network_transition(
                        previous.as_ref(),
                        info.status == "connected",
                        info.ssid.as_deref(),
                    ) {
                        Some(ConnectionNotice::Connected(ssid)) => Some(
                            Notice::transient(NoticeKind::Network, "Wi-Fi connected")
                                .with_detail(ssid),
                        ),
                        Some(ConnectionNotice::Disconnected) => {
                            Some(Notice::transient(NoticeKind::Network, "Wi-Fi disconnected"))
                        }
                        None => None,
                    };
                    if let Some(notice) = notice {
                        notifications::show(&state, notice);
                    }
                }
                services.update(|data| data.network = Some(info));
            }
        });
        let (hardware_tx, hardware_rx) = async_channel::bounded(1);
        let (refresh_tx, refresh_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while !process::stopped() {
                let brightness = modules::backlight_device()
                    .as_deref()
                    .and_then(modules::brightness_level);
                if hardware_tx
                    .send_blocking((modules::battery_status(), brightness))
                    .is_err()
                {
                    return;
                }
                if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =
                    refresh_rx.recv_timeout(Duration::from_secs(15))
                {
                    return;
                }
            }
        });
        let weak = Rc::downgrade(&services);
        let state = osd_state.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok((battery, brightness)) = hardware_rx.recv().await {
                let (Some(services), Some(state)) = (weak.upgrade(), state.upgrade()) else {
                    break;
                };
                let old = services.data.borrow().battery.clone();
                if old.available && battery.available {
                    let previous = Some((old.plugged, old.charging));
                    if let Some(event) =
                        notifications::power_transition(previous, battery.plugged, battery.charging)
                    {
                        let title = match event {
                            PowerNotice::Connected => "Power connected",
                            PowerNotice::Disconnected => "Power disconnected",
                            PowerNotice::ChargingStarted => "Charging started",
                            PowerNotice::ChargingComplete => "Charging complete",
                        };
                        notifications::show(
                            &state,
                            Notice::transient(NoticeKind::Power, title)
                                .with_detail(&battery.tooltip),
                        );
                    }
                }
                services.update(|data| {
                    data.battery = battery;
                    data.brightness = brightness;
                });
            }
        });
        let (tx, rx) = async_channel::bounded(32);
        modules::spawn_device_monitor(tx);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = rx.recv().await {
                let Some(state) = osd_state.upgrade() else {
                    break;
                };
                match event {
                    modules::DeviceEvent::Peripheral(connected, name) => notifications::show(
                        &state,
                        Notice::transient(
                            NoticeKind::Peripheral,
                            if connected {
                                "Device connected"
                            } else {
                                "Device disconnected"
                            },
                        )
                        .with_detail(name),
                    ),
                    _ => {
                        let _ = refresh_tx.try_send(());
                    }
                }
            }
        });
        services
    }

    fn consume<T: 'static>(
        services: &Rc<Self>,
        rx: async_channel::Receiver<T>,
        update: impl Fn(&mut SystemState, T) + 'static,
    ) {
        let weak = Rc::downgrade(services);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(value) = rx.recv().await {
                let Some(services) = weak.upgrade() else {
                    break;
                };
                services.update(|data| update(data, value));
            }
        });
    }

    fn update(&self, update: impl FnOnce(&mut SystemState)) {
        let mut data = self.data.borrow_mut();
        let previous = data.clone();
        update(&mut data);
        if previous == *data {
            return;
        }
        drop(data);
        self.listeners
            .borrow_mut()
            .retain(|listener| listener(&self.data.borrow()));
    }

    pub fn subscribe(&self, listener: impl Fn(&SystemState) -> bool + 'static) {
        listener(&self.data.borrow());
        self.listeners.borrow_mut().push(Box::new(listener));
    }
}
