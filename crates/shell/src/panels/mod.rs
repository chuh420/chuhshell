pub mod control;

use ui::{NodeId, input::AreaId};

#[derive(Clone, Copy, Debug, PartialEq)]
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
}
