use std::{
    collections::HashMap,
    os::unix::net::UnixListener,
    process::Command,
    time::{Duration, Instant},
};

use calloop::{
    Interest, LoopHandle, Mode, PostAction,
    channel::Sender,
    futures::Scheduler,
    generic::Generic,
    timer::{TimeoutAction, TimeoutFuture, Timer},
};
use platform::{
    Damage, Edges, Event, InputRegion, KeyEvent, KeyboardMode, Layer, LayerConfig, OutputId,
    Platform, PointerEvent, PointerKind, SurfaceId,
};
use services::{
    audio::{self, AudioHandle, AudioState},
    clock, hyprland,
    system::{self, ResourceMonitor},
    upower::{self, PowerState},
};
use ui::{Canvas, TextEngine, anim::Millis};

use crate::{
    bar::{Bar, BarAction},
    history::History,
    ipc::{self, Reply},
    model::{BarModel, format_clock},
    osd::{self, Osd},
    overlay::{self, Overlay},
    panels::{PanelAction, PanelData},
    state::{BatteryAlert, ShellState},
    tooltip::Tooltips,
};

pub enum ServiceEvent {
    Notification(services::notifications::Event),
    NotificationServer(services::notifications::Handle),
    Icon(u32, Option<ui::tiny_skia::Pixmap>),
    Tray(services::tray::Event),
    TrayServer(services::tray::Handle),
    TrayItem(String, Option<services::tray::TrayItem>),
    TrayIcon(String, Option<ui::tiny_skia::Pixmap>),
    TrayMenu(String, Vec<services::tray::MenuEntry>),
    Network(services::network::Event),
    NetworkReady(services::network::Handle),
    Hyprland(hyprland::Event),
    Audio(AudioState),
    Power(PowerState),
    BrightnessDone,
    Apps(Vec<services::apps::AppEntry>),
    AppIcon(String, crate::panels::AppIcon),
    ClipList(u64, Vec<services::clipboard::ClipEntry>),
    ClipThumb(u64, String, Option<std::sync::Arc<ui::tiny_skia::Pixmap>>),
    WallList(services::wallpapers::Listing),
    Profiles(services::upower::Profiles),
    WallApplied(Result<(), String>),
    CursorList(Vec<crate::cursorctl::CursorItem>),
    CursorApplied(Result<services::cursors::State, String>),
    ThemeGenerated(Result<(), String>),
    Preview(String, String, String),
    PreviewPixmap(String, Option<std::sync::Arc<ui::tiny_skia::Pixmap>>),
    WallDecoded(
        OutputId,
        u64,
        crate::wallpaper::DecodeKind,
        Option<std::sync::Arc<ui::tiny_skia::Pixmap>>,
        Option<std::sync::Arc<ui::tiny_skia::Pixmap>>,
    ),
}

struct BarSurface {
    surface: SurfaceId,
    view: Option<(Bar, Canvas)>,
}

pub struct App {
    pub platform: Platform,
    pub engine: TextEngine,
    pub state: ShellState,
    pub model: BarModel,
    bars: HashMap<OutputId, BarSurface>,
    tooltips: Tooltips,
    osd: Osd,
    pub(crate) overlay: Overlay,
    pub(crate) notes: crate::notify::Notifications,
    pub(crate) tray: crate::trayctl::Tray,
    pub(crate) net: crate::netctl::Net,
    pub(crate) apps: crate::appsctl::Apps,
    pub(crate) clip: crate::clipctl::Clip,
    pub(crate) walls: crate::wallctl::Walls,
    pub(crate) cursors: crate::cursorctl::Cursors,
    pub(crate) theme: crate::themectl::ThemeCtl,
    pub(crate) profiles: services::upower::Profiles,
    cpu_history: Vec<f64>,
    ram_history: Vec<f64>,
    system_detail: Option<system::Detail>,
    pub(crate) battery_stats: Option<services::system::BatteryStats>,
    pub(crate) wall_views: HashMap<OutputId, crate::wallpaper::WallSurface>,
    repeat: Option<(KeyEvent, Millis)>,
    resources: ResourceMonitor,
    pub(crate) audio: Option<AudioHandle>,
    history: History,
    pub(crate) handle: LoopHandle<'static, App>,
    pub(crate) scheduler: Scheduler<()>,
    pub(crate) events: Sender<ServiceEvent>,
    pub(crate) config: crate::configctl::ConfigCtl,
    brightness_throttle: bool,
    brightness_running: bool,
    start: Instant,
    pub running: bool,
}

fn bar_config(output: OutputId) -> LayerConfig {
    let top = (6.0 * crate::theme::scale()).round() as i32;
    let side = (10.0 * crate::theme::scale()).round() as i32;
    let height = (28.0 * crate::theme::scale()).ceil() as u32;
    LayerConfig {
        namespace: "material-bar".into(),
        layer: Layer::Top,
        anchor: Edges {
            top: true,
            bottom: false,
            left: true,
            right: true,
        },
        margin_top: top,
        margin_right: side,
        margin_bottom: 0,
        margin_left: side,
        width: 0,
        height,
        exclusive_zone: height as i32,
        keyboard: KeyboardMode::None,
        input: InputRegion::Full,
        output: Some(output),
    }
}

impl App {
    pub fn new(
        platform: Platform,
        handle: LoopHandle<'static, App>,
        scheduler: Scheduler<()>,
        events: Sender<ServiceEvent>,
    ) -> Self {
        let theme_settings = crate::themectl::load_initial();
        let mut engine = TextEngine::new(&crate::theme::font_families());
        let osd = Osd::new(&mut engine);
        let mut model = BarModel::placeholder();
        model.clock = format_clock(&clock::now());
        let state = ShellState {
            brightness: system::brightness(),
            wired: system::wired(),
            ..ShellState::default()
        };
        Self {
            platform,
            engine,
            state,
            model,
            bars: HashMap::new(),
            tooltips: Tooltips::default(),
            osd,
            overlay: Overlay::new(),
            notes: crate::notify::Notifications::default(),
            tray: crate::trayctl::Tray::default(),
            net: crate::netctl::Net::default(),
            apps: crate::appsctl::Apps::default(),
            clip: crate::clipctl::Clip::default(),
            walls: crate::wallctl::Walls::default(),
            cursors: crate::cursorctl::Cursors::default(),
            theme: crate::themectl::ThemeCtl::new(theme_settings),
            profiles: services::upower::Profiles::default(),
            cpu_history: Vec::new(),
            ram_history: Vec::new(),
            system_detail: None,
            battery_stats: None,
            wall_views: HashMap::new(),
            repeat: None,
            resources: ResourceMonitor::default(),
            audio: None,
            history: History::load(),
            handle,
            scheduler,
            events,
            config: crate::configctl::ConfigCtl::default(),
            brightness_throttle: false,
            brightness_running: false,
            start: Instant::now(),
            running: true,
        }
    }

