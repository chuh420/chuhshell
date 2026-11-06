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

impl Store {
    fn allocate(&mut self) -> u32 {
        loop {
            self.next = self.next.wrapping_add(1).max(1);
            if !self.alive.contains_key(&self.next) {
                return self.next;
            }
        }
    }
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
    let data: Vec<u8> = Vec::<u8>::try_from(fields[6].clone()).ok()?;
    pixels(
        int(&fields[0])?,
        int(&fields[1])?,
        int(&fields[2])?,
        boolean(&fields[3])?,
        int(&fields[5])?,
        &data,
    )
}

pub const MAX_IMAGE_SIDE: u32 = 2048;

pub fn pixels(
    width: i64,
    height: i64,
    stride: i64,
    alpha: bool,
    channels: i64,
    data: &[u8],
) -> Option<Image> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    let stride = usize::try_from(stride).ok()?;
    let channels = usize::try_from(channels).ok()?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || !(3..=4).contains(&channels)
    {
        return None;
    }
    let row = (width as usize).checked_mul(channels)?;
    if stride < row {
        return None;
    }
    let needed = (height as usize - 1)
        .checked_mul(stride)?
        .checked_add(row)?;
    if data.len() < needed {
        return None;
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let line = &data[y * stride..y * stride + row];
        for pixel in line.chunks_exact(channels) {
            rgba.extend_from_slice(&[
                pixel[0],
                pixel[1],
                pixel[2],
                if alpha && channels == 4 {
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

pub const MAX_SUMMARY: usize = 1024;
pub const MAX_BODY: usize = 16 * 1024;
pub const MAX_NAME: usize = 256;
pub const MAX_ACTIONS: usize = 16;

pub fn clip(mut text: String, limit: usize) -> String {
    if let Some((index, _)) = text.char_indices().nth(limit) {
        text.truncate(index);
        text.push('…');
    }
    text
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
        app_name: clip(app_name, MAX_NAME),
        app_icon: clip(app_icon, 4 * MAX_NAME),
        summary: clip(summary, MAX_SUMMARY),
        body: clip(body, MAX_BODY),
        actions: actions
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .take(MAX_ACTIONS)
            .map(|pair| Action {
                identifier: clip(pair[0].clone(), MAX_NAME),
                text: clip(pair[1].clone(), MAX_NAME),
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
        let id = store.allocate();
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
    callback: Callback,
}

impl Handle {
    pub fn post(
        &self,
        app_name: &str,
        icon: &str,
        summary: &str,
        body: &str,
        urgency: Urgency,
    ) -> u32 {
        let Ok(mut store) = self.store.lock() else {
            return 0;
        };
        let id = store.allocate();
        let mut notification = parse(
            id,
            app_name.into(),
            icon.into(),
            summary.into(),
            body.into(),
            Vec::new(),
            &HashMap::new(),
            -1,
        );
        notification.urgency = urgency;
        store.alive.insert(id, notification.clone());
        drop(store);
        (self.callback)(Event::Added(notification));
        id
    }

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
    serve_on(None, callback).await
}

pub async fn serve_on(
    address: Option<&str>,
    callback: impl Fn(Event) + Send + Sync + 'static,
) -> zbus::Result<Handle> {
    let store = Arc::new(Mutex::new(Store::default()));
    let callback: Callback = Arc::new(callback);
    let server = Server {
        store: store.clone(),
        callback: callback.clone(),
    };
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address),
        None => zbus::connection::Builder::session(),
    };
    let connection = builder.name(NAME).serve_at(PATH, server).build().await?;
    Ok(Handle {
        connection,
        store,
        callback,
    })
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn clip_keeps_char_boundaries(text in "\\PC{0,64}", limit in 0usize..40) {
            let clipped = clip(text.clone(), limit);
            if text.chars().count() <= limit {
                prop_assert_eq!(clipped, text);
            } else {
                prop_assert_eq!(clipped.chars().count(), limit + 1);
                prop_assert!(clipped.ends_with('…'));
            }
        }

        #[test]
        fn image_data_never_overreads(
            width in prop_oneof![-2i64..40, Just(i64::MAX), Just(1i64 << 32), Just(100_000)],
            height in prop_oneof![-2i64..40, Just(i64::MAX), Just(1i64 << 32), Just(100_000)],
            stride in prop_oneof![-2i64..200, Just(i64::MAX), Just(usize::MAX as i64)],
            alpha in any::<bool>(),
            channels in prop_oneof![-1i64..8, Just(i64::MAX)],
            data in prop::collection::vec(any::<u8>(), 0..4096),
        ) {
            if let Some(Image::Pixels { width: w, height: h, rgba }) = pixels(width, height, stride, alpha, channels, &data) {
                prop_assert_eq!(rgba.len(), w as usize * h as usize * 4);
                prop_assert!(w <= MAX_IMAGE_SIDE && h <= MAX_IMAGE_SIDE);
                prop_assert!((h as usize - 1) * stride as usize + w as usize * channels as usize <= data.len());
            }
        }

        #[test]
        fn exact_images_decode(width in 1u32..24, height in 1u32..24, pad in 0usize..8, alpha in any::<bool>()) {
            let channels = if alpha { 4 } else { 3 };
            let stride = width as usize * channels + pad;
            let data: Vec<u8> = (0..stride * height as usize).map(|i| i as u8).collect();
            let image = pixels(width as i64, height as i64, stride as i64, alpha, channels as i64, &data);
            let Some(Image::Pixels { rgba, .. }) = image else {
                return Err(TestCaseError::fail("image refused"));
            };
            let last = (height as usize - 1) * stride + (width as usize - 1) * channels;
            let tail = &rgba[rgba.len() - 4..];
            prop_assert_eq!(&tail[..3], &data[last..last + 3]);
            prop_assert_eq!(tail[3], if alpha { data[last + 3] } else { 255 });
            prop_assert!(pixels(width as i64, height as i64, stride as i64, alpha, channels as i64, &data[..data.len() - pad - 1]).is_none());
        }
    }
}
