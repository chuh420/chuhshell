use services::{audio::AudioState, system::Wired, upower::PowerState};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyboardLayout {
    pub label: String,
    pub name: String,
    pub device: String,
}

pub struct ShellState {
    pub panel: String,
    pub panel_screen: String,
    pub panel_anchor: f64,
    pub keep_awake: bool,
    pub workspace_occupied: bool,
    pub dnd: bool,
    pub hotkeys_paused: bool,
    pub osd_kind: String,
    pub osd_text: String,
    pub osd_serial: u64,
    pub audio: AudioState,
    pub power: PowerState,
    pub brightness: u32,
    pub wanted_brightness: Option<u32>,
    pub wired: Wired,
    pub keyboard: KeyboardLayout,
    pub keyboard_initialized: bool,
    pub focused_monitor: Option<String>,
    pub battery_alerts: Vec<u32>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            panel: String::new(),
            panel_screen: String::new(),
            panel_anchor: 0.0,
            keep_awake: false,
            workspace_occupied: false,
            dnd: false,
            hotkeys_paused: false,
            osd_kind: String::new(),
            osd_text: String::new(),
            osd_serial: 0,
            audio: AudioState::default(),
            power: PowerState::default(),
            brightness: 50,
            wanted_brightness: None,
            wired: Wired {
                network: "Offline".into(),
                connected: false,
            },
            keyboard: KeyboardLayout {
                label: "…".into(),
                name: String::new(),
                device: String::new(),
            },
            keyboard_initialized: false,
            focused_monitor: None,
            battery_alerts: Vec::new(),
        }
    }
}

pub fn round_js(value: f64) -> f64 {
    (value + 0.5).floor()
}

impl ShellState {
    pub fn volume(&self) -> u32 {
        self.audio
            .sink()
            .filter(|sink| sink.has_audio)
            .map_or(0, |sink| round_js(sink.volume as f64 * 100.0) as u32)
    }

    pub fn muted(&self) -> bool {
        self.audio
            .sink()
            .filter(|sink| sink.has_audio)
            .is_none_or(|sink| sink.muted)
    }

    pub fn mic_muted(&self) -> bool {
        self.audio
            .source()
            .filter(|source| source.has_audio)
            .is_none_or(|source| source.muted)
    }

    pub fn battery_present(&self) -> bool {
        self.power
            .battery
            .as_ref()
            .is_some_and(|battery| battery.present)
    }

    pub fn battery_level(&self) -> u32 {
        self.power
            .battery
            .as_ref()
            .map_or(0, |battery| round_js(battery.percentage) as u32)
    }

    pub fn battery_plugged(&self) -> bool {
        self.battery_present() && !self.power.on_battery
    }

    pub fn battery_power(&self) -> f64 {
        self.power
            .battery
            .as_ref()
            .map_or(0.0, |battery| battery.energy_rate.abs())
    }

    pub fn open_at(&mut self, which: &str, screen: &str) {
        self.panel_screen = screen.to_string();
        self.panel_anchor = 0.0;
        self.toggle(which, true);
    }

    pub fn open_module(&mut self, which: &str, x: f64, screen: &str) {
        self.panel_screen = screen.to_string();
        self.panel_anchor = x;
        self.toggle(which, true);
    }

    pub fn toggle(&mut self, which: &str, preserve_screen: bool) {
        if !preserve_screen {
            self.panel_screen = self.focused_monitor.clone().unwrap_or_default();
            self.panel_anchor = 0.0;
        }
        self.panel = if self.panel == which {
            String::new()
        } else {
            which.to_string()
        };
    }

    pub fn show_osd(&mut self, kind: &str) {
        if kind != "keyboard" && ["control", "audio", "battery"].contains(&self.panel.as_str()) {
            return;
        }
        self.osd_kind = kind.to_string();
        self.osd_serial += 1;
    }

    pub fn show_osd_text(&mut self, kind: &str, text: &str) {
        if kind == "hotkeys" {
            self.hotkeys_paused = text == "paused";
        }
        self.osd_text = text.to_string();
        self.osd_kind = kind.to_string();
        self.osd_serial += 1;
    }

