use std::collections::HashMap;

use zbus::{OwnedObjectPath, OwnedVariant, Variant, interface};

use super::{
    Event, Handle, Props, WifiDetails, WifiNetwork, WifiState, airplane_mode, boolean, error_text,
    int, oneshot, prop, string,
};

pub const NM: &str = "org.freedesktop.NetworkManager";
pub const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const DEVICE_TYPE_WIFI: u32 = 2;
const ALLOW_INTERACTION: u32 = 0x1;

pub type Settings = HashMap<String, HashMap<String, OwnedVariant>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessPoint {
    pub path: String,
    pub ssid: Vec<u8>,
    pub strength: u8,
    pub security: &'static str,
    pub frequency: u32,
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

pub fn ssid_text(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid).into_owned()
}

pub fn security(flags: u32, wpa: u32, rsn: u32) -> &'static str {
    let keys = wpa | rsn;
    if keys & 0x200 != 0 {
        "8021x"
    } else if keys & (0x100 | 0x400) != 0 {
        "psk"
    } else if flags & 0x1 != 0 {
        "wep"
    } else {
        "open"
    }
}

pub fn device_state(state: u32) -> &'static str {
    match state {
        100 => "connected",
        40..=90 => "connecting",
        110 | 120 => "disconnecting",
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
                security: point.security.to_string(),
                connected: connected && active == Some(point.ssid.as_slice()),
                saved: known.is_some(),
                known: known.map(|entry| entry.path.clone()).unwrap_or_default(),
                autoconnect: known.is_none_or(|entry| entry.autoconnect),
                hidden: false,
            }
        })
        .collect();
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.strength.total_cmp(&a.strength))
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
    let key_mgmt = setting(settings, "802-11-wireless-security", "key-mgmt")
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
        security: match key_mgmt.as_str() {
            "" => "open",
            "wpa-eap" | "ieee8021x" => "8021x",
            "none" => "wep",
            _ => "psk",
        },
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

pub fn psk_reply(password: &str) -> Settings {
    let mut security = HashMap::new();
    security.insert(
        "psk".to_string(),
        OwnedVariant::from(zbus::Str::from(password.to_string())),
    );
    HashMap::from([("802-11-wireless-security".to_string(), security)])
}

fn owned(value: Variant<'_>) -> Option<OwnedVariant> {
    OwnedVariant::try_from(value).ok()
}

fn connection_settings(ssid: &str, security: &str, hidden: bool) -> Settings {
    let mut wireless = HashMap::new();
    if let Some(value) = owned(Variant::from(ssid.as_bytes().to_vec())) {
        wireless.insert("ssid".to_string(), value);
    }
    if hidden {
        wireless.insert("hidden".to_string(), OwnedVariant::from(true));
    }
    let mut settings = HashMap::from([("802-11-wireless".to_string(), wireless)]);
    if security == "psk" {
        let mut group = HashMap::new();
        group.insert(
            "key-mgmt".to_string(),
            OwnedVariant::from(zbus::Str::from("wpa-psk")),
        );
        settings.insert("802-11-wireless-security".to_string(), group);
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
}

pub struct Agent {
    pub handle: Handle,
}

#[interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        connection: Settings,
        _connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
    ) -> Result<Settings, AgentError> {
        if setting_name != "802-11-wireless-security" || flags & ALLOW_INTERACTION == 0 {
            return Err(AgentError::NoSecrets("No secrets".into()));
        }
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.wifi_pending = Some(sender);
        }
        (self.handle.callback)(Event::WifiPrompt(prompt_name(&connection)));
        match receiver.await {
            Some(password) => Ok(psk_reply(&password)),
            None => Err(AgentError::UserCanceled("Cancelled".into())),
        }
    }

    async fn cancel_get_secrets(&self, _connection_path: OwnedObjectPath, _setting_name: String) {
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.wifi_pending = None;
        }
        (self.handle.callback)(Event::WifiPrompt(String::new()));
    }

    async fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}

    async fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
}

