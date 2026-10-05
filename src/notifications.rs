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
    Clipboard,
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
            Self::Clipboard => "󰅍",
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
    ChargingStopped,
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
    previous: Option<(bool, crate::modules::BatteryState)>,
    plugged: bool,
    state: crate::modules::BatteryState,
) -> Option<PowerNotice> {
    let (was_plugged, previous_state) = previous?;
    if was_plugged != plugged {
        return Some(if plugged {
            PowerNotice::Connected
        } else {
            PowerNotice::Disconnected
        });
    }
    use crate::modules::BatteryState;
    if plugged && previous_state != state {
        if state == BatteryState::Charging {
            return Some(PowerNotice::ChargingStarted);
        }
        if previous_state == BatteryState::Charging {
            return Some(if state == BatteryState::Full {
                PowerNotice::ChargingComplete
            } else {
                PowerNotice::ChargingStopped
            });
        }
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
        crate::ui::animate_close(&window);
        *state.osd.borrow_mut() = Some(window.clone());
        window
    };

    if let Some(widgets) = state.osd_widgets.borrow().as_ref() {
        widgets.update(&notice);
    }
    window.set_monitor(crate::ui::active_monitor().as_ref());
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

pub struct CommandRequest {
    command: String,
    reply: async_channel::Sender<Result<Notice, String>>,
}

pub fn is_command(command: &str) -> bool {
    matches!(
        command,
        "volume-up"
            | "volume-down"
            | "volume-mute"
            | "microphone-mute"
            | "brightness-up"
            | "brightness-down"
            | "brightness-key-up"
            | "brightness-key-down"
            | "brightness-scroll-up"
            | "brightness-scroll-down"
    )
}

pub(crate) enum Work {
    Notification(CommandRequest),
    Control(
        crate::controls::Action,
        async_channel::Sender<(
            crate::controls::Kind,
            Result<crate::controls::Applied, String>,
        )>,
    ),
}

fn work_batch(rx: &async_channel::Receiver<Work>, pending: &mut Option<Work>) -> Option<Vec<Work>> {
    let first = pending.take().or_else(|| rx.recv_blocking().ok())?;
    if matches!(&first, Work::Control(..)) {
        std::thread::sleep(Duration::from_millis(60));
    }
    let mut batch = vec![first];
    while batch.len() < 32 {
        let Ok(next) = rx.try_recv() else {
            break;
        };
        let merge = match (&batch[0], &next) {
            (Work::Control(left, _), Work::Control(right, _)) => left.kind() == right.kind(),
            (Work::Notification(left), Work::Notification(right)) => {
                adjustment(&left.command).is_some() && left.command == right.command
            }
            _ => false,
        };
        if merge {
            batch.push(next);
        } else {
            *pending = Some(next);
            break;
        }
    }
    Some(batch)
}

pub(crate) fn worker() -> async_channel::Sender<Work> {
    static WORKER: std::sync::OnceLock<async_channel::Sender<Work>> = std::sync::OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (tx, rx) = async_channel::bounded::<Work>(32);
            std::thread::spawn(move || {
                let mut pending = None;
                while let Some(batch) = work_batch(&rx, &mut pending) {
                    if crate::process::stopped() {
                        break;
                    }
                    match batch.last().unwrap() {
                        Work::Control(action, _) => {
                            let kind = action.kind();
                            let result = crate::controls::execute(action.clone());
                            for request in batch {
                                if let Work::Control(_, reply) = request {
                                    let _ = reply.send_blocking((kind, result.clone()));
                                }
                            }
                        }
                        Work::Notification(request) => {
                            let result = execute(&request.command, batch.len() as u32);
                            for request in batch {
                                if let Work::Notification(request) = request {
                                    let _ = request.reply.try_send(result.clone());
                                }
                            }
                        }
                    }
                }
            });
            tx
        })
        .clone()
}

