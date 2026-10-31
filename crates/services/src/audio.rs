use std::{
    cell::RefCell, collections::HashMap, io::Cursor, rc::Rc, sync::Arc, thread, time::Duration,
};

use pipewire as pw;
use pw::{
    device::{Device, DeviceListener},
    metadata::{Metadata, MetadataListener},
    node::{Node, NodeListener},
    spa::{
        param::ParamType,
        pod::{
            Object, Pod, Property, PropertyFlags, Value, ValueArray, deserialize::PodDeserializer,
            serialize::PodSerializer,
        },
        sys as spa_sys,
        utils::dict::DictRef,
    },
    types::ObjectType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Sink,
    Source,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub direction: Direction,
    pub volume: f32,
    pub muted: bool,
    pub has_audio: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    pub nodes: Vec<AudioNode>,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
}

impl AudioState {
    fn default_node(&self, name: Option<&String>, direction: Direction) -> Option<&AudioNode> {
        let name = name?;
        self.nodes
            .iter()
            .find(|node| node.direction == direction && &node.name == name)
    }

    pub fn sink(&self) -> Option<&AudioNode> {
        self.default_node(self.default_sink.as_ref(), Direction::Sink)
    }

    pub fn source(&self) -> Option<&AudioNode> {
        self.default_node(self.default_source.as_ref(), Direction::Source)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    SetVolume { node: u32, volume: f32 },
    SetMute { node: u32, muted: bool },
}

#[derive(Clone)]
pub struct AudioHandle {
    sender: Arc<std::sync::Mutex<Option<pw::channel::Sender<Command>>>>,
}

impl AudioHandle {
    pub fn send(&self, command: Command) {
        if let Ok(guard) = self.sender.lock()
            && let Some(sender) = guard.as_ref()
            && sender.send(command).is_err()
        {
            log::warn!("audio: command dropped");
        }
    }
}

struct Entry {
    proxy: Node,
    _listener: NodeListener,
    name: String,
    description: String,
    direction: Direction,
    volumes: Vec<f32>,
    muted: bool,
    has_audio: bool,
    device_id: Option<u32>,
    profile_device: Option<i32>,
}

struct DeviceEntry {
    proxy: Device,
    _listener: DeviceListener,
    routes: HashMap<i32, i32>,
}

#[derive(Default)]
struct Inner {
    nodes: HashMap<u32, Entry>,
    devices: HashMap<u32, DeviceEntry>,
    metadata: Option<(Metadata, MetadataListener)>,
    default_sink: Option<String>,
    default_source: Option<String>,
    last: Option<AudioState>,
}

impl Inner {
    fn snapshot(&self) -> AudioState {
        let mut nodes: Vec<AudioNode> = self
            .nodes
            .iter()
            .map(|(id, entry)| AudioNode {
                id: *id,
                name: entry.name.clone(),
                description: entry.description.clone(),
                direction: entry.direction,
                volume: average_volume(&entry.volumes),
                muted: entry.muted,
                has_audio: entry.has_audio,
            })
            .collect();
        nodes.sort_by_key(|node| node.id);
        AudioState {
            nodes,
            default_sink: self.default_sink.clone(),
            default_source: self.default_source.clone(),
        }
    }
}

pub fn average_volume(volumes: &[f32]) -> f32 {
    if volumes.is_empty() {
        return 0.0;
    }
    let sum: f32 = volumes.iter().map(|value| value.max(0.0).cbrt()).sum();
    sum / volumes.len() as f32
}

pub fn default_name(value: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(value?).ok()?;
    value.get("name")?.as_str().map(str::to_owned)
}

fn props_pod(properties: Vec<Property>) -> Option<Vec<u8>> {
    let value = Value::Object(Object {
        type_: spa_sys::SPA_TYPE_OBJECT_Props,
        id: spa_sys::SPA_PARAM_Props,
        properties,
    });
    PodSerializer::serialize(Cursor::new(Vec::new()), &value)
        .ok()
        .map(|(cursor, _)| cursor.into_inner())
}

fn route_pod(index: i32, device: i32, props: Vec<Property>) -> Option<Vec<u8>> {
    let value = Value::Object(Object {
        type_: spa_sys::SPA_TYPE_OBJECT_ParamRoute,
        id: spa_sys::SPA_PARAM_Route,
        properties: vec![
            Property {
                key: spa_sys::SPA_PARAM_ROUTE_index,
                flags: PropertyFlags::empty(),
                value: Value::Int(index),
            },
            Property {
                key: spa_sys::SPA_PARAM_ROUTE_device,
                flags: PropertyFlags::empty(),
                value: Value::Int(device),
            },
            Property {
                key: spa_sys::SPA_PARAM_ROUTE_props,
                flags: PropertyFlags::empty(),
                value: Value::Object(Object {
                    type_: spa_sys::SPA_TYPE_OBJECT_Props,
                    id: spa_sys::SPA_PARAM_Route,
                    properties: props,
                }),
            },
            Property {
                key: spa_sys::SPA_PARAM_ROUTE_save,
                flags: PropertyFlags::empty(),
                value: Value::Bool(true),
            },
        ],
    });
    PodSerializer::serialize(Cursor::new(Vec::new()), &value)
        .ok()
        .map(|(cursor, _)| cursor.into_inner())
}

fn parse_route(pod: &Pod) -> Option<(i32, i32)> {
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes())
    else {
        return None;
    };
    let mut index = None;
    let mut device = None;
    for property in object.properties {
        match (property.key, property.value) {
            (spa_sys::SPA_PARAM_ROUTE_index, Value::Int(value)) => index = Some(value),
            (spa_sys::SPA_PARAM_ROUTE_device, Value::Int(value)) => device = Some(value),
            _ => {}
        }
    }
    Some((device?, index?))
}

