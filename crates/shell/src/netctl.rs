use std::{collections::HashMap, time::Duration};

use services::network::{
    self, BluetoothPrompt, BluetoothState, DeviceProfiles, WifiDetails, WifiState,
};

use crate::{
    app::{App, ServiceEvent},
    panels::NetAction,
};

#[derive(Default)]
pub struct Net {
    pub handle: Option<network::Handle>,
    pub wifi: WifiState,
    pub details: Option<WifiDetails>,
    pub down: Vec<f64>,
    pub up: Vec<f64>,
    pub prompt: String,
    pub bluetooth: BluetoothState,
    pub profiles: HashMap<String, DeviceProfiles>,
    pub bluetooth_prompt: BluetoothPrompt,
    pub bluetooth_error: String,
    refresh_queued: bool,
    bluetooth_queued: bool,
    wifi_panel: bool,
    bluetooth_panel: bool,
}

impl Net {
    pub fn network_connected(&self, wired: bool) -> bool {
        self.wifi.connected || wired
    }

    pub fn strength(&self) -> f64 {
        self.wifi
            .networks
            .iter()
            .find(|network| network.connected)
            .map_or(100.0, |network| network.strength)
    }

    pub fn connected_devices(&self) -> Vec<(String, i32)> {
        self.bluetooth
            .devices
            .iter()
            .filter(|device| device.connected)
            .map(|device| (device.name.clone(), device.battery as i32))
            .collect()
    }
}

