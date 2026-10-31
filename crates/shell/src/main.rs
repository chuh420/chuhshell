mod bar;
mod logger;
mod model;
#[allow(dead_code)]
mod theme;
mod tooltip;

use std::{collections::HashMap, time::Duration, time::Instant};

use calloop::{
    EventLoop,
    channel::{self, Channel},
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use platform::{
    Damage, Edges, Event, InputRegion, KeyboardMode, Layer, LayerConfig, OutputId, Platform,
    PointerEvent, PointerKind, SurfaceId,
};
use ui::{Canvas, TextEngine, anim::Millis};

use services::{clock, hyprland, system::ResourceMonitor};

use crate::{
    bar::{Bar, BarAction},
    model::{BarModel, format_clock},
    theme::UI_SCALE,
    tooltip::Tooltips,
};

pub const APP_NAME: &str = "idk-yet";

struct BarSurface {
    surface: SurfaceId,
    view: Option<(Bar, Canvas)>,
}

struct App {
    platform: Platform,
    engine: TextEngine,
    model: BarModel,
    bars: HashMap<OutputId, BarSurface>,
    tooltips: Tooltips,
    resources: ResourceMonitor,
    keyboard_name: Option<String>,
    start: Instant,
    running: bool,
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
    fn now(&self) -> Millis {
        self.start.elapsed().as_secs_f64() * 1000.0
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
            }
            hyprland::Event::Keyboard(keyboard) => {
                if self
                    .keyboard_name
                    .as_ref()
                    .is_some_and(|name| *name != keyboard.name)
                {
                    log::debug!("keyboard layout changed to {}", keyboard.name);
                }
                self.keyboard_name = Some(keyboard.name.clone());
                self.model.keyboard_label = keyboard.label;
            }
            hyprland::Event::Raw { name, .. } => {
                if name == "configreloaded" {
                    self.model.hotkeys_paused = false;
                }
            }
        }
        self.refresh_bars();
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

    fn action(&mut self, action: BarAction) {
        let result = match action {
            BarAction::Workspace(id) => hyprland::focus_workspace(id),
            BarAction::KeyboardNext => hyprland::next_layout(),
            other => {
                log::debug!("bar action {other:?}");
                Ok(())
            }
        };
        if let Err(err) = result {
            log::warn!("{err}");
        }
    }

    fn bar_by_surface(&mut self, surface: SurfaceId) -> Option<&mut BarSurface> {
        self.bars.values_mut().find(|bar| bar.surface == surface)
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
        let Some(entry) = self.bar_by_surface(event.surface) else {
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

    fn handle(&mut self, event: Event) {
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
                self.draw_bar(surface, resized);
            }
            Event::Closed(surface) => {
                self.tooltips.closed(surface);
                self.bars.retain(|_, bar| bar.surface != surface);
            }
            Event::Frame(surface) => {
                let now = self.now();
                if self.tooltips.owns(surface) {
                    self.tooltips
                        .frame(&mut self.platform, &mut self.engine, surface, now);
                } else {
                    self.draw_bar(surface, false);
                }
            }
            Event::Pointer(event) => self.pointer(event),
        }
    }

    fn process(&mut self) {
        while self.platform.has_events() {
            for event in self.platform.take_events() {
                self.handle(event);
            }
        }
        let now = self.now();
        self.tooltips
            .poll(&mut self.platform, &mut self.engine, now);
    }

    fn timeout(&self) -> Option<Duration> {
        let deadline = self.tooltips.next_deadline()?;
        let wait = (deadline - self.now()).max(0.0);
        Some(Duration::from_secs_f64(wait / 1000.0))
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM])?;
    handle.insert_source(signals, |_, _, app: &mut App| app.running = false)?;
    let (platform, source) = Platform::connect()?;
    handle.insert_source(source, |_, queue, app: &mut App| {
        queue.dispatch_pending(&mut app.platform)
    })?;
    let (sender, events): (_, Channel<hyprland::Event>) = channel::channel();
    hyprland::spawn(move |event| {
        let _ = sender.send(event);
    })?;
    handle.insert_source(events, |event, _, app: &mut App| {
        if let channel::Event::Msg(event) = event {
            app.hyprland(event);
        }
    })?;
    handle.insert_source(Timer::immediate(), |_, _, app: &mut App| {
        app.sample_resources();
        TimeoutAction::ToDuration(Duration::from_secs(2))
    })?;
    handle.insert_source(
        Timer::from_duration(clock::until_next_minute(&clock::now())),
        |_, _, app: &mut App| {
            app.tick_clock();
            TimeoutAction::ToDuration(clock::until_next_minute(&clock::now()))
        },
    )?;
    let mut app = App {
        platform,
        engine: TextEngine::new(theme::FONT_FAMILIES),
        model: BarModel::placeholder(),
        bars: HashMap::new(),
        tooltips: Tooltips::default(),
        resources: ResourceMonitor::default(),
        keyboard_name: None,
        start: Instant::now(),
        running: true,
    };
    while app.running {
        let timeout = app.timeout();
        event_loop.dispatch(timeout, &mut app)?;
        app.process();
    }
    Ok(())
}

fn main() {
    logger::init();
    if let Err(err) = run() {
        log::error!("{err}");
        std::process::exit(1);
    }
}
