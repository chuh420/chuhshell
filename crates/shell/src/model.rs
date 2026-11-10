#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Availability {
    pub wifi: bool,
    pub bluetooth: bool,
    pub brightness: bool,
    pub clipboard: bool,
    pub screenshot: bool,
    pub profiles: bool,
    pub vpn: bool,
}

impl Default for Availability {
    fn default() -> Self {
        Self {
            wifi: true,
            bluetooth: true,
            brightness: true,
            clipboard: true,
            screenshot: true,
            profiles: true,
            vpn: true,
        }
    }
}

impl Availability {
    pub fn panel(&self, name: &str) -> bool {
        match name {
            "wifi" => self.wifi,
            "bluetooth" => self.bluetooth,
            "clipboard" => self.clipboard,
            "vpn" => self.vpn,
            _ => true,
        }
    }
}

pub struct BarModel {
    pub panel: String,
    pub title: Option<String>,
    pub title_key: String,
    pub workspaces: Vec<i32>,
    pub occupied: Vec<i32>,
    pub active_workspace: i32,
    pub clock: String,
    pub cpu: Option<u32>,
    pub ram_used: f64,
    pub ram_total: f64,
    pub keyboard_label: String,
    pub available: Availability,
    pub network_connected: bool,
    pub wifi_connected: bool,
    pub wifi_strength: f64,
    pub bluetooth_powered: bool,
    pub bluetooth_connected: Vec<(String, i32)>,
    pub vpn: crate::vpnctl::VpnState,
    pub vpn_tooltip: String,
    pub muted: bool,
    pub volume: u32,
    pub battery_present: bool,
    pub battery_level: u32,
    pub battery_plugged: bool,
    pub hotkeys_paused: bool,
    pub dnd: bool,
    pub unread_notifications: u32,
    pub tray: Vec<(String, Option<std::sync::Arc<ui::tiny_skia::Pixmap>>)>,
}

impl Default for BarModel {
    fn default() -> Self {
        Self {
            panel: String::new(),
            title: None,
            title_key: "none".into(),
            workspaces: Vec::new(),
            occupied: Vec::new(),
            active_workspace: 1,
            clock: format_clock(&jiff::Zoned::now()),
            cpu: None,
            ram_used: 0.0,
            ram_total: 0.0,
            keyboard_label: "…".into(),
            available: Availability::default(),
            network_connected: false,
            wifi_connected: false,
            wifi_strength: 0.0,
            bluetooth_powered: false,
            bluetooth_connected: Vec::new(),
            vpn: crate::vpnctl::VpnState::Off,
            vpn_tooltip: String::new(),
            muted: false,
            volume: 0,
            battery_present: false,
            battery_level: 0,
            battery_plugged: false,
            hotkeys_paused: false,
            dnd: false,
            unread_notifications: 0,
            tray: Vec::new(),
        }
    }
}

impl BarModel {
    pub fn placeholder() -> Self {
        Self {
            network_connected: true,
            wifi_connected: true,
            wifi_strength: 100.0,
            bluetooth_powered: true,
            volume: 50,
            battery_present: true,
            battery_level: 100,
            battery_plugged: true,
            ..Self::default()
        }
    }
}

pub fn format_clock(time: &jiff::Zoned) -> String {
    format_clock_with(time, &config::current().general.clock_format)
}

pub fn format_clock_with(time: &jiff::Zoned, format: &str) -> String {
    jiff::fmt::strtime::format(format, time)
        .or_else(|_| jiff::fmt::strtime::format(config::DEFAULT_CLOCK, time))
        .unwrap_or_default()
}

pub fn battery_icon(level: u32, plugged: bool) -> &'static str {
    const DISCHARGE: [&str; 11] = ["󰂎", "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰂀", "󰂁", "󰂂", "󰁹"];
    const CHARGE: [&str; 11] = ["󰢟", "󰢜", "󰂆", "󰂇", "󰂈", "󰢝", "󰂉", "󰢞", "󰂊", "󰂋", "󰂅"];
    let index = ((level as f64 / 10.0).round() as usize).min(10);
    if plugged {
        CHARGE[index]
    } else {
        DISCHARGE[index]
    }
}

pub fn network_icon(model: &BarModel) -> &'static str {
    if !model.network_connected {
        return "󰤭";
    }
    if !model.wifi_connected {
        return "󰈀";
    }
    match model.wifi_strength {
        s if s > 75.0 => "󰤨",
        s if s > 50.0 => "󰤥",
        s if s > 25.0 => "󰤢",
        _ => "󰤟",
    }
}

pub fn bluetooth_shortcut(devices: &[(String, i32)]) -> String {
    devices
        .iter()
        .map(|(name, battery)| {
            if *battery >= 0 {
                format!("{name} · {battery}%")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_matches_qml_format() {
        let time: jiff::Zoned = "2026-10-31T14:05:00+03:00[Europe/Moscow]"
            .parse()
            .expect("valid time");
        assert_eq!(format_clock(&time), "Sat, 31 Oct  ·  14:05");
        assert_eq!(format_clock_with(&time, "%H:%M:%S"), "14:05:00");
        assert_eq!(format_clock_with(&time, "%H:%"), "Sat, 31 Oct  ·  14:05");
    }

    #[test]
    fn bluetooth_shortcut_lists_connected_devices() {
        let devices = vec![("Buds".to_string(), 80), ("Mouse".to_string(), -1)];
        assert_eq!(bluetooth_shortcut(&devices), "Buds · 80%\nMouse");
    }

    #[test]
    fn battery_icons_round_like_qml() {
        assert_eq!(battery_icon(100, true), "󰂅");
        assert_eq!(battery_icon(4, false), "󰂎");
        assert_eq!(battery_icon(5, false), "󰁺");
    }
}
