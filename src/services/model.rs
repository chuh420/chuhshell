use crate::{
    modules, niri,
    service_state::{AudioState, ServiceState},
};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SystemState {
    pub niri: niri::Snapshot,
    pub audio: ServiceState<AudioState>,
    pub network: Option<modules::NetworkInfo>,
    pub temperature: ServiceState<i64>,
    pub brightness: ServiceState<(u8, &'static str)>,
    pub battery: modules::BatteryStatus,
}

#[derive(Clone, Copy, Default)]
pub struct Changes {
    pub workspaces: bool,
    pub layouts: bool,
    pub audio: bool,
    pub network: bool,
    pub temperature: bool,
    pub brightness: bool,
    pub battery: bool,
}

impl Changes {
    pub(super) fn between(previous: &SystemState, next: &SystemState) -> Self {
        Self {
            workspaces: previous.niri.workspaces != next.niri.workspaces,
            layouts: previous.niri.layouts != next.niri.layouts,
            audio: previous.audio != next.audio,
            network: previous.network != next.network,
            temperature: previous.temperature != next.temperature,
            brightness: previous.brightness != next.brightness,
            battery: previous.battery != next.battery,
        }
    }
    pub(super) fn all() -> Self {
        Self {
            workspaces: true,
            layouts: true,
            audio: true,
            network: true,
            temperature: true,
            brightness: true,
            battery: true,
        }
    }
}
