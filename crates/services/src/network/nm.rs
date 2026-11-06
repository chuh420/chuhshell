use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, future::Either};
use zbus::{
    MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant, interface, message::Type,
};

use super::{
    Event, Handle, Managed, Props, WifiDetails, WifiNetwork, WifiState, airplane_mode, boolean,
    error_text, int, interface as find_interface, oneshot, prop, string,
};

pub const NM: &str = "org.freedesktop.NetworkManager";
pub const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const OBJECTS: &str = "/org/freedesktop";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const IP4: &str = "org.freedesktop.NetworkManager.IP4Config";
const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const SECURITY: &str = "802-11-wireless-security";
const DEVICE_TYPE_WIFI: i64 = 2;
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;
const SCAN_LIMIT: Duration = Duration::from_secs(15);
const HIDDEN_LIMIT: Duration = Duration::from_secs(30);
const ACTIVATION_LIMIT: Duration = Duration::from_secs(120);
pub const WRONG_PASSWORD: &str = "Wrong password. Try again.";
pub const CHECK_PASSWORD: &str = "Connection failed. Check the password and try again.";

pub type Settings = HashMap<String, HashMap<String, OwnedVariant>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Key {
    #[default]
    Open,
    Owe,
    Wep,
    Psk,
    Sae,
    Mixed,
    Eap,
}

impl Key {
    pub fn from_flags(flags: u32, wpa: u32, rsn: u32) -> Self {
        let keys = wpa | rsn;
        let psk = keys & 0x100 != 0;
        let sae = keys & 0x400 != 0;
        if keys & (0x200 | 0x2000) != 0 {
            Key::Eap
        } else if psk && sae {
            Key::Mixed
        } else if psk {
            Key::Psk
        } else if sae {
            Key::Sae
        } else if keys & (0x800 | 0x1000) != 0 {
            Key::Owe
        } else if flags & 0x1 != 0 {
            Key::Wep
        } else {
            Key::Open
        }
    }

    pub fn from_key_mgmt(key_mgmt: &str) -> Self {
        match key_mgmt {
            "" => Key::Open,
            "owe" => Key::Owe,
            "none" => Key::Wep,
            "sae" => Key::Sae,
            "wpa-eap" | "wpa-eap-suite-b-192" | "ieee8021x" => Key::Eap,
            _ => Key::Psk,
        }
    }

    pub fn security(self) -> &'static str {
        match self {
            Key::Open | Key::Owe => "open",
            Key::Wep => "wep",
            Key::Psk | Key::Sae | Key::Mixed => "psk",
            Key::Eap => "8021x",
        }
    }

    pub fn key_mgmt(self) -> Option<&'static str> {
        match self {
            Key::Open => None,
            Key::Owe => Some("owe"),
            Key::Wep => Some("none"),
            Key::Psk | Key::Mixed => Some("wpa-psk"),
            Key::Sae => Some("sae"),
            Key::Eap => Some("wpa-eap"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessPoint {
    pub path: String,
    pub ssid: Vec<u8>,
    pub strength: u8,
    pub key: Key,
    pub frequency: u32,
}

impl AccessPoint {
    pub fn from_props(path: &str, props: &Props) -> Self {
        let number = |name: &str| prop(props, name).and_then(int).unwrap_or(0) as u32;
        AccessPoint {
            path: path.to_string(),
            ssid: prop(props, "Ssid").map(bytes).unwrap_or_default(),
            strength: number("Strength").min(100) as u8,
            key: Key::from_flags(number("Flags"), number("WpaFlags"), number("RsnFlags")),
            frequency: number("Frequency"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub path: String,
    pub ssid: Vec<u8>,
    pub name: String,
    pub autoconnect: bool,
    pub hidden: bool,
    pub security: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Activated,
    Cancelled,
    Failed(String),
}

pub fn bytes(value: &Variant<'_>) -> Vec<u8> {
    match value {
        Variant::Array(array) => array
            .inner()
            .iter()
            .filter_map(|item| match item {
                Variant::U8(byte) => Some(*byte),
                _ => None,
            })
            .collect(),
        Variant::Variant(inner) => bytes(inner),
        _ => Vec::new(),
    }
}

pub fn paths(value: &Variant<'_>) -> Vec<String> {
    match value {
        Variant::Array(array) => array
            .inner()
            .iter()
            .filter_map(|item| match item {
                Variant::ObjectPath(path) => Some(path.as_str().to_string()),
                _ => None,
            })
            .collect(),
        Variant::Variant(inner) => paths(inner),
        _ => Vec::new(),
    }
}

pub fn ssid_text(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid).into_owned()
}

pub fn device_state(state: u32) -> &'static str {
    match state {
        100 => "connected",
        40..=90 => "connecting",
        110 => "disconnecting",
        120 => "failed",
        30 => "disconnected",
        _ => "off",
    }
}

pub fn band(frequency: u32) -> &'static str {
    if frequency >= 5925 {
        "6 GHz"
    } else if frequency >= 4900 {
        "5 GHz"
    } else {
        "2.4 GHz"
    }
}

pub fn channel(frequency: u32) -> i64 {
    let frequency = frequency as i64;
    match frequency {
        2484 => 14,
        2412..=2472 => (frequency - 2407) / 5,
        5000..=5895 => (frequency - 5000) / 5,
        5955..=7115 => (frequency - 5950) / 5,
        _ => 0,
    }
}

pub fn rssi(strength: u8) -> i64 {
    strength as i64 / 2 - 100
}

pub fn failure(device_reasons: &[u32], active_reason: u32) -> Outcome {
    let seen = |reasons: &[u32]| device_reasons.iter().any(|reason| reasons.contains(reason));
    if seen(&[8, 9, 10, 11]) {
        Outcome::Failed(CHECK_PASSWORD.into())
    } else if seen(&[53]) {
        Outcome::Failed("Network not found".into())
    } else if seen(&[5, 6, 15, 16, 17]) || active_reason == 5 {
        Outcome::Failed("No IP address from the network".into())
    } else if seen(&[7]) || active_reason == 9 || active_reason == 2 {
        Outcome::Cancelled
    } else if active_reason == 6 {
        Outcome::Failed("Connection timed out".into())
    } else {
        Outcome::Failed("Connection failed".into())
    }
}

pub fn signal_matters(
    member: &str,
    path: &str,
    interface: &str,
    changed: &[&str],
    panel: bool,
    scanning: bool,
) -> bool {
    let any = |names: &[&str]| changed.iter().any(|key| names.contains(key));
    match member {
        "StateChanged" => !path.contains("/ActiveConnection/"),
        "NewConnection" | "ConnectionRemoved" | "Updated" => true,
        "AccessPointAdded" | "AccessPointRemoved" => panel,
        "PropertiesChanged" => match interface {
            NM => any(&[
                "WirelessEnabled",
                "WirelessHardwareEnabled",
                "ActiveConnections",
                "PrimaryConnection",
                "Devices",
            ]),
            DEVICE => any(&["State", "ActiveConnection", "Managed"]),
            WIRELESS => {
                any(&["ActiveAccessPoint"])
                    || (panel && any(&["AccessPoints"]))
                    || (scanning && any(&["LastScan"]))
            }
            ACCESS_POINT => panel && any(&["Strength", "Ssid", "Flags", "WpaFlags", "RsnFlags"]),
            _ => false,
        },
        _ => false,
    }
}

pub fn pick_device(objects: &Managed) -> Option<(String, &Props, Option<&Props>)> {
    let mut devices: Vec<(String, &Props, Option<&Props>)> = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let device = interfaces
                .iter()
                .find(|(name, _)| name.as_str() == DEVICE)
                .map(|(_, props)| props)?;
            if prop(device, "DeviceType").and_then(int) != Some(DEVICE_TYPE_WIFI) {
                return None;
            }
            let wireless = interfaces
                .iter()
                .find(|(name, _)| name.as_str() == WIRELESS)
                .map(|(_, props)| props);
            Some((path.to_string(), device, wireless))
        })
        .collect();
    devices.sort_by(|a, b| {
        let active = |props: &Props| {
            prop(props, "ActiveConnection")
                .map(string)
                .is_some_and(|path| !path.is_empty() && path != "/")
        };
        active(b.1)
            .cmp(&active(a.1))
            .then_with(|| natural(&a.0).cmp(&natural(&b.0)))
    });
    devices.into_iter().next()
}

