use std::{
    collections::HashMap,
    process::Command,
    sync::{Arc, Mutex},
    time::Instant,
};

use futures_util::StreamExt;
use zbus::{
    Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant, interface,
    message::Type,
};

mod nm;

use crate::oneshot;

const IWD: &str = "net.connman.iwd";
const BLUEZ: &str = "org.bluez";
const WIFI_AGENT: &str = "/quickshell/wifi_agent";
const BLUETOOTH_AGENT: &str = "/quickshell/bluetooth_agent";
const OWNER_CHANGED: &str = "org.freedesktop.DBus";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiNetwork {
    pub path: String,
    pub name: String,
    pub available: bool,
    pub strength: f64,
    pub security: String,
    pub connected: bool,
    pub saved: bool,
    pub known: String,
    pub autoconnect: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiState {
    pub powered: bool,
    pub state: String,
    pub scanning: bool,
    pub networks: Vec<WifiNetwork>,
    pub busy: bool,
    pub message: String,
    pub device: String,
    pub connected: bool,
    pub network: String,
    pub airplane: bool,
    pub backend: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiDetails {
    pub connected: bool,
    pub band: String,
    pub channel: i64,
    pub rssi: i64,
    pub rx_rate: f64,
    pub mode: String,
    pub ip: String,
    pub gateway: String,
    pub down: Option<f64>,
    pub up: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothDevice {
    pub path: String,
    pub name: String,
    pub address: String,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub pairing: bool,
    pub battery: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothState {
    pub available: bool,
    pub powered: bool,
    pub discovering: bool,
    pub name: String,
    pub devices: Vec<BluetoothDevice>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioProfile {
    pub index: i64,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceProfiles {
    pub id: i64,
    pub current: Option<i64>,
    pub profiles: Vec<AudioProfile>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothPrompt {
    pub name: String,
    pub mode: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub enum Event {
    Wifi(WifiState),
    Details(WifiDetails),
    Bluetooth(BluetoothState),
    Profiles(HashMap<String, DeviceProfiles>),
    BluetoothError(String),
    WifiPrompt(String),
    BluetoothPrompt(BluetoothPrompt),
    WifiChanged,
    BluetoothChanged,
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

enum Answer {
    Reject,
    Accept(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    None,
    Iwd,
    NetworkManager,
}

pub fn iwd_signal_matters(
    member: &str,
    interfaces: &[&str],
    changed: &[&str],
    panel: bool,
) -> bool {
    let scan_only = |name: &&str| {
        matches!(
            *name,
            "net.connman.iwd.Network" | "net.connman.iwd.BasicServiceSet"
        ) || name.starts_with("org.freedesktop.DBus.")
    };
    match member {
        "InterfacesAdded" | "InterfacesRemoved" => panel || !interfaces.iter().all(scan_only),
        "PropertiesChanged" => match interfaces.first().copied().unwrap_or_default() {
            "net.connman.iwd.Station" => panel || changed.iter().any(|key| *key != "Scanning"),
            "net.connman.iwd.Network" => {
                panel
                    || changed
                        .iter()
                        .any(|key| matches!(*key, "Connected" | "KnownNetwork"))
            }
            "net.connman.iwd.BasicServiceSet" => panel,
            _ => true,
        },
        _ => true,
    }
}

pub fn choose_backend(networkmanager: bool, iwd: bool) -> Backend {
    if networkmanager {
        Backend::NetworkManager
    } else if iwd {
        Backend::Iwd
    } else {
        Backend::None
    }
}

#[derive(Default)]
struct Inner {
    backend: Backend,
    nm_agent: bool,
    iwd_agent: bool,
    panel: bool,
    nm_scan: Option<(Instant, i64)>,
    nm_saved: HashMap<String, Option<nm::Saved>>,
    pending_path: String,
    busy: bool,
    message: String,
    last_wifi: Option<WifiState>,
    last_bluetooth: Option<BluetoothState>,
    last_profiles: Option<HashMap<String, DeviceProfiles>>,
    wifi_pending: Option<oneshot::Sender<String>>,
    bluetooth_pending: Option<(Option<oneshot::Sender<Answer>>, String)>,
    pairing: Vec<String>,
    counters: Option<(Instant, u64, u64)>,
    addresses: (String, String),
    addresses_checked: Option<Instant>,
}

#[derive(Clone)]
pub struct Handle {
    connection: Connection,
    inner: Arc<Mutex<Inner>>,
    callback: Callback,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "net.connman.iwd.Agent.Error")]
enum WifiAgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Canceled(String),
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum BluetoothAgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

fn string(value: &Variant<'_>) -> String {
    match value {
        Variant::Str(text) => text.as_str().to_string(),
        Variant::ObjectPath(path) => path.as_str().to_string(),
        Variant::Variant(inner) => string(inner),
        _ => String::new(),
    }
}

fn boolean(value: &Variant<'_>) -> Option<bool> {
    match value {
        Variant::Bool(value) => Some(*value),
        Variant::Variant(inner) => boolean(inner),
        _ => None,
    }
}

fn int(value: &Variant<'_>) -> Option<i64> {
    match value {
        Variant::U8(value) => Some(*value as i64),
        Variant::I16(value) => Some(*value as i64),
        Variant::U16(value) => Some(*value as i64),
        Variant::I32(value) => Some(*value as i64),
        Variant::U32(value) => Some(*value as i64),
        Variant::I64(value) => Some(*value),
        Variant::U64(value) => Some(*value as i64),
        Variant::Variant(inner) => int(inner),
        _ => None,
    }
}

type Props = HashMap<String, OwnedVariant>;
type Managed = HashMap<OwnedObjectPath, HashMap<zbus::names::OwnedInterfaceName, Props>>;

fn prop<'a>(props: &'a Props, name: &str) -> Option<&'a Variant<'static>> {
    props.get(name).map(|value| &**value)
}

fn text(props: &Props, name: &str) -> String {
    prop(props, name).map(string).unwrap_or_default()
}

fn interface<'a>(managed: &'a Managed, path: &str, name: &str) -> Option<&'a Props> {
    managed
        .iter()
        .find(|(object, _)| object.as_str() == path)
        .and_then(|(_, interfaces)| interfaces.iter().find(|(iface, _)| iface.as_str() == name))
        .map(|(_, props)| props)
}

fn airplane_mode() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/rfkill") else {
        return false;
    };
    let states: Vec<bool> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("rfkill"))
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("soft")).ok())
        .map(|value| value.trim() == "1")
        .collect();
    !states.is_empty() && states.iter().all(|state| *state)
}

fn error_text(error: &zbus::Error) -> String {
    match error {
        zbus::Error::Failure(message) => message.clone(),
        zbus::Error::MethodError(name, message, _) => match message {
            Some(message) => format!("{name}: {message}"),
            None => name.to_string(),
        },
        other => other.to_string(),
    }
}

fn wifi_failure(error: &str) -> String {
    if error.contains("Failed") {
        "Connection failed. Check the password and try again.".into()
    } else {
        error.rsplit(':').next().unwrap_or(error).trim().to_string()
    }
}

fn run_json(args: &[&str]) -> serde_json::Value {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice(&output.stdout).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn addresses(device: &str) -> (String, String) {
    let info = run_json(&["ip", "-j", "-4", "addr", "show", "dev", device]);
    let routes = run_json(&["ip", "-j", "route", "show", "default", "dev", device]);
    let ip = info
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| item["addr_info"].as_array().cloned().unwrap_or_default())
        .find_map(|entry| {
            let local = entry["local"].as_str()?;
            let prefix = entry["prefixlen"].as_u64()?;
            Some(format!("{local}/{prefix}"))
        })
        .unwrap_or_default();
    let gateway = routes
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|route| route["gateway"].as_str().map(str::to_string))
        .unwrap_or_default();
    (ip, gateway)
}