fn apply_props(entry: &mut Entry, pod: &Pod) -> bool {
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes())
    else {
        return false;
    };
    let mut changed = false;
    for property in object.properties {
        match (property.key, property.value) {
            (spa_sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(volumes))) => {
                if entry.volumes != volumes {
                    entry.volumes = volumes;
                    changed = true;
                }
                entry.has_audio = true;
            }
            (spa_sys::SPA_PROP_mute, Value::Bool(muted)) => {
                if entry.muted != muted {
                    entry.muted = muted;
                    changed = true;
                }
                entry.has_audio = true;
            }
            _ => {}
        }
    }
    changed
}

fn dict(props: Option<&DictRef>, key: &str) -> String {
    props
        .and_then(|props| props.get(key))
        .unwrap_or_default()
        .to_string()
}

type Callback = Arc<dyn Fn(AudioState) + Send + Sync>;

fn emit(inner: &Rc<RefCell<Inner>>, callback: &Callback) {
    let state = inner.borrow().snapshot();
    let mut guard = inner.borrow_mut();
    if guard.last.as_ref() != Some(&state) {
        guard.last = Some(state.clone());
        drop(guard);
        callback(state);
    }
}

fn run(
    callback: Callback,
    slot: &Arc<std::sync::Mutex<Option<pw::channel::Sender<Command>>>>,
) -> Result<(), pw::Error> {
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let inner = Rc::new(RefCell::new(Inner::default()));

    let weak_loop = main_loop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _, res, message| {
            log::warn!("pipewire error {id} {res}: {message}");
            if id == 0
                && let Some(main_loop) = weak_loop.upgrade()
            {
                main_loop.quit();
            }
        })
        .register();

    let (sender, receiver) = pw::channel::channel::<Command>();
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(sender);
    }
    let command_inner = inner.clone();
    let _receiver = receiver.attach(main_loop.loop_(), move |command| {
        let inner = command_inner.borrow();
        let (node, properties) = match command {
            Command::SetVolume { node, volume } => {
                let channels = inner.nodes.get(&node).map_or(2, |e| e.volumes.len().max(1));
                let linear = volume.clamp(0.0, 1.0).powi(3);
                (
                    node,
                    vec![Property {
                        key: spa_sys::SPA_PROP_channelVolumes,
                        flags: PropertyFlags::empty(),
                        value: Value::ValueArray(ValueArray::Float(vec![linear; channels])),
                    }],
                )
            }
            Command::SetMute { node, muted } => (
                node,
                vec![Property {
                    key: spa_sys::SPA_PROP_mute,
                    flags: PropertyFlags::empty(),
                    value: Value::Bool(muted),
                }],
            ),
        };
        let Some(entry) = inner.nodes.get(&node) else {
            return;
        };
        let route = entry
            .device_id
            .zip(entry.profile_device)
            .and_then(|(device_id, profile)| {
                let device = inner.devices.get(&device_id)?;
                let index = *device.routes.get(&profile)?;
                Some((device, index, profile))
            });
        match route {
            Some((device, index, profile)) => {
                if let Some(bytes) = route_pod(index, profile, properties)
                    && let Some(pod) = Pod::from_bytes(&bytes)
                {
                    device.proxy.set_param(ParamType::Route, 0, pod);
                }
            }
            None => {
                if let Some(bytes) = props_pod(properties)
                    && let Some(pod) = Pod::from_bytes(&bytes)
                {
                    entry.proxy.set_param(ParamType::Props, 0, pod);
                }
            }
        }
    });

    let weak_registry = registry.downgrade();
    let global_inner = inner.clone();
    let global_callback = callback.clone();
    let remove_inner = inner.clone();
    let remove_callback = callback.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let Some(registry) = weak_registry.upgrade() else {
                return;
            };
            match global.type_ {
                ObjectType::Node => {
                    let props = global.props;
                    let class = dict(props, "media.class");
                    let direction = match class.as_str() {
                        "Audio/Sink" => Direction::Sink,
                        "Audio/Source" => Direction::Source,
                        _ => return,
                    };
                    let Ok(proxy) = registry.bind::<Node, _>(global) else {
                        return;
                    };
                    let id = global.id;
                    let param_inner = global_inner.clone();
                    let param_callback = global_callback.clone();
                    let info_inner = global_inner.clone();
                    let listener = proxy
                        .add_listener_local()
                        .info(move |info| {
                            let Some(props) = info.props() else {
                                return;
                            };
                            if let Some(entry) = info_inner.borrow_mut().nodes.get_mut(&id) {
                                if let Some(device) =
                                    props.get("device.id").and_then(|v| v.parse().ok())
                                {
                                    entry.device_id = Some(device);
                                }
                                if let Some(profile) = props
                                    .get("card.profile.device")
                                    .and_then(|v| v.parse().ok())
                                {
                                    entry.profile_device = Some(profile);
                                }
                            }
                        })
                        .param(move |_, _, _, _, pod| {
                            let Some(pod) = pod else {
                                return;
                            };
                            let changed = param_inner
                                .borrow_mut()
                                .nodes
                                .get_mut(&id)
                                .is_some_and(|entry| apply_props(entry, pod));
                            if changed {
                                emit(&param_inner, &param_callback);
                            }
                        })
                        .register();
                    proxy.subscribe_params(&[ParamType::Props]);
                    let name = dict(props, "node.name");
                    let description = {
                        let description = dict(props, "node.description");
                        if description.is_empty() {
                            dict(props, "node.nick")
                        } else {
                            description
                        }
                    };
                    let device_id = dict(props, "device.id").parse().ok();
                    let profile_device = dict(props, "card.profile.device").parse().ok();
                    global_inner.borrow_mut().nodes.insert(
                        id,
                        Entry {
                            proxy,
                            _listener: listener,
                            name,
                            description,
                            direction,
                            volumes: Vec::new(),
                            muted: false,
                            has_audio: false,
                            device_id,
                            profile_device,
                        },
                    );
                    emit(&global_inner, &global_callback);
                }
                ObjectType::Device => {
                    if dict(global.props, "media.class") != "Audio/Device" {
                        return;
                    }
                    let Ok(proxy) = registry.bind::<Device, _>(global) else {
                        return;
                    };
                    let id = global.id;
                    let route_inner = global_inner.clone();
                    let listener = proxy
                        .add_listener_local()
                        .param(move |_, _, _, _, pod| {
                            let Some((device, index)) = pod.and_then(parse_route) else {
                                return;
                            };
                            if let Some(entry) = route_inner.borrow_mut().devices.get_mut(&id) {
                                entry.routes.insert(device, index);
                            }
                        })
                        .register();
                    proxy.subscribe_params(&[ParamType::Route]);
                    global_inner.borrow_mut().devices.insert(
                        id,
                        DeviceEntry {
                            proxy,
                            _listener: listener,
                            routes: HashMap::new(),
                        },
                    );
                }
                ObjectType::Metadata => {
                    if dict(global.props, "metadata.name") != "default" {
                        return;
                    }
                    let Ok(metadata) = registry.bind::<Metadata, _>(global) else {
                        return;
                    };
                    let meta_inner = global_inner.clone();
                    let meta_callback = global_callback.clone();
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _, value| {
                            if subject != 0 {
                                return 0;
                            }
                            {
                                let mut inner = meta_inner.borrow_mut();
                                match key {
                                    Some("default.audio.sink") => {
                                        inner.default_sink = default_name(value)
                                    }
                                    Some("default.audio.source") => {
                                        inner.default_source = default_name(value)
                                    }
                                    None => {
                                        inner.default_sink = None;
                                        inner.default_source = None;
                                    }
                                    _ => return 0,
                                }
                            }
                            emit(&meta_inner, &meta_callback);
                            0
                        })
                        .register();
                    global_inner.borrow_mut().metadata = Some((metadata, listener));
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            remove_inner.borrow_mut().devices.remove(&id);
            let removed = remove_inner.borrow_mut().nodes.remove(&id).is_some();
            if removed {
                emit(&remove_inner, &remove_callback);
            }
        })
        .register();

    main_loop.run();
    if let Ok(mut guard) = slot.lock() {
        *guard = None;
    }
    Ok(())
}