fn natural(path: &str) -> (usize, String) {
    (path.len(), path.to_string())
}

pub fn ip4(config: &Props) -> (String, String) {
    let address = prop(config, "AddressData")
        .and_then(|value| match value {
            Variant::Array(array) => array.inner().first().cloned(),
            _ => None,
        })
        .and_then(|entry| match entry {
            Variant::Dict(dict) => {
                let mut address = String::new();
                let mut prefix = 0;
                for (key, value) in dict.iter() {
                    match string(key).as_str() {
                        "address" => address = string(value),
                        "prefix" => prefix = int(value).unwrap_or(0),
                        _ => {}
                    }
                }
                (!address.is_empty()).then(|| format!("{address}/{prefix}"))
            }
            _ => None,
        })
        .unwrap_or_default();
    let gateway = prop(config, "Gateway").map(string).unwrap_or_default();
    (address, gateway)
}

pub fn build_networks(
    points: &[AccessPoint],
    saved: &[Saved],
    active: Option<&[u8]>,
    connected: bool,
) -> Vec<WifiNetwork> {
    let mut best: Vec<&AccessPoint> = Vec::new();
    for point in points.iter().filter(|point| !point.ssid.is_empty()) {
        match best.iter_mut().find(|other| other.ssid == point.ssid) {
            Some(other) if other.strength < point.strength => *other = point,
            Some(_) => {}
            None => best.push(point),
        }
    }
    let mut networks: Vec<WifiNetwork> = best
        .into_iter()
        .map(|point| {
            let known = saved.iter().find(|entry| entry.ssid == point.ssid);
            WifiNetwork {
                path: point.path.clone(),
                name: ssid_text(&point.ssid),
                available: true,
                strength: point.strength as f64,
                security: point.key.security().to_string(),
                connected: connected && active == Some(point.ssid.as_slice()),
                saved: known.is_some(),
                known: known.map(|entry| entry.path.clone()).unwrap_or_default(),
                autoconnect: known.is_none_or(|entry| entry.autoconnect),
                hidden: known.is_some_and(|entry| entry.hidden),
            }
        })
        .collect();
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.strength.total_cmp(&a.strength))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut offline: Vec<&Saved> = saved
        .iter()
        .filter(|entry| !networks.iter().any(|network| network.known == entry.path))
        .collect();
    offline.sort_by(|a, b| a.path.cmp(&b.path));
    networks.extend(offline.into_iter().map(|entry| WifiNetwork {
        path: String::new(),
        name: entry.name.clone(),
        available: false,
        strength: 0.0,
        security: entry.security.to_string(),
        connected: false,
        saved: true,
        known: entry.path.clone(),
        autoconnect: entry.autoconnect,
        hidden: entry.hidden,
    }));
    networks
}

fn setting<'a>(settings: &'a Settings, group: &str, key: &str) -> Option<&'a Variant<'static>> {
    settings.get(group)?.get(key).map(|value| &**value)
}

pub fn saved_from(path: &str, settings: &Settings) -> Option<Saved> {
    let kind = setting(settings, "connection", "type").map(string)?;
    if kind != "802-11-wireless" {
        return None;
    }
    let ssid = setting(settings, "802-11-wireless", "ssid")
        .map(bytes)
        .unwrap_or_default();
    let name = setting(settings, "connection", "id")
        .map(string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| ssid_text(&ssid));
    let key_mgmt = setting(settings, SECURITY, "key-mgmt")
        .map(string)
        .unwrap_or_default();
    Some(Saved {
        path: path.to_string(),
        ssid,
        name,
        autoconnect: setting(settings, "connection", "autoconnect")
            .and_then(boolean)
            .unwrap_or(true),
        hidden: setting(settings, "802-11-wireless", "hidden")
            .and_then(boolean)
            .unwrap_or(false),
        security: Key::from_key_mgmt(&key_mgmt).security(),
    })
}

pub fn prompt_name(settings: &Settings) -> String {
    setting(settings, "802-11-wireless", "ssid")
        .map(bytes)
        .filter(|ssid| !ssid.is_empty())
        .map(|ssid| ssid_text(&ssid))
        .or_else(|| setting(settings, "connection", "id").map(string))
        .unwrap_or_else(|| "Wi-Fi".into())
}

fn text_value(value: &str) -> OwnedVariant {
    OwnedVariant::from(zbus::Str::from(value.to_string()))
}

pub fn secrets_reply(
    connection: &Settings,
    setting_name: &str,
    password: &str,
) -> Option<Settings> {
    let key = match setting_name {
        SECURITY => {
            let key_mgmt = setting(connection, SECURITY, "key-mgmt")
                .map(string)
                .unwrap_or_default();
            match Key::from_key_mgmt(&key_mgmt) {
                Key::Psk | Key::Sae | Key::Mixed => "psk",
                Key::Wep => "wep-key0",
                Key::Open | Key::Owe | Key::Eap => return None,
            }
        }
        "802-1x" => "password",
        _ => return None,
    };
    Some(HashMap::from([(
        setting_name.to_string(),
        HashMap::from([(key.to_string(), text_value(password))]),
    )]))
}

fn owned(value: Variant<'_>) -> Option<OwnedVariant> {
    OwnedVariant::try_from(value).ok()
}

