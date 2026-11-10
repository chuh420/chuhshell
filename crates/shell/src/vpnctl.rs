use std::{collections::BTreeMap, sync::Arc, time::Duration, time::Instant};

use serde::{Deserialize, Serialize};
use services::{notifications::Urgency, vpn};

use crate::{
    app::{App, ServiceEvent},
    panels::VpnAction,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VpnState {
    #[default]
    Off,
    Connecting,
    On,
    Disconnecting,
}

impl VpnState {
    pub fn name(self) -> &'static str {
        match self {
            VpnState::Off => "disconnected",
            VpnState::Connecting => "connecting",
            VpnState::On => "connected",
            VpnState::Disconnecting => "disconnecting",
        }
    }

    pub fn busy(self) -> bool {
        matches!(self, VpnState::Connecting | VpnState::Disconnecting)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VpnJob {
    Connect(vpn::Target),
    Disconnect,
    Setting(String, String),
    Settings,
    Refresh,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
}

fn saved_path() -> std::path::PathBuf {
    config::paths().state.join("vpn.json")
}

fn load_saved() -> Saved {
    std::fs::read_to_string(saved_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn store_saved(saved: &Saved) {
    let path = saved_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(saved) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                log::warn!("vpn state: {err}");
            }
        }
        Err(err) => log::warn!("vpn state: {err}"),
    }
}

pub fn duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

pub fn bytes(value: u64) -> String {
    let value = value as f64;
    if value >= 1073741824.0 {
        format!("{:.2} GiB", value / 1073741824.0)
    } else if value >= 1048576.0 {
        format!("{:.1} MiB", value / 1048576.0)
    } else if value >= 1024.0 {
        format!("{:.0} KiB", value / 1024.0)
    } else {
        format!("{value:.0} B")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VpnData {
    pub state: VpnState,
    pub target: String,
    pub connection: Option<vpn::Connection>,
    pub server: Option<vpn::ServerInfo>,
    pub ip: String,
    pub uptime: u64,
    pub rx: u64,
    pub tx: u64,
    pub down: Vec<f64>,
    pub up: Vec<f64>,
    pub catalog: Option<Arc<vpn::Catalog>>,
    pub loading: bool,
    pub refreshing: bool,
    pub home: Option<String>,
    pub exclude: Vec<String>,
    pub settings: BTreeMap<String, String>,
    pub pending_setting: String,
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
    pub message: String,
}

impl VpnData {
    pub fn fastest(&self) -> Option<&vpn::Country> {
        self.catalog.as_ref()?.fastest(&self.exclude)
    }

    pub fn country(&self) -> Option<String> {
        if let Some(server) = &self.server {
            return Some(server.country.clone());
        }
        let name = &self.connection.as_ref()?.server;
        let code: String = name
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        (code.len() == 2).then(|| code.to_ascii_uppercase())
    }
}

#[derive(Default)]
pub struct Vpn {
    pub available: bool,
    pub link: bool,
    pub connection: Option<vpn::Connection>,
    stamp: Option<u64>,
    pub state: VpnState,
    pub target: String,
    pub ip: String,
    pub catalog: Option<Arc<vpn::Catalog>>,
    catalog_stamp: Option<u64>,
    catalog_for: Option<String>,
    loading: bool,
    refreshing: bool,
    pub settings: BTreeMap<String, String>,
    pending_setting: String,
    pub message: String,
    pub down: Vec<f64>,
    pub up: Vec<f64>,
    last: Option<(u64, u64, Instant)>,
    rx: u64,
    tx: u64,
    panel: bool,
    opened: u64,
    pub saved: Saved,
    home: Option<String>,
    exclude: Vec<String>,
    user_disconnect: bool,
    started: bool,
}

impl Vpn {
    pub fn server_name(&self) -> Option<&str> {
        self.connection.as_ref().map(|c| c.server.as_str())
    }

    fn server(&self) -> Option<vpn::ServerInfo> {
        let catalog = self.catalog.as_ref()?;
        let current = catalog.current.as_ref()?;
        (Some(current.name.as_str()) == self.server_name()).then(|| current.clone())
    }

    pub fn bar_state(&self) -> VpnState {
        if self.state.busy() {
            self.state
        } else if self.link {
            VpnState::On
        } else {
            VpnState::Off
        }
    }

    pub fn tooltip(&self) -> String {
        match self.bar_state() {
            VpnState::Connecting => format!("Proton VPN · connecting to {}…", self.target),
            VpnState::Disconnecting => "Proton VPN · disconnecting…".into(),
            VpnState::On => {
                let mut text = String::from("Proton VPN · connected");
                if let Some(server) = self.server() {
                    text.push_str(&format!(
                        "\n{} · {}, {}",
                        server.name,
                        server.city,
                        vpn::country_name(&server.country)
                    ));
                } else if let Some(name) = self.server_name() {
                    text.push_str(&format!("\n{name}"));
                }
                if let Some(connection) = &self.connection
                    && connection.since > 0
                {
                    text.push_str(&format!(
                        "\nup {}",
                        duration(vpn::now().saturating_sub(connection.since))
                    ));
                }
                text.push_str("\nright click: disconnect");
                text
            }
            VpnState::Off => {
                let mut text = String::from("Proton VPN · not connected");
                if let Some(country) = self
                    .catalog
                    .as_ref()
                    .and_then(|catalog| catalog.fastest(&self.exclude))
                {
                    text.push_str(&format!("\nright click: connect to {}", country.name));
                } else {
                    text.push_str("\nright click: connect");
                }
                text
            }
        }
    }

    pub fn data(&self) -> VpnData {
        let state = self.bar_state();
        VpnData {
            state,
            target: self.target.clone(),
            connection: self.connection.clone(),
            server: self.server(),
            ip: if self.ip.is_empty() {
                self.connection
                    .as_ref()
                    .map(|c| c.ip.clone())
                    .unwrap_or_default()
            } else {
                self.ip.clone()
            },
            uptime: self
                .connection
                .as_ref()
                .filter(|c| c.since > 0 && self.link)
                .map_or(0, |c| vpn::now().saturating_sub(c.since)),
            rx: self.rx,
            tx: self.tx,
            down: self.down.clone(),
            up: self.up.clone(),
            catalog: self.catalog.clone(),
            loading: self.loading,
            refreshing: self.refreshing,
            home: self.home.clone(),
            exclude: self.exclude.clone(),
            settings: self.settings.clone(),
            pending_setting: self.pending_setting.clone(),
            favorites: self.saved.favorites.clone(),
            recent: self.saved.recent.clone(),
            message: self.message.clone(),
        }
    }

    fn remember(&mut self, code: &str) {
        let code = code.to_ascii_uppercase();
        self.saved.recent.retain(|known| *known != code);
        self.saved.recent.insert(0, code);
        self.saved.recent.truncate(6);
        store_saved(&self.saved);
    }
}

impl App {
    pub fn start_vpn(&mut self) {
        self.vpn.available = vpn::available();
        self.available.vpn = self.vpn.available;
        if !self.vpn.available {
            services::programs::warn_missing(vpn::PROGRAM, "VPN chip");
            return;
        }
        if self.vpn.started {
            self.vpn_config_changed();
            return;
        }
        self.vpn.started = true;
        self.vpn.saved = load_saved();
        self.vpn_config_changed();
        self.poll_vpn();
        self.load_vpn_catalog(false);
    }

    pub fn vpn_config_changed(&mut self) {
        self.vpn.home = vpn::home_country();
        self.vpn.exclude = vpn::excluded();
    }

    pub fn poll_vpn(&mut self) {
        if !self.vpn.available {
            return;
        }
        let link = vpn::link_up();
        let stamp = vpn::persistence_stamp();
        let mut changed = link != self.vpn.link;
        if stamp != self.vpn.stamp {
            self.vpn.stamp = stamp;
            self.vpn.connection = if stamp.is_some() {
                vpn::read_connection()
            } else {
                None
            };
            changed = true;
        }
        if link != self.vpn.link {
            let was = self.vpn.link;
            self.vpn.link = link;
            if !link {
                self.vpn.ip.clear();
                self.vpn.down.clear();
                self.vpn.up.clear();
                self.vpn.last = None;
                self.vpn.rx = 0;
                self.vpn.tx = 0;
                if was && !self.vpn.user_disconnect && !self.vpn.state.busy() {
                    self.vpn_notify(
                        "VPN connection lost",
                        "Proton VPN is no longer connected.",
                        Urgency::Critical,
                    );
                }
                self.vpn.user_disconnect = false;
            }
        }
        if changed {
            let name = self.vpn.server_name().map(str::to_string);
            if name.is_some() && name != self.vpn.catalog_for {
                self.load_vpn_catalog(false);
            }
            self.sync();
        }
    }

    fn sample_vpn(&mut self) {
        let Some((rx, tx)) = vpn::traffic() else {
            return;
        };
        let now = Instant::now();
        if let Some((last_rx, last_tx, at)) = self.vpn.last {
            let seconds = now.duration_since(at).as_secs_f64().max(0.001);
            self.vpn
                .down
                .push(rx.saturating_sub(last_rx) as f64 / seconds);
            self.vpn
                .up
                .push(tx.saturating_sub(last_tx) as f64 / seconds);
            for history in [&mut self.vpn.down, &mut self.vpn.up] {
                let excess = history.len().saturating_sub(60);
                history.drain(..excess);
            }
        }
        self.vpn.last = Some((rx, tx, now));
        self.vpn.rx = rx;
        self.vpn.tx = tx;
    }

    pub fn vpn_panel_changed(&mut self, panel: &str) {
        let open = panel == "vpn";
        if open == self.vpn.panel {
            return;
        }
        self.vpn.panel = open;
        if !open {
            self.vpn.down.clear();
            self.vpn.up.clear();
            self.vpn.last = None;
            self.vpn.message.clear();
            return;
        }
        self.poll_vpn();
        self.load_vpn_catalog(false);
        if self.vpn.settings.is_empty() {
            self.vpn_job(VpnJob::Settings);
        }
        self.sample_vpn();
        self.vpn.opened += 1;
        let opened = self.vpn.opened;
        self.add_timer(Duration::from_secs(1), move |app| {
            if !app.vpn.panel || app.vpn.opened != opened {
                return None;
            }
            app.poll_vpn();
            if app.vpn.link {
                app.sample_vpn();
            }
            app.sync();
            Some(Duration::from_secs(1))
        });
    }

    pub fn load_vpn_catalog(&mut self, force: bool) {
        if self.vpn.loading {
            return;
        }
        let stamp = vpn::serverlist_stamp();
        let current = self.vpn.server_name().map(str::to_string);
        if !force
            && self.vpn.catalog.is_some()
            && stamp == self.vpn.catalog_stamp
            && current == self.vpn.catalog_for
        {
            return;
        }
        if stamp.is_none() {
            return;
        }
        self.vpn.loading = true;
        self.vpn.catalog_stamp = stamp;
        self.vpn.catalog_for = current.clone();
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let result = vpn::load_catalog(current.as_deref());
            crate::memory::trim();
            let _ = sender.send(ServiceEvent::VpnCatalog(result.map(Arc::new)));
        });
    }

    pub fn vpn_catalog_loaded(&mut self, result: Result<Arc<vpn::Catalog>, String>) {
        self.vpn.loading = false;
        match result {
            Ok(catalog) => self.vpn.catalog = Some(catalog),
            Err(err) => {
                log::warn!("vpn server list: {err}");
                self.vpn.catalog_stamp = None;
            }
        }
        self.sync();
    }

    fn vpn_job(&mut self, job: VpnJob) {
        let args: Vec<String> = match &job {
            VpnJob::Connect(target) => target.args(),
            VpnJob::Disconnect => vec!["disconnect".into()],
            VpnJob::Setting(key, value) => {
                vec!["config".into(), "set".into(), key.clone(), value.clone()]
            }
            VpnJob::Settings => vec!["config".into(), "list".into()],
            VpnJob::Refresh => vec!["countries".into(), "list".into()],
        };
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let result = vpn::run(&args);
            let _ = sender.send(ServiceEvent::VpnDone(job, result));
        });
    }

    pub fn vpn_done(&mut self, job: VpnJob, result: Result<String, String>) {
        match job {
            VpnJob::Connect(target) => {
                self.vpn.state = VpnState::Off;
                self.poll_vpn();
                match result {
                    Ok(output) => {
                        self.vpn.message.clear();
                        self.vpn.ip = vpn::new_ip(&output).unwrap_or_default();
                        let summary = match self.vpn.server_name() {
                            Some(name) => format!("Connected to {name}"),
                            None => "VPN connected".to_string(),
                        };
                        if let Some(code) = self.vpn_country() {
                            self.vpn.remember(&code);
                        }
                        let mut body = vpn::last_line(&output);
                        if body.is_empty() {
                            body = target.describe();
                        }
                        self.vpn_notify(&summary, &body, Urgency::Low);
                    }
                    Err(err) => {
                        log::warn!("vpn connect: {err}");
                        self.vpn.message = err.clone();
                        self.vpn_notify(
                            &format!("Could not connect to {}", target.describe()),
                            &err,
                            Urgency::Normal,
                        );
                    }
                }
            }
            VpnJob::Disconnect => {
                self.vpn.state = VpnState::Off;
                self.poll_vpn();
                match result {
                    Ok(_) => {
                        self.vpn.message.clear();
                        self.vpn_notify(
                            "VPN disconnected",
                            "Traffic goes through your own connection now.",
                            Urgency::Low,
                        );
                    }
                    Err(err) => {
                        log::warn!("vpn disconnect: {err}");
                        self.vpn.message = err;
                    }
                }
            }
            VpnJob::Setting(_, _) => {
                self.vpn.pending_setting.clear();
                match result {
                    Ok(_) => self.vpn_job(VpnJob::Settings),
                    Err(err) => {
                        log::warn!("vpn setting: {err}");
                        self.vpn.message = err;
                    }
                }
            }
            VpnJob::Settings => match result {
                Ok(output) => self.vpn.settings = vpn::parse_settings(&output),
                Err(err) => log::warn!("vpn settings: {err}"),
            },
            VpnJob::Refresh => {
                self.vpn.refreshing = false;
                match result {
                    Ok(_) => self.load_vpn_catalog(true),
                    Err(err) => self.vpn.message = err,
                }
            }
        }
        self.sync();
    }

    fn vpn_country(&self) -> Option<String> {
        self.vpn.data().country()
    }

    fn vpn_notify(&self, summary: &str, body: &str, urgency: Urgency) {
        match &self.notes.handle {
            Some(handle) => {
                handle.post("Proton VPN", "network-vpn", summary, body, urgency);
            }
            None => log::info!("{summary}: {body}"),
        }
    }

    pub fn vpn_connect(&mut self, target: vpn::Target) {
        if !self.vpn.available || self.vpn.state.busy() {
            return;
        }
        self.vpn.state = VpnState::Connecting;
        self.vpn.target = match &target {
            vpn::Target::Fastest => "the fastest server".into(),
            other => other.describe(),
        };
        self.vpn.message.clear();
        self.vpn_job(VpnJob::Connect(target));
        self.sync();
    }

    pub fn vpn_connect_fastest(&mut self) {
        let target = match self.vpn.data().fastest() {
            Some(country) => vpn::Target::Country(country.code.clone()),
            None if self.vpn.exclude.is_empty() => vpn::Target::Fastest,
            None => {
                self.vpn.message = "the server list is not loaded yet".into();
                self.load_vpn_catalog(false);
                self.sync();
                return;
            }
        };
        self.vpn_connect(target);
    }

    pub fn vpn_disconnect(&mut self) {
        if !self.vpn.available || self.vpn.state.busy() {
            return;
        }
        self.vpn.state = VpnState::Disconnecting;
        self.vpn.user_disconnect = true;
        self.vpn.message.clear();
        self.vpn_job(VpnJob::Disconnect);
        self.sync();
    }

    pub fn vpn_toggle(&mut self) {
        if self.vpn.link {
            self.vpn_disconnect();
        } else {
            self.vpn_connect_fastest();
        }
    }

    pub fn vpn_action(&mut self, action: VpnAction) {
        match action {
            VpnAction::Fastest => self.vpn_connect_fastest(),
            VpnAction::Connect(target) => self.vpn_connect(target),
            VpnAction::Disconnect => self.vpn_disconnect(),
            VpnAction::Reconnect => {
                let target = match self.vpn_country() {
                    Some(code) => vpn::Target::Country(code),
                    None => vpn::Target::Fastest,
                };
                self.vpn_connect(target);
            }
            VpnAction::Refresh => {
                if self.vpn.refreshing {
                    return;
                }
                self.vpn.refreshing = true;
                self.vpn_job(VpnJob::Refresh);
                self.vpn_job(VpnJob::Settings);
                self.sync();
            }
            VpnAction::Setting(key) => {
                if !self.vpn.pending_setting.is_empty() {
                    return;
                }
                let current = self.vpn.settings.get(&key).cloned().unwrap_or_default();
                let Some(value) = vpn::next_value(&key, &current) else {
                    return;
                };
                self.vpn.pending_setting = key.clone();
                self.vpn_job(VpnJob::Setting(key, value.to_string()));
                self.sync();
            }
            VpnAction::Favorite(code) => {
                let favorites = &mut self.vpn.saved.favorites;
                if let Some(index) = favorites.iter().position(|known| *known == code) {
                    favorites.remove(index);
                } else {
                    favorites.push(code);
                }
                store_saved(&self.vpn.saved);
                self.sync();
            }
            VpnAction::CopyIp(ip) => {
                match std::process::Command::new("wl-copy")
                    .arg(&ip)
                    .stdin(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(child) => crate::app::reap(child),
                    Err(err) => log::warn!("wl-copy: {err}"),
                }
            }
        }
    }

    pub fn vpn_ipc(&mut self, args: &[String]) -> Result<String, String> {
        if !self.vpn.available {
            return Err(format!("{} is not installed", vpn::PROGRAM));
        }
        match args.first().map(String::as_str) {
            None => {
                self.state.toggle("vpn", false);
                self.sync();
            }
            Some("toggle") => self.vpn_toggle(),
            Some("connect") => match args.get(1) {
                Some(code) => self.vpn_connect(vpn::Target::Country(code.to_ascii_uppercase())),
                None => self.vpn_connect_fastest(),
            },
            Some("disconnect") => self.vpn_disconnect(),
            Some("status") => {
                let data = self.vpn.data();
                return Ok(serde_json::json!({
                    "state": data.state.name(),
                    "server": self.vpn.server_name(),
                    "country": data.country(),
                    "ip": data.ip,
                    "uptime": data.uptime,
                    "fastest": data.fastest().map(|country| country.code.clone()),
                    "exclude": data.exclude,
                })
                .to_string());
            }
            Some(other) => return Err(format!("unknown vpn command: {other}")),
        }
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations_and_sizes() {
        assert_eq!(duration(5), "5s");
        assert_eq!(duration(65), "1m 05s");
        assert_eq!(duration(3725), "1h 02m");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2048), "2 KiB");
        assert_eq!(bytes(5 * 1048576), "5.0 MiB");
        assert_eq!(bytes(3 * 1073741824), "3.00 GiB");
    }

    #[test]
    fn country_comes_from_server_name() {
        let data = VpnData {
            connection: Some(vpn::Connection {
                server: "AZ#27".into(),
                ..vpn::Connection::default()
            }),
            ..VpnData::default()
        };
        assert_eq!(data.country(), Some("AZ".into()));
        let data = VpnData {
            connection: Some(vpn::Connection {
                server: "SE-AZ#1".into(),
                ..vpn::Connection::default()
            }),
            server: Some(vpn::ServerInfo {
                country: "AZ".into(),
                ..vpn::ServerInfo::default()
            }),
            ..VpnData::default()
        };
        assert_eq!(data.country(), Some("AZ".into()));
    }

    #[test]
    fn saved_state_round_trips() {
        let saved = Saved {
            favorites: vec!["NL".into()],
            recent: vec!["AZ".into(), "DE".into()],
        };
        let text = serde_json::to_string(&saved).unwrap_or_default();
        assert_eq!(serde_json::from_str::<Saved>(&text).ok(), Some(saved));
        assert_eq!(
            serde_json::from_str::<Saved>("{}").ok(),
            Some(Saved::default())
        );
    }
}