    pub fn now(&self) -> Millis {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
    pub fn start_services(&mut self) {
        self.start_notifications();
        self.start_tray();
        self.start_network();
        self.start_apps();
        self.start_wallpapers();
        self.start_cursors();
        self.start_theme();
        self.start_power_panels();
        let sender = self.events.clone();
        if let Err(err) = hyprland::spawn(move |event| {
            let _ = sender.send(ServiceEvent::Hyprland(event));
        }) {
            log::error!("hyprland: {err}");
        }
        let sender = self.events.clone();
        match audio::spawn(move |state| {
            let _ = sender.send(ServiceEvent::Audio(state));
        }) {
            Ok(handle) => self.audio = Some(handle),
            Err(err) => log::error!("pipewire: {err}"),
        }
        let sender = self.events.clone();
        let handle = self.handle.clone();
        let task = async move {
            loop {
                let callback_sender = sender.clone();
                if let Err(err) = upower::watch(move |state| {
                    let _ = callback_sender.send(ServiceEvent::Power(state));
                })
                .await
                {
                    log::warn!("upower: {err}");
                }
                TimeoutFuture::from_duration(&handle, Duration::from_secs(2)).await;
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("upower: {err}");
        }
        self.add_timer(Duration::ZERO, |app| {
            app.sample_resources();
            Some(Duration::from_secs(2))
        });
        self.add_timer(clock::until_next_minute(&clock::now()), |app| {
            app.tick_clock();
            Some(clock::until_next_minute(&clock::now()))
        });
        self.add_timer(Duration::from_secs(5), |app| {
            app.poll_status();
            Some(Duration::from_secs(5))
        });
        self.add_timer(Duration::from_secs(3), |app| {
            app.apply_profile();
            None
        });
        self.add_timer(Duration::ZERO, |app| {
            app.record_battery();
            Some(Duration::from_secs(60))
        });
    }

    pub(crate) fn add_timer(
        &self,
        first: Duration,
        mut f: impl FnMut(&mut App) -> Option<Duration> + 'static,
    ) {
        let result =
            self.handle
                .insert_source(Timer::from_duration(first), move |_, _, app| match f(app) {
                    Some(next) => TimeoutAction::ToDuration(next),
                    None => TimeoutAction::Drop,
                });
        if let Err(err) = result {
            log::error!("timer: {err}");
        }
    }

    pub fn listen_ipc(&self, listener: UnixListener) {
        let source = Generic::new(listener, Interest::READ, Mode::Level);
        let result = self.handle.insert_source(source, |_, listener, app| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => match ipc::read_request(&stream) {
                        Ok(args) => {
                            let reply = app.ipc(&args);
                            ipc::write_reply(&stream, reply);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {}
                        Err(err) => log::warn!("ipc: {err}"),
                    },
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(err) => {
                        log::warn!("ipc accept: {err}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        });
        if let Err(err) = result {
            log::error!("ipc: {err}");
        }
    }

    pub fn service(&mut self, event: ServiceEvent) {
        match event {
            ServiceEvent::Notification(event) => self.notification_event(event),
            ServiceEvent::NotificationServer(handle) => {
                self.notes.handle = Some(handle);
                self.flush_config_error();
            }
            ServiceEvent::Icon(id, pixmap) => self.icon_loaded(id, pixmap),
            ServiceEvent::Tray(event) => self.tray_event(event),
            ServiceEvent::TrayServer(handle) => self.tray_server(handle),
            ServiceEvent::TrayItem(key, item) => self.tray_item_loaded(key, item),
            ServiceEvent::TrayIcon(key, pixmap) => self.tray_icon_loaded(key, pixmap),
            ServiceEvent::TrayMenu(key, entries) => self.tray_menu_loaded(key, entries),
            ServiceEvent::Network(event) => self.network_event(event),
            ServiceEvent::NetworkReady(handle) => self.network_ready(handle),
            ServiceEvent::Hyprland(event) => self.hyprland(event),
            ServiceEvent::Audio(state) => {
                self.state.audio = state;
                self.sync();
            }
            ServiceEvent::Power(power) => self.power(power),
            ServiceEvent::Apps(catalog) => self.apps_loaded(catalog),
            ServiceEvent::AppIcon(id, icon) => self.app_icon_loaded(id, icon),
            ServiceEvent::ClipList(generation, entries) => self.clip_listed(generation, entries),
            ServiceEvent::ClipThumb(generation, id, pixmap) => {
                self.clip_thumb_loaded(generation, id, pixmap)
            }
            ServiceEvent::WallList(listing) => self.wallpapers_listed(listing),
            ServiceEvent::Profiles(profiles) => self.profiles_changed(profiles),
            ServiceEvent::WallApplied(result) => self.wallpaper_applied(result),
            ServiceEvent::CursorList(items) => self.cursors_listed(items),
            ServiceEvent::CursorApplied(result) => self.cursor_applied(result),
            ServiceEvent::ThemeGenerated(result) => self.theme_generated(result),
            ServiceEvent::Preview(path, revision, picture) => {
                self.preview_ready(path, revision, picture)
            }
            ServiceEvent::PreviewPixmap(picture, pixmap) => {
                self.preview_pixmap_loaded(picture, pixmap)
            }
            ServiceEvent::WallDecoded(output, generation, kind, front, incoming) => {
                self.wallpaper_decoded(output, generation, kind, front, incoming)
            }
            ServiceEvent::BrightnessDone => {
                self.brightness_running = false;
                if self.brightness_throttle {
                    return;
                }
                self.state.wanted_brightness = None;
                self.poll_status();
            }
        }
    }

    fn hyprland(&mut self, event: hyprland::Event) {
        match event {
            hyprland::Event::State(state) => {
                self.model.workspaces = state.workspaces.iter().map(|w| w.id).collect();
                self.model.active_workspace = state.active_workspace.unwrap_or(1);
                self.model.title = state.active_window.as_ref().map(|w| w.title.clone());
                self.model.title_key = state
                    .active_window
                    .as_ref()
                    .map_or_else(|| "none".to_string(), |w| w.address.clone());
                self.state.focused_monitor = state.focused_monitor;
            }
            hyprland::Event::Keyboard(keyboard) => {
                let changed =
                    self.state.keyboard_initialized && keyboard.name != self.state.keyboard.name;
                self.state.keyboard = crate::state::KeyboardLayout {
                    label: keyboard.label,
                    name: keyboard.name,
                    device: keyboard.device,
                };
                self.state.keyboard_initialized = true;
                if changed {
                    self.state.show_osd("keyboard");
                }
            }
            hyprland::Event::Raw { name, .. } => {
                if name == "configreloaded" {
                    self.state.hotkeys_paused = false;
                }
            }
        }
        self.sync();
    }

    fn power(&mut self, power: PowerState) {
        let was_present = self.state.battery_present();
        let was_plugged = self.state.battery_plugged();
        let was_level = self.state.battery_level();
        self.state.power = power;
        let plugged = self.state.battery_plugged();
        if plugged != was_plugged {
            self.state.battery_alerts.clear();
            self.apply_profile();
        } else if self.state.battery_present() != was_present {
            self.apply_profile();
        }
        if self.state.battery_level() != was_level
            && let Some(alert) = self.state.battery_alert()
        {
            self.battery_alert(alert);
        }
        self.sync();
    }

    fn battery_alert(&self, alert: BatteryAlert) {
        let result = match alert {
            BatteryAlert::Suspend => Command::new(chuhctl()).arg("suspend").spawn(),
            BatteryAlert::Notify { critical, level } => Command::new("notify-send")
                .args([
                    "-a",
                    "Battery",
                    "-u",
                    if critical { "critical" } else { "normal" },
                    "-i",
                    "battery-caution",
                    &format!("Battery low · {level}%"),
                    if critical {
                        "The laptop will suspend at 3%. Plug in the charger."
                    } else {
                        "Plug in the charger soon."
                    },
                ])
                .spawn(),
        };
        match result {
            Ok(child) => reap(child),
            Err(err) => log::warn!("battery alert: {err}"),
        }
    }

    pub(crate) fn apply_profile(&self) {
        if !self.state.battery_present() {
            return;
        }
        let plugged = self.state.battery_plugged();
        let task = async move {
            if let Err(err) = upower::apply_profile(plugged).await {
                log::warn!("power profile: {err}");
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::warn!("power profile: {err}");
        }
    }

    fn record_battery(&mut self) {
        if !self.state.battery_present() {
            return;
        }
        let level = self.state.battery_level();
        let power = self.state.battery_power();
        self.history.record(level, power);
    }

    fn poll_status(&mut self) {
        let wired = system::wired();
        let mut changed = wired != self.state.wired;
        self.state.wired = wired;
        if self.state.wanted_brightness.is_none() {
            let brightness = system::brightness();
            changed |= brightness != self.state.brightness;
            self.state.brightness = brightness;
        }
        if changed {
            self.sync();
        }
    }

    fn sample_resources(&mut self) {
        let sample = self.resources.sample();
        let changed = sample.cpu != self.model.cpu
            || format!("{:.1}", sample.ram_used) != format!("{:.1}", self.model.ram_used)
            || sample.ram_total != self.model.ram_total;
        if changed {
            self.model.cpu = sample.cpu;
            self.model.ram_used = sample.ram_used;
            self.model.ram_total = sample.ram_total;
            self.refresh_bars();
        }
        let mut panel_changed = changed;
        if let Some(cpu) = sample.cpu {
            self.cpu_history.push(cpu as f64);
            self.ram_history.push(sample.ram_used);
            for history in [&mut self.cpu_history, &mut self.ram_history] {
                if history.len() > 60 {
                    history.drain(..history.len() - 60);
                }
            }
            panel_changed = true;
        }
        if let Some(detail) = self.resources.take_detail() {
            self.system_detail = Some(detail);
            panel_changed = true;
        }
        if panel_changed && self.state.panel == "system" {
            self.sync_overlay();
        }
    }

    pub(crate) fn system_panel_changed(&mut self, panel: &str) {
        let enabled = panel == "system";
        if !enabled {
            self.system_detail = None;
        }
        if self.resources.set_detail(enabled) {
            self.add_timer(Duration::from_millis(600), |app| {
                app.sample_resources();
                None
            });
        }
    }

    pub(crate) fn kill_process(&self, pid: i32, force: bool) {
        let signal = if force { "-KILL" } else { "-TERM" };
        match Command::new("kill")
            .args([signal, &pid.to_string()])
            .spawn()
        {
            Ok(child) => reap(child),
            Err(err) => log::warn!("kill {pid}: {err}"),
        }
    }

    fn tick_clock(&mut self) {
        let text = format_clock(&clock::now());
        if text != self.model.clock {
            self.model.clock = text;
            self.refresh_bars();
        }
        self.sync_overlay();
    }

    fn change_volume(&mut self, step: f64, osd: bool) {
        let Some(sink) = self
            .state
            .audio
            .sink()
            .filter(|sink| sink.has_audio)
            .cloned()
        else {
            return;
        };
        let Some(audio) = &self.audio else {
            return;
        };
        if step > 0.0 && sink.muted {
            audio.send(audio::Command::SetMute {
                node: sink.id,
                muted: false,
            });
        }
        let volume = ((sink.volume as f64 + step) * 100.0 + 0.5).floor() / 100.0;
        audio.send(audio::Command::SetVolume {
            node: sink.id,
            volume: volume.clamp(0.0, 1.0) as f32,
        });
        if osd {
            self.state.show_osd("volume");
            self.sync();
        }
    }

    fn toggle_mute(&mut self, osd: bool) {
        let Some(sink) = self
            .state
            .audio
            .sink()
            .filter(|sink| sink.has_audio)
            .cloned()
        else {
            return;
        };
        if let Some(audio) = &self.audio {
            audio.send(audio::Command::SetMute {
                node: sink.id,
                muted: !sink.muted,
            });
        }
        if osd {
            self.state.show_osd("volume");
            self.sync();
        }
    }

    fn toggle_mic(&mut self) {
        let Some(source) = self
            .state
            .audio
            .source()
            .filter(|source| source.has_audio)
            .cloned()
        else {
            return;
        };
        if let Some(audio) = &self.audio {
            audio.send(audio::Command::SetMute {
                node: source.id,
                muted: !source.muted,
            });
        }
        self.state.show_osd("mic");
        self.sync();
    }

    fn set_brightness(&mut self, value: f64) {
        let wanted = ((value + 0.5).floor()).clamp(5.0, 100.0) as u32;
        self.state.wanted_brightness = Some(wanted);
        self.state.brightness = wanted;
        if !self.brightness_throttle {
            self.brightness_throttle = true;
            self.add_timer(Duration::from_millis(50), |app| app.brightness_tick());
        }
    }

    fn brightness_tick(&mut self) -> Option<Duration> {
        if self.brightness_running {
            return Some(Duration::from_millis(50));
        }
        self.brightness_throttle = false;
        let wanted = self.state.wanted_brightness?;
        match Command::new("brightnessctl")
            .args(["-m", "set", &format!("{wanted}%")])
            .stdout(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                self.brightness_running = true;
                let sender = self.events.clone();
                std::thread::spawn(move || {
                    let _ = child.wait();
                    let _ = sender.send(ServiceEvent::BrightnessDone);
                });
            }
            Err(err) => {
                log::warn!("brightnessctl: {err}");
                self.state.wanted_brightness = None;
            }
        }
        None
    }

    fn step_brightness(&mut self, step: f64) {
        self.set_brightness(self.state.brightness as f64 + step);
        self.state.show_osd("brightness");
        self.sync();
    }

    fn set_keep_awake(&mut self, enabled: bool) {
        self.state.keep_awake = enabled;
        let surfaces: Vec<SurfaceId> = self.bars.values().map(|bar| bar.surface).collect();
        for surface in surfaces {
            self.platform.set_idle_inhibit(surface, enabled);
        }
    }

    pub fn ipc(&mut self, args: &[String]) -> Reply {
        let Some(command) = args.first() else {
            return Reply::Error("missing command".into());
        };
        let panel = |app: &mut App, name: &str| {
            app.state.toggle(name, false);
            app.sync();
        };
        match command.as_str() {
            "launcher" => panel(self, "launcher"),
            "menu" => panel(self, "control"),
            "calendar" => panel(self, "calendar"),
            "wallpapers" => panel(self, "wallpapers"),
            "wifi" => panel(self, "wifi"),
            "bluetooth" => panel(self, "bluetooth"),
            "audio" => panel(self, "audio"),
            "battery" => panel(self, "battery"),
            "system" => panel(self, "system"),
            "notifications" => panel(self, "notifications"),
            "clipboard" => panel(self, "clipboard"),
            "power" => panel(self, "power"),
            "awake" => self.set_keep_awake(!self.state.keep_awake),
            "dnd" => {
                self.state.dnd = !self.state.dnd;
                self.sync();
            }
            "volumeUp" => self.change_volume(0.05, true),
            "volumeDown" => self.change_volume(-0.05, true),
            "volumeMute" => self.toggle_mute(true),
            "micMute" => self.toggle_mic(),
            "osd" => {
                let kind = args.get(1).cloned().unwrap_or_default();
                let text = args.get(2).cloned().unwrap_or_default();
                self.state.show_osd_text(&kind, &text);
                self.sync();
            }
            "brightnessUp" => self.step_brightness(5.0),
            "brightnessDown" => self.step_brightness(-5.0),
            "tray" => {
                let id = args.get(1).cloned().unwrap_or_default();
                self.tray_ipc(&id);
            }
            "close" => {
                self.state.panel.clear();
                self.sync();
            }
            "theme" => {
                let mode = match args.get(1).map(String::as_str) {
                    None => return Reply::Ok(self.theme.mode.name().to_string()),
                    Some("scheme") => {
                        let scheme = match args.get(2).map(String::as_str) {
                            None => return Reply::Ok(self.theme.scheme.to_string()),
                            Some("next") => services::theme::next_scheme(self.theme.scheme),
                            Some(value) => match services::theme::parse_scheme(value) {
                                Some(scheme) => scheme,
                                None => {
                                    return Reply::Error(format!(
                                        "unknown scheme: {value} (one of {})",
                                        services::theme::SCHEMES.join(", ")
                                    ));
                                }
                            },
                        };
                        self.set_theme_scheme(scheme);
                        return Reply::Ok(String::new());
                    }
                    Some("toggle") => match self.theme.mode {
                        services::theme::Mode::Auto => services::theme::Mode::Static,
                        services::theme::Mode::Static => services::theme::Mode::Auto,
                    },
                    Some(value) => match services::theme::Mode::parse(value) {
                        Some(mode) => mode,
                        None => return Reply::Error(format!("unknown theme mode: {value}")),
                    },
                };
                self.set_theme_mode(mode);
            }
            "inspect" => return Reply::Ok(self.inspect()),
            other => return Reply::Error(format!("unknown command: {other}")),
        }
        Reply::Ok(String::new())
    }

    fn inspect(&self) -> String {
        let battery = &self.state.power.battery;
        serde_json::json!({
            "panel": self.state.panel,
            "applications": self.apps.results.len(),
            "hidden": self.apps.hidden,
                        "wifi": self.net.wifi.state,
            "bluetooth": self.net.bluetooth.powered,
            "wallpapers": self.walls.data.items.len(),
            "wallpaper": self.walls.data.current,
            "previews": self.walls.previews.len(),
            "cursors": self.cursors.data.items.len(),
            "cursor": self.cursors.state.theme,
            "cursorAnimation": self.cursors.state.animation,
            "cursorSize": self.cursors.state.size,
            "cursorColors": self.cursors.data.colors,
            "cursorApplied": self.cursors.state.effective(),
            "keepAwake": self.state.keep_awake,
            "screen": self.state.panel_screen,
            "battery": {
                "present": self.state.battery_present(),
                "raw": battery.as_ref().map(|battery| battery.percentage / 100.0),
                "level": self.state.battery_level(),
                "plugged": self.state.battery_plugged(),
            },
            "brightness": self.state.brightness,            "notifications": self.notifications_json(),
            "popups": self.notes.popups.len(),
            "dnd": self.state.dnd,
            "theme": self.theme.mode.name(),
            "themeScheme": self.theme.scheme,
        })
        .to_string()
    }
    pub(crate) fn sync(&mut self) {
        let state = &self.state;
        self.model.panel = state.panel.clone();
        self.model.keyboard_label = state.keyboard.label.clone();
        self.model.hotkeys_paused = state.hotkeys_paused;
        self.model.dnd = state.dnd;
        self.model.volume = state.volume();
        self.model.muted = state.muted();
        self.model.battery_present = state.battery_present();
        self.model.battery_level = state.battery_level();
        self.model.battery_plugged = state.battery_plugged();
        self.model.wifi_connected = self.net.wifi.connected;
        self.model.wifi_strength = self.net.strength();
        self.model.bluetooth_powered = self.net.bluetooth.powered;
        self.model.bluetooth_connected = self.net.connected_devices();
        self.model.network_connected = self.net.network_connected(state.wired.connected);
        self.overlay.wifi_prompt = !self.net.prompt.is_empty();
        self.model.unread_notifications = self.notes.unread();
        self.model.tray = self.tray.bar_entries();
        self.overlay.tray_tall = self.tray.only_menu || self.tray.expanded;
        self.refresh_bars();
        self.sync_osd();
        self.sync_overlay();
        self.sync_popups();
    }

    pub(crate) fn ring_bells(&mut self, now: Millis) {
        let mut surfaces = Vec::new();
        for entry in self.bars.values_mut() {
            if let Some((bar, _)) = entry.view.as_mut() {
                bar.ring_bell(now);
                surfaces.push(entry.surface);
            }
        }
        for surface in surfaces {
            self.platform.schedule_frame(surface);
        }
    }

    fn panel_data(&self) -> PanelData {
        let now = clock::now();
        PanelData {
            time: now.strftime("%H:%M").to_string(),
            date: now.strftime("%A, %-d %B").to_string(),
            network_connected: self.model.network_connected,
            network_name: if self.net.wifi.connected {
                self.net.wifi.network.clone()
            } else {
                self.state.wired.network.clone()
            },
            bluetooth_powered: self.net.bluetooth.powered,
            bluetooth_name: self.net.bluetooth.name.clone(),
            muted: self.state.muted(),
            volume: self.state.volume(),
            brightness: self.state.brightness,
            keep_awake: self.state.keep_awake,
            theme_auto: self.theme.mode == services::theme::Mode::Auto,
            theme_scheme: self.theme.scheme,
            dnd: self.state.dnd,
            notifications: self.notes.list.clone(),
            ages: self.notes.ages(),
            tray_title: self.tray.title(),
            tray_only_menu: self.tray.only_menu,
            tray_serial: self.tray.serial,
            tray_menu: self.tray.menu.clone(),
            wifi: self.net.wifi.clone(),
            wifi_details: self.net.details.clone(),
            wifi_down: self.net.down.clone(),
            wifi_up: self.net.up.clone(),
            wifi_prompt: self.net.prompt.clone(),
            bluetooth: self.net.bluetooth.clone(),
            bluetooth_profiles: self.net.profiles.clone(),
            bluetooth_prompt: self.net.bluetooth_prompt.clone(),
            bluetooth_error: self.net.bluetooth_error.clone(),
            apps: self.apps.results.clone(),
            hidden: self.apps.hidden.clone(),
            clipboard: self.clip.entries.clone(),
            wall: self.walls.data.clone(),
            previews: self.walls.previews.clone(),
            cursors: self.cursors.data.clone(),
            power: self.state.power.clone(),
            battery_plugged: self.state.battery_plugged(),
            battery_stats: self.battery_stats,
            profiles: self.profiles.clone(),
            battery_history: self.history.points.clone(),
            audio: self.state.audio.clone(),
            resources: system::Resources {
                cpu: self.model.cpu,
                ram_used: self.model.ram_used,
                ram_total: self.model.ram_total,
            },
            cpu_history: self.cpu_history.clone(),
            ram_history: self.ram_history.clone(),
            system_detail: self.system_detail.clone(),
        }
    }

    fn sync_overlay(&mut self) {
        let now = self.now();
        let data = self.panel_data();
        self.overlay.set_data(&mut self.engine, &data, now);
        self.overlay.anchor = self.state.panel_anchor;
        if self.overlay.panel != self.state.panel {
            let panel = self.state.panel.clone();
            self.overlay.panel_changed(&mut self.engine, &panel, now);
            self.panel_opened(&panel);
            self.network_panel_changed(&panel);
            self.apps_panel_changed(&panel);
            self.clip_panel_changed(&panel);
            self.walls_panel_changed(&panel);
            self.power_panel_changed(&panel);
            self.system_panel_changed(&panel);
            self.model.unread_notifications = self.notes.unread();
            self.refresh_bars();
        }
        self.overlay.set_icons(&self.notes.icons);
        self.overlay.set_app_icons(&self.apps.icons);
        self.overlay.set_clip_thumbs(&self.clip.thumbs);
        self.overlay.set_wall_pixmaps(&self.walls.pixmaps);
        self.ensure_overlay();
        self.overlay.schedule(&mut self.platform);
    }

    fn output_by_name(&self, name: &str) -> Option<OutputId> {
        let outputs = self.platform.outputs();
        outputs
            .iter()
            .copied()
            .find(|id| {
                self.platform
                    .output_desc(*id)
                    .is_some_and(|desc| desc.name == name)
            })
            .or_else(|| self.focused_output())
    }

    fn ensure_overlay(&mut self) {
        if !self.overlay.visible() {
            self.teardown_overlay();
            return;
        }
        let output = self.output_by_name(&self.state.panel_screen.clone());
        if self.overlay.surface.is_some() && self.overlay.output != output {
            self.teardown_overlay();
        }
        if self.overlay.surface.is_none()
            && let Some(output) = output
        {
            let surface = self
                .platform
                .create_layer_surface(&overlay::layer_config(output));
            let card = self.platform.create_subsurface(surface);
            self.overlay.attach(surface, card, output);
        }
        let mode = if self.state.panel.is_empty() {
            KeyboardMode::None
        } else {
            KeyboardMode::Exclusive
        };
        if let Some(surface) = self.overlay.surface
            && self.overlay.keyboard != mode
        {
            self.platform.set_keyboard_mode(surface, mode);
            self.overlay.keyboard = mode;
        }
    }

    fn teardown_overlay(&mut self) {
        self.repeat = None;
        if let Some(surface) = self.overlay.surface {
            self.tooltips.drop_parent(&mut self.platform, surface);
        }
        let (card, surface) = self.overlay.detach();
        if let Some(card) = card {
            self.platform.destroy_surface(card);
        }
        if let Some(surface) = surface {
            self.platform.destroy_surface(surface);
        }
        crate::memory::trim();
    }

    fn panel_action(&mut self, action: PanelAction) {
        match action {
            PanelAction::Toggle(name) => {
                self.state.toggle(name, false);
                self.sync();
            }
            PanelAction::Run(name) => run(name),
            PanelAction::Suspend => {
                self.state.panel.clear();
                self.sync();
                run("suspend");
            }
            PanelAction::ToggleMute => self.toggle_mute(false),
            PanelAction::SetVolume(value) => {
                let sink = self
                    .state
                    .audio
                    .sink()
                    .filter(|sink| sink.has_audio)
                    .map(|sink| sink.id);
                if let (Some(node), Some(audio)) = (sink, &self.audio) {
                    audio.send(audio::Command::SetVolume {
                        node,
                        volume: (value / 100.0).clamp(0.0, 1.0) as f32,
                    });
                }
            }
            PanelAction::SetBrightness(value) => {
                self.set_brightness(value);
                self.sync();
            }
            PanelAction::ToggleTheme => {
                let mode = match self.theme.mode {
                    services::theme::Mode::Auto => services::theme::Mode::Static,
                    services::theme::Mode::Static => services::theme::Mode::Auto,
                };
                self.set_theme_mode(mode);
            }
            PanelAction::ThemeScheme => {
                let scheme = services::theme::next_scheme(self.theme.scheme);
                self.set_theme_scheme(scheme);
            }
            PanelAction::ToggleKeepAwake => {
                self.set_keep_awake(!self.state.keep_awake);
                self.sync();
            }
            PanelAction::BluetoothPower => {
                let powered = self.net.bluetooth.powered;
                self.net_action(crate::panels::NetAction::BtPower(!powered));
            }
            PanelAction::Net(action) => self.net_action(action),
            PanelAction::Launch(id) => self.launch_application(&id),
            PanelAction::ToggleHidden(id) => self.toggle_application_hidden(&id),
            PanelAction::Search(query) => self.search_applications(&query),
            PanelAction::ClipCopy(id) => self.clip_copy(&id),
            PanelAction::ClipDelete(id) => self.clip_delete(&id),
            PanelAction::ClipWipe => self.clip_wipe(),
            PanelAction::WallPreviews(items) => self.request_previews(items),
            PanelAction::WallRefresh => self.refresh_wallpapers(),
            PanelAction::CursorApply(name) => self.apply_cursor(&name),
            PanelAction::CursorAnimation => self.toggle_cursor_animation(),
            PanelAction::CursorColors => self.toggle_cursor_colors(),
            PanelAction::CursorSize => self.cycle_cursor_size(),
            PanelAction::CursorRefresh => self.refresh_cursors(),
            PanelAction::CursorPixmaps => self.request_preview_pixmaps(),
            PanelAction::SetProfile(profile) => self.set_profile(profile),
            PanelAction::SetMicVolume(value) => self.set_mic_volume(value),
            PanelAction::SetDefaultNode(id) => self.set_default_node(id),
            PanelAction::ToggleMic => self.toggle_mic(),
            PanelAction::Session(action) => self.session_action(action),
            PanelAction::Kill(pid, force) => self.kill_process(pid, force),
            PanelAction::WallApply(path, x, y) => {
                let (ox, oy) = self.overlay.canvas_origin();
                let screen = self
                    .overlay
                    .output
                    .and_then(|output| self.platform.output_desc(output))
                    .map(|desc| desc.name)
                    .unwrap_or_default();
                let origin = crate::wallctl::Origin {
                    x: x + ox as f32,
                    y: y + oy as f32,
                    screen,
                };
                self.apply_wallpaper(&path, origin);
            }
            PanelAction::Close => {
                self.state.panel.clear();
                self.sync();
            }
            PanelAction::ToggleDnd => {
                self.state.dnd = !self.state.dnd;
                self.sync();
            }
            PanelAction::ClearAll => {
                self.clear_notifications();
                self.sync();
            }
            PanelAction::Card(action) => self.card_action(action, false),
            action @ (PanelAction::TrayExpanded(_)
            | PanelAction::TrayLoadMenu
            | PanelAction::TrayActivate
            | PanelAction::TrayQuit
            | PanelAction::TrayEvent(..)) => self.tray_action(action),
        }
    }
    fn draw_overlay(&mut self, now: Millis) {
        self.overlay.set_now(now);
        let actions = self.overlay.draw(&mut self.platform, &mut self.engine, now);
        if let Some(surface) = self.overlay.surface {
            let tooltip = self.overlay.tooltip();
            self.tooltips
                .update(&mut self.platform, surface, tooltip, now);
        }
        for action in actions {
            self.panel_action(action);
        }
        self.request_app_icons();
        self.request_clip_thumbs();
        self.request_preview_pixmaps();
    }

    fn overlay_pointer(&mut self, event: PointerEvent) {
        let now = self.now();
        self.overlay.set_now(now);
        let actions = self
            .overlay
            .pointer(event.kind, event.x, event.y, &mut self.engine, now);
        let cursor = self.overlay.cursor();
        let tooltip = self.overlay.tooltip();
        self.platform.set_cursor(cursor);
        self.overlay.schedule(&mut self.platform);
        self.tooltips
            .update(&mut self.platform, event.surface, tooltip, now);
        for action in actions {
            self.panel_action(action);
        }
    }

    fn key(&mut self, event: KeyEvent) {
        if event.surface.is_none() || event.surface != self.overlay.surface {
            self.repeat = None;
            return;
        }
        let modifier = (0xffe1..=0xffee).contains(&event.keysym);
        if !event.pressed {
            if self
                .repeat
                .as_ref()
                .is_some_and(|(held, _)| held.keysym == event.keysym)
            {
                self.repeat = None;
            }
        } else if !modifier {
            let delay = self.platform.key_repeat().map(|repeat| repeat.delay as f64);
            self.repeat = delay.map(|delay| (event.clone(), self.now() + delay));
        }
        self.dispatch_key(event);
    }

    fn dispatch_key(&mut self, event: KeyEvent) {
        let now = self.now();
        self.overlay.set_now(now);
        let actions = self.overlay.key(&mut self.engine, &event);
        self.overlay.schedule(&mut self.platform);
        for action in actions {
            self.panel_action(action);
        }
    }

    fn sync_osd(&mut self) {
        let now = self.now();
        let triggered = self.osd.sync(&self.state, &mut self.engine, now);
        if triggered {
            let output = self.focused_output();
            if let Some(output) = output
                && self.osd.output != Some(output)
            {
                if let Some(surface) = self.osd.detach() {
                    self.platform.destroy_surface(surface);
                }
                let surface = self
                    .platform
                    .create_layer_surface(&osd::layer_config(output));
                self.osd.attach(surface, output);
            }
        }
        if let Some(surface) = self.osd.surface {
            self.platform.schedule_frame(surface);
        }
    }
    pub(crate) fn focused_output(&self) -> Option<OutputId> {
        let outputs = self.platform.outputs();
        let name = self.state.focused_monitor.as_deref();
        outputs
            .iter()
            .copied()
            .find(|id| {
                self.platform
                    .output_desc(*id)
                    .is_some_and(|desc| Some(desc.name.as_str()) == name)
            })
            .or_else(|| outputs.first().copied())
    }

    fn refresh_bars(&mut self) {
        let now = self.now();
        let mut surfaces = Vec::new();
        for entry in self.bars.values_mut() {
            if let Some((bar, _)) = entry.view.as_mut() {
                bar.apply(&mut self.engine, &self.model, now, false);
                surfaces.push(entry.surface);
            }
        }
        for surface in surfaces {
            self.platform.schedule_frame(surface);
        }
    }

    fn action(&mut self, action: BarAction, surface: SurfaceId) {
        let screen = self
            .bars
            .iter()
            .find(|(_, bar)| bar.surface == surface)
            .and_then(|(output, _)| self.platform.output_desc(*output))
            .map(|desc| desc.name)
            .unwrap_or_default();
        let result = match action {
            BarAction::Open { panel, anchor } => {
                match anchor {
                    Some(x) => self.state.open_module(panel, x, &screen),
                    None => self.state.open_at(panel, &screen),
                }
                self.sync();
                Ok(())
            }
            BarAction::Tray { key, right, anchor } => {
                self.tray_click(&key, right, anchor, &screen);
                Ok(())
            }
            BarAction::TrayScroll { key, delta } => {
                self.tray_scroll(&key, delta);
                Ok(())
            }
            BarAction::Workspace(id) => hyprland::focus_workspace(id),
            BarAction::KeyboardNext => hyprland::next_layout(),
            BarAction::Volume(delta) => {
                self.change_volume(if delta > 0 { 0.05 } else { -0.05 }, true);
                Ok(())
            }
            BarAction::ToggleMute => {
                self.toggle_mute(true);
                Ok(())
            }
            BarAction::ToggleDnd => {
                self.state.dnd = !self.state.dnd;
                self.sync();
                Ok(())
            }
        };
        if let Err(err) = result {
            log::warn!("{err}");
        }
    }

    pub(crate) fn retheme(&mut self) {
        let now = self.now();
        let mut surfaces = Vec::new();
        for entry in self.bars.values_mut() {
            if let Some((bar, canvas)) = entry.view.as_mut() {
                *bar = Bar::new(
                    &mut self.engine,
                    canvas.width(),
                    canvas.height(),
                    &self.model,
                    now,
                );
                surfaces.push(entry.surface);
            }
        }
        for surface in surfaces {
            if self.state.keep_awake {
                self.platform.set_idle_inhibit(surface, true);
            }
            self.draw_bar(surface, true);
        }
        self.osd.retheme(&mut self.engine);
        if let Some(surface) = self.osd.surface {
            self.platform.schedule_frame(surface);
        }
        self.overlay.retheme(&mut self.engine, now);
        self.sync();
        crate::memory::trim();
    }

    pub(crate) fn rebuild_surfaces(&mut self) {
        self.state.panel.clear();
        self.overlay.panel.clear();
        self.teardown_overlay();
        if let Some(surface) = self.osd.detach() {
            self.platform.destroy_surface(surface);
        }
        let outputs: Vec<OutputId> = self.bars.keys().copied().collect();
        for output in outputs {
            if let Some(bar) = self.bars.remove(&output) {
                self.tooltips.drop_parent(&mut self.platform, bar.surface);
                self.platform.destroy_surface(bar.surface);
            }
            let surface = self.platform.create_layer_surface(&bar_config(output));
            self.bars.insert(
                output,
                BarSurface {
                    surface,
                    view: None,
                },
            );
        }
        self.retheme();
    }

    fn draw_bar(&mut self, surface: SurfaceId, full: bool) {
        let now = self.now();
        let Some(entry) = self.bars.values_mut().find(|bar| bar.surface == surface) else {
            return;
        };
        let Some((bar, canvas)) = entry.view.as_mut() else {
            return;
        };
        let running = bar.tick(now);
        let events = bar.input.refresh(&bar.tree);
        if !events.is_empty() {
            bar.handle(&events, now);
        }
        let mut damage = bar.tree.collect_damage();
        if full {
            damage = Damage::full(canvas.width() as i32, canvas.height() as i32);
        }
        if damage.is_empty() {
            if running {
                self.platform.schedule_frame(surface);
            }
            return;
        }
        canvas.render(&mut bar.tree, &mut self.engine, &damage);
        self.platform
            .present(surface, canvas.data(), &damage, running);
    }

    fn pointer(&mut self, event: PointerEvent) {
        if self.overlay.owns(event.surface) {
            self.overlay_pointer(event);
            return;
        }
        if self.popups_owns(event.surface) {
            self.popups_pointer(event);
            return;
        }
        let now = self.now();
        let Some(entry) = self
            .bars
            .values_mut()
            .find(|bar| bar.surface == event.surface)
        else {
            return;
        };
        let Some((bar, _)) = entry.view.as_mut() else {
            return;
        };
        let events = match event.kind {
            PointerKind::Enter | PointerKind::Motion => {
                bar.input.motion(&bar.tree, event.x, event.y)
            }
            PointerKind::Leave => bar.input.leave(&bar.tree),
            PointerKind::Press(button) => bar.input.press(&bar.tree, button, event.x, event.y),
            PointerKind::Release(button) => bar.input.release(&bar.tree, button, event.x, event.y),
            PointerKind::Wheel { y, .. } => {
                if y == 0 {
                    Vec::new()
                } else {
                    bar.input.wheel(&bar.tree, event.x, event.y, y)
                }
            }
        };
        let actions = bar.handle(&events, now);
        let cursor = bar.input.cursor();
        let tooltip = bar.tooltip();
        self.platform.set_cursor(cursor);
        for action in actions {
            self.action(action, event.surface);
        }
        self.platform.schedule_frame(event.surface);
        self.tooltips
            .update(&mut self.platform, event.surface, tooltip, now);
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::OutputAdded(output) => {
                self.add_wallpaper(output);
                if !self.bars.contains_key(&output) {
                    let surface = self.platform.create_layer_surface(&bar_config(output));
                    self.bars.insert(
                        output,
                        BarSurface {
                            surface,
                            view: None,
                        },
                    );
                }
            }
            Event::OutputRemoved(output) => {
                self.remove_wallpaper(output);
                if let Some(bar) = self.bars.remove(&output) {
                    self.platform.destroy_surface(bar.surface);
                }
                if self.osd.output == Some(output)
                    && let Some(surface) = self.osd.detach()
                {
                    self.platform.destroy_surface(surface);
                }
            }
            Event::Configure {
                surface,
                width,
                height,
            } => {
                let now = self.now();
                if self.tooltips.owns(surface) {
                    self.tooltips.configure(
                        &mut self.platform,
                        &mut self.engine,
                        surface,
                        width,
                        height,
                        now,
                    );
                    return;
                }
                if self.wallpaper_configure(surface, width, height) {
                    return;
                }
                if self.overlay.owns(surface) {
                    self.overlay.configure(width, height);
                    self.draw_overlay(now);
                    return;
                }
                if self.popups_owns(surface) {
                    self.popups_configure(width, height);
                    return;
                }
                if self.osd.surface == Some(surface) {
                    self.osd.configure(width, height);
                    self.osd.draw(&mut self.platform, &mut self.engine, now);
                    return;
                }
                let Some(entry) = self.bars.values_mut().find(|bar| bar.surface == surface) else {
                    return;
                };
                let resized = entry
                    .view
                    .as_ref()
                    .is_none_or(|(_, canvas)| canvas.width() != width || canvas.height() != height);
                if resized {
                    let Some(canvas) = Canvas::new(width, height) else {
                        return;
                    };
                    let bar = Bar::new(&mut self.engine, width, height, &self.model, now);
                    entry.view = Some((bar, canvas));
                }
                if self.state.keep_awake {
                    self.platform.set_idle_inhibit(surface, true);
                }
                self.draw_bar(surface, resized);
            }
            Event::Closed(surface) => {
                self.wallpaper_closed(surface);
                self.tooltips.closed(surface);
                self.popups_closed(surface);
                if self.overlay.owns(surface) {
                    self.teardown_overlay();
                    self.state.panel.clear();
                    self.overlay.panel.clear();
                }
                if self.osd.surface == Some(surface) {
                    self.osd.detach();
                }
                self.bars.retain(|_, bar| bar.surface != surface);
            }
            Event::Frame(surface) => {
                let now = self.now();
                if self.wallpaper_frame(surface) {
                    return;
                }
                if self.tooltips.owns(surface) {
                    self.tooltips
                        .frame(&mut self.platform, &mut self.engine, surface, now);
                } else if self.overlay.owns(surface) {
                    self.draw_overlay(now);
                    if self.overlay.finished() {
                        self.teardown_overlay();
                    }
                } else if self.popups_owns(surface) {
                    self.popups_draw(now);
                } else if self.osd.surface == Some(surface) {
                    self.osd.draw(&mut self.platform, &mut self.engine, now);
                    if self.osd.finished()
                        && let Some(surface) = self.osd.detach()
                    {
                        self.platform.destroy_surface(surface);
                    }
                } else {
                    self.draw_bar(surface, false);
                }
            }
            Event::Pointer(event) => self.pointer(event),
            Event::Key(event) => self.key(event),
            Event::KeyboardFocus(focus) => {
                if focus.is_none() || focus != self.overlay.surface {
                    self.repeat = None;
                }
            }
        }
    }

    pub fn process(&mut self) {
        while self.platform.has_events() {
            for event in self.platform.take_events() {
                self.handle(event);
            }
        }
        let now = self.now();
        self.tooltips
            .poll(&mut self.platform, &mut self.engine, now);
        if self
            .overlay
            .next_deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.overlay.schedule(&mut self.platform);
        }
        if self
            .repeat
            .as_ref()
            .is_some_and(|(event, _)| event.surface != self.overlay.surface)
        {
            self.repeat = None;
        }
        if let Some((event, at)) = self.repeat.clone()
            && at <= now
        {
            let interval = self
                .platform
                .key_repeat()
                .map_or(40.0, |repeat| 1000.0 / repeat.rate.max(1) as f64);
            self.repeat = Some((event.clone(), at + interval));
            self.dispatch_key(KeyEvent {
                repeat: true,
                ..event
            });
        }
        if self
            .osd
            .next_deadline()
            .is_some_and(|deadline| deadline <= now)
            && let Some(surface) = self.osd.surface
        {
            self.platform.schedule_frame(surface);
        }
    }

    pub fn timeout(&self) -> Option<Duration> {
        let deadline = [
            self.tooltips.next_deadline(),
            self.osd.next_deadline(),
            self.overlay.next_deadline(),
            self.repeat.as_ref().map(|(_, at)| *at),
        ]
        .into_iter()
        .flatten()
        .reduce(f64::min)?;
        let wait = (deadline - self.now()).max(0.0);
        Some(Duration::from_secs_f64(wait / 1000.0))
    }
}

pub(crate) fn run(action: &str) {
    match Command::new(chuhctl()).arg(action).spawn() {
        Ok(child) => reap(child),
        Err(err) => log::warn!("{action}: {err}"),
    }
}

fn chuhctl() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.local/bin/chuhctl")
}

pub(crate) fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}