pub fn connection_settings(ssid: &[u8], key: Key, hidden: bool) -> Settings {
    let mut wireless = HashMap::new();
    if let Some(value) = owned(Variant::from(ssid.to_vec())) {
        wireless.insert("ssid".to_string(), value);
    }
    if hidden {
        wireless.insert("hidden".to_string(), OwnedVariant::from(true));
    }
    let mut settings = HashMap::from([("802-11-wireless".to_string(), wireless)]);
    if let Some(key_mgmt) = key.key_mgmt() {
        let mut group = HashMap::from([("key-mgmt".to_string(), text_value(key_mgmt))]);
        if key == Key::Wep {
            group.insert("wep-key-type".to_string(), OwnedVariant::from(2u32));
        }
        settings.insert(SECURITY.to_string(), group);
    }
    settings
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    UserCanceled(String),
    NoSecrets(String),
    PermissionDenied(String),
}

pub struct Agent {
    pub handle: Handle,
}

#[interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
    ) -> Result<Settings, AgentError> {
        if !self.handle.sent_by(&header, NM).await {
            return Err(AgentError::PermissionDenied("Not NetworkManager".into()));
        }
        if flags & ALLOW_INTERACTION == 0 || secrets_reply(&connection, &setting_name, "").is_none()
        {
            return Err(AgentError::NoSecrets("No secrets".into()));
        }
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.wifi_pending = Some(sender);
            inner.pending_path = connection_path.to_string();
            if flags & REQUEST_NEW != 0 {
                inner.message = WRONG_PASSWORD.into();
            }
        }
        if flags & REQUEST_NEW != 0 {
            self.handle.refresh_wifi().await;
        }
        (self.handle.callback)(Event::WifiPrompt(prompt_name(&connection)));
        match receiver.await {
            Some(password) => secrets_reply(&connection, &setting_name, &password)
                .ok_or_else(|| AgentError::NoSecrets("No secrets".into())),
            None => Err(AgentError::UserCanceled("Cancelled".into())),
        }
    }

    async fn cancel_get_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connection_path: OwnedObjectPath,
        _setting_name: String,
    ) {
        if !self.handle.sent_by(&header, NM).await {
            return;
        }
        let pending = self.handle.inner.lock().ok().and_then(|mut inner| {
            (inner.pending_path == connection_path.as_str()).then(|| {
                inner.pending_path.clear();
                inner.wifi_pending.take()
            })
        });
        if let Some(pending) = pending {
            drop(pending);
            (self.handle.callback)(Event::WifiPrompt(String::new()));
        }
    }

    async fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}

    async fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
}

struct Snapshot {
    objects: Managed,
    device: String,
}

impl Handle {
    async fn nm_objects(&self) -> zbus::Result<Managed> {
        let reply = self
            .call(
                NM,
                OBJECTS,
                "org.freedesktop.DBus.ObjectManager",
                "GetManagedObjects",
                &(),
            )
            .await?;
        reply.body().deserialize::<Managed>()
    }

    async fn nm_snapshot(&self) -> Result<Snapshot, String> {
        let objects = self.nm_objects().await.map_err(|err| error_text(&err))?;
        let device = pick_device(&objects)
            .map(|(path, _, _)| path)
            .ok_or_else(|| "No Wi-Fi adapter found".to_string())?;
        Ok(Snapshot { objects, device })
    }

    pub(super) async fn nm_device(&self) -> Result<String, String> {
        self.nm_snapshot().await.map(|snapshot| snapshot.device)
    }

    async fn nm_saved(&self) -> Vec<Saved> {
        let Ok(reply) = self
            .call(
                NM,
                SETTINGS,
                "org.freedesktop.NetworkManager.Settings",
                "ListConnections",
                &(),
            )
            .await
        else {
            return Vec::new();
        };
        let Ok(paths) = reply.body().deserialize::<Vec<OwnedObjectPath>>() else {
            return Vec::new();
        };
        let mut cache = self
            .inner
            .lock()
            .map(|inner| inner.nm_saved.clone())
            .unwrap_or_default();
        cache.retain(|path, _| paths.iter().any(|known| known.as_str() == path));
        let mut saved = Vec::new();
        for path in paths {
            let entry = match cache.get(path.as_str()) {
                Some(entry) => entry.clone(),
                None => {
                    let entry = match self
                        .call(NM, path.as_str(), CONNECTION, "GetSettings", &())
                        .await
                    {
                        Ok(reply) => reply
                            .body()
                            .deserialize::<Settings>()
                            .ok()
                            .and_then(|settings| saved_from(path.as_str(), &settings)),
                        Err(_) => continue,
                    };
                    cache.insert(path.to_string(), entry.clone());
                    entry
                }
            };
            saved.extend(entry);
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.nm_saved = cache;
        }
        saved
    }

    pub(super) fn nm_forget_cached(&self, path: Option<&str>) {
        if let Ok(mut inner) = self.inner.lock() {
            match path {
                Some(path) => {
                    inner.nm_saved.remove(path);
                }
                None => inner.nm_saved.clear(),
            }
        }
    }

    pub(super) async fn nm_read_wifi(&self) -> Result<WifiState, String> {
        let snapshot = self.nm_snapshot().await?;
        let objects = &snapshot.objects;
        let empty = Props::new();
        let root = find_interface(objects, ROOT, NM).unwrap_or(&empty);
        let device = find_interface(objects, &snapshot.device, DEVICE).unwrap_or(&empty);
        let wireless = find_interface(objects, &snapshot.device, WIRELESS).unwrap_or(&empty);
        let powered = prop(root, "WirelessEnabled")
            .and_then(boolean)
            .unwrap_or(false)
            && prop(root, "WirelessHardwareEnabled")
                .and_then(boolean)
                .unwrap_or(true);
        let state = device_state(prop(device, "State").and_then(int).unwrap_or(0) as u32);
        let connected = powered && state == "connected";
        let active_path = prop(wireless, "ActiveAccessPoint")
            .map(string)
            .unwrap_or_default();
        let points: Vec<AccessPoint> = if powered {
            prop(wireless, "AccessPoints")
                .map(paths)
                .unwrap_or_default()
                .iter()
                .filter_map(|path| {
                    find_interface(objects, path, ACCESS_POINT)
                        .map(|props| AccessPoint::from_props(path, props))
                })
                .collect()
        } else {
            Vec::new()
        };
        let active = points
            .iter()
            .find(|point| point.path == active_path)
            .map(|point| point.ssid.clone());
        let saved = self.nm_saved().await;
        let networks = build_networks(&points, &saved, active.as_deref(), connected);
        let network = if connected {
            active
                .map(|ssid| ssid_text(&ssid))
                .unwrap_or_else(|| "Wi-Fi".into())
        } else {
            "Offline".into()
        };
        let last_scan = prop(wireless, "LastScan").and_then(int).unwrap_or(-1);
        let (busy, message, scanning) = self
            .inner
            .lock()
            .map(|mut inner| {
                let scanning = inner.nm_scan.is_some_and(|(started, base)| {
                    powered && last_scan == base && started.elapsed() < SCAN_LIMIT
                });
                if !scanning {
                    inner.nm_scan = None;
                }
                (inner.busy, inner.message.clone(), scanning)
            })
            .unwrap_or_default();
        Ok(WifiState {
            powered,
            state: if powered { state } else { "off" }.into(),
            scanning,
            networks,
            busy,
            message,
            device: prop(device, "Interface").map(string).unwrap_or_default(),
            connected,
            network,
            airplane: airplane_mode(),
            backend: "networkmanager".into(),
        })
    }