pub fn submit(
    _state: &Rc<AppState>,
    command: &str,
) -> async_channel::Receiver<Result<Notice, String>> {
    let (tx, rx) = async_channel::bounded(1);
    let request = Work::Notification(CommandRequest {
        command: command.into(),
        reply: tx.clone(),
    });
    if worker().try_send(request).is_err() {
        let _ = tx.try_send(Err("Too many pending system commands".into()));
    }
    rx
}

pub fn handle_command(state: &Rc<AppState>, command: &str) -> bool {
    if !is_command(command) {
        return false;
    }
    let rx = submit(state, command);
    let state = Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move {
        if let Ok(result) = rx.recv().await
            && let Some(state) = state.upgrade()
        {
            match result {
                Ok(notice) => show(&state, notice),
                Err(error) => {
                    eprintln!("chuhshell: {error}");
                    show(
                        &state,
                        Notice::transient(NoticeKind::Peripheral, "Action failed")
                            .with_detail(error),
                    );
                }
            }
        }
    });
    true
}

fn adjustment(command: &str) -> Option<(u32, char)> {
    match command {
        "volume-up" | "brightness-up" | "brightness-scroll-up" => Some((5, '+')),
        "volume-down" | "brightness-down" | "brightness-scroll-down" => Some((5, '-')),
        "brightness-key-up" => Some((10, '+')),
        "brightness-key-down" => Some((10, '-')),
        _ => None,
    }
}
fn adjustment_value(command: &str, count: u32) -> Option<String> {
    adjustment(command)
        .map(|(step, direction)| format!("{}%{direction}", step.saturating_mul(count)))
}

fn execute(command: &str, count: u32) -> Result<Notice, String> {
    use crate::process::run;
    match command {
        "volume-up" | "volume-down" | "volume-mute" => {
            if command == "volume-mute" {
                run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])?;
            } else {
                run(
                    "wpctl",
                    &[
                        "set-volume",
                        "-l",
                        "1.0",
                        "@DEFAULT_AUDIO_SINK@",
                        &adjustment_value(command, count).ok_or("Invalid volume command")?,
                    ],
                )?;
            }
            let value = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
            let (percent, muted) = parse_wpctl_volume(&value).ok_or("Invalid audio response")?;
            Ok(Notice::transient(
                NoticeKind::Volume,
                if muted {
                    format!("Volume muted · {percent}%")
                } else {
                    format!("Volume · {percent}%")
                },
            )
            .with_progress(if muted { 0 } else { percent }))
        }
        "microphone-mute" => {
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])?;
            let value = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"])?;
            let (percent, muted) =
                parse_wpctl_volume(&value).ok_or("Invalid microphone response")?;
            Ok(Notice::transient(
                NoticeKind::Microphone,
                if muted {
                    "Microphone muted"
                } else {
                    "Microphone on"
                },
            )
            .with_detail(format!("{percent}%")))
        }
        _ if is_command(command) => {
            let device = modules::backlight_device().ok_or("Backlight unavailable")?;
            let adjustment =
                adjustment_value(command, count).ok_or("Invalid brightness command")?;
            run("brightnessctl", &["-d", &device, "set", &adjustment])?;
            let (percent, _) =
                modules::brightness_level(&device).ok_or("Could not read brightness")?;
            Ok(
                Notice::transient(NoticeKind::Brightness, format!("Brightness · {percent}%"))
                    .with_progress(percent),
            )
        }
        _ => Err("Unknown system command".into()),
    }
}