pub fn spawn(
    callback: impl Fn(AudioState) + Send + Sync + 'static,
) -> std::io::Result<AudioHandle> {
    let slot = Arc::new(std::sync::Mutex::new(None));
    let handle = AudioHandle {
        sender: slot.clone(),
    };
    let callback: Callback = Arc::new(callback);
    thread::Builder::new()
        .name("pipewire".into())
        .spawn(move || {
            pw::init();
            loop {
                if let Err(err) = run(callback.clone(), &slot) {
                    log::warn!("pipewire: {err}");
                }
                callback(AudioState::default());
                thread::sleep(Duration::from_secs(2));
            }
        })?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_cubic_like_wpctl() {
        assert!((average_volume(&[0.125, 0.125]) - 0.5).abs() < 1e-6);
        assert_eq!(average_volume(&[]), 0.0);
    }

    #[test]
    fn default_metadata_parses_name() {
        assert_eq!(
            default_name(Some("{\"name\":\"alsa_output.pci\"}")),
            Some("alsa_output.pci".to_string())
        );
        assert_eq!(default_name(None), None);
    }

    #[test]
    fn route_round_trip() {
        let bytes = route_pod(3, 1, Vec::new()).expect("serialized");
        let pod = Pod::from_bytes(&bytes).expect("pod");
        assert_eq!(parse_route(pod), Some((1, 3)));
    }

    #[test]
    fn props_round_trip() {
        let bytes = props_pod(vec![Property {
            key: spa_sys::SPA_PROP_mute,
            flags: PropertyFlags::empty(),
            value: Value::Bool(true),
        }])
        .expect("serialized");
        let pod = Pod::from_bytes(&bytes).expect("pod");
        let (_, value) = PodDeserializer::deserialize_any_from(pod.as_bytes()).expect("value");
        let Value::Object(object) = value else {
            panic!("object expected");
        };
        assert_eq!(object.properties[0].value, Value::Bool(true));
    }
}
