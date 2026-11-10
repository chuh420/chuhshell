use std::{
    collections::HashMap,
    os::unix::fs::MetadataExt,
    sync::{Arc, Mutex},
};

use futures_util::StreamExt;
use zbus::{
    Connection, MatchRule, MessageStream, OwnedVariant, Variant, interface,
    message::{Header, Type},
    object_server::SignalEmitter,
};

pub const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayItem {
    pub key: String,
    pub service: String,
    pub path: String,
    pub owner: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub icon_name: String,
    pub attention_icon_name: String,
    pub icon_theme_path: String,
    pub icon_pixmap: Option<Pixels>,
    pub attention_pixmap: Option<Pixels>,
    pub item_is_menu: bool,
    pub menu: String,
}

impl TrayItem {
    pub fn needs_attention(&self) -> bool {
        self.status == "NeedsAttention"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub has_children: bool,
    pub checked: bool,
    pub children: Vec<MenuEntry>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Registered(String),
    NameLost(String),
    ItemChanged(String),
    MenuChanged(String),
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
    callback: Callback,
}

pub fn split_key(key: &str) -> (String, String) {
    match key.find('/') {
        Some(index) => (key[..index].to_string(), key[index..].to_string()),
        None => (key.to_string(), "/StatusNotifierItem".to_string()),
    }
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header
            .sender()
            .map(|sender| sender.to_string())
            .unwrap_or_default();
        let key = if service.starts_with('/') {
            format!("{sender}{service}")
        } else if service.contains('/') {
            service.to_string()
        } else {
            format!("{service}/StatusNotifierItem")
        };
        let added = match self.items.lock() {
            Ok(mut items) if !items.contains(&key) => {
                items.push(key.clone());
                true
            }
            _ => false,
        };
        if added {
            if let Err(err) = Self::status_notifier_item_registered(&emitter, &key).await {
                log::warn!("tray signal: {err}");
            }
            if let Err(err) = self
                .registered_status_notifier_items_changed(&emitter)
                .await
            {
                log::warn!("tray signal: {err}");
            }
            (self.callback)(Event::Registered(key));
        }
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        if let Err(err) = Self::status_notifier_host_registered(&emitter).await {
            log::warn!("tray signal: {err}");
        }
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct Handle {
    connection: Connection,
    items: Arc<Mutex<Vec<String>>>,
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
        Variant::I32(value) => Some(*value as i64),
        Variant::U32(value) => Some(*value as i64),
        Variant::I64(value) => Some(*value),
        Variant::Variant(inner) => int(inner),
        _ => None,
    }
}

fn pixmap(value: &Variant<'_>) -> Option<Pixels> {
    let array = match value {
        Variant::Array(array) => array,
        Variant::Variant(inner) => return pixmap(inner),
        _ => return None,
    };
    let mut best: Option<(i32, i32, Vec<u8>)> = None;
    for entry in array.iter() {
        let Variant::Structure(structure) = entry else {
            continue;
        };
        let fields = structure.fields();
        if fields.len() < 3 {
            continue;
        }
        let (Some(width), Some(height)) = (int(&fields[0]), int(&fields[1])) else {
            continue;
        };
        let Ok(data) = Vec::<u8>::try_from(fields[2].clone()) else {
            continue;
        };
        let (width, height) = (width as i32, height as i32);
        if width <= 0 || height <= 0 || data.len() < (width * height * 4) as usize {
            continue;
        }
        let better = match &best {
            None => true,
            Some((bw, _, _)) => {
                if *bw >= 34 {
                    width >= 34 && width < *bw
                } else {
                    width > *bw
                }
            }
        };
        if better {
            best = Some((width, height, data));
        }
    }
    let (width, height, data) = best?;
    let mut rgba = Vec::with_capacity(data.len());
    for pixel in data.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
    }
    Some(Pixels {
        width: width as u32,
        height: height as u32,
        rgba: Arc::new(rgba),
    })
}

