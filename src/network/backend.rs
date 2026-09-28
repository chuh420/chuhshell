use glib::variant::{FromVariant, ObjectPath, ToVariant};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const NM: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const PROFILE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
type Properties = HashMap<String, glib::Variant>;
type Settings = HashMap<String, Properties>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    Open,
    Personal,
    Sae,
    Enhanced,
    Enterprise,
    Legacy,
}

impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open network",
            Self::Personal => "WPA / WPA2 Personal",
            Self::Sae => "WPA3 Personal",
            Self::Enhanced => "Enhanced Open",
            Self::Enterprise => "Enterprise",
            Self::Legacy => "WEP / legacy security",
        }
    }

    pub fn password(self) -> bool {
        matches!(self, Self::Personal | Self::Sae)
    }

    pub fn supported(self) -> bool {
        !matches!(self, Self::Enterprise | Self::Legacy)
    }

    fn compatible(self, other: Self) -> bool {
        self == other || (self.password() && other.password())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub ssid: Vec<u8>,
    pub name: String,
    pub access_point: String,
    pub profile: Option<String>,
    pub security: Security,
    pub strength: Option<u8>,
    pub frequency: u32,
    pub active: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub enabled: bool,
    pub hardware_enabled: bool,
    pub adapters: Vec<Adapter>,
    pub device: Option<Adapter>,
    pub networks: Vec<Network>,
    pub address: String,
}

pub enum Action {
    Radio(bool),
    Scan(String),
    Connect {
        device: String,
        network: Network,
        password: String,
    },
    Disconnect(String),
    Forget(String),
}

struct Client {
    bus: gio::DBusConnection,
    deadline: Instant,
}

fn value<T: FromVariant + Default>(properties: &Properties, key: &str) -> T {
    properties
        .get(key)
        .and_then(T::from_variant)
        .unwrap_or_default()
}

fn path(properties: &Properties, key: &str) -> String {
    properties
        .get(key)
        .and_then(ObjectPath::from_variant)
        .map(|p| p.to_string())
        .unwrap_or_else(|| "/".into())
}

fn object(path: &str) -> Result<ObjectPath, String> {
    ObjectPath::try_from(path).map_err(|_| "Invalid network object".into())
}

impl Client {
    fn new(seconds: u64) -> Result<Self, String> {
        Ok(Self {
            bus: gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
                .map_err(|_| "Cannot reach the system bus".to_string())?,
            deadline: Instant::now() + Duration::from_secs(seconds),
        })
    }

    fn call(
        &self,
        path: &str,
        interface: &str,
        method: &str,
        args: Option<&glib::Variant>,
    ) -> Result<glib::Variant, String> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || crate::process::stopped() {
            return Err("Network operation timed out. Please try again.".into());
        }
        self.bus
            .call_sync(
                Some(NM),
                path,
                interface,
                method,
                args,
                None,
                gio::DBusCallFlags::NONE,
                remaining.as_millis().min(10_000) as i32,
                gio::Cancellable::NONE,
            )
            .map_err(|error| {
                let message = error.to_string();
                message
                    .strip_prefix("GDBus.Error:")
                    .unwrap_or(&message)
                    .to_string()
            })
    }

    fn properties(&self, path: &str, interface: &str) -> Result<Properties, String> {
        self.call(
            path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(interface,).to_variant()),
        )?
        .child_value(0)
        .get()
        .ok_or_else(|| "Invalid network response".into())
    }

    fn settings(&self, path: &str) -> Result<Settings, String> {
        self.call(path, PROFILE, "GetSettings", None)?
            .child_value(0)
            .get()
            .ok_or_else(|| "Cannot read saved network".into())
    }

    fn paths(&self, path: &str, interface: &str, method: &str) -> Result<Vec<ObjectPath>, String> {
        self.call(path, interface, method, None)?
            .child_value(0)
            .get()
            .ok_or_else(|| "Cannot read network list".into())
    }
}

fn ap_security(properties: &Properties) -> Security {
    let flags = value::<u32>(properties, "WpaFlags") | value::<u32>(properties, "RsnFlags");
    if flags & 0x200 != 0 || flags & 0x2000 != 0 {
        Security::Enterprise
    } else if flags & 0x100 != 0 {
        Security::Personal
    } else if flags & 0x400 != 0 {
        Security::Sae
    } else if flags & 0x1800 != 0 {
        Security::Enhanced
    } else if value::<u32>(properties, "Flags") & 1 != 0 {
        Security::Legacy
    } else {
        Security::Open
    }
}