    pub(super) async fn nm_diagnostics(&self, details: &mut WifiDetails) -> bool {
        let Ok(snapshot) = self.nm_snapshot().await else {
            return false;
        };
        let objects = &snapshot.objects;
        let empty = Props::new();
        let device = find_interface(objects, &snapshot.device, DEVICE).unwrap_or(&empty);
        let wireless = find_interface(objects, &snapshot.device, WIRELESS).unwrap_or(&empty);
        let active = prop(wireless, "ActiveAccessPoint")
            .map(string)
            .unwrap_or_default();
        if let Some(props) = find_interface(objects, &active, ACCESS_POINT) {
            let point = AccessPoint::from_props(&active, props);
            details.band = band(point.frequency).into();
            details.channel = channel(point.frequency);
            details.rssi = rssi(point.strength);
        }
        details.rx_rate = prop(wireless, "Bitrate").and_then(int).unwrap_or(0) as f64 / 1000.0;
        let config = prop(device, "Ip4Config").map(string).unwrap_or_default();
        match find_interface(objects, &config, IP4) {
            Some(props) => {
                (details.ip, details.gateway) = ip4(props);
                true
            }
            None => false,
        }
    }

    pub(super) async fn nm_power(&self, enabled: bool) -> zbus::Result<bool> {
        self.set_property(NM, ROOT, NM, "WirelessEnabled", Variant::Bool(enabled))
            .await
            .map(|_| false)
    }

    pub(super) async fn nm_scan(&self, device: &str) -> zbus::Result<bool> {
        self.nm_request_scan(device, None).await.map(|_| true)
    }

