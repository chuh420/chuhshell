pub mod bluetooth;
pub mod clipboard;
pub mod control;
pub mod kit;
pub mod launcher;
pub mod notifications;
pub mod picker;
pub mod tray;
pub mod wallpapers;
pub mod wifi;

use std::{collections::HashMap, sync::Arc};

use services::notifications::Notification;

use crate::card::CardAction;

use ui::{NodeId, input::AreaId};

#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
    Toggle(&'static str),
    Run(&'static str),
    Suspend,
    ToggleMute,
    SetVolume(f64),
    SetBrightness(f64),
    ToggleKeepAwake,
    BluetoothPower,
    Close,
    ToggleDnd,
    ClearAll,
    Card(CardAction),
    TrayExpanded(bool),
    TrayLoadMenu,
    TrayActivate,
    TrayQuit,
    TrayEvent(i32, &'static str),
    Net(NetAction),
    Launch(String),
    ToggleHidden(String),
    Search(String),
    ClipCopy(String),
    ClipDelete(String),
    ClipWipe,
    WallPreviews(Vec<crate::wallctl::PreviewKey>),
    WallApply(String, f32, f32),
    WallRefresh,
}

#[derive(Clone, Debug)]
pub enum AppIcon {
    Letter,
    Image(Option<Arc<ui::tiny_skia::Pixmap>>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NetAction {
    Airplane(bool),
    WifiPower(bool),
    Scan,
    Connect(String),
    Disconnect,
    Forget(String),
    Autoconnect(String, bool),
    ConnectHidden(String),
    Password(String),
    Cancel,
    BtPower(bool),
    BtScan(bool),
    BtPair(String),
    BtDevice(String, &'static str),
    BtTrust(String, bool),
    BtRename(String, String),
    BtForget(String),
    BtProfile(i64, i64),
    BtReply(bool, String),
}

pub struct TooltipSpot {
    pub key: AreaId,
    pub text: String,
    pub node: NodeId,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelData {
    pub time: String,
    pub date: String,
    pub network_connected: bool,
    pub network_name: String,
    pub bluetooth_powered: bool,
    pub bluetooth_name: String,
    pub muted: bool,
    pub volume: u32,
    pub brightness: u32,
    pub keep_awake: bool,
    pub dnd: bool,
    pub notifications: Vec<Notification>,
    pub ages: HashMap<u32, String>,
    pub tray_title: String,
    pub tray_only_menu: bool,
    pub tray_serial: u64,
    pub tray_menu: Option<Vec<services::tray::MenuEntry>>,
    pub wifi: services::network::WifiState,
    pub wifi_details: Option<services::network::WifiDetails>,
    pub wifi_down: Vec<f64>,
    pub wifi_up: Vec<f64>,
    pub wifi_prompt: String,
    pub bluetooth: services::network::BluetoothState,
    pub bluetooth_profiles: HashMap<String, services::network::DeviceProfiles>,
    pub bluetooth_prompt: services::network::BluetoothPrompt,
    pub bluetooth_error: String,
    pub apps: Arc<Vec<services::apps::AppEntry>>,
    pub hidden: Vec<String>,
    pub clipboard: Option<Arc<Vec<services::clipboard::ClipEntry>>>,
    pub wall: Arc<crate::wallctl::WallData>,
    pub previews: Arc<HashMap<crate::wallctl::PreviewKey, String>>,
}
