use std::path::PathBuf;

pub struct BarModel {
    pub panel: String,
    pub title: Option<String>,
    pub workspaces: Vec<i32>,
    pub active_workspace: i32,
    pub clock: String,
    pub cpu: Option<u32>,
    pub ram_used: f64,
    pub ram_total: f64,
    pub keyboard_label: String,
    pub network_connected: bool,
    pub wifi_connected: bool,
    pub wifi_strength: u32,
    pub bluetooth_powered: bool,
    pub bluetooth_connected: Vec<(String, i32)>,
    pub muted: bool,
    pub volume: u32,
    pub battery_present: bool,
    pub battery_level: u32,
    pub battery_plugged: bool,
    pub hotkeys_paused: bool,
    pub dnd: bool,
    pub unread_notifications: u32,
}

impl Default for BarModel {
    fn default() -> Self {
        Self {
            panel: String::new(),
            title: None,
            workspaces: vec![1],
            active_workspace: 1,
            clock: format_clock(&jiff::Zoned::now()),
            cpu: None,
            ram_used: 0.0,
            ram_total: 0.0,
            keyboard_label: String::new(),
            network_connected: false,
            wifi_connected: false,
            wifi_strength: 0,
            bluetooth_powered: false,
            bluetooth_connected: Vec::new(),
            muted: false,
            volume: 0,
            battery_present: false,
            battery_level: 0,
            battery_plugged: false,
            hotkeys_paused: false,
            dnd: false,
            unread_notifications: 0,
        }
    }
}

impl BarModel {
    pub fn placeholder() -> Self {
        Self {
            keyboard_label: "RU".into(),
            network_connected: true,
            wifi_connected: true,
            wifi_strength: 100,
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
    time.strftime("%a, %-d %b  ·  %H:%M").to_string()
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
        s if s > 75 => "󰤨",
        s if s > 50 => "󰤥",
        s if s > 25 => "󰤢",
        _ => "󰤟",
    }
}

pub fn rice_path(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config/rice").join(name)
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