fn counters(device: &str) -> Option<(Instant, u64, u64)> {
    let base = format!("/sys/class/net/{device}/statistics/");
    let read = |name: &str| -> Option<u64> {
        std::fs::read_to_string(format!("{base}{name}"))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((Instant::now(), read("rx_bytes")?, read("tx_bytes")?))
}

struct WifiAgent {
    handle: Handle,
}

#[interface(name = "net.connman.iwd.Agent")]
impl WifiAgent {
    async fn request_passphrase(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        network: OwnedObjectPath,
    ) -> Result<String, WifiAgentError> {
        if !self.handle.sent_by(&header, IWD).await {
            return Err(WifiAgentError::Canceled("Not iwd".into()));
        }
        let name = self
            .handle
            .property(IWD, network.as_str(), "net.connman.iwd.Network", "Name")
            .await
            .map(|value| string(&value))
            .unwrap_or_default();
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.wifi_pending = Some(sender);
        }
        (self.handle.callback)(Event::WifiPrompt(name));
        match receiver.await {
            Some(password) => Ok(password),
            None => Err(WifiAgentError::Canceled("Cancelled".into())),
        }
    }

    async fn cancel(&self, #[zbus(header)] header: zbus::message::Header<'_>, _reason: &str) {
        if !self.handle.sent_by(&header, IWD).await {
            return;
        }
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.wifi_pending = None;
        }
        (self.handle.callback)(Event::WifiPrompt(String::new()));
    }

    async fn release(&self) {}
}

struct BluetoothAgent {
    handle: Handle,
}

impl BluetoothAgent {
    async fn ask(
        &self,
        header: &zbus::message::Header<'_>,
        mode: &str,
        device: &OwnedObjectPath,
        value: String,
        wait: bool,
    ) -> Result<String, BluetoothAgentError> {
        if !self.handle.sent_by(header, BLUEZ).await {
            return Err(BluetoothAgentError::Rejected("Not BlueZ".into()));
        }
        let name = self
            .handle
            .property(BLUEZ, device.as_str(), "org.bluez.Device1", "Alias")
            .await
            .map(|value| string(&value))
            .unwrap_or_default();
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.bluetooth_pending = Some((wait.then_some(sender), mode.to_string()));
        }
        (self.handle.callback)(Event::BluetoothPrompt(BluetoothPrompt {
            name,
            mode: mode.to_string(),
            value,
        }));
        if !wait {
            return Ok(String::new());
        }
        match receiver.await {
            Some(Answer::Accept(value)) => Ok(value),
            Some(Answer::Reject) => Err(BluetoothAgentError::Rejected("Rejected".into())),
            None => Err(BluetoothAgentError::Canceled("Cancelled".into())),
        }
    }
}

#[interface(name = "org.bluez.Agent1")]
impl BluetoothAgent {
    async fn request_confirmation(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), BluetoothAgentError> {
        self.ask(&header, "confirm", &device, format!("{passkey:06}"), true)
            .await
            .map(|_| ())
    }

    async fn request_pin_code(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
    ) -> Result<String, BluetoothAgentError> {
        self.ask(&header, "pin", &device, String::new(), true).await
    }

    async fn request_passkey(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
    ) -> Result<u32, BluetoothAgentError> {
        let value = self
            .ask(&header, "passkey", &device, String::new(), true)
            .await?;
        value
            .parse()
            .map_err(|_| BluetoothAgentError::Rejected("Invalid passkey".into()))
    }

    async fn request_authorization(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
    ) -> Result<(), BluetoothAgentError> {
        self.ask(&header, "authorize", &device, String::new(), true)
            .await
            .map(|_| ())
    }

    async fn authorize_service(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
        _uuid: &str,
    ) -> Result<(), BluetoothAgentError> {
        self.ask(&header, "authorize", &device, String::new(), true)
            .await
            .map(|_| ())
    }

    async fn display_pin_code(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
        pin: &str,
    ) {
        let _ = self
            .ask(&header, "display", &device, pin.to_string(), false)
            .await;
    }

    async fn display_passkey(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        device: OwnedObjectPath,
        passkey: u32,
        _entered: u16,
    ) {
        let _ = self
            .ask(&header, "display", &device, format!("{passkey:06}"), false)
            .await;
    }

    async fn cancel(&self, #[zbus(header)] header: zbus::message::Header<'_>) {
        if !self.handle.sent_by(&header, BLUEZ).await {
            return;
        }
        if let Ok(mut inner) = self.handle.inner.lock() {
            inner.bluetooth_pending = None;
        }
        (self.handle.callback)(Event::BluetoothPrompt(BluetoothPrompt::default()));
    }

    async fn release(&self, #[zbus(header)] header: zbus::message::Header<'_>) {
        self.cancel(header).await;
    }
}