impl App {
    pub fn start_network(&mut self) {
        let sender = self.events.clone();
        let task = async move {
            let callback_sender = sender.clone();
            match network::start(move |event| {
                let _ = callback_sender.send(ServiceEvent::Network(event));
            })
            .await
            {
                Ok(handle) => {
                    let _ = sender.send(ServiceEvent::NetworkReady(handle));
                }
                Err(err) => log::error!("network: {err}"),
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("network: {err}");
        }
    }

    fn net_task<F>(&self, run: impl FnOnce(network::Handle) -> F)
    where
        F: std::future::Future<Output = ()> + 'static,
    {
        let Some(handle) = self.net.handle.clone() else {
            return;
        };
        if let Err(err) = self.scheduler.schedule(run(handle)) {
            log::warn!("network task: {err}");
        }
    }

    pub fn network_ready(&mut self, handle: network::Handle) {
        self.net.handle = Some(handle.clone());
        let watcher = handle.clone();
        self.net_task(move |_| async move {
            if let Err(err) = watcher.watch().await {
                log::error!("network signals: {err}");
            }
        });
        self.net_task(|handle| async move {
            handle.refresh_bluetooth().await;
            handle.refresh_wifi().await;
        });
        self.add_timer(Duration::from_secs(60), |app| {
            app.net_task(|handle| async move { handle.refresh_wifi().await });
            Some(Duration::from_secs(60))
        });
        let panel = self.state.panel.clone();
        self.network_panel_changed(&panel);
    }

    pub fn network_event(&mut self, event: network::Event) {
        match event {
            network::Event::Wifi(state) => {
                self.available.wifi = !state.backend.is_empty();
                if !self.available.wifi {
                    services::programs::warn_missing("iwd or NetworkManager", "Wi-Fi");
                }
                self.net.wifi = state;
                self.sync();
            }
            network::Event::Details(details) => {
                if details.connected {
                    if let (Some(down), Some(up)) = (details.down, details.up) {
                        self.net.down.push(down);
                        self.net.up.push(up);
                        let excess = self.net.down.len().saturating_sub(60);
                        self.net.down.drain(..excess);
                        let excess = self.net.up.len().saturating_sub(60);
                        self.net.up.drain(..excess);
                    }
                    self.net.details = Some(details);
                } else {
                    self.net.details = None;
                }
                self.sync();
            }
            network::Event::Bluetooth(state) => {
                self.available.bluetooth = state.available;
                if !state.available {
                    services::programs::warn_missing("a BlueZ adapter", "Bluetooth");
                }
                self.net.bluetooth = state;
                self.sync();
            }
            network::Event::Profiles(profiles) => {
                self.net.profiles = profiles;
                self.sync();
            }
            network::Event::BluetoothError(message) => {
                self.net.bluetooth_error = message;
                self.sync();
            }
            network::Event::WifiPrompt(name) => {
                self.net.prompt = name;
                if !self.net.prompt.is_empty() {
                    self.state.panel = "wifi".into();
                }
                self.sync();
            }
            network::Event::BluetoothPrompt(prompt) => {
                let open = !prompt.name.is_empty();
                self.net.bluetooth_prompt = prompt;
                if open {
                    self.state.panel = "bluetooth".into();
                }
                self.sync();
            }
            network::Event::WifiChanged => {
                if !self.net.refresh_queued {
                    self.net.refresh_queued = true;
                    self.add_timer(Duration::from_millis(150), |app| {
                        app.net.refresh_queued = false;
                        app.net_task(|handle| async move { handle.refresh_wifi().await });
                        None
                    });
                }
            }
            network::Event::BluetoothChanged => {
                if !self.net.bluetooth_queued {
                    self.net.bluetooth_queued = true;
                    self.add_timer(Duration::ZERO, |app| {
                        app.net.bluetooth_queued = false;
                        app.net_task(|handle| async move { handle.refresh_bluetooth().await });
                        None
                    });
                }
            }
        }
    }

    pub fn network_panel_changed(&mut self, panel: &str) {
        if self.net.handle.is_none() {
            return;
        }
        if panel != "bluetooth" && self.net.bluetooth.discovering {
            self.net_action(NetAction::BtScan(false));
        }
        let wifi = panel == "wifi";
        if let Some(handle) = &self.net.handle {
            handle.set_panel(wifi);
        }
        if wifi && !self.net.wifi_panel {
            self.net.wifi_panel = true;
            self.net_task(|handle| async move {
                handle.reset_details();
            });
            self.network_event(network::Event::WifiChanged);
            self.add_timer(Duration::from_secs(10), |app| {
                if !app.net.wifi_panel {
                    return None;
                }
                app.net_task(|handle| async move { handle.refresh_wifi().await });
                Some(Duration::from_secs(10))
            });
            self.add_timer(Duration::from_millis(1000), |app| {
                if !app.net.wifi_panel {
                    return None;
                }
                app.net_task(|handle| async move { handle.details().await });
                Some(Duration::from_millis(1000))
            });
            self.add_timer(Duration::from_millis(200), |app| {
                if app.net.wifi_panel {
                    app.net_task(|handle| async move { handle.details().await });
                }
                None
            });
        } else if !wifi && self.net.wifi_panel {
            self.net.wifi_panel = false;
        }
        let bluetooth = panel == "bluetooth";
        if bluetooth && !self.net.bluetooth_panel {
            self.net.bluetooth_panel = true;
            self.net_task(|handle| async move { handle.profiles(true).await });
            self.add_timer(Duration::from_secs(3), |app| {
                if !app.net.bluetooth_panel {
                    return None;
                }
                app.net_task(|handle| async move { handle.profiles(false).await });
                Some(Duration::from_secs(3))
            });
        } else if !bluetooth && self.net.bluetooth_panel {
            self.net.bluetooth_panel = false;
        }
        if !wifi {
            self.net.details = None;
            self.net.down.clear();
            self.net.up.clear();
        }
        if wifi && self.net.wifi.powered && !self.net.wifi.scanning && !self.net.wifi.busy {
            self.net_action(NetAction::Scan);
        }
        if !bluetooth && !self.net.bluetooth_prompt.name.is_empty() {
            self.net_action(NetAction::BtReply(false, String::new()));
        }
        if !wifi && !self.net.prompt.is_empty() {
            self.net_action(NetAction::Cancel);
        }
    }

    pub fn net_action(&mut self, action: NetAction) {
        match action {
            NetAction::Airplane(enabled) => {
                self.net_task(move |handle| async move { handle.airplane(enabled).await })
            }
            NetAction::WifiPower(enabled) => {
                self.net_task(move |handle| async move { handle.wifi_power(enabled).await })
            }
            NetAction::Scan => self.net_task(|handle| async move { handle.scan().await }),
            NetAction::Connect(path) => {
                self.net_task(move |handle| async move { handle.connect(path).await })
            }
            NetAction::Disconnect => {
                self.net_task(|handle| async move { handle.disconnect().await })
            }
            NetAction::Forget(path) => {
                self.net_task(move |handle| async move { handle.forget(path).await })
            }
            NetAction::Autoconnect(path, enabled) => {
                self.net_task(move |handle| async move { handle.autoconnect(path, enabled).await })
            }
            NetAction::ConnectHidden(name) => {
                self.net_task(move |handle| async move { handle.connect_hidden(name).await })
            }
            NetAction::Password(password) => {
                if let Some(handle) = &self.net.handle {
                    handle.password(password);
                }
            }
            NetAction::Cancel => {
                if let Some(handle) = &self.net.handle {
                    handle.cancel();
                }
            }
            NetAction::BtPower(enabled) => {
                self.net_task(move |handle| async move { handle.bt_power(enabled).await })
            }
            NetAction::BtScan(enabled) => {
                self.net_task(move |handle| async move { handle.bt_scan(enabled).await })
            }
            NetAction::BtPair(path) => {
                self.net_task(move |handle| async move { handle.bt_pair(path).await })
            }
            NetAction::BtDevice(path, method) => {
                self.net_task(move |handle| async move { handle.bt_device(path, method).await })
            }
            NetAction::BtTrust(path, enabled) => {
                self.net_task(move |handle| async move { handle.bt_trust(path, enabled).await })
            }
            NetAction::BtRename(path, alias) => {
                self.net_task(move |handle| async move { handle.bt_rename(path, alias).await })
            }
            NetAction::BtForget(path) => {
                self.net_task(move |handle| async move { handle.bt_forget(path).await })
            }
            NetAction::BtProfile(id, index) => {
                self.net_task(move |handle| async move { handle.bt_profile(id, index).await });
                self.add_timer(Duration::from_millis(600), |app| {
                    app.net_task(|handle| async move { handle.profiles(false).await });
                    None
                });
            }
            NetAction::BtReply(accepted, value) => {
                self.net_task(
                    move |handle| async move { handle.bluetooth_reply(accepted, value).await },
                )
            }
        }
    }
}
