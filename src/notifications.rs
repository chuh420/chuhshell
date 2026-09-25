use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;

use crate::app::AppState;
use crate::modules;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Volume,
    Microphone,
    Brightness,
    Keyboard,
    Network,
    Power,
    Peripheral,
}

impl NoticeKind {
    fn icon(self) -> &'static str {
        match self {
            Self::Volume => "󰕾",
            Self::Microphone => "󰍬",
            Self::Brightness => "󰃟",
            Self::Keyboard => "󰌌",
            Self::Network => "󰖩",
            Self::Power => "󰂄",
            Self::Peripheral => "󰂱",
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::Network | Self::Power | Self::Peripheral => Duration::from_secs(4),
            _ => Duration::from_millis(1300),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    pub detail: Option<String>,
    pub progress: Option<u8>,
}

pub struct OsdWidgets {
    icon: gtk::Label,
    title: gtk::Label,
    detail: gtk::Label,
    progress: gtk::ProgressBar,
}

impl OsdWidgets {
    fn update(&self, notice: &Notice) {
        self.icon.set_text(notice.kind.icon());
        self.title.set_text(&notice.title);
        self.detail.set_visible(notice.detail.is_some());
        if let Some(detail) = &notice.detail {
            self.detail.set_text(detail);
        }
        self.progress.set_visible(notice.progress.is_some());
        if let Some(progress) = notice.progress {
            self.progress.set_fraction(f64::from(progress) / 100.0);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionNotice {
    Connected(String),
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerNotice {
    Connected,
    Disconnected,
    ChargingStarted,
    ChargingComplete,
}

pub fn network_transition(
    previous: Option<&(bool, Option<String>)>,
    connected: bool,
    ssid: Option<&str>,
) -> Option<ConnectionNotice> {
    let (previous_connected, previous_ssid) = previous?;
    if !*previous_connected && connected {
        return Some(ConnectionNotice::Connected(
            ssid.unwrap_or("wi-fi").to_owned(),
        ));
    }
    if *previous_connected && !connected {
        return Some(ConnectionNotice::Disconnected);
    }
    if connected && previous_ssid.as_deref() != ssid {
        return Some(ConnectionNotice::Connected(
            ssid.unwrap_or("wi-fi").to_owned(),
        ));
    }
    None
}

pub fn power_transition(
    previous: Option<(bool, bool)>,
    plugged: bool,
    charging: bool,
) -> Option<PowerNotice> {
    let (was_plugged, was_charging) = previous?;
    if was_plugged != plugged {
        return Some(if plugged {
            PowerNotice::Connected
        } else {
            PowerNotice::Disconnected
        });
    }
    if plugged && was_charging != charging {
        return Some(if charging {
            PowerNotice::ChargingStarted
        } else {
            PowerNotice::ChargingComplete
        });
    }
    None
}

pub fn layout_label(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "russian" => "RU".to_owned(),
        "english (us)" => "US".to_owned(),
        _ => name
            .split_whitespace()
            .next_back()
            .unwrap_or(name)
            .to_uppercase(),
    }
}

impl Notice {
    pub fn transient(kind: NoticeKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            detail: None,
            progress: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_progress(mut self, progress: u8) -> Self {
        self.progress = Some(progress.min(100));
        self
    }
}

pub fn show(state: &Rc<AppState>, notice: Notice) {
    if let Some(center) = state
        .notification_center
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
    {
        center.push_internal(&notice);
    }
    let generation = state.osd_generation.get().wrapping_add(1);
    state.osd_generation.set(generation);
    if let Some(timeout) = state.osd_timeout.borrow_mut().take() {
        timeout.remove();
    }
    let window = if let Some(window) = state.osd.borrow().clone() {
        window
    } else {
        let window = gtk::Window::builder()
            .title("chuhshell notification")
            .default_width(320)
            .build();
        window.set_widget_name("notification");
        window.add_css_class("notification-window");
        window.init_layer_shell();
        window.set_namespace(Some("chuhshell-notification"));
        window.set_layer(layer_shell::Layer::Overlay);
        window.set_anchor(layer_shell::Edge::Bottom, true);
        window.set_anchor(layer_shell::Edge::Right, true);
        window.set_anchor(layer_shell::Edge::Top, false);
        window.set_anchor(layer_shell::Edge::Left, false);
        window.set_margin(layer_shell::Edge::Bottom, 48);
        window.set_margin(layer_shell::Edge::Right, 24);
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(layer_shell::KeyboardMode::None);

        *state.osd_widgets.borrow_mut() = Some(build_content(&window));

        let state_weak = Rc::downgrade(state);
        window.connect_close_request(move |_| {
            if let Some(state) = state_weak.upgrade() {
                state.osd.borrow_mut().take();
                state.osd_widgets.borrow_mut().take();
                if let Some(timeout) = state.osd_timeout.borrow_mut().take() {
                    timeout.remove();
                }
            }
            glib::Propagation::Proceed
        });
        *state.osd.borrow_mut() = Some(window.clone());
        window
    };

    if let Some(widgets) = state.osd_widgets.borrow().as_ref() {
        widgets.update(&notice);
    }
    window.present();

    let expected = generation;
    let weak_window = window.downgrade();
    let state_weak = Rc::downgrade(state);
    let timeout = glib::timeout_add_local_once(notice.kind.timeout(), move || {
        if let Some(state) = state_weak.upgrade()
            && state.osd_generation.get() == expected
        {
            state.osd_timeout.borrow_mut().take();
            if let Some(window) = weak_window.upgrade() {
                window.close();
            }
        }
    });
    *state.osd_timeout.borrow_mut() = Some(timeout);
}

fn build_content(window: &impl IsA<gtk::Window>) -> OsdWidgets {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    content.add_css_class("notification-content");
    let icon = gtk::Label::new(None);
    icon.add_css_class("notification-icon");
    content.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 5);
    text.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(None);
    title.set_xalign(0.0);
    title.add_css_class("notification-title");
    text.append(&title);
    let detail = gtk::Label::new(None);
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    detail.add_css_class("notification-detail");
    text.append(&detail);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("notification-progress");
    text.append(&progress);
    content.append(&text);
    window.set_child(Some(&content));
    OsdWidgets {
        icon,
        title,
        detail,
        progress,
    }
}

pub fn handle_command(state: &Rc<AppState>, command: &str) -> bool {
    match command {
        "volume-up" => set_audio(state, "5%+", false),
        "volume-down" => set_audio(state, "5%-", false),
        "volume-mute" => set_audio(state, "toggle", true),
        "microphone-mute" => set_microphone(state),
        "brightness-up" => set_brightness(state, "5%+"),
        "brightness-down" => set_brightness(state, "5%-"),
        "brightness-key-up" => set_brightness(state, "10%+"),
        "brightness-key-down" => set_brightness(state, "10%-"),
        "brightness-scroll-up" => set_brightness(state, "5%+"),
        "brightness-scroll-down" => set_brightness(state, "5%-"),
        _ => return false,
    }
    true
}

fn set_audio(state: &Rc<AppState>, adjustment: &str, mute_toggle: bool) {
    let mut command = std::process::Command::new("wpctl");
    if mute_toggle {
        command.args(["set-mute", "@DEFAULT_AUDIO_SINK@", adjustment]);
    } else {
        command.args([
            "set-volume",
            "-l",
            "1.0",
            "@DEFAULT_AUDIO_SINK@",
            adjustment,
        ]);
    }
    if command.status().is_ok_and(|status| status.success())
        && let Some(value) =
            modules::child_process("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
        && let Some((percent, muted)) = parse_wpctl_volume(&value)
    {
        let notice = Notice::transient(
            NoticeKind::Volume,
            if muted {
                format!("Volume muted · {percent}%")
            } else {
                format!("Volume · {percent}%")
            },
        )
        .with_progress(if muted { 0 } else { percent });
        show(state, notice);
    }
}

fn set_microphone(state: &Rc<AppState>) {
    if std::process::Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
        .status()
        .is_ok_and(|status| status.success())
        && let Some(value) =
            modules::child_process("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
        && let Some((percent, muted)) = parse_wpctl_volume(&value)
    {
        show(
            state,
            Notice::transient(
                NoticeKind::Microphone,
                if muted {
                    "Microphone muted"
                } else {
                    "Microphone on"
                },
            )
            .with_detail(format!("{percent}%")),
        );
    }
}

fn set_brightness(state: &Rc<AppState>, adjustment: &str) {
    let Some(device) = modules::backlight_device() else {
        return;
    };
    if std::process::Command::new("brightnessctl")
        .args(["-d", &device, "set", adjustment])
        .status()
        .is_ok_and(|status| status.success())
        && let Some((percent, _)) = modules::brightness_level(&device)
    {
        show(
            state,
            Notice::transient(NoticeKind::Brightness, format!("Brightness · {percent}%"))
                .with_progress(percent),
        );
    }
}

fn parse_wpctl_volume(value: &str) -> Option<(u8, bool)> {
    let volume = value.split_whitespace().nth(1)?.parse::<f32>().ok()?;
    Some((
        (volume * 100.0).round().clamp(0.0, 100.0) as u8,
        value.contains("MUTED"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_clamped() {
        assert_eq!(
            Notice::transient(NoticeKind::Volume, "volume")
                .with_progress(140)
                .progress,
            Some(100)
        );
    }

    #[test]
    fn persistent_events_have_longer_timeout_than_osd_values() {
        assert!(NoticeKind::Network.timeout() > NoticeKind::Volume.timeout());
        assert!(NoticeKind::Power.timeout() > NoticeKind::Brightness.timeout());
    }

    #[test]
    fn parses_wpctl_volume_and_mute_state() {
        assert_eq!(parse_wpctl_volume("Volume: 0.48"), Some((48, false)));
        assert_eq!(parse_wpctl_volume("Volume: 0.75 [MUTED]"), Some((75, true)));
        assert_eq!(parse_wpctl_volume("not available"), None);
    }

    #[test]
    fn initial_connection_state_does_not_emit_notification() {
        assert_eq!(network_transition(None, true, Some("home")), None);
        assert_eq!(power_transition(None, true, true), None);
    }

    #[test]
    fn network_transition_reports_connect_disconnect_and_network_change() {
        assert_eq!(
            network_transition(Some(&(false, None)), true, Some("Home WiFi")),
            Some(ConnectionNotice::Connected("Home WiFi".to_owned()))
        );
        assert_eq!(
            network_transition(Some(&(true, Some("Home WiFi".to_owned()))), false, None),
            Some(ConnectionNotice::Disconnected)
        );
        assert_eq!(
            network_transition(Some(&(true, Some("Old".to_owned()))), true, Some("New")),
            Some(ConnectionNotice::Connected("New".to_owned()))
        );
    }

    #[test]
    fn power_transition_reports_adapter_and_charging_edges() {
        assert_eq!(
            power_transition(Some((false, false)), true, true),
            Some(PowerNotice::Connected)
        );
        assert_eq!(
            power_transition(Some((true, true)), false, false),
            Some(PowerNotice::Disconnected)
        );
        assert_eq!(
            power_transition(Some((true, false)), true, true),
            Some(PowerNotice::ChargingStarted)
        );
        assert_eq!(
            power_transition(Some((true, true)), true, false),
            Some(PowerNotice::ChargingComplete)
        );
    }

    #[test]
    fn keyboard_layout_label_is_short_and_readable() {
        assert_eq!(layout_label("Russian"), "RU");
        assert_eq!(layout_label("English (US)"), "US");
        assert_eq!(layout_label("German"), "GERMAN");
    }
}