impl Handle {
    async fn owner(&self, service: &str) -> Option<String> {
        self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            &(service,),
        )
        .await
        .ok()?
        .body()
        .deserialize::<String>()
        .ok()
    }

    async fn sent_by(&self, header: &zbus::message::Header<'_>, service: &str) -> bool {
        let Some(sender) = header.sender() else {
            return false;
        };
        self.owner(service)
            .await
            .is_some_and(|owner| owner == sender.as_str())
    }

    pub fn set_panel(&self, open: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.panel = open;
        }
    }

    pub fn unique_name(&self) -> Option<String> {
        self.connection.unique_name().map(|name| name.to_string())
    }

    fn emit(&self, event: Event) {
        (self.callback)(event);
    }

    async fn call<B>(
        &self,
        service: &str,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::Serialize + zbus::DynamicType,
    {
        self.connection
            .call_method(Some(service), path, Some(iface), method, body)
            .await
    }

    async fn property(
        &self,
        service: &str,
        path: &str,
        iface: &str,
        name: &str,
    ) -> zbus::Result<OwnedVariant> {
        let reply = self
            .call(
                service,
                path,
                "org.freedesktop.DBus.Properties",
                "Get",
                &(iface, name),
            )
            .await?;
        reply.body().deserialize::<OwnedVariant>()
    }

    async fn set_property(
        &self,
        service: &str,
        path: &str,
        iface: &str,
        name: &str,
        value: Variant<'_>,
    ) -> zbus::Result<()> {
        self.call(
            service,
            path,
            "org.freedesktop.DBus.Properties",
            "Set",
            &(iface, name, value),
        )
        .await
        .map(|_| ())
    }

    async fn managed(&self, service: &str) -> zbus::Result<Managed> {
        let reply = self
            .call(
                service,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "GetManagedObjects",
                &(),
            )
            .await?;
        reply.body().deserialize::<Managed>()
    }

    fn station(
        managed: &Managed,
    ) -> Option<(String, &HashMap<zbus::names::OwnedInterfaceName, Props>)> {
        let mut stations: Vec<(String, &HashMap<zbus::names::OwnedInterfaceName, Props>)> = managed
            .iter()
            .filter(|(_, interfaces)| {
                interfaces
                    .keys()
                    .any(|name| name.as_str() == "net.connman.iwd.Device")
            })
            .map(|(path, interfaces)| (path.to_string(), interfaces))
            .collect();
        let connected = |interfaces: &HashMap<zbus::names::OwnedInterfaceName, Props>| {
            interfaces
                .iter()
                .find(|(name, _)| name.as_str() == "net.connman.iwd.Station")
                .is_some_and(|(_, props)| text(props, "State") == "connected")
        };
        stations.sort_by(|a, b| {
            connected(b.1)
                .cmp(&connected(a.1))
                .then_with(|| (a.0.len(), &a.0).cmp(&(b.0.len(), &b.0)))
        });
        stations.into_iter().next()
    }

    async fn iwd_read_wifi(&self) -> Result<WifiState, String> {
        let managed = self.managed(IWD).await.map_err(|err| error_text(&err))?;
        let (path, interfaces) =
            Self::station(&managed).ok_or_else(|| "No Wi-Fi adapter found".to_string())?;
        let find = |name: &str| {
            interfaces
                .iter()
                .find(|(iface, _)| iface.as_str() == name)
                .map(|(_, props)| props)
        };
        let empty = Props::new();
        let device = find("net.connman.iwd.Device").unwrap_or(&empty);
        let station = find("net.connman.iwd.Station");
        let powered = prop(device, "Powered").and_then(boolean).unwrap_or(false);
        let mut networks = Vec::new();
        if powered && station.is_some() {
            let reply = self
                .call(
                    IWD,
                    &path,
                    "net.connman.iwd.Station",
                    "GetOrderedNetworks",
                    &(),
                )
                .await
                .map_err(|err| error_text(&err))?;
            let ordered: Vec<(OwnedObjectPath, i16)> =
                reply.body().deserialize().map_err(|err| err.to_string())?;
            for (network, signal) in ordered {
                let props = interface(&managed, network.as_str(), "net.connman.iwd.Network")
                    .unwrap_or(&empty);
                let known = text(props, "KnownNetwork");
                let autoconnect = interface(&managed, &known, "net.connman.iwd.KnownNetwork")
                    .and_then(|props| prop(props, "AutoConnect").and_then(boolean))
                    .unwrap_or(true);
                let name = props
                    .get("Name")
                    .map(|value| string(value))
                    .unwrap_or_else(|| "Network".into());
                networks.push(WifiNetwork {
                    path: network.to_string(),
                    name,
                    available: true,
                    strength: ((signal as f64 / 100.0 + 100.0) * 2.0).clamp(0.0, 100.0),
                    security: text(props, "Type"),
                    connected: prop(props, "Connected").and_then(boolean).unwrap_or(false),
                    saved: props.contains_key("KnownNetwork"),
                    known,
                    autoconnect,
                    hidden: false,
                });
            }
        }
        let seen: Vec<String> = networks
            .iter()
            .map(|network| network.known.clone())
            .collect();
        let mut known_entries: Vec<(&OwnedObjectPath, &Props)> = managed
            .iter()
            .filter_map(|(path, interfaces)| {
                interfaces
                    .iter()
                    .find(|(iface, _)| iface.as_str() == "net.connman.iwd.KnownNetwork")
                    .map(|(_, props)| (path, props))
            })
            .collect();
        known_entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (known, props) in known_entries {
            if seen.contains(&known.to_string()) {
                continue;
            }
            networks.push(WifiNetwork {
                path: String::new(),
                name: text(props, "Name"),
                available: false,
                strength: 0.0,
                security: text(props, "Type"),
                connected: false,
                saved: true,
                known: known.to_string(),
                autoconnect: prop(props, "AutoConnect").and_then(boolean).unwrap_or(true),
                hidden: prop(props, "Hidden").and_then(boolean).unwrap_or(false),
            });
        }
        let station_state = station
            .map(|props| text(props, "State"))
            .unwrap_or_else(|| "off".into());
        let connected = powered && station_state == "connected";
        let network = if connected {
            let path = station
                .map(|props| text(props, "ConnectedNetwork"))
                .unwrap_or_default();
            interface(&managed, &path, "net.connman.iwd.Network")
                .and_then(|props| props.get("Name").map(|value| string(value)))
                .or_else(|| {
                    networks
                        .iter()
                        .find(|network| network.connected)
                        .map(|network| network.name.clone())
                })
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
            state: station_state,
            scanning: station
                .and_then(|props| prop(props, "Scanning").and_then(boolean))
                .unwrap_or(false),
            networks,
            busy,
            message,
            device: text(device, "Name"),
            connected,
            network,
            airplane: airplane_mode(),
            backend: "iwd".into(),
        })
    }

    async fn has_owner(&self, name: &str) -> bool {
        match self
            .call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                &(name,),
            )
            .await
        {
            Ok(reply) => reply.body().deserialize::<bool>().unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn detect_backend(&self) -> Backend {
        let backend = choose_backend(self.has_owner(nm::NM).await, self.has_owner(IWD).await);
        let register = match self.inner.lock() {
            Ok(mut inner) => {
                if inner.backend != backend {
                    log::info!("wifi backend: {backend:?}");
                }
                inner.backend = backend;
                let flag = match backend {
                    Backend::NetworkManager => Some(&mut inner.nm_agent),
                    Backend::Iwd => Some(&mut inner.iwd_agent),
                    Backend::None => None,
                };
                flag.is_some_and(|registered| !std::mem::replace(registered, true))
            }
            Err(_) => false,
        };
        if register {
            let result = match backend {
                Backend::NetworkManager => self.nm_register_agent().await,
                Backend::Iwd => self.iwd_register_agent().await,
                Backend::None => Ok(()),
            };
            match result {
                Ok(()) => log::info!("wifi agent registered with {backend:?}"),
                Err(err) => {
                    log::warn!("wifi agent for {backend:?}: {}", error_text(&err));
                    if let Ok(mut inner) = self.inner.lock() {
                        inner.nm_agent &= backend != Backend::NetworkManager;
                        inner.iwd_agent &= backend != Backend::Iwd;
                        inner.message = error_text(&err);
                    }
                }
            }
        }
        backend
    }

    async fn iwd_register_agent(&self) -> zbus::Result<()> {
        let path = zbus::ObjectPath::try_from(WIFI_AGENT)?;
        match self
            .call(
                IWD,
                "/net/connman/iwd",
                "net.connman.iwd.AgentManager",
                "RegisterAgent",
                &(path,),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(zbus::Error::MethodError(name, _, _)) if name.ends_with(".AlreadyExists") => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn backend(&self) -> Backend {
        self.inner
            .lock()
            .map(|inner| inner.backend)
            .unwrap_or_default()
    }

    async fn read_wifi(&self) -> Result<WifiState, String> {
        match self.detect_backend().await {
            Backend::Iwd => self.iwd_read_wifi().await,
            Backend::NetworkManager => self.nm_read_wifi().await,
            Backend::None => Err(String::new()),
        }
    }

    pub async fn refresh_wifi(&self) {
        let state = match self.read_wifi().await {
            Ok(state) => state,
            Err(error) => {
                let airplane = airplane_mode();
                let (busy, _) = self
                    .inner
                    .lock()
                    .map(|inner| (inner.busy, inner.message.clone()))
                    .unwrap_or_default();
                WifiState {
                    powered: false,
                    state: if airplane { "airplane" } else { "unavailable" }.into(),
                    scanning: false,
                    networks: Vec::new(),
                    busy,
                    message: if airplane { String::new() } else { error },
                    device: String::new(),
                    connected: false,
                    network: "Offline".into(),
                    airplane,
                    backend: match self.backend() {
                        Backend::None => String::new(),
                        Backend::Iwd => "iwd".into(),
                        Backend::NetworkManager => "networkmanager".into(),
                    },
                }
            }
        };
        let changed = match self.inner.lock() {
            Ok(mut inner) => {
                let changed = inner.last_wifi.as_ref() != Some(&state);
                inner.last_wifi = Some(state.clone());
                changed
            }
            Err(_) => true,
        };
        if changed {
            self.emit(Event::Wifi(state));
        }
    }

    pub fn reset_details(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.counters = None;
            inner.addresses_checked = None;
        }
    }

    pub async fn details(&self) {
        let last = self
            .inner
            .lock()
            .ok()
            .and_then(|inner| inner.last_wifi.clone());
        let device = last
            .as_ref()
            .map(|state| state.device.clone())
            .unwrap_or_default();
        if !last.as_ref().is_some_and(|state| state.connected) || device.is_empty() {
            self.reset_details();
            self.emit(Event::Details(WifiDetails::default()));
            return;
        }
        let mut details = WifiDetails {
            connected: true,
            ..WifiDetails::default()
        };
        let mut addressed = false;
        if self.backend() == Backend::NetworkManager {
            addressed = self.nm_diagnostics(&mut details).await;
        } else if let Ok(managed) = self.managed(IWD).await
            && let Some((path, _)) = Self::station(&managed)
            && let Ok(reply) = self
                .call(
                    IWD,
                    &path,
                    "net.connman.iwd.StationDiagnostic",
                    "GetDiagnostics",
                    &(),
                )
                .await
            && let Ok(values) = reply.body().deserialize::<Props>()
        {
            let number = |name: &str| prop(&values, name).and_then(int).unwrap_or(0);
            let frequency = number("Frequency");
            details.band = if frequency >= 5925 {
                "6 GHz"
            } else if frequency >= 4900 {
                "5 GHz"
            } else {
                "2.4 GHz"
            }
            .into();
            details.channel = number("Channel");
            details.rssi = prop(&values, "AverageRSSI")
                .and_then(int)
                .unwrap_or_else(|| number("RSSI"));
            details.rx_rate = number("RxBitrate") as f64 / 10.0;
            details.mode = text(&values, "RxMode");
        }
        let now = Instant::now();
        let stale = self
            .inner
            .lock()
            .ok()
            .and_then(|inner| inner.addresses_checked)
            .is_none_or(|checked| now.duration_since(checked).as_secs_f64() > 10.0);
        if addressed {
            if let Ok(mut inner) = self.inner.lock() {
                inner.addresses = (details.ip.clone(), details.gateway.clone());
                inner.addresses_checked = Some(now);
            }
        } else if stale {
            let found = addresses(&device);
            if let Ok(mut inner) = self.inner.lock() {
                inner.addresses = found;
                inner.addresses_checked = Some(now);
            }
        }
        let current = counters(&device);
        if let Ok(mut inner) = self.inner.lock() {
            details.ip = inner.addresses.0.clone();
            details.gateway = inner.addresses.1.clone();
            if let (Some(current), Some(previous)) = (current, inner.counters)
                && current.0 > previous.0
            {
                let elapsed = current.0.duration_since(previous.0).as_secs_f64();
                details.down = Some((current.1.saturating_sub(previous.1)) as f64 / elapsed);
                details.up = Some((current.2.saturating_sub(previous.2)) as f64 / elapsed);
            }
            inner.counters = current;
        }
        self.emit(Event::Details(details));
    }

    fn set_busy(&self, busy: bool, message: Option<String>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.busy = busy;
            if let Some(message) = message {
                inner.message = message;
            }
        }
    }

    async fn fail(&self, error: &zbus::Error) {
        self.set_busy(false, Some(wifi_failure(&error_text(error))));
        self.emit(Event::WifiPrompt(String::new()));
        self.refresh_wifi().await;
    }

    async fn station_path(&self) -> Result<String, String> {
        match self.detect_backend().await {
            Backend::NetworkManager => return self.nm_device().await,
            Backend::None => return Err("No Wi-Fi service found".into()),
            Backend::Iwd => {}
        }
        let managed = self.managed(IWD).await.map_err(|err| error_text(&err))?;
        Self::station(&managed)
            .map(|(path, _)| path)
            .ok_or_else(|| "No Wi-Fi adapter found".to_string())
    }

    async fn wifi_command(&self, run: impl AsyncFnOnce(&Self, String) -> zbus::Result<bool>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.message.clear();
        }
        let path = match self.station_path().await {
            Ok(path) => path,
            Err(error) => {
                self.set_busy(false, Some(wifi_failure(&error)));
                self.emit(Event::WifiPrompt(String::new()));
                self.refresh_wifi().await;
                return;
            }
        };
        match run(self, path).await {
            Ok(finished) => {
                if finished {
                    self.set_busy(false, Some(String::new()));
                }
                self.refresh_wifi().await;
            }
            Err(error) => self.fail(&error).await,
        }
    }

    pub async fn wifi_power(&self, enabled: bool) {
        self.wifi_command(async |handle, path| {
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_power(enabled).await;
            }
            let managed = handle.managed(IWD).await?;
            let mut devices: Vec<String> = managed
                .iter()
                .filter(|(_, interfaces)| {
                    interfaces
                        .keys()
                        .any(|name| name.as_str() == "net.connman.iwd.Device")
                })
                .map(|(device, _)| device.to_string())
                .collect();
            if devices.is_empty() {
                devices.push(path);
            }
            for device in devices {
                handle
                    .set_property(
                        IWD,
                        &device,
                        "net.connman.iwd.Device",
                        "Powered",
                        Variant::Bool(enabled),
                    )
                    .await?;
            }
            Ok(false)
        })
        .await;
    }

    pub async fn scan(&self) {
        self.wifi_command(async |handle, path| {
            handle.refresh_wifi().await;
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_scan(&path).await;
            }
            handle
                .call(IWD, &path, "net.connman.iwd.Station", "Scan", &())
                .await
                .map(|_| true)
        })
        .await;
    }

    pub async fn connect(&self, network: String) {
        self.wifi_command(async |handle, path| {
            handle.set_busy(true, None);
            handle.refresh_wifi().await;
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_connect(&path, &network).await;
            }
            handle
                .call(IWD, &network, "net.connman.iwd.Network", "Connect", &())
                .await
                .map(|_| true)
        })
        .await;
    }

    pub async fn disconnect(&self) {
        self.wifi_command(async |handle, path| {
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_disconnect(&path).await;
            }
            handle
                .call(IWD, &path, "net.connman.iwd.Station", "Disconnect", &())
                .await
                .map(|_| true)
        })
        .await;
    }

    pub async fn forget(&self, known: String) {
        self.wifi_command(async |handle, _| {
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_forget(&known).await;
            }
            handle
                .call(IWD, &known, "net.connman.iwd.KnownNetwork", "Forget", &())
                .await
                .map(|_| false)
        })
        .await;
    }

    pub async fn autoconnect(&self, known: String, enabled: bool) {
        self.wifi_command(async |handle, _| {
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_autoconnect(&known, enabled).await;
            }
            handle
                .set_property(
                    IWD,
                    &known,
                    "net.connman.iwd.KnownNetwork",
                    "AutoConnect",
                    Variant::Bool(enabled),
                )
                .await
                .map(|_| false)
        })
        .await;
    }

    pub async fn connect_hidden(&self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.set_busy(false, Some("Enter the network name".into()));
            self.emit(Event::WifiPrompt(String::new()));
            self.refresh_wifi().await;
            return;
        }
        self.wifi_command(async |handle, path| {
            handle.set_busy(true, None);
            handle.refresh_wifi().await;
            if handle.backend() == Backend::NetworkManager {
                return handle.nm_connect_hidden(&path, &name).await;
            }
            handle
                .call(
                    IWD,
                    &path,
                    "net.connman.iwd.Station",
                    "ConnectHiddenNetwork",
                    &(name.as_str(),),
                )
                .await
                .map(|_| true)
        })
        .await;
    }

    pub fn password(&self, password: String) {
        let pending = self.inner.lock().ok().and_then(|mut inner| {
            inner.message.clear();
            inner.pending_path.clear();
            inner.wifi_pending.take()
        });
        if let Some(sender) = pending {
            sender.send(password);
            self.emit(Event::WifiPrompt(String::new()));
        }
    }

    pub fn cancel(&self) {
        let pending = self.inner.lock().ok().and_then(|mut inner| {
            inner.message.clear();
            inner.pending_path.clear();
            inner.wifi_pending.take()
        });
        if pending.is_some() {
            drop(pending);
            self.emit(Event::WifiPrompt(String::new()));
        }
    }

    pub async fn airplane(&self, enabled: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.message.clear();
        }
        let result = Command::new("rfkill")
            .args([if enabled { "block" } else { "unblock" }, "all"])
            .status();
        match result {
            Ok(status) if status.success() => {}
            Ok(status) => {
                self.set_busy(false, Some(format!("rfkill exited with {status}")));
            }
            Err(err) => self.set_busy(false, Some(err.to_string())),
        }
        self.emit(Event::WifiChanged);
        self.refresh_bluetooth().await;
    }

    pub async fn refresh_bluetooth(&self) {
        let state = match self.managed(BLUEZ).await {
            Ok(managed) => {
                let pairing = self
                    .inner
                    .lock()
                    .map(|inner| inner.pairing.clone())
                    .unwrap_or_default();
                let empty = Props::new();
                let found = managed.values().find_map(|interfaces| {
                    interfaces
                        .iter()
                        .find(|(name, _)| name.as_str() == "org.bluez.Adapter1")
                        .map(|(_, props)| props)
                });
                let available = found.is_some();
                let adapter = found.unwrap_or(&empty);
                let mut devices = Vec::new();
                for (path, interfaces) in &managed {
                    let find = |name: &str| {
                        interfaces
                            .iter()
                            .find(|(iface, _)| iface.as_str() == name)
                            .map(|(_, props)| props)
                    };
                    let Some(device) = find("org.bluez.Device1") else {
                        continue;
                    };
                    let flag = |name: &str| prop(device, name).and_then(boolean).unwrap_or(false);
                    if !(device.contains_key("Name") || flag("Paired") || flag("Connected")) {
                        continue;
                    }
                    let name = device
                        .get("Alias")
                        .or_else(|| device.get("Address"))
                        .map(|value| string(value))
                        .unwrap_or_else(|| "Device".into());
                    devices.push(BluetoothDevice {
                        path: path.to_string(),
                        name,
                        address: text(device, "Address"),
                        icon: text(device, "Icon"),
                        paired: flag("Paired"),
                        trusted: flag("Trusted"),
                        connected: flag("Connected"),
                        pairing: pairing.contains(&path.to_string()),
                        battery: find("org.bluez.Battery1")
                            .and_then(|props| prop(props, "Percentage").and_then(int))
                            .unwrap_or(-1),
                    });
                }
                devices.sort_by(|a, b| {
                    (!a.connected, a.name.to_lowercase())
                        .cmp(&(!b.connected, b.name.to_lowercase()))
                });
                BluetoothState {
                    available,
                    powered: prop(adapter, "Powered").and_then(boolean).unwrap_or(false),
                    discovering: prop(adapter, "Discovering")
                        .and_then(boolean)
                        .unwrap_or(false),
                    name: adapter
                        .get("Alias")
                        .map(|value| string(value))
                        .unwrap_or_else(|| "No adapter".into()),
                    devices,
                }
            }
            Err(error) => BluetoothState {
                available: false,
                powered: false,
                discovering: false,
                name: error_text(&error),
                devices: Vec::new(),
            },
        };
        let changed = match self.inner.lock() {
            Ok(mut inner) => {
                let changed = inner.last_bluetooth.as_ref() != Some(&state);
                inner.last_bluetooth = Some(state.clone());
                changed
            }
            Err(_) => true,
        };
        if changed {
            self.emit(Event::Bluetooth(state));
        }
    }

    async fn adapter(&self) -> zbus::Result<String> {
        let managed = self.managed(BLUEZ).await?;
        let mut paths: Vec<String> = managed
            .iter()
            .filter(|(_, interfaces)| {
                interfaces
                    .keys()
                    .any(|name| name.as_str() == "org.bluez.Adapter1")
            })
            .map(|(path, _)| path.to_string())
            .collect();
        paths.sort();
        paths
            .into_iter()
            .next()
            .ok_or_else(|| zbus::Error::Failure("No Bluetooth adapter found".into()))
    }

    async fn bluetooth_fail(&self, message: String) {
        let pending = self.inner.lock().ok().and_then(|mut inner| {
            inner.pairing.clear();
            inner.bluetooth_pending.take()
        });
        drop(pending);
        self.refresh_bluetooth().await;
        self.emit(Event::BluetoothError(message));
        self.emit(Event::BluetoothPrompt(BluetoothPrompt::default()));
    }

    async fn bluetooth(&self, run: impl AsyncFnOnce(&Self, String) -> zbus::Result<()>) {
        let result = match self.adapter().await {
            Ok(adapter) => run(self, adapter).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.bluetooth_fail(error_text(&error)).await;
        }
    }

    pub async fn bt_power(&self, enabled: bool) {
        self.bluetooth(async |handle, adapter| {
            handle
                .set_property(
                    BLUEZ,
                    &adapter,
                    "org.bluez.Adapter1",
                    "Powered",
                    Variant::Bool(enabled),
                )
                .await
        })
        .await;
    }

    pub async fn bt_scan(&self, enabled: bool) {
        self.bluetooth(async |handle, adapter| {
            let method = if enabled {
                "StartDiscovery"
            } else {
                "StopDiscovery"
            };
            handle
                .call(BLUEZ, &adapter, "org.bluez.Adapter1", method, &())
                .await
                .map(|_| ())
        })
        .await;
    }

    pub async fn bt_pair(&self, device: String) {
        self.bluetooth(async |handle, _| {
            if let Ok(mut inner) = handle.inner.lock() {
                inner.pairing.push(device.clone());
            }
            handle.refresh_bluetooth().await;
            handle
                .call(BLUEZ, &device, "org.bluez.Device1", "Pair", &())
                .await?;
            if let Ok(mut inner) = handle.inner.lock() {
                inner.pairing.retain(|path| *path != device);
            }
            handle.refresh_bluetooth().await;
            handle
                .set_property(
                    BLUEZ,
                    &device,
                    "org.bluez.Device1",
                    "Trusted",
                    Variant::Bool(true),
                )
                .await?;
            handle.emit(Event::BluetoothPrompt(BluetoothPrompt::default()));
            handle
                .call(BLUEZ, &device, "org.bluez.Device1", "Connect", &())
                .await
                .map(|_| ())
        })
        .await;
    }

    pub async fn bt_device(&self, device: String, method: &'static str) {
        self.bluetooth(async |handle, _| {
            if method == "CancelPairing" {
                if let Ok(mut inner) = handle.inner.lock() {
                    inner.pairing.retain(|path| *path != device);
                }
                handle.refresh_bluetooth().await;
            }
            handle
                .call(BLUEZ, &device, "org.bluez.Device1", method, &())
                .await
                .map(|_| ())
        })
        .await;
    }

    pub async fn bt_trust(&self, device: String, enabled: bool) {
        self.bluetooth(async |handle, _| {
            handle
                .set_property(
                    BLUEZ,
                    &device,
                    "org.bluez.Device1",
                    "Trusted",
                    Variant::Bool(enabled),
                )
                .await
        })
        .await;
    }

    pub async fn bt_rename(&self, device: String, alias: String) {
        let alias = alias.trim().to_string();
        self.bluetooth(async |handle, _| {
            handle
                .set_property(
                    BLUEZ,
                    &device,
                    "org.bluez.Device1",
                    "Alias",
                    Variant::Str(alias.as_str().into()),
                )
                .await
        })
        .await;
    }

    pub async fn bt_forget(&self, device: String) {
        self.bluetooth(async |handle, adapter| {
            let path = zbus::ObjectPath::try_from(device.as_str())?;
            handle
                .call(
                    BLUEZ,
                    &adapter,
                    "org.bluez.Adapter1",
                    "RemoveDevice",
                    &(path,),
                )
                .await
                .map(|_| ())
        })
        .await;
    }

    pub async fn bt_profile(&self, id: i64, index: i64) {
        let result = Command::new("wpctl")
            .args(["set-profile", &id.to_string(), &index.to_string()])
            .status();
        if let Err(err) = result {
            self.bluetooth_fail(err.to_string()).await;
        }
    }

    pub async fn bluetooth_reply(&self, accepted: bool, value: String) {
        let pending = self
            .inner
            .lock()
            .ok()
            .and_then(|mut inner| inner.bluetooth_pending.take());
        let mut failure = None;
        if let Some((sender, mode)) = pending
            && let Some(sender) = sender
        {
            if !accepted {
                sender.send(Answer::Reject);
            } else if mode == "passkey" {
                if !value.is_empty()
                    && value.len() <= 6
                    && value.chars().all(|c| c.is_ascii_digit())
                {
                    sender.send(Answer::Accept(value));
                } else {
                    sender.send(Answer::Reject);
                    failure = Some("Passkey must contain one to six digits".to_string());
                }
            } else if mode == "pin" {
                if (1..=16).contains(&value.len()) {
                    sender.send(Answer::Accept(value));
                } else {
                    sender.send(Answer::Reject);
                    failure = Some("PIN must contain 1 to 16 bytes".to_string());
                }
            } else {
                sender.send(Answer::Accept(String::new()));
            }
        }
        match failure {
            Some(message) => self.bluetooth_fail(message).await,
            None => self.emit(Event::BluetoothPrompt(BluetoothPrompt::default())),
        }
    }

    pub async fn profiles(&self, force: bool) {
        let dump = run_json(&["pw-dump"]);
        let mut result = HashMap::new();
        for item in dump.as_array().into_iter().flatten() {
            let info = &item["info"];
            let props = &info["props"];
            if item["type"].as_str() != Some("PipeWire:Interface:Device")
                || props["device.api"].as_str() != Some("bluez5")
            {
                continue;
            }
            let params = &info["params"];
            let current = params["Profile"]
                .as_array()
                .and_then(|entries| entries.first())
                .and_then(|entry| entry["index"].as_i64());
            let profiles = params["EnumProfile"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| {
                    entry["name"].as_str() != Some("off")
                        && entry["available"].as_str() != Some("no")
                })
                .map(|entry| AudioProfile {
                    index: entry["index"].as_i64().unwrap_or(0),
                    name: entry["name"].as_str().unwrap_or("").to_string(),
                    description: entry["description"].as_str().unwrap_or("").to_string(),
                })
                .collect();
            result.insert(
                props["api.bluez5.address"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                DeviceProfiles {
                    id: item["id"].as_i64().unwrap_or(0),
                    current,
                    profiles,
                },
            );
        }
        let changed = match self.inner.lock() {
            Ok(mut inner) => {
                let changed = force || inner.last_profiles.as_ref() != Some(&result);
                inner.last_profiles = Some(result.clone());
                changed
            }
            Err(_) => true,
        };
        if changed {
            self.emit(Event::Profiles(result));
        }
    }

    fn nm_signal(&self, message: &zbus::Message, member: &str) -> bool {
        let header = message.header();
        let path = header.path().map(|p| p.as_str()).unwrap_or_default();
        match member {
            "Updated" => self.nm_forget_cached(Some(path)),
            "ConnectionRemoved" => {
                let removed = message.body().deserialize::<OwnedObjectPath>().ok();
                self.nm_forget_cached(removed.as_ref().map(|path| path.as_str()));
            }
            _ => {}
        }
        let (interface, changed) = if member == "PropertiesChanged" {
            match message.body().deserialize::<(String, Props, Vec<String>)>() {
                Ok((interface, changed, invalidated)) => (
                    interface,
                    changed.into_keys().chain(invalidated).collect::<Vec<_>>(),
                ),
                Err(_) => return false,
            }
        } else {
            (
                header
                    .interface()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default(),
                Vec::new(),
            )
        };
        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
        let (panel, scanning) = self
            .inner
            .lock()
            .map(|inner| (inner.panel, inner.nm_scan.is_some()))
            .unwrap_or_default();
        nm::signal_matters(member, path, &interface, &changed, panel, scanning)
    }

    fn iwd_signal(&self, message: &zbus::Message, member: &str) -> bool {
        let body = message.body();
        let (interfaces, changed): (Vec<String>, Vec<String>) = match member {
            "InterfacesAdded" => {
                match body.deserialize::<(OwnedObjectPath, HashMap<String, Props>)>() {
                    Ok((_, added)) => (added.into_keys().collect(), Vec::new()),
                    Err(_) => return true,
                }
            }
            "InterfacesRemoved" => match body.deserialize::<(OwnedObjectPath, Vec<String>)>() {
                Ok((_, removed)) => (removed, Vec::new()),
                Err(_) => return true,
            },
            "PropertiesChanged" => match body.deserialize::<(String, Props, Vec<String>)>() {
                Ok((interface, changed, invalidated)) => (
                    vec![interface],
                    changed.into_keys().chain(invalidated).collect(),
                ),
                Err(_) => return true,
            },
            _ => return true,
        };
        let interfaces: Vec<&str> = interfaces.iter().map(String::as_str).collect();
        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
        let panel = self.inner.lock().map(|inner| inner.panel).unwrap_or(true);
        iwd_signal_matters(member, &interfaces, &changed, panel)
    }

    pub async fn watch(&self) -> zbus::Result<()> {
        let mut streams = Vec::new();
        for (sender, member) in [
            (IWD, "PropertiesChanged"),
            (IWD, "InterfacesAdded"),
            (IWD, "InterfacesRemoved"),
            (nm::NM, "PropertiesChanged"),
            (nm::NM, "StateChanged"),
            (nm::NM, "NewConnection"),
            (nm::NM, "ConnectionRemoved"),
            (nm::NM, "Updated"),
            (nm::NM, "AccessPointAdded"),
            (nm::NM, "AccessPointRemoved"),
            (BLUEZ, "PropertiesChanged"),
            (BLUEZ, "InterfacesAdded"),
            (BLUEZ, "InterfacesRemoved"),
        ] {
            let rule = MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(sender)
                .member(member)
                .build()?;
            streams.push((
                sender,
                MessageStream::for_match_rule(rule, &self.connection, None).await?,
            ));
        }
        for service in [IWD, nm::NM, BLUEZ] {
            let rule = MatchRule::builder()
                .msg_type(Type::Signal)
                .sender("org.freedesktop.DBus")
                .member("NameOwnerChanged")
                .arg(0, service)
                .build()?;
            streams.push((
                OWNER_CHANGED,
                MessageStream::for_match_rule(rule, &self.connection, None).await?,
            ));
        }
        let mut owners: HashMap<&'static str, String> = HashMap::new();
        for service in [IWD, nm::NM, BLUEZ] {
            owners.insert(service, self.owner(service).await.unwrap_or_default());
        }
        let tagged = streams
            .into_iter()
            .map(|(service, stream)| StreamExt::map(stream, move |message| (service, message)));
        let mut merged = futures_util::stream::select_all(tagged);
        while let Some((service, message)) = merged.next().await {
            let Ok(message) = message else {
                continue;
            };
            let header = message.header();
            if service == OWNER_CHANGED {
                let Ok((name, _, owner)) = message.body().deserialize::<(String, String, String)>()
                else {
                    continue;
                };
                let Some(service) = [IWD, nm::NM, BLUEZ].into_iter().find(|s| *s == name) else {
                    continue;
                };
                owners.insert(service, owner);
                if service == BLUEZ {
                    self.emit(Event::BluetoothChanged);
                    continue;
                }
                if let Ok(mut inner) = self.inner.lock() {
                    if service == nm::NM {
                        inner.nm_agent = false;
                        inner.nm_saved.clear();
                        inner.nm_scan = None;
                    } else {
                        inner.iwd_agent = false;
                    }
                }
                self.emit(Event::WifiChanged);
                continue;
            }
            let sender = header
                .sender()
                .map(|name| name.as_str())
                .unwrap_or_default();
            if owners.get(service).is_none_or(|owner| owner != sender) {
                continue;
            }
            if service == BLUEZ {
                self.emit(Event::BluetoothChanged);
                continue;
            }
            let member = header.member().map(|m| m.as_str()).unwrap_or_default();
            if service == nm::NM && !self.nm_signal(&message, member) {
                continue;
            }
            if service == IWD && !self.iwd_signal(&message, member) {
                continue;
            }
            self.emit(Event::WifiChanged);
        }
        Ok(())
    }
}