    async fn nm_request_scan(&self, device: &str, ssid: Option<&[u8]>) -> zbus::Result<bool> {
        let base = self
            .property(NM, device, WIRELESS, "LastScan")
            .await
            .ok()
            .and_then(|value| int(&value))
            .unwrap_or(-1);
        let mut options: HashMap<String, OwnedVariant> = HashMap::new();
        if let Some(ssid) = ssid
            && let Some(value) = owned(Variant::from(vec![ssid.to_vec()]))
        {
            options.insert("ssids".to_string(), value);
        }
        match self
            .call(NM, device, WIRELESS, "RequestScan", &(options,))
            .await
        {
            Ok(_) => {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.nm_scan = Some((Instant::now(), base));
                }
                Ok(true)
            }
            Err(zbus::Error::MethodError(name, _, _)) if name.ends_with(".NotAllowed") => Ok(false),
            Err(err) => Err(err),
        }
    }

    async fn nm_access_point(&self, path: &str) -> Option<AccessPoint> {
        let reply = self
            .call(
                NM,
                path,
                "org.freedesktop.DBus.Properties",
                "GetAll",
                &(ACCESS_POINT,),
            )
            .await
            .ok()?;
        let props = reply.body().deserialize::<Props>().ok()?;
        Some(AccessPoint::from_props(path, &props))
    }

    async fn nm_activation_streams(
        &self,
        device: &str,
    ) -> zbus::Result<futures_util::stream::SelectAll<MessageStream>> {
        let rules = [
            MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(NM)
                .interface(ACTIVE)
                .member("StateChanged")
                .build()?,
            MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(NM)
                .path(device)
                .interface(DEVICE)
                .member("StateChanged")
                .build()?,
            MatchRule::builder()
                .msg_type(Type::Signal)
                .sender("org.freedesktop.DBus")
                .member("NameOwnerChanged")
                .arg(0, NM)
                .build()?,
        ];
        let mut streams = Vec::new();
        for rule in rules {
            streams.push(MessageStream::for_match_rule(rule, &self.connection, None).await?);
        }
        Ok(futures_util::stream::select_all(streams))
    }

    async fn nm_follow(
        &self,
        mut streams: futures_util::stream::SelectAll<MessageStream>,
        active: &str,
    ) -> Outcome {
        let mut reasons = Vec::new();
        match self.property(NM, active, ACTIVE, "State").await {
            Ok(value) if int(&value) == Some(2) => return Outcome::Activated,
            Ok(value) if int(&value) == Some(4) => return failure(&reasons, 0),
            Ok(_) => {}
            Err(_) => return Outcome::Failed("Connection failed".into()),
        }
        let mut limit = oneshot::sleep(ACTIVATION_LIMIT);
        loop {
            let next = match futures_util::future::select(streams.next(), &mut limit).await {
                Either::Left((Some(Ok(message)), _)) => message,
                Either::Left((Some(Err(_)), _)) => continue,
                Either::Left((None, _)) | Either::Right(_) => {
                    return Outcome::Failed("Connection timed out".into());
                }
            };
            let header = next.header();
            let member = header.member().map(|m| m.as_str()).unwrap_or_default();
            if member == "NameOwnerChanged" {
                return Outcome::Failed("NetworkManager stopped".into());
            }
            let iface = header.interface().map(|i| i.as_str()).unwrap_or_default();
            let path = header.path().map(|p| p.as_str()).unwrap_or_default();
            if iface == DEVICE {
                if let Ok((_, _, reason)) = next.body().deserialize::<(u32, u32, u32)>() {
                    reasons.push(reason);
                }
            } else if iface == ACTIVE
                && path == active
                && let Ok((state, reason)) = next.body().deserialize::<(u32, u32)>()
            {
                match state {
                    2 => return Outcome::Activated,
                    4 => return failure(&reasons, reason),
                    _ => {}
                }
            }
        }
    }

    async fn nm_activate(
        &self,
        device: &str,
        start: impl AsyncFnOnce(&Self) -> zbus::Result<(Option<OwnedObjectPath>, OwnedObjectPath)>,
    ) -> zbus::Result<bool> {
        let streams = self.nm_activation_streams(device).await?;
        let (created, active) = start(self).await?;
        let outcome = self.nm_follow(streams, active.as_str()).await;
        if outcome != Outcome::Activated
            && let Some(created) = created
        {
            if let Err(err) = self
                .call(NM, created.as_str(), CONNECTION, "Delete", &())
                .await
            {
                log::warn!(
                    "networkmanager: removing a failed profile: {}",
                    error_text(&err)
                );
            }
            self.nm_forget_cached(Some(created.as_str()));
        }
        match outcome {
            Outcome::Activated | Outcome::Cancelled => Ok(true),
            Outcome::Failed(message) => Err(zbus::Error::Failure(message)),
        }
    }

    async fn nm_add(&self, device: &str, settings: Settings, point: &str) -> zbus::Result<bool> {
        let device_path = zbus::ObjectPath::try_from(device)?;
        let point_path = zbus::ObjectPath::try_from(point)?;
        self.nm_activate(device, async |handle| {
            let reply = handle
                .call(
                    NM,
                    ROOT,
                    NM,
                    "AddAndActivateConnection",
                    &(settings, device_path, point_path),
                )
                .await?;
            let (created, active) = reply
                .body()
                .deserialize::<(OwnedObjectPath, OwnedObjectPath)>()?;
            Ok((Some(created), active))
        })
        .await
    }

    pub(super) async fn nm_connect(&self, device: &str, point: &str) -> zbus::Result<bool> {
        let target = self
            .nm_access_point(point)
            .await
            .ok_or_else(|| zbus::Error::Failure("Network not found".into()))?;
        let saved = self.nm_saved().await;
        if let Some(entry) = saved.iter().find(|entry| entry.ssid == target.ssid) {
            let connection = zbus::ObjectPath::try_from(entry.path.as_str())?;
            let device_path = zbus::ObjectPath::try_from(device)?;
            let point_path = zbus::ObjectPath::try_from(point)?;
            return self
                .nm_activate(device, async |handle| {
                    let reply = handle
                        .call(
                            NM,
                            ROOT,
                            NM,
                            "ActivateConnection",
                            &(connection, device_path, point_path),
                        )
                        .await?;
                    Ok((None, reply.body().deserialize::<OwnedObjectPath>()?))
                })
                .await;
        }
        if target.key == Key::Eap {
            return Err(zbus::Error::Failure(
                "Enterprise networks need a saved connection".into(),
            ));
        }
        let settings = connection_settings(&target.ssid, target.key, false);
        self.nm_add(device, settings, point).await
    }

    async fn nm_find(&self, device: &str, ssid: &[u8]) -> Option<AccessPoint> {
        let objects = self.nm_objects().await.ok()?;
        let wireless = find_interface(&objects, device, WIRELESS)?;
        prop(wireless, "AccessPoints")
            .map(paths)
            .unwrap_or_default()
            .iter()
            .filter_map(|path| {
                find_interface(&objects, path, ACCESS_POINT)
                    .map(|props| AccessPoint::from_props(path, props))
            })
            .filter(|point| point.ssid == ssid)
            .max_by_key(|point| point.strength)
    }

    pub(super) async fn nm_connect_hidden(&self, device: &str, name: &str) -> zbus::Result<bool> {
        let ssid = name.as_bytes();
        let mut found = self.nm_find(device, ssid).await;
        let deadline = Instant::now() + HIDDEN_LIMIT;
        while found.is_none() && Instant::now() < deadline {
            let polls = if self.nm_request_scan(device, Some(ssid)).await? {
                10
            } else {
                2
            };
            for _ in 0..polls {
                oneshot::sleep(Duration::from_millis(500)).await;
                found = self.nm_find(device, ssid).await;
                if found.is_some() {
                    break;
                }
            }
        }
        let point = found.ok_or_else(|| zbus::Error::Failure("Network not found".into()))?;
        if point.key == Key::Eap {
            return Err(zbus::Error::Failure(
                "Enterprise networks need a saved connection".into(),
            ));
        }
        let saved = self.nm_saved().await;
        if saved.iter().any(|entry| entry.ssid == ssid) {
            return self.nm_connect(device, &point.path).await;
        }
        let settings = connection_settings(ssid, point.key, true);
        self.nm_add(device, settings, "/").await
    }

    pub(super) async fn nm_disconnect(&self, device: &str) -> zbus::Result<bool> {
        self.call(NM, device, DEVICE, "Disconnect", &())
            .await
            .map(|_| true)
    }

    pub(super) async fn nm_forget(&self, connection: &str) -> zbus::Result<bool> {
        let result = self
            .call(NM, connection, CONNECTION, "Delete", &())
            .await
            .map(|_| false);
        self.nm_forget_cached(Some(connection));
        result
    }

    pub(super) async fn nm_autoconnect(
        &self,
        connection: &str,
        enabled: bool,
    ) -> zbus::Result<bool> {
        let reply = self
            .call(NM, connection, CONNECTION, "GetSettings", &())
            .await?;
        let mut settings = reply.body().deserialize::<Settings>()?;
        if settings.contains_key(SECURITY) {
            let reply = self
                .call(NM, connection, CONNECTION, "GetSecrets", &(SECURITY,))
                .await?;
            let secrets = reply.body().deserialize::<Settings>()?;
            for (group, values) in secrets {
                settings.entry(group).or_default().extend(values);
            }
        }
        settings
            .entry("connection".to_string())
            .or_default()
            .insert("autoconnect".to_string(), OwnedVariant::from(enabled));
        let result = self
            .call(NM, connection, CONNECTION, "Update", &(settings,))
            .await
            .map(|_| false);
        self.nm_forget_cached(Some(connection));
        result
    }

    pub(super) async fn nm_register_agent(&self) -> zbus::Result<()> {
        self.call(
            NM,
            "/org/freedesktop/NetworkManager/AgentManager",
            "org.freedesktop.NetworkManager.AgentManager",
            "Register",
            &("chuhshell",),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(path: &str, ssid: &str, strength: u8, key: Key) -> AccessPoint {
        AccessPoint {
            path: path.into(),
            ssid: ssid.as_bytes().to_vec(),
            strength,
            key,
            frequency: 2437,
        }
    }

    #[test]
    fn key_management_from_access_point_flags() {
        assert_eq!(Key::from_flags(0, 0, 0), Key::Open);
        assert_eq!(Key::from_flags(1, 0, 0), Key::Wep);
        assert_eq!(Key::from_flags(1, 0, 0x188), Key::Psk);
        assert_eq!(Key::from_flags(1, 0x148, 0), Key::Psk);
        assert_eq!(Key::from_flags(1, 0, 0x488), Key::Sae);
        assert_eq!(Key::from_flags(1, 0, 0x588), Key::Mixed);
        assert_eq!(Key::from_flags(1, 0, 0x888), Key::Owe);
        assert_eq!(Key::from_flags(0, 0, 0x1000), Key::Owe);
        assert_eq!(Key::from_flags(1, 0x200, 0), Key::Eap);
        assert_eq!(Key::from_flags(1, 0, 0x2000), Key::Eap);
        assert_eq!(Key::from_flags(1, 0, 0xd88), Key::Mixed);
    }

    #[test]
    fn keys_map_to_ui_security_and_profiles() {
        for (key, security, key_mgmt) in [
            (Key::Open, "open", None),
            (Key::Owe, "open", Some("owe")),
            (Key::Wep, "wep", Some("none")),
            (Key::Psk, "psk", Some("wpa-psk")),
            (Key::Mixed, "psk", Some("wpa-psk")),
            (Key::Sae, "psk", Some("sae")),
            (Key::Eap, "8021x", Some("wpa-eap")),
        ] {
            assert_eq!(key.security(), security, "{key:?}");
            assert_eq!(key.key_mgmt(), key_mgmt, "{key:?}");
        }
        for (key_mgmt, key) in [
            ("", Key::Open),
            ("owe", Key::Owe),
            ("none", Key::Wep),
            ("wpa-psk", Key::Psk),
            ("sae", Key::Sae),
            ("wpa-eap", Key::Eap),
            ("wpa-eap-suite-b-192", Key::Eap),
            ("ieee8021x", Key::Eap),
        ] {
            assert_eq!(Key::from_key_mgmt(key_mgmt), key, "{key_mgmt}");
        }
    }

    #[test]
    fn states_and_radio() {
        assert_eq!(device_state(100), "connected");
        assert_eq!(device_state(70), "connecting");
        assert_eq!(device_state(60), "connecting");
        assert_eq!(device_state(30), "disconnected");
        assert_eq!(device_state(110), "disconnecting");
        assert_eq!(device_state(120), "failed");
        assert_eq!(device_state(20), "off");
        assert_eq!(device_state(10), "off");
        assert_eq!(channel(2412), 1);
        assert_eq!(channel(2437), 6);
        assert_eq!(channel(2484), 14);
        assert_eq!(channel(5180), 36);
        assert_eq!(channel(5975), 5);
        assert_eq!(channel(1), 0);
        assert_eq!(band(2412), "2.4 GHz");
        assert_eq!(band(5180), "5 GHz");
        assert_eq!(band(6000), "6 GHz");
        assert_eq!(rssi(100), -50);
        assert_eq!(rssi(50), -75);
        assert_eq!(rssi(0), -100);
    }

    #[test]
    fn failure_reasons_read_like_the_iwd_messages() {
        assert_eq!(failure(&[8, 7], 9), Outcome::Failed(CHECK_PASSWORD.into()));
        assert_eq!(failure(&[11], 3), Outcome::Failed(CHECK_PASSWORD.into()));
        assert_eq!(failure(&[7], 9), Outcome::Cancelled);
        assert_eq!(failure(&[], 2), Outcome::Cancelled);
        assert_eq!(
            failure(&[53], 3),
            Outcome::Failed("Network not found".into())
        );
        assert_eq!(
            failure(&[17], 3),
            Outcome::Failed("No IP address from the network".into())
        );
        assert_eq!(
            failure(&[], 5),
            Outcome::Failed("No IP address from the network".into())
        );
        assert_eq!(
            failure(&[], 6),
            Outcome::Failed("Connection timed out".into())
        );
        assert_eq!(failure(&[], 3), Outcome::Failed("Connection failed".into()));
    }

    #[test]
    fn idle_signals_are_ignored() {
        let ap = "/org/freedesktop/NetworkManager/AccessPoint/4";
        let device = "/org/freedesktop/NetworkManager/Devices/2";
        assert!(!signal_matters(
            "PropertiesChanged",
            ap,
            ACCESS_POINT,
            &["LastSeen"],
            true,
            false
        ));
        assert!(!signal_matters(
            "PropertiesChanged",
            ap,
            ACCESS_POINT,
            &["Strength"],
            false,
            false
        ));
        assert!(signal_matters(
            "PropertiesChanged",
            ap,
            ACCESS_POINT,
            &["Strength"],
            true,
            false
        ));
        assert!(!signal_matters(
            "PropertiesChanged",
            device,
            WIRELESS,
            &["LastScan"],
            true,
            false
        ));
        assert!(signal_matters(
            "PropertiesChanged",
            device,
            WIRELESS,
            &["LastScan"],
            false,
            true
        ));
        assert!(signal_matters(
            "PropertiesChanged",
            device,
            WIRELESS,
            &["ActiveAccessPoint"],
            false,
            false
        ));
        assert!(!signal_matters(
            "PropertiesChanged",
            device,
            WIRELESS,
            &["AccessPoints"],
            false,
            false
        ));
        assert!(signal_matters(
            "PropertiesChanged",
            device,
            WIRELESS,
            &["AccessPoints"],
            true,
            false
        ));
        assert!(signal_matters(
            "PropertiesChanged",
            device,
            DEVICE,
            &["State", "StateReason"],
            false,
            false
        ));
        assert!(!signal_matters(
            "PropertiesChanged",
            device,
            DEVICE,
            &["Ip4Config"],
            false,
            false
        ));
        assert!(!signal_matters(
            "PropertiesChanged",
            device,
            "org.freedesktop.NetworkManager.Device.Statistics",
            &["RxBytes"],
            true,
            false
        ));
        assert!(signal_matters(
            "PropertiesChanged",
            ROOT,
            NM,
            &["WirelessEnabled"],
            false,
            false
        ));
        assert!(!signal_matters(
            "PropertiesChanged",
            ROOT,
            NM,
            &["Connectivity"],
            false,
            false
        ));
        assert!(signal_matters(
            "StateChanged",
            device,
            DEVICE,
            &[],
            false,
            false
        ));
        assert!(!signal_matters(
            "StateChanged",
            "/org/freedesktop/NetworkManager/ActiveConnection/3",
            ACTIVE,
            &[],
            false,
            false
        ));
        assert!(!signal_matters(
            "AccessPointAdded",
            device,
            WIRELESS,
            &[],
            false,
            false
        ));
        assert!(signal_matters(
            "AccessPointAdded",
            device,
            WIRELESS,
            &[],
            true,
            false
        ));
        assert!(signal_matters(
            "Updated",
            "/org/freedesktop/NetworkManager/Settings/1",
            CONNECTION,
            &[],
            false,
            false
        ));
    }

    #[test]
    fn networks_merge_access_points_and_saved_connections() {
        let points = vec![
            point("/ap/1", "home", 40, Key::Psk),
            point("/ap/2", "home", 80, Key::Psk),
            point("/ap/3", "cafe", 90, Key::Open),
            point("/ap/4", "", 99, Key::Open),
            point("/ap/5", "lab", 90, Key::Sae),
        ];
        let saved = vec![
            Saved {
                path: "/conn/1".into(),
                ssid: b"home".to_vec(),
                name: "home".into(),
                autoconnect: false,
                hidden: false,
                security: "psk",
            },
            Saved {
                path: "/conn/2".into(),
                ssid: b"office".to_vec(),
                name: "office".into(),
                autoconnect: true,
                hidden: true,
                security: "8021x",
            },
        ];
        let networks = build_networks(&points, &saved, Some(b"home"), true);
        let names: Vec<&str> = networks.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["home", "cafe", "lab", "office"]);
        assert_eq!(networks[0].path, "/ap/2");
        assert!(networks[0].connected && networks[0].saved && !networks[0].autoconnect);
        assert_eq!(networks[0].known, "/conn/1");
        assert!(!networks[1].saved && networks[1].autoconnect);
        assert_eq!(networks[2].security, "psk");
        assert!(!networks[3].available && networks[3].hidden);
        assert_eq!(networks[3].security, "8021x");
        let offline = build_networks(&points, &saved, Some(b"home"), false);
        assert!(offline.iter().all(|n| !n.connected));
        assert!(build_networks(&[], &[], None, false).is_empty());
    }

    fn text(value: &str) -> OwnedVariant {
        text_value(value)
    }

    fn wifi_settings(key_mgmt: Option<&str>) -> Settings {
        let mut settings: Settings = HashMap::new();
        settings.insert(
            "connection".into(),
            HashMap::from([
                ("type".to_string(), text("802-11-wireless")),
                ("id".to_string(), text("Home 5G")),
                ("autoconnect".to_string(), OwnedVariant::from(false)),
            ]),
        );
        settings.insert(
            "802-11-wireless".into(),
            HashMap::from([(
                "ssid".to_string(),
                owned(Variant::from(b"home".to_vec())).expect("ssid"),
            )]),
        );
        if let Some(key_mgmt) = key_mgmt {
            settings.insert(
                SECURITY.into(),
                HashMap::from([("key-mgmt".to_string(), text(key_mgmt))]),
            );
        }
        settings
    }

    #[test]
    fn saved_connection_properties_parse() {
        let settings = wifi_settings(Some("sae"));
        let saved = saved_from("/conn/7", &settings).expect("wifi connection");
        assert_eq!(saved.ssid, b"home");
        assert_eq!(saved.name, "Home 5G");
        assert!(!saved.autoconnect && !saved.hidden);
        assert_eq!(saved.security, "psk");
        assert_eq!(prompt_name(&settings), "home");
        let owe = saved_from("/conn/9", &wifi_settings(Some("owe"))).expect("owe");
        assert_eq!(owe.security, "open");
        let open = saved_from("/conn/10", &wifi_settings(None)).expect("open");
        assert_eq!(open.security, "open");
        let mut wired = settings.clone();
        wired
            .get_mut("connection")
            .expect("group")
            .insert("type".into(), text("802-3-ethernet"));
        assert_eq!(saved_from("/conn/8", &wired), None);
        assert_eq!(saved_from("/conn/11", &HashMap::new()), None);
        assert_eq!(prompt_name(&HashMap::new()), "Wi-Fi");
    }

    fn secret(reply: &Settings, group: &str, key: &str) -> Option<String> {
        reply.get(group)?.get(key).map(|value| string(value))
    }

    #[test]
    fn secrets_follow_the_key_management() {
        let psk =
            secrets_reply(&wifi_settings(Some("wpa-psk")), SECURITY, "hunter22").expect("psk");
        assert_eq!(secret(&psk, SECURITY, "psk").as_deref(), Some("hunter22"));
        let sae = secrets_reply(&wifi_settings(Some("sae")), SECURITY, "x").expect("sae");
        assert_eq!(secret(&sae, SECURITY, "psk").as_deref(), Some("x"));
        let wep = secrets_reply(&wifi_settings(Some("none")), SECURITY, "abcde").expect("wep");
        assert_eq!(secret(&wep, SECURITY, "wep-key0").as_deref(), Some("abcde"));
        let eap = secrets_reply(&wifi_settings(Some("wpa-eap")), "802-1x", "pw").expect("eap");
        assert_eq!(secret(&eap, "802-1x", "password").as_deref(), Some("pw"));
        assert!(secrets_reply(&wifi_settings(Some("wpa-eap")), SECURITY, "pw").is_none());
        assert!(secrets_reply(&wifi_settings(Some("owe")), SECURITY, "pw").is_none());
        assert!(secrets_reply(&wifi_settings(None), SECURITY, "pw").is_none());
        assert!(secrets_reply(&wifi_settings(Some("wpa-psk")), "vpn", "pw").is_none());
    }

    #[test]
    fn new_connections_carry_the_right_key_management() {
        let open = connection_settings(b"cafe", Key::Open, false);
        assert!(!open.contains_key(SECURITY));
        assert!(!open["802-11-wireless"].contains_key("hidden"));
        let sae = connection_settings(b"lab", Key::Sae, true);
        assert_eq!(
            sae[SECURITY].get("key-mgmt").map(|v| string(v)).as_deref(),
            Some("sae")
        );
        assert_eq!(
            sae["802-11-wireless"].get("ssid").map(|v| bytes(v)),
            Some(b"lab".to_vec())
        );
        assert_eq!(
            sae["802-11-wireless"]
                .get("hidden")
                .and_then(|v| boolean(v)),
            Some(true)
        );
        let mixed = connection_settings(b"lab", Key::Mixed, false);
        assert_eq!(
            mixed[SECURITY]
                .get("key-mgmt")
                .map(|v| string(v))
                .as_deref(),
            Some("wpa-psk")
        );
        let wep = connection_settings(b"old", Key::Wep, false);
        assert_eq!(
            wep[SECURITY].get("wep-key-type").and_then(|v| int(v)),
            Some(2)
        );
        let raw = connection_settings(&[0xff, 0x00, 0x41], Key::Open, false);
        assert_eq!(
            raw["802-11-wireless"].get("ssid").map(|v| bytes(v)),
            Some(vec![0xff, 0x00, 0x41])
        );
    }

    fn props(entries: Vec<(&str, OwnedVariant)>) -> Props {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn object(
        path: &str,
        interfaces: Vec<(&str, Props)>,
    ) -> (
        OwnedObjectPath,
        HashMap<zbus::names::OwnedInterfaceName, Props>,
    ) {
        (
            OwnedObjectPath::try_from(path).expect("path"),
            interfaces
                .into_iter()
                .map(|(name, props)| {
                    (
                        zbus::names::OwnedInterfaceName::try_from(name).expect("name"),
                        props,
                    )
                })
                .collect(),
        )
    }

    fn path_value(path: &str) -> OwnedVariant {
        owned(Variant::from(
            zbus::ObjectPath::try_from(path).expect("path"),
        ))
        .expect("path")
    }

    #[test]
    fn device_choice_prefers_the_active_wifi_adapter() {
        let wifi = |active: &str, name: &str| {
            vec![
                (
                    DEVICE,
                    props(vec![
                        ("DeviceType", OwnedVariant::from(2u32)),
                        ("ActiveConnection", path_value(active)),
                        ("Interface", text(name)),
                    ]),
                ),
                (WIRELESS, props(vec![])),
            ]
        };
        let mut objects: Managed = HashMap::from([
            object(
                "/org/freedesktop/NetworkManager/Devices/1",
                vec![(
                    DEVICE,
                    props(vec![("DeviceType", OwnedVariant::from(1u32))]),
                )],
            ),
            object(
                "/org/freedesktop/NetworkManager/Devices/10",
                wifi("/", "wlan1"),
            ),
            object(
                "/org/freedesktop/NetworkManager/Devices/9",
                wifi("/", "wlan0"),
            ),
        ]);
        let (path, _, wireless) = pick_device(&objects).expect("device");
        assert_eq!(path, "/org/freedesktop/NetworkManager/Devices/9");
        assert!(wireless.is_some());
        objects.extend([object(
            "/org/freedesktop/NetworkManager/Devices/12",
            wifi(
                "/org/freedesktop/NetworkManager/ActiveConnection/1",
                "wlan2",
            ),
        )]);
        let (path, _, _) = pick_device(&objects).expect("device");
        assert_eq!(path, "/org/freedesktop/NetworkManager/Devices/12");
        assert!(pick_device(&HashMap::new()).is_none());
    }

    #[test]
    fn ip4_config_reads_the_first_address() {
        let entry = HashMap::from([
            ("address".to_string(), text("10.77.0.23")),
            ("prefix".to_string(), OwnedVariant::from(24u32)),
        ]);
        let config = props(vec![
            (
                "AddressData",
                owned(Variant::from(vec![entry])).expect("address data"),
            ),
            ("Gateway", text("10.77.0.1")),
        ]);
        assert_eq!(
            ip4(&config),
            ("10.77.0.23/24".to_string(), "10.77.0.1".to_string())
        );
        assert_eq!(ip4(&Props::new()), (String::new(), String::new()));
    }

    #[test]
    fn access_point_properties_parse() {
        let point = AccessPoint::from_props(
            "/ap/1",
            &props(vec![
                ("Ssid", owned(Variant::from(b"lab".to_vec())).expect("ssid")),
                ("Strength", OwnedVariant::from(250u8)),
                ("Flags", OwnedVariant::from(1u32)),
                ("WpaFlags", OwnedVariant::from(0u32)),
                ("RsnFlags", OwnedVariant::from(0x488u32)),
                ("Frequency", OwnedVariant::from(5180u32)),
            ]),
        );
        assert_eq!(point.ssid, b"lab");
        assert_eq!(point.strength, 100);
        assert_eq!(point.key, Key::Sae);
        assert_eq!(point.frequency, 5180);
        assert_eq!(
            AccessPoint::from_props("/ap/2", &Props::new()),
            AccessPoint {
                path: "/ap/2".into(),
                ..AccessPoint::default()
            }
        );
        let list = owned(Variant::from(vec![
            zbus::ObjectPath::try_from("/a/1").expect("path"),
            zbus::ObjectPath::try_from("/a/2").expect("path"),
        ]))
        .expect("paths");
        assert_eq!(paths(&list), vec!["/a/1", "/a/2"]);
        assert!(paths(&text("x")).is_empty());
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    fn key() -> impl Strategy<Value = Key> {
        prop::sample::select(vec![
            Key::Open,
            Key::Owe,
            Key::Wep,
            Key::Psk,
            Key::Sae,
            Key::Mixed,
            Key::Eap,
        ])
    }

    proptest! {
        #[test]
        fn flags_give_a_known_security(flags in any::<u32>(), wpa in any::<u32>(), rsn in any::<u32>()) {
            let key = Key::from_flags(flags, wpa, rsn);
            prop_assert!(["open", "wep", "psk", "8021x"].contains(&key.security()));
            let back = Key::from_key_mgmt(key.key_mgmt().unwrap_or(""));
            prop_assert_eq!(back.security(), key.security());
        }

        #[test]
        fn networks_are_unique_and_complete(
            points in prop::collection::vec(("[a-c]{0,2}", 0u8..=100, key()), 0..12),
            saved in prop::collection::vec(("[a-d]{0,2}", any::<bool>()), 0..6),
            active in proptest::option::of("[a-c]{1,2}"),
            connected in any::<bool>(),
        ) {
            let points: Vec<AccessPoint> = points
                .into_iter()
                .enumerate()
                .map(|(index, (ssid, strength, key))| AccessPoint {
                    path: format!("/ap/{index}"),
                    ssid: ssid.into_bytes(),
                    strength,
                    key,
                    frequency: 2412,
                })
                .collect();
            let saved: Vec<Saved> = saved
                .into_iter()
                .enumerate()
                .map(|(index, (ssid, autoconnect))| Saved {
                    path: format!("/conn/{index}"),
                    name: ssid.clone(),
                    ssid: ssid.into_bytes(),
                    autoconnect,
                    hidden: false,
                    security: "psk",
                })
                .collect();
            let networks = build_networks(&points, &saved, active.as_ref().map(|a| a.as_bytes()), connected);
            let mut visible: Vec<&str> = networks.iter().filter(|n| n.available).map(|n| n.name.as_str()).collect();
            let count = visible.len();
            visible.sort();
            visible.dedup();
            prop_assert_eq!(visible.len(), count);
            prop_assert!(networks.iter().all(|n| !n.name.is_empty() || !n.available));
            prop_assert!(networks.iter().filter(|n| n.connected).count() <= 1);
            prop_assert!(networks.iter().all(|n| !n.connected || connected));
            for entry in &saved {
                prop_assert_eq!(networks.iter().filter(|n| n.known == entry.path).count(), 1);
            }
            for point in points.iter().filter(|point| !point.ssid.is_empty()) {
                let best = points.iter().filter(|other| other.ssid == point.ssid).map(|other| other.strength).max();
                let shown = networks.iter().find(|n| n.available && n.name.as_bytes() == point.ssid.as_slice());
                prop_assert_eq!(shown.map(|n| n.strength as u8), best);
            }
            let first_offline = networks.iter().position(|n| !n.available).unwrap_or(networks.len());
            prop_assert!(networks[first_offline..].iter().all(|n| !n.available));
        }

        #[test]
        fn hidden_and_new_profiles_keep_the_ssid(ssid in prop::collection::vec(any::<u8>(), 1..33), key in key(), hidden in any::<bool>()) {
            let settings = connection_settings(&ssid, key, hidden);
            prop_assert_eq!(settings["802-11-wireless"].get("ssid").map(|v| bytes(v)), Some(ssid));
            prop_assert_eq!(settings.contains_key(SECURITY), key != Key::Open);
        }

        #[test]
        fn failure_messages_are_never_empty(reasons in prop::collection::vec(0u32..70, 0..6), active in 0u32..16) {
            match failure(&reasons, active) {
                Outcome::Failed(message) => prop_assert!(!message.is_empty() && !message.contains(':')),
                Outcome::Cancelled | Outcome::Activated => {}
            }
        }
    }
}
