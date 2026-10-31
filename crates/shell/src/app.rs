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
    Damage, Edges, Event, InputRegion, KeyboardMode, Layer, LayerConfig, OutputId, Platform,
    PointerEvent, PointerKind, SurfaceId,
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
    state::{BatteryAlert, ShellState},
    theme::UI_SCALE,
    tooltip::Tooltips,
};

pub enum ServiceEvent {
    Hyprland(hyprland::Event),
    Audio(AudioState),
    Power(PowerState),
    BrightnessDone,
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
    resources: ResourceMonitor,
    audio: Option<AudioHandle>,
    history: History,
    handle: LoopHandle<'static, App>,
    scheduler: Scheduler<()>,
    events: Sender<ServiceEvent>,
    brightness_throttle: bool,
    brightness_running: bool,
    start: Instant,
    pub running: bool,
}

fn bar_config(output: OutputId) -> LayerConfig {
    let top = (6.0 * UI_SCALE).round() as i32;
    let side = (10.0 * UI_SCALE).round() as i32;
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
        height: (28.0 * UI_SCALE).ceil() as u32,
        exclusive_zone: 42 - top,
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
        let mut engine = TextEngine::new(crate::theme::FONT_FAMILIES);
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
            resources: ResourceMonitor::default(),
            audio: None,
            history: History::load(),
            handle,
            scheduler,
            events,
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

    fn add_timer(
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
            ServiceEvent::Hyprland(event) => self.hyprland(event),
            ServiceEvent::Audio(state) => {
                self.state.audio = state;
                self.sync();
            }
            ServiceEvent::Power(power) => self.power(power),
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
            BatteryAlert::Suspend => Command::new(chuhshell()).arg("suspend").spawn(),
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

    fn apply_profile(&self) {
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
    }

    fn tick_clock(&mut self) {
        let text = format_clock(&clock::now());
        if text != self.model.clock {
            self.model.clock = text;
            self.refresh_bars();
        }
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
            "tray" => {}
            "close" => {
                self.state.panel.clear();
                self.sync();
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
            "applications": 0,
            "hidden": [],
            "wifi": "",
            "bluetooth": false,
            "wallpapers": 0,
            "wallpaper": "",
            "previews": 0,
            "keepAwake": self.state.keep_awake,
            "screen": self.state.panel_screen,
            "battery": {
                "present": self.state.battery_present(),
                "raw": battery.as_ref().map(|battery| battery.percentage / 100.0),
                "level": self.state.battery_level(),
                "plugged": self.state.battery_plugged(),
            },
            "brightness": self.state.brightness,
            "notifications": [],
            "popups": 0,
            "dnd": self.state.dnd,
        })
        .to_string()
    }

    fn sync(&mut self) {
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
        self.model.network_connected = self.model.wifi_connected || state.wired.connected;
        self.refresh_bars();
        self.sync_osd();
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

    fn focused_output(&self) -> Option<OutputId> {
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

    fn action(&mut self, action: BarAction) {
        let result = match action {
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
            other => {
                log::debug!("bar action {other:?}");
                Ok(())
            }
        };
        if let Err(err) = result {
            log::warn!("{err}");
        }
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
        for action in actions {
            self.action(action);
        }
        self.platform.set_cursor(cursor);
        self.platform.schedule_frame(event.surface);
        self.tooltips
            .update(&mut self.platform, event.surface, tooltip, now);
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::OutputAdded(output) => {
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
                self.tooltips.closed(surface);
                if self.osd.surface == Some(surface) {
                    self.osd.detach();
                }
                self.bars.retain(|_, bar| bar.surface != surface);
            }
            Event::Frame(surface) => {
                let now = self.now();
                if self.tooltips.owns(surface) {
                    self.tooltips
                        .frame(&mut self.platform, &mut self.engine, surface, now);
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
            .osd
            .next_deadline()
            .is_some_and(|deadline| deadline <= now)
            && let Some(surface) = self.osd.surface
        {
            self.platform.schedule_frame(surface);
        }
    }

    pub fn timeout(&self) -> Option<Duration> {
        let deadline = [self.tooltips.next_deadline(), self.osd.next_deadline()]
            .into_iter()
            .flatten()
            .reduce(f64::min)?;
        let wait = (deadline - self.now()).max(0.0);
        Some(Duration::from_secs_f64(wait / 1000.0))
    }
}

fn chuhshell() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.local/bin/chuhshell")
}

fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}