fn profile_security(settings: &Settings) -> Security {
    match settings
        .get("802-11-wireless-security")
        .map(|p| value::<String>(p, "key-mgmt"))
        .as_deref()
    {
        None => Security::Open,
        Some("wpa-psk") => Security::Personal,
        Some("sae") => Security::Sae,
        Some("owe") => Security::Enhanced,
        Some("wpa-eap" | "wpa-eap-suite-b-192" | "ieee8021x") => Security::Enterprise,
        _ => Security::Legacy,
    }
}

fn profile_matches(settings: &Settings, ssid: &[u8], security: Security, interface: &str) -> bool {
    let Some(wifi) = settings.get("802-11-wireless") else {
        return false;
    };
    let assigned = settings
        .get("connection")
        .map(|p| value::<String>(p, "interface-name"))
        .unwrap_or_default();
    value::<Vec<u8>>(wifi, "ssid") == ssid
        && value::<String>(wifi, "mode") != "ap"
        && security.compatible(profile_security(settings))
        && (assigned.is_empty() || assigned == interface)
}

pub fn snapshot(preferred: Option<&str>) -> Result<Snapshot, String> {
    snapshot_with(&Client::new(12)?, preferred)
}

fn snapshot_with(client: &Client, preferred: Option<&str>) -> Result<Snapshot, String> {
    let manager = client.properties(ROOT, NM)?;
    let mut adapters = Vec::new();
    let mut devices = HashMap::new();
    for device in client.paths(ROOT, NM, "GetDevices")? {
        let properties = client.properties(device.as_str(), DEVICE)?;
        if value::<u32>(&properties, "DeviceType") == 2 {
            adapters.push(Adapter {
                path: device.to_string(),
                name: value(&properties, "Interface"),
            });
            devices.insert(device.to_string(), properties);
        }
    }
    adapters.sort_by(|a, b| a.name.cmp(&b.name));
    let device = preferred
        .and_then(|name| adapters.iter().find(|a| a.name == name))
        .or_else(|| {
            adapters
                .iter()
                .find(|a| value::<u32>(&devices[&a.path], "State") == 100)
        })
        .or_else(|| adapters.first())
        .cloned();
    let mut result = Snapshot {
        enabled: value(&manager, "WirelessEnabled"),
        hardware_enabled: value(&manager, "WirelessHardwareEnabled"),
        adapters,
        device,
        networks: Vec::new(),
        address: String::new(),
    };
    let Some(device) = &result.device else {
        return Ok(result);
    };
    let properties = &devices[&device.path];
    let wifi = client.properties(&device.path, WIRELESS)?;
    let active_ap = path(&wifi, "ActiveAccessPoint");
    let active_connection = path(properties, "ActiveConnection");
    let active_profile = if active_connection != "/" {
        client
            .properties(&active_connection, ACTIVE)
            .map(|p| path(&p, "Connection"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let ip = path(properties, "Ip4Config");
    if ip != "/"
        && let Ok(properties) = client.properties(&ip, "org.freedesktop.NetworkManager.IP4Config")
    {
        let addresses: Vec<Properties> = value(&properties, "AddressData");
        result.address = addresses
            .iter()
            .map(|p| value::<String>(p, "address"))
            .collect::<Vec<_>>()
            .join(", ");
    }
    if result.enabled && result.hardware_enabled {
        for ap in client.paths(&device.path, WIRELESS, "GetAllAccessPoints")? {
            let Ok(properties) =
                client.properties(ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint")
            else {
                continue;
            };
            let ssid: Vec<u8> = value(&properties, "Ssid");
            if ssid.is_empty() {
                continue;
            }
            let network = Network {
                name: String::from_utf8_lossy(&ssid).into_owned(),
                ssid,
                access_point: ap.to_string(),
                profile: None,
                security: ap_security(&properties),
                strength: Some(value(&properties, "Strength")),
                frequency: value(&properties, "Frequency"),
                active: ap.as_str() == active_ap
                    && value::<u32>(&devices[&device.path], "State") == 100,
                hidden: false,
            };
            if let Some(existing) = result
                .networks
                .iter_mut()
                .find(|n| n.ssid == network.ssid && n.security == network.security)
            {
                if network.active || (!existing.active && network.strength > existing.strength) {
                    *existing = network;
                }
            } else {
                result.networks.push(network);
            }
        }
    }
    let mut profiles = client.paths(
        &format!("{ROOT}/Settings"),
        "org.freedesktop.NetworkManager.Settings",
        "ListConnections",
    )?;
    profiles.sort_by_key(|p| p.as_str() != active_profile);
    for profile in profiles {
        let Ok(settings) = client.settings(profile.as_str()) else {
            continue;
        };
        let Some(wifi) = settings.get("802-11-wireless") else {
            continue;
        };
        if value::<String>(wifi, "mode") == "ap" {
            continue;
        }
        if let Some(connection) = settings.get("connection") {
            let interface: String = value(connection, "interface-name");
            if !interface.is_empty() && interface != device.name {
                continue;
            }
        }
        let ssid: Vec<u8> = value(wifi, "ssid");
        if ssid.is_empty() {
            continue;
        }
        let security = profile_security(&settings);
        if let Some(network) = result
            .networks
            .iter_mut()
            .find(|n| n.ssid == ssid && n.security.compatible(security) && n.profile.is_none())
        {
            network.profile = Some(profile.to_string());
            network.hidden = value(wifi, "hidden");
        } else {
            result.networks.push(Network {
                name: String::from_utf8_lossy(&ssid).into_owned(),
                ssid,
                access_point: "/".into(),
                profile: Some(profile.to_string()),
                security,
                strength: None,
                frequency: 0,
                active: profile.as_str() == active_profile
                    && value::<u32>(&devices[&device.path], "State") == 100,
                hidden: value(wifi, "hidden"),
            });
        }
    }
    result.networks.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.strength.is_some().cmp(&a.strength.is_some()))
            .then(b.strength.cmp(&a.strength))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(result)
}

pub fn perform(action: Action) -> Result<String, String> {
    perform_with(&Client::new(55)?, action)
}

fn perform_with(client: &Client, action: Action) -> Result<String, String> {
    match action {
        Action::Radio(enabled) => {
            client.call(
                ROOT,
                "org.freedesktop.DBus.Properties",
                "Set",
                Some(&(NM, "WirelessEnabled", enabled.to_variant()).to_variant()),
            )?;
            Ok(if enabled {
                "Wi-Fi enabled"
            } else {
                "Wi-Fi disabled"
            }
            .into())
        }
        Action::Scan(device) => {
            client.call(
                &device,
                WIRELESS,
                "RequestScan",
                Some(&(Properties::new(),).to_variant()),
            )?;
            Ok("Scan requested".into())
        }
        Action::Disconnect(device) => {
            client.call(&device, DEVICE, "Disconnect", None)?;
            Ok("Disconnected".into())
        }
        Action::Forget(profile) => {
            client.call(&profile, PROFILE, "Delete", None)?;
            Ok("Saved network removed".into())
        }
        Action::Connect {
            device,
            mut network,
            password,
        } => {
            if network.ssid.is_empty() || network.ssid.len() > 32 {
                return Err("Network names must contain 1–32 bytes".into());
            }
            if network.security.password() && (network.profile.is_none() || !password.is_empty()) {
                let valid = if network.security == Security::Sae {
                    !password.is_empty()
                } else {
                    (8..=63).contains(&password.len())
                        || (password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()))
                };
                if !valid {
                    return Err("Enter a valid Wi-Fi password (WPA/WPA2: 8–63 characters or a 64-digit hexadecimal key).".into());
                }
            }
            if network.profile.is_none() {
                let interface: String = value(&client.properties(&device, DEVICE)?, "Interface");
                for profile in client.paths(
                    &format!("{ROOT}/Settings"),
                    "org.freedesktop.NetworkManager.Settings",
                    "ListConnections",
                )? {
                    let Ok(settings) = client.settings(profile.as_str()) else {
                        continue;
                    };
                    if profile_matches(&settings, &network.ssid, network.security, &interface) {
                        network.profile = Some(profile.to_string());
                        break;
                    }
                }
            }
            let device = object(&device)?;
            let ap = object(&network.access_point)?;
            let active = if let Some(profile) = &network.profile {
                if !password.is_empty() && network.security.password() {
                    let mut settings = client.settings(profile)?;
                    let security = settings
                        .entry("802-11-wireless-security".into())
                        .or_default();
                    security.insert("psk".into(), password.to_variant());
                    security.insert("psk-flags".into(), 0u32.to_variant());
                    client.call(profile, PROFILE, "Update", Some(&(settings,).to_variant()))?;
                }
                client
                    .call(
                        ROOT,
                        NM,
                        "ActivateConnection",
                        Some(&(object(profile)?, &device, &ap).to_variant()),
                    )?
                    .child_get::<ObjectPath>(0)
            } else {
                if !network.security.supported() {
                    return Err(
                        "This network needs an existing Enterprise or legacy connection profile."
                            .into(),
                    );
                }
                let mut settings = Settings::from([
                    (
                        "connection".into(),
                        Properties::from([
                            ("id".into(), network.name.to_variant()),
                            ("type".into(), "802-11-wireless".to_variant()),
                            ("autoconnect".into(), true.to_variant()),
                        ]),
                    ),
                    (
                        "802-11-wireless".into(),
                        Properties::from([
                            ("ssid".into(), network.ssid.to_variant()),
                            ("mode".into(), "infrastructure".to_variant()),
                            ("hidden".into(), network.hidden.to_variant()),
                        ]),
                    ),
                    (
                        "ipv4".into(),
                        Properties::from([("method".into(), "auto".to_variant())]),
                    ),
                    (
                        "ipv6".into(),
                        Properties::from([("method".into(), "auto".to_variant())]),
                    ),
                ]);
                if network.security != Security::Open {
                    let key = match network.security {
                        Security::Sae => "sae",
                        Security::Enhanced => "owe",
                        _ => "wpa-psk",
                    };
                    let mut security = Properties::from([("key-mgmt".into(), key.to_variant())]);
                    if network.security.password() {
                        security.insert("psk".into(), password.to_variant());
                    }
                    settings.insert("802-11-wireless-security".into(), security);
                }
                client
                    .call(
                        ROOT,
                        NM,
                        "AddAndActivateConnection",
                        Some(&(settings, &device, &ap).to_variant()),
                    )?
                    .child_get::<ObjectPath>(1)
            };
            drop(password);
            loop {
                if Instant::now() >= client.deadline || crate::process::stopped() {
                    if let Ok(cleanup) = Client::new(5) {
                        let _ = cleanup.call(
                            ROOT,
                            NM,
                            "DeactivateConnection",
                            Some(&(&active,).to_variant()),
                        );
                    }
                    return Err("Connection timed out. Check the password and try again.".into());
                }
                let properties = client.properties(active.as_str(), ACTIVE).map_err(|_| {
                    "Connection failed or timed out. Check the password and try again.".to_string()
                })?;
                match value::<u32>(&properties, "State") {
                    2 => return Ok(format!("Connected to {}", network.name)),
                    3 | 4 => {
                        return Err(
                            "Could not connect. Check the password and network availability."
                                .into(),
                        );
                    }
                    _ => {}
                }
                crate::process::pause(Duration::from_millis(400));
            }
        }
    }
}

#[cfg(test)]
pub fn regression_checks() {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(NM, 0u32).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    )
    .unwrap();
    let xml = format!(
        r#"<node><interface name="{NM}"><method name="GetDevices"><arg type="ao" direction="out"/></method><property name="WirelessEnabled" type="b" access="read"/><property name="WirelessHardwareEnabled" type="b" access="read"/></interface><interface name="{DEVICE}"><property name="DeviceType" type="u" access="read"/><property name="State" type="u" access="read"/><property name="Interface" type="s" access="read"/></interface><interface name="{WIRELESS}"><method name="GetAllAccessPoints"><arg type="ao" direction="out"/></method><method name="RequestScan"><arg type="a{{sv}}" direction="in"/></method></interface><interface name="org.freedesktop.NetworkManager.Settings"><method name="ListConnections"><arg type="ao" direction="out"/></method></interface></node>"#
    );
    let info = gio::DBusNodeInfo::for_xml(&xml).unwrap();
    let device = format!("{ROOT}/Devices/1");
    let other = format!("{ROOT}/Devices/2");
    let mut registrations = Vec::new();
    for (path, interface) in [
        (ROOT.to_string(), NM),
        (device.clone(), DEVICE),
        (device.clone(), WIRELESS),
        (other.clone(), DEVICE),
        (other.clone(), WIRELESS),
        (
            format!("{ROOT}/Settings"),
            "org.freedesktop.NetworkManager.Settings",
        ),
    ] {
        let devices = vec![object(&device).unwrap(), object(&other).unwrap()];
        registrations.push(
            bus.register_object(&path, &info.lookup_interface(interface).unwrap())
                .method_call(move |_, _, _, _, method, _, invocation| match method {
                    "GetDevices" => invocation.return_value(Some(&(devices.clone(),).to_variant())),
                    "RequestScan" => invocation.return_dbus_error(
                        "org.freedesktop.NetworkManager.Device.NotAllowed",
                        "Scanning denied by test service",
                    ),
                    _ => invocation.return_value(Some(&(Vec::<ObjectPath>::new(),).to_variant())),
                })
                .property(|_, _, path, _, property| match property {
                    "WirelessEnabled" | "WirelessHardwareEnabled" => true.to_variant(),
                    "DeviceType" => 2u32.to_variant(),
                    "State" => (if path.ends_with("/2") { 100u32 } else { 30u32 }).to_variant(),
                    "Interface" => (if path.ends_with("/2") {
                        "wlan1"
                    } else {
                        "wlan0"
                    })
                    .to_variant(),
                    _ => false.to_variant(),
                })
                .build()
                .unwrap(),
        );
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let client_bus = bus.clone();
    std::thread::spawn(move || {
        let client = Client {
            bus: client_bus,
            deadline: Instant::now() + Duration::from_secs(4),
        };
        let automatic = snapshot_with(&client, None).unwrap();
        assert_eq!(automatic.device.unwrap().name, "wlan1");
        let selected = snapshot_with(&client, Some("wlan0")).unwrap();
        assert_eq!(selected.device.unwrap().name, "wlan0");
        let error = perform_with(&client, Action::Scan(device)).unwrap_err();
        assert!(error.contains("Scanning denied"));
        let expired = Client {
            bus: client.bus,
            deadline: Instant::now(),
        };
        assert!(
            snapshot_with(&expired, None)
                .unwrap_err()
                .contains("timed out")
        );
        tx.send(()).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        match rx.try_recv() {
            Ok(()) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("Network regression worker failed")
            }
            Err(_) => {
                assert!(Instant::now() < deadline);
                crate::ui_tests::pump(5);
            }
        }
    }
    for registration in registrations {
        bus.unregister_object(registration).unwrap();
    }
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ReleaseName",
        Some(&(NM,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_selection_respects_ssid_security_interface_and_mode() {
        let mut settings = Settings::from([
            (
                "802-11-wireless".into(),
                Properties::from([("ssid".into(), vec![255u8, 0, 65].to_variant())]),
            ),
            (
                "802-11-wireless-security".into(),
                Properties::from([("key-mgmt".into(), "wpa-psk".to_variant())]),
            ),
            (
                "connection".into(),
                Properties::from([("interface-name".into(), "wlan0".to_variant())]),
            ),
        ]);
        assert!(profile_matches(
            &settings,
            &[255, 0, 65],
            Security::Personal,
            "wlan0"
        ));
        assert!(!profile_matches(
            &settings,
            &[255, 0, 65],
            Security::Personal,
            "wlan1"
        ));
        assert!(!profile_matches(
            &settings,
            &[255, 0, 65],
            Security::Open,
            "wlan0"
        ));
        assert!(!profile_matches(
            &settings,
            b"different",
            Security::Personal,
            "wlan0"
        ));
        settings
            .get_mut("802-11-wireless")
            .unwrap()
            .insert("mode".into(), "ap".to_variant());
        assert!(!profile_matches(
            &settings,
            &[255, 0, 65],
            Security::Personal,
            "wlan0"
        ));
    }

    #[test]
    fn security_flags_and_profiles_keep_enterprise_and_open_separate() {
        for (flags, expected) in [
            (0u32, Security::Open),
            (0x100, Security::Personal),
            (0x400, Security::Sae),
            (0x800, Security::Enhanced),
            (0x200, Security::Enterprise),
            (0x2100, Security::Enterprise),
        ] {
            assert_eq!(
                ap_security(&Properties::from([("RsnFlags".into(), flags.to_variant())])),
                expected
            );
        }
        assert_eq!(
            ap_security(&Properties::from([("Flags".into(), 1u32.to_variant())])),
            Security::Legacy
        );
        assert!(!Security::Open.compatible(Security::Personal));
        assert!(!Security::Enterprise.compatible(Security::Personal));
        for (key, expected) in [
            ("wpa-psk", Security::Personal),
            ("sae", Security::Sae),
            ("owe", Security::Enhanced),
            ("wpa-eap", Security::Enterprise),
            ("none", Security::Legacy),
        ] {
            let settings = Settings::from([(
                "802-11-wireless-security".into(),
                Properties::from([("key-mgmt".into(), key.to_variant())]),
            )]);
            assert_eq!(profile_security(&settings), expected);
        }
    }
}