impl Handle {
    async fn nm_props(&self, path: &str, iface: &str) -> zbus::Result<Props> {
        let reply = self
            .call(
                NM,
                path,
                "org.freedesktop.DBus.Properties",
                "GetAll",
                &(iface,),
            )
            .await?;
        reply.body().deserialize::<Props>()
    }

    async fn nm_paths(
        &self,
        path: &str,
        iface: &str,
        method: &str,
    ) -> zbus::Result<Vec<OwnedObjectPath>> {
        let reply = self.call(NM, path, iface, method, &()).await?;
        reply.body().deserialize::<Vec<OwnedObjectPath>>()
    }

    pub(super) async fn nm_device(&self) -> Result<(String, Props), String> {
        let devices = self
            .nm_paths(ROOT, NM, "GetDevices")
            .await
            .map_err(|err| error_text(&err))?;
        for device in devices {
            if let Ok(props) = self.nm_props(device.as_str(), DEVICE).await
                && prop(&props, "DeviceType").and_then(int) == Some(DEVICE_TYPE_WIFI as i64)
            {
                return Ok((device.to_string(), props));
            }
        }
        Err("No Wi-Fi adapter found".into())
    }

    async fn nm_access_point(&self, path: &str) -> Option<AccessPoint> {
        let props = self.nm_props(path, ACCESS_POINT).await.ok()?;
        let number = |name: &str| prop(&props, name).and_then(int).unwrap_or(0) as u32;
        Some(AccessPoint {
            path: path.to_string(),
            ssid: prop(&props, "Ssid").map(bytes).unwrap_or_default(),
            strength: number("Strength").min(100) as u8,
            security: security(number("Flags"), number("WpaFlags"), number("RsnFlags")),
            frequency: number("Frequency"),
        })
    }

    async fn nm_saved(&self) -> Vec<Saved> {
        let Ok(paths) = self
            .nm_paths(
                SETTINGS,
                "org.freedesktop.NetworkManager.Settings",
                "ListConnections",
            )
            .await
        else {
            return Vec::new();
        };
        let mut saved = Vec::new();
        for path in paths {
            if let Ok(reply) = self
                .call(NM, path.as_str(), CONNECTION, "GetSettings", &())
                .await
                && let Ok(settings) = reply.body().deserialize::<Settings>()
                && let Some(entry) = saved_from(path.as_str(), &settings)
            {
                saved.push(entry);
            }
        }
        saved
    }

    pub(super) async fn nm_read_wifi(&self) -> Result<WifiState, String> {
        let root = self
            .nm_props(ROOT, NM)
            .await
            .map_err(|err| error_text(&err))?;
        let (path, device) = self.nm_device().await?;
        let powered = prop(&root, "WirelessEnabled")
            .and_then(boolean)
            .unwrap_or(false)
            && prop(&root, "WirelessHardwareEnabled")
                .and_then(boolean)
                .unwrap_or(true);
        let state = device_state(prop(&device, "State").and_then(int).unwrap_or(0) as u32);
        let connected = powered && state == "connected";
        let wireless = self.nm_props(&path, WIRELESS).await.unwrap_or_default();
        let active_path = prop(&wireless, "ActiveAccessPoint")
            .map(string)
            .unwrap_or_default();
        let mut points = Vec::new();
        if powered {
            for point in self
                .nm_paths(&path, WIRELESS, "GetAllAccessPoints")
                .await
                .unwrap_or_default()
            {
                if let Some(point) = self.nm_access_point(point.as_str()).await {
                    points.push(point);
                }
            }
        }
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
        let (busy, message) = self
            .inner
            .lock()
            .map(|inner| (inner.busy, inner.message.clone()))
            .unwrap_or_default();
        Ok(WifiState {
            powered,
            state: if powered { state } else { "off" }.into(),
            scanning: false,
            networks,
            busy,
            message,
            device: prop(&device, "Interface").map(string).unwrap_or_default(),
            connected,
            network,
            airplane: airplane_mode(),
            backend: "networkmanager".into(),
        })
    }