fn mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            match chars.peek() {
                Some('_') => {
                    out.push('_');
                    chars.next();
                }
                Some(_) => out.push('&'),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn entry(value: &Variant<'_>) -> Option<MenuEntry> {
    let structure = match value {
        Variant::Structure(structure) => structure,
        Variant::Variant(inner) => return entry(inner),
        _ => return None,
    };
    let fields = structure.fields();
    if fields.len() < 3 {
        return None;
    }
    let id = int(&fields[0])? as i32;
    let props: HashMap<String, OwnedVariant> =
        HashMap::try_from(fields[1].try_to_owned().ok()?).ok()?;
    let children: Vec<MenuEntry> = match &fields[2] {
        Variant::Array(array) => array.iter().filter_map(entry).collect(),
        _ => Vec::new(),
    };
    let get = |name: &str| props.get(name).map(|value| &**value);
    let text = |name: &str| get(name).map(string).unwrap_or_default();
    if !get("visible").and_then(boolean).unwrap_or(true) {
        return None;
    }
    Some(MenuEntry {
        id,
        label: mnemonic(&text("label")),
        enabled: get("enabled").and_then(boolean).unwrap_or(true),
        separator: text("type") == "separator",
        has_children: text("children-display") == "submenu" || !children.is_empty(),
        checked: get("toggle-state").and_then(int) == Some(1),
        children,
    })
}

impl Handle {
    pub async fn load(&self, key: &str) -> zbus::Result<TrayItem> {
        let (service, path) = split_key(key);
        let dbus = zbus::fdo::DBusProxy::new(&self.connection).await?;
        let owner = match service.as_str().try_into() {
            Ok(name) => dbus
                .get_name_owner(name)
                .await
                .map(|owner| owner.to_string())
                .unwrap_or_else(|_| service.clone()),
            Err(_) => service.clone(),
        };
        let properties = zbus::fdo::PropertiesProxy::builder(&self.connection)
            .destination(service.clone())
            .path(path.clone())
            .build()
            .await?;
        let interface = zbus::names::InterfaceName::try_from(ITEM)?;
        let values = properties.get_all(interface).await?;
        let get = |name: &str| values.get(name).map(|value| &**value);
        let text = |name: &str| get(name).map(string).unwrap_or_default();
        Ok(TrayItem {
            key: key.to_string(),
            service: service.clone(),
            path,
            owner,
            id: text("Id"),
            title: text("Title"),
            status: text("Status"),
            icon_name: text("IconName"),
            attention_icon_name: text("AttentionIconName"),
            icon_theme_path: text("IconThemePath"),
            icon_pixmap: get("IconPixmap").and_then(pixmap),
            attention_pixmap: get("AttentionIconPixmap").and_then(pixmap),
            item_is_menu: get("ItemIsMenu").and_then(boolean).unwrap_or(false),
            menu: text("Menu"),
        })
    }

    async fn call<B>(
        &self,
        item: &TrayItem,
        interface: &str,
        path: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::Serialize + zbus::DynamicType,
    {
        self.connection
            .call_method(
                Some(item.service.as_str()),
                path,
                Some(interface),
                method,
                body,
            )
            .await
    }

    pub async fn activate(&self, item: &TrayItem) {
        if let Err(err) = self
            .call(item, ITEM, &item.path, "Activate", &(0i32, 0i32))
            .await
        {
            log::warn!("tray activate {}: {err}", item.id);
        }
    }

    pub async fn scroll(&self, item: &TrayItem, delta: i32) {
        if let Err(err) = self
            .call(item, ITEM, &item.path, "Scroll", &(delta, "vertical"))
            .await
        {
            log::warn!("tray scroll {}: {err}", item.id);
        }
    }

    pub async fn menu(&self, item: &TrayItem) -> Vec<MenuEntry> {
        let parent = 0i32;
        if item.menu.is_empty() || item.menu == "/" {
            return Vec::new();
        }
        if let Err(err) = self
            .call(item, MENU, &item.menu, "AboutToShow", &(parent,))
            .await
        {
            log::debug!("tray about to show {}: {err}", item.id);
        }
        let empty: Vec<&str> = Vec::new();
        let reply = match self
            .call(item, MENU, &item.menu, "GetLayout", &(parent, -1i32, empty))
            .await
        {
            Ok(reply) => reply,
            Err(err) => {
                log::warn!("tray menu {}: {err}", item.id);
                return Vec::new();
            }
        };
        type Layout = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);
        let Ok((_, (_, _, children))) = reply.body().deserialize::<(u32, Layout)>() else {
            return Vec::new();
        };
        children.iter().filter_map(|child| entry(child)).collect()
    }

    pub async fn event(&self, item: &TrayItem, id: i32, name: &str) {
        if item.menu.is_empty() || item.menu == "/" {
            return;
        }
        let body = (id, name, Variant::I32(0), 0u32);
        if let Err(err) = self.call(item, MENU, &item.menu, "Event", &body).await {
            log::warn!("tray menu event {}: {err}", item.id);
        }
    }

    pub async fn unregister(&self, key: &str) {
        let removed = match self.items.lock() {
            Ok(mut items) => {
                let before = items.len();
                items.retain(|item| item != key);
                before != items.len()
            }
            Err(_) => false,
        };
        if !removed {
            return;
        }
        let Ok(emitter) = SignalEmitter::new(&self.connection, WATCHER_PATH) else {
            return;
        };
        if let Err(err) = Watcher::status_notifier_item_unregistered(&emitter, key).await {
            log::warn!("tray signal: {err}");
        }
    }

    pub async fn quit(&self, items: &[TrayItem], target: &TrayItem) {
        let title = |item: &TrayItem| {
            if item.title.is_empty() {
                item.id.clone()
            } else {
                item.title.clone()
            }
        };
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&self.connection).await else {
            return;
        };
        let own_uid = std::fs::metadata("/proc/self").map(|meta| meta.uid()).ok();
        let mut pids = Vec::new();
        for item in items
            .iter()
            .filter(|item| item.id == target.id && title(item) == title(target))
        {
            let Ok(name) = item.service.as_str().try_into() else {
                continue;
            };
            let Ok(pid) = dbus.get_connection_unix_process_id(name).await else {
                continue;
            };
            let uid = std::fs::metadata(format!("/proc/{pid}"))
                .map(|meta| meta.uid())
                .ok();
            if pid > 1 && uid.is_some() && uid == own_uid && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        if pids.len() != 1 {
            log::warn!("tray quit: could not uniquely identify the tray application");
            return;
        }
        if let Err(err) =
            crate::programs::signal(pids[0] as i32, crate::programs::Signal::Terminate)
        {
            log::warn!("tray quit: {err}");
        }
    }

    pub async fn watch(
        &self,
        callback: impl Fn(Event) + Send + Sync + 'static,
    ) -> zbus::Result<()> {
        let mut streams = Vec::new();
        for rule in [
            MatchRule::builder()
                .msg_type(Type::Signal)
                .interface(ITEM)
                .build()?,
            MatchRule::builder()
                .msg_type(Type::Signal)
                .interface(MENU)
                .build()?,
            MatchRule::builder()
                .msg_type(Type::Signal)
                .sender("org.freedesktop.DBus")
                .member("NameOwnerChanged")
                .build()?,
        ] {
            streams.push(MessageStream::for_match_rule(rule, &self.connection, None).await?);
        }
        let mut stream = futures_util::stream::select_all(streams);
        while let Some(message) = stream.next().await {
            let Ok(message) = message else {
                continue;
            };
            let header = message.header();
            let interface = header
                .interface()
                .map(|name| name.to_string())
                .unwrap_or_default();
            let member = header
                .member()
                .map(|name| name.to_string())
                .unwrap_or_default();
            let sender = header
                .sender()
                .map(|name| name.to_string())
                .unwrap_or_default();
            if member == "NameOwnerChanged" {
                if let Ok((name, _, new)) = message.body().deserialize::<(String, String, String)>()
                    && new.is_empty()
                {
                    callback(Event::NameLost(name));
                }
            } else if interface == ITEM {
                callback(Event::ItemChanged(sender));
            } else if interface == MENU {
                callback(Event::MenuChanged(sender));
            }
        }
        Ok(())
    }
}

pub async fn serve(callback: impl Fn(Event) + Send + Sync + 'static) -> zbus::Result<Handle> {
    let items = Arc::new(Mutex::new(Vec::new()));
    let watcher = Watcher {
        items: items.clone(),
        callback: Arc::new(callback),
    };
    let connection = zbus::connection::Builder::session()
        .name(WATCHER)
        .serve_at(WATCHER_PATH, watcher)
        .build()
        .await?;
    Ok(Handle { connection, items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_split_like_tray_py() {
        assert_eq!(
            split_key(":1.5/org/ayatana/NotificationItem/x"),
            (":1.5".into(), "/org/ayatana/NotificationItem/x".into())
        );
        assert_eq!(
            split_key("org.kde.StatusNotifierItem-1-1"),
            (
                "org.kde.StatusNotifierItem-1-1".into(),
                "/StatusNotifierItem".into()
            )
        );
    }

    #[test]
    fn dbusmenu_mnemonics() {
        assert_eq!(mnemonic("_File"), "&File");
        assert_eq!(mnemonic("a__b"), "a_b");
    }
}