pub(crate) fn parse_wpctl_volume(value: &str) -> Option<(u8, bool)> {
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
    fn repeated_adjustments_merge_without_crossing_toggles_or_direction_changes() {
        let (tx, rx) = async_channel::bounded(32);
        for command in [
            "volume-up",
            "volume-up",
            "volume-mute",
            "volume-mute",
            "volume-down",
            "brightness-key-up",
            "brightness-key-up",
        ] {
            let (reply, _) = async_channel::bounded(1);
            tx.try_send(Work::Notification(CommandRequest {
                command: command.into(),
                reply,
            }))
            .unwrap();
        }
        drop(tx);
        let mut pending = None;
        let mut batches = Vec::new();
        while let Some(batch) = work_batch(&rx, &mut pending) {
            let Work::Notification(first) = &batch[0] else {
                panic!("Expected notification");
            };
            batches.push((first.command.clone(), batch.len()));
        }
        assert_eq!(
            batches,
            vec![
                ("volume-up".into(), 2),
                ("volume-mute".into(), 1),
                ("volume-mute".into(), 1),
                ("volume-down".into(), 1),
                ("brightness-key-up".into(), 2)
            ]
        );
        assert_eq!(
            adjustment_value("brightness-key-up", 2).as_deref(),
            Some("20%+")
        );
        assert_eq!(
            adjustment_value("volume-down", 32).as_deref(),
            Some("160%-")
        );
    }

    #[test]
    fn sliders_merge_only_before_the_next_command_boundary() {
        use crate::controls::Action;
        let (tx, rx) = async_channel::bounded(32);
        let (reply, _) = async_channel::bounded(32);
        for action in [Action::Volume(40), Action::Volume(55)] {
            tx.try_send(Work::Control(action, reply.clone())).unwrap();
        }
        let (notice, _) = async_channel::bounded(1);
        tx.try_send(Work::Notification(CommandRequest {
            command: "volume-mute".into(),
            reply: notice,
        }))
        .unwrap();
        tx.try_send(Work::Control(Action::Volume(25), reply.clone()))
            .unwrap();
        tx.try_send(Work::Control(Action::Brightness(70), reply))
            .unwrap();
        drop(tx);
        let mut pending = None;
        let first = work_batch(&rx, &mut pending).unwrap();
        assert_eq!(first.len(), 2);
        let Work::Control(action, _) = first.last().unwrap() else {
            panic!("Expected slider");
        };
        assert_eq!(*action, Action::Volume(55));
        let mute = work_batch(&rx, &mut pending).unwrap();
        assert!(
            matches!(&mute[0], Work::Notification(request) if request.command == "volume-mute")
        );
        let volume = work_batch(&rx, &mut pending).unwrap();
        assert!(matches!(&volume[0], Work::Control(Action::Volume(25), _)));
        let brightness = work_batch(&rx, &mut pending).unwrap();
        assert!(matches!(
            &brightness[0],
            Work::Control(Action::Brightness(70), _)
        ));
        assert!(work_batch(&rx, &mut pending).is_none());
    }

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
        assert_eq!(
            power_transition(None, true, crate::modules::BatteryState::Charging),
            None
        );
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
        use crate::modules::BatteryState::{Charging, Discharging, Full, NotCharging};
        assert_eq!(
            power_transition(Some((false, Discharging)), true, Charging),
            Some(PowerNotice::Connected)
        );
        assert_eq!(
            power_transition(Some((true, Charging)), false, Discharging),
            Some(PowerNotice::Disconnected)
        );
        assert_eq!(
            power_transition(Some((true, NotCharging)), true, Charging),
            Some(PowerNotice::ChargingStarted)
        );
        assert_eq!(
            power_transition(Some((true, Charging)), true, Full),
            Some(PowerNotice::ChargingComplete)
        );
        assert_eq!(
            power_transition(Some((true, Charging)), true, NotCharging),
            Some(PowerNotice::ChargingStopped)
        );
        assert_eq!(
            power_transition(Some((true, NotCharging)), true, Full),
            None
        );
    }

    #[test]
    fn keyboard_layout_label_is_short_and_readable() {
        assert_eq!(layout_label("Russian"), "RU");
        assert_eq!(layout_label("English (US)"), "US");
        assert_eq!(layout_label("German"), "GERMAN");
    }
}