    pub(super) async fn nm_diagnostics(&self, details: &mut WifiDetails) {
        let Ok((path, _)) = self.nm_device().await else {
            return;
        };
        let Ok(wireless) = self.nm_props(&path, WIRELESS).await else {
            return;
        };
        let active = prop(&wireless, "ActiveAccessPoint")
            .map(string)
            .unwrap_or_default();
        if let Some(point) = self.nm_access_point(&active).await {
            details.band = band(point.frequency).into();
            details.channel = channel(point.frequency);
            details.rssi = rssi(point.strength);
        }
        details.rx_rate = prop(&wireless, "Bitrate").and_then(int).unwrap_or(0) as f64 / 1000.0;
    }

    pub(super) async fn nm_power(&self, enabled: bool) -> zbus::Result<bool> {
        self.set_property(NM, ROOT, NM, "WirelessEnabled", Variant::Bool(enabled))
            .await
            .map(|_| false)
    }

    pub(super) async fn nm_scan(&self, device: &str) -> zbus::Result<bool> {
        let options: HashMap<String, OwnedVariant> = HashMap::new();
        self.call(NM, device, WIRELESS, "RequestScan", &(options,))
            .await
            .map(|_| true)
    }

    pub(super) async fn nm_connect(&self, device: &str, point: &str) -> zbus::Result<bool> {
        let saved = self.nm_saved().await;
        let target = self.nm_access_point(point).await;
        let device_path = zbus::ObjectPath::try_from(device)?;
        let point_path = zbus::ObjectPath::try_from(point)?;
        if let Some(entry) = target
            .as_ref()
            .and_then(|target| saved.iter().find(|entry| entry.ssid == target.ssid))
        {
            let connection = zbus::ObjectPath::try_from(entry.path.as_str())?;
            self.call(
                NM,
                ROOT,
                NM,
                "ActivateConnection",
                &(connection, device_path, point_path),
            )
            .await?;
            return Ok(true);
        }
        let (ssid, kind) = target
            .map(|target| (ssid_text(&target.ssid), target.security))
            .unwrap_or_default();
        if kind == "8021x" {
            return Err(zbus::Error::Failure(
                "Enterprise networks need a saved connection".into(),
            ));
        }
        let settings = connection_settings(&ssid, kind, false);
        self.call(
            NM,
            ROOT,
            NM,
            "AddAndActivateConnection",
            &(settings, device_path, point_path),
        )
        .await
        .map(|_| true)
    }

    pub(super) async fn nm_connect_hidden(&self, device: &str, name: &str) -> zbus::Result<bool> {
        let settings = connection_settings(name, "psk", true);
        let device_path = zbus::ObjectPath::try_from(device)?;
        let none = zbus::ObjectPath::try_from("/")?;
        self.call(
            NM,
            ROOT,
            NM,
            "AddAndActivateConnection",
            &(settings, device_path, none),
        )
        .await
        .map(|_| true)
    }

    pub(super) async fn nm_disconnect(&self, device: &str) -> zbus::Result<bool> {
        self.call(NM, device, DEVICE, "Disconnect", &())
            .await
            .map(|_| true)
    }

