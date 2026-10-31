use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use zbus::{Connection, OwnedVariant, Variant, interface, object_server::SignalEmitter};

pub const PATH: &str = "/org/freedesktop/Notifications";
pub const NAME: &str = "org.freedesktop.Notifications";

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub identifier: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Image {
    Path(String),
    Pixels {
        width: u32,
        height: u32,
        rgba: Arc<Vec<u8>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Low = 0,
    Normal = 1,
    Critical = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<Action>,
    pub urgency: Urgency,
    pub transient: bool,
    pub resident: bool,
    pub expire_timeout: i32,
    pub desktop_entry: String,
    pub image: Option<Image>,
    pub tag: String,
}

#[derive(Clone, Debug)]
pub enum Event {
    Added(Notification),
    Updated {
        notification: Notification,
        summary_changed: bool,
        body_changed: bool,
    },
    Closed(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Default)]
struct Store {
    next: u32,
    alive: HashMap<u32, Notification>,
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

struct Server {
    store: Arc<Mutex<Store>>,
    callback: Callback,
}

fn string(value: &Variant<'_>) -> Option<String> {
    match value {
        Variant::Str(text) => Some(text.as_str().to_string()),
        Variant::Variant(inner) => string(inner),
        _ => None,
    }
}

fn boolean(value: &Variant<'_>) -> Option<bool> {
    match value {
        Variant::Bool(value) => Some(*value),
        Variant::U8(value) => Some(*value != 0),
        Variant::I32(value) => Some(*value != 0),
        Variant::U32(value) => Some(*value != 0),
        Variant::Variant(inner) => boolean(inner),
        _ => None,
    }
}

fn byte(value: &Variant<'_>) -> Option<u8> {
    match value {
        Variant::U8(value) => Some(*value),
        Variant::I32(value) => u8::try_from(*value).ok(),
        Variant::U32(value) => u8::try_from(*value).ok(),
        Variant::Variant(inner) => byte(inner),
        _ => None,
    }
}

fn int(value: &Variant<'_>) -> Option<i64> {
    match value {
        Variant::I32(value) => Some(*value as i64),
        Variant::U32(value) => Some(*value as i64),
        Variant::U8(value) => Some(*value as i64),
        _ => None,
    }
}

fn image_data(value: &Variant<'_>) -> Option<Image> {
    let structure = match value {
        Variant::Structure(structure) => structure,
        Variant::Variant(inner) => return image_data(inner),
        _ => return None,
    };
    let fields = structure.fields();
    if fields.len() < 7 {
        return None;
    }
    let width = u32::try_from(int(&fields[0])?).ok()?;
    let height = u32::try_from(int(&fields[1])?).ok()?;
    let stride = usize::try_from(int(&fields[2])?).ok()?;
    let alpha = boolean(&fields[3])?;
    let channels = usize::try_from(int(&fields[5])?).ok()?;
    let data: Vec<u8> = Vec::<u8>::try_from(fields[6].clone()).ok()?;
    if width == 0 || height == 0 || channels < 3 {
        return None;
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let i = y * stride + x * channels;
            let pixel = data.get(i..i + channels)?;
            rgba.extend_from_slice(&[
                pixel[0],
                pixel[1],
                pixel[2],
                if alpha && channels >= 4 {
                    pixel[3]
                } else {
                    255
                },
            ]);
        }
    }
    Some(Image::Pixels {
        width,
        height,
        rgba: Arc::new(rgba),
    })
}

#[allow(clippy::too_many_arguments)]
fn parse(
    id: u32,
    app_name: String,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: &HashMap<String, OwnedVariant>,
    expire_timeout: i32,
) -> Notification {
    let hint = |name: &str| hints.get(name).map(|value| &**value);
    let image = ["image-data", "image_data", "icon_data"]
        .iter()
        .find_map(|name| hint(name).and_then(image_data))
        .or_else(|| {
            ["image-path", "image_path"]
                .iter()
                .find_map(|name| hint(name).and_then(string))
                .filter(|path| !path.is_empty())
                .map(Image::Path)
        });
    let urgency = match hint("urgency").and_then(byte) {
        Some(0) => Urgency::Low,
        Some(2) => Urgency::Critical,
        _ => Urgency::Normal,
    };
    let tag = ["x-canonical-private-synchronous", "synchronous"]
        .iter()
        .find_map(|name| hint(name).and_then(string).filter(|tag| !tag.is_empty()))
        .unwrap_or_default();
    Notification {
        id,
        app_name,
        app_icon,
        summary,
        body,
        actions: actions
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| Action {
                identifier: pair[0].clone(),
                text: pair[1].clone(),
            })
            .collect(),
        urgency,
        transient: hint("transient").and_then(boolean).unwrap_or(false),
        resident: hint("resident").and_then(boolean).unwrap_or(false),
        expire_timeout,
        desktop_entry: hint("desktop-entry").and_then(string).unwrap_or_default(),
        image,
        tag,
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    async fn get_capabilities(&self) -> Vec<String> {
        [
            "actions",
            "body",
            "body-hyperlinks",
            "body-markup",
            "icon-static",
            "persistence",
        ]
        .iter()
        .map(|cap| cap.to_string())
        .collect()
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        let Ok(mut store) = self.store.lock() else {
            return 0;
        };
        if replaces_id != 0
            && let Some(old) = store.alive.get(&replaces_id).cloned()
        {
            let notification = parse(
                replaces_id,
                app_name,
                app_icon,
                summary,
                body,
                actions,
                &hints,
                expire_timeout,
            );
            let summary_changed = old.summary != notification.summary;
            let body_changed = old.body != notification.body;
            store.alive.insert(replaces_id, notification.clone());
            drop(store);
            (self.callback)(Event::Updated {
                notification,
                summary_changed,
                body_changed,
            });
            return replaces_id;
        }
        store.next += 1;
        let id = store.next;
        let notification = parse(
            id,
            app_name,
            app_icon,
            summary,
            body,
            actions,
            &hints,
            expire_timeout,
        );
        store.alive.insert(id, notification.clone());
        drop(store);
        (self.callback)(Event::Added(notification));
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let removed = self
            .store
            .lock()
            .ok()
            .and_then(|mut store| store.alive.remove(&id));
        if removed.is_some() {
            if let Err(err) =
                Self::notification_closed(&emitter, id, CloseReason::Closed as u32).await
            {
                log::warn!("notification closed signal: {err}");
            }
            (self.callback)(Event::Closed(id));
        }
    }

    async fn get_server_information(&self) -> (String, String, String, String) {
        (
            "quickshell".into(),
            "quickshell".into(),
            "0.2.1".into(),
            "1.2".into(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct Handle {
    connection: Connection,
    store: Arc<Mutex<Store>>,
}

impl Handle {
    pub async fn close(&self, id: u32, reason: CloseReason) {
        let removed = self
            .store
            .lock()
            .ok()
            .and_then(|mut store| store.alive.remove(&id));
        if removed.is_none() {
            return;
        }
        match SignalEmitter::new(&self.connection, PATH) {
            Ok(emitter) => {
                if let Err(err) = Server::notification_closed(&emitter, id, reason as u32).await {
                    log::warn!("notification closed signal: {err}");
                }
            }
            Err(err) => log::warn!("notification emitter: {err}"),
        }
    }

    pub async fn invoke(&self, id: u32, action: &str) {
        match SignalEmitter::new(&self.connection, PATH) {
            Ok(emitter) => {
                if let Err(err) = Server::action_invoked(&emitter, id, action).await {
                    log::warn!("action invoked signal: {err}");
                }
            }
            Err(err) => log::warn!("notification emitter: {err}"),
        }
    }
}

pub async fn serve(callback: impl Fn(Event) + Send + Sync + 'static) -> zbus::Result<Handle> {
    let store = Arc::new(Mutex::new(Store::default()));
    let server = Server {
        store: store.clone(),
        callback: Arc::new(callback),
    };
    let connection = zbus::connection::Builder::session()
        .name(NAME)
        .serve_at(PATH, server)
        .build()
        .await?;
    Ok(Handle { connection, store })
}