    pub fn battery_alert(&mut self) -> Option<BatteryAlert> {
        let level = self.battery_level();
        if !self.battery_present() || self.battery_plugged() || level == 0 {
            return None;
        }
        if level <= 3 && !self.battery_alerts.contains(&3) {
            self.battery_alerts.push(3);
            return Some(BatteryAlert::Suspend);
        }
        let crossed = [5, 10, 20].into_iter().find(|limit| level <= *limit)?;
        if self.battery_alerts.contains(&crossed) {
            return None;
        }
        self.battery_alerts
            .extend([5, 10, 20].into_iter().filter(|limit| *limit >= crossed));
        Some(BatteryAlert::Notify {
            critical: crossed == 5,
            level,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatteryAlert {
    Suspend,
    Notify { critical: bool, level: u32 },
}

pub fn osd_icon(state: &ShellState) -> &'static str {
    match state.osd_kind.as_str() {
        "volume" => {
            let volume = state.volume();
            if state.muted() {
                "󰖁"
            } else if volume > 66 {
                "󰕾"
            } else if volume > 33 {
                "󰖀"
            } else {
                "󰕿"
            }
        }
        "brightness" => {
            if state.brightness > 66 {
                "󰃠"
            } else if state.brightness > 33 {
                "󰃟"
            } else {
                "󰃞"
            }
        }
        "mic" => {
            if state.mic_muted() {
                "󰍭"
            } else {
                "󰍬"
            }
        }
        "clipboard" => "󰅍",
        "layout" => "󰕰",
        "hotkeys" if state.hotkeys_paused => "󰌐",
        _ => "󰌌",
    }
}

pub fn osd_caption(state: &ShellState) -> String {
    match state.osd_kind.as_str() {
        "volume" => {
            if state.muted() {
                "muted".into()
            } else {
                format!("{}%", state.volume())
            }
        }
        "brightness" => format!("{}%", state.brightness),
        "mic" => if state.mic_muted() {
            "microphone muted"
        } else {
            "microphone on"
        }
        .into(),
        "clipboard" | "layout" => state.osd_text.clone(),
        "hotkeys" => if state.hotkeys_paused {
            "hotkeys paused · super + esc"
        } else {
            "hotkeys active"
        }
        .into(),
        _ => {
            if state.keyboard.name.is_empty() {
                state.keyboard.label.clone()
            } else {
                state.keyboard.name.clone()
            }
        }
    }
}

pub fn osd_level(state: &ShellState) -> u32 {
    match state.osd_kind.as_str() {
        "volume" => {
            if state.muted() {
                0
            } else {
                state.volume()
            }
        }
        "brightness" => state.brightness,
        _ => 0,
    }
}

pub fn osd_has_bar(state: &ShellState) -> bool {
    matches!(state.osd_kind.as_str(), "volume" | "brightness")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_battery(level: f64) -> ShellState {
        ShellState {
            power: PowerState {
                battery: Some(services::upower::Battery {
                    present: true,
                    percentage: level,
                    ..Default::default()
                }),
                on_battery: true,
            },
            ..ShellState::default()
        }
    }

    #[test]
    fn battery_alerts_follow_qml() {
        let mut state = on_battery(19.0);
        assert_eq!(
            state.battery_alert(),
            Some(BatteryAlert::Notify {
                critical: false,
                level: 19
            })
        );
        assert_eq!(state.battery_alert(), None);
        state = ShellState {
            battery_alerts: state.battery_alerts.clone(),
            ..on_battery(5.0)
        };
        assert_eq!(
            state.battery_alert(),
            Some(BatteryAlert::Notify {
                critical: true,
                level: 5
            })
        );
        state = ShellState {
            battery_alerts: state.battery_alerts.clone(),
            ..on_battery(3.0)
        };
        assert_eq!(state.battery_alert(), Some(BatteryAlert::Suspend));
    }

    #[test]
    fn osd_is_suppressed_over_panels_except_keyboard() {
        let mut state = ShellState {
            panel: "audio".into(),
            ..ShellState::default()
        };
        state.show_osd("volume");
        assert_eq!(state.osd_serial, 0);
        state.show_osd("keyboard");
        assert_eq!(state.osd_serial, 1);
    }

    #[test]
    fn toggle_closes_same_panel() {
        let mut state = ShellState::default();
        state.toggle("launcher", false);
        assert_eq!(state.panel, "launcher");
        state.toggle("launcher", false);
        assert_eq!(state.panel, "");
    }

    #[test]
    fn js_round_halves_up() {
        assert_eq!(round_js(2.5), 3.0);
        assert_eq!(round_js(-0.5), 0.0);
    }
}