    pub(super) async fn nm_forget(&self, connection: &str) -> zbus::Result<bool> {
        self.call(NM, connection, CONNECTION, "Delete", &())
            .await
            .map(|_| false)
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
        let secured = settings.contains_key("802-11-wireless-security");
        if secured {
            let reply = self
                .call(
                    NM,
                    connection,
                    CONNECTION,
                    "GetSecrets",
                    &("802-11-wireless-security",),
                )
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
        self.call(NM, connection, CONNECTION, "Update", &(settings,))
            .await
            .map(|_| false)
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

    fn point(path: &str, ssid: &str, strength: u8, security: &'static str) -> AccessPoint {
        AccessPoint {
            path: path.into(),
            ssid: ssid.as_bytes().to_vec(),
            strength,
            security,
            frequency: 2437,
        }
    }

    #[test]
    fn security_flags() {
        assert_eq!(security(0, 0, 0), "open");
        assert_eq!(security(1, 0, 0), "wep");
        assert_eq!(security(1, 0, 0x188), "psk");
        assert_eq!(security(1, 0, 0x400), "psk");
        assert_eq!(security(1, 0x200, 0), "8021x");
    }

    #[test]
    fn states_and_radio() {
        assert_eq!(device_state(100), "connected");
        assert_eq!(device_state(70), "connecting");
        assert_eq!(device_state(30), "disconnected");
        assert_eq!(device_state(20), "off");
        assert_eq!(channel(2437), 6);
        assert_eq!(channel(2484), 14);
        assert_eq!(channel(5180), 36);
        assert_eq!(channel(5975), 5);
        assert_eq!(band(5180), "5 GHz");
        assert_eq!(band(6000), "6 GHz");
        assert_eq!(rssi(100), -50);
        assert_eq!(rssi(50), -75);
    }

    #[test]
    fn networks_merge_access_points_and_saved_connections() {
        let points = vec![
            point("/ap/1", "home", 40, "psk"),
            point("/ap/2", "home", 80, "psk"),
            point("/ap/3", "cafe", 90, "open"),
            point("/ap/4", "", 99, "open"),
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
        assert_eq!(names, vec!["home", "cafe", "office"]);
        assert_eq!(networks[0].path, "/ap/2");
        assert!(networks[0].connected && networks[0].saved && !networks[0].autoconnect);
        assert_eq!(networks[0].known, "/conn/1");
        assert!(!networks[1].saved && networks[1].autoconnect);
        assert!(!networks[2].available && networks[2].hidden);
        assert_eq!(networks[2].security, "8021x");
        let offline = build_networks(&points, &saved, Some(b"home"), false);
        assert!(offline.iter().all(|n| !n.connected));
    }

    fn text(value: &str) -> OwnedVariant {
        OwnedVariant::from(zbus::Str::from(value.to_string()))
    }

    #[test]
    fn saved_connection_properties_parse() {
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
        settings.insert(
            "802-11-wireless-security".into(),
            HashMap::from([("key-mgmt".to_string(), text("sae"))]),
        );
        let saved = saved_from("/conn/7", &settings).expect("wifi connection");
        assert_eq!(saved.ssid, b"home");
        assert_eq!(saved.name, "Home 5G");
        assert!(!saved.autoconnect);
        assert_eq!(saved.security, "psk");
        assert_eq!(prompt_name(&settings), "home");
        let mut wired = settings.clone();
        wired
            .get_mut("connection")
            .expect("group")
            .insert("type".into(), text("802-3-ethernet"));
        assert_eq!(saved_from("/conn/8", &wired), None);
    }

    #[test]
    fn secrets_and_new_connections() {
        let reply = psk_reply("hunter22");
        let psk = reply
            .get("802-11-wireless-security")
            .and_then(|group| group.get("psk"))
            .map(|value| string(value));
        assert_eq!(psk.as_deref(), Some("hunter22"));
        let open = connection_settings("cafe", "open", false);
        assert!(!open.contains_key("802-11-wireless-security"));
        let hidden = connection_settings("lab", "psk", true);
        assert_eq!(
            hidden["802-11-wireless"].get("ssid").map(|v| bytes(v)),
            Some(b"lab".to_vec())
        );
        assert_eq!(
            hidden["802-11-wireless"]
                .get("hidden")
                .and_then(|v| boolean(v)),
            Some(true)
        );
    }
}