pub async fn start(callback: impl Fn(Event) + Send + Sync + 'static) -> zbus::Result<Handle> {
    let address = std::env::var("CHUHSHELL_NETWORK_BUS").ok();
    start_on(
        address.as_deref().filter(|address| !address.is_empty()),
        callback,
    )
    .await
}

pub async fn start_on(
    address: Option<&str>,
    callback: impl Fn(Event) + Send + Sync + 'static,
) -> zbus::Result<Handle> {
    let connection = match address {
        Some(address) => {
            log::info!("network bus: {address}");
            zbus::connection::Builder::address(address).build().await?
        }
        None => Connection::system().await?,
    };
    let handle = Handle {
        connection: connection.clone(),
        inner: Arc::new(Mutex::new(Inner::default())),
        callback: Arc::new(callback),
    };
    connection
        .object_server()
        .at(
            BLUETOOTH_AGENT,
            BluetoothAgent {
                handle: handle.clone(),
            },
        )
        .await?;
    connection
        .object_server()
        .at(
            WIFI_AGENT,
            WifiAgent {
                handle: handle.clone(),
            },
        )
        .await?;
    connection
        .object_server()
        .at(
            nm::AGENT_PATH,
            nm::Agent {
                handle: handle.clone(),
            },
        )
        .await?;
    let path = zbus::ObjectPath::try_from(BLUETOOTH_AGENT)?;
    if handle
        .call(
            BLUEZ,
            "/org/bluez",
            "org.bluez.AgentManager1",
            "RegisterAgent",
            &(path.clone(), "KeyboardDisplay"),
        )
        .await
        .is_ok()
    {
        let _ = handle
            .call(
                BLUEZ,
                "/org/bluez",
                "org.bluez.AgentManager1",
                "RequestDefaultAgent",
                &(path,),
            )
            .await;
    }
    handle.detect_backend().await;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iwd_scan_noise_waits_for_the_panel() {
        let station = ["net.connman.iwd.Station"];
        let network = ["net.connman.iwd.Network"];
        assert!(!iwd_signal_matters(
            "PropertiesChanged",
            &station,
            &["Scanning"],
            false
        ));
        assert!(iwd_signal_matters(
            "PropertiesChanged",
            &station,
            &["Scanning"],
            true
        ));
        assert!(iwd_signal_matters(
            "PropertiesChanged",
            &station,
            &["State", "Scanning"],
            false
        ));
        assert!(!iwd_signal_matters(
            "PropertiesChanged",
            &network,
            &["ExtendedServiceSet"],
            false
        ));
        assert!(iwd_signal_matters(
            "PropertiesChanged",
            &network,
            &["Connected"],
            false
        ));
        assert!(iwd_signal_matters(
            "PropertiesChanged",
            &["net.connman.iwd.Device"],
            &["Powered"],
            false
        ));
        assert!(iwd_signal_matters(
            "PropertiesChanged",
            &["net.connman.iwd.KnownNetwork"],
            &["AutoConnect"],
            false
        ));
        assert!(!iwd_signal_matters("InterfacesAdded", &network, &[], false));
        assert!(iwd_signal_matters("InterfacesAdded", &network, &[], true));
        assert!(!iwd_signal_matters(
            "InterfacesRemoved",
            &["net.connman.iwd.Network", "net.connman.iwd.BasicServiceSet"],
            &[],
            false
        ));
        assert!(iwd_signal_matters(
            "InterfacesAdded",
            &["net.connman.iwd.KnownNetwork"],
            &[],
            false
        ));
        assert!(iwd_signal_matters(
            "InterfacesRemoved",
            &["net.connman.iwd.Station"],
            &[],
            false
        ));
    }

    #[test]
    fn backend_prefers_networkmanager() {
        assert_eq!(choose_backend(true, true), Backend::NetworkManager);
        assert_eq!(choose_backend(false, true), Backend::Iwd);
        assert_eq!(choose_backend(true, false), Backend::NetworkManager);
        assert_eq!(choose_backend(false, false), Backend::None);
    }

    #[test]
    fn failures_read_like_wifi_py() {
        assert_eq!(
            wifi_failure("net.connman.iwd.Failed: Operation failed"),
            "Connection failed. Check the password and try again."
        );
        assert_eq!(
            wifi_failure("net.connman.iwd.NotFound: Network not found"),
            "Network not found"
        );
    }
}
