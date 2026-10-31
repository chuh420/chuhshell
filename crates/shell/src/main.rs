mod bar;
mod logger;
#[allow(dead_code)]
mod theme;

use std::collections::HashMap;

use calloop::{
    EventLoop,
    signals::{Signal, Signals},
};
use platform::{
    Damage, Edges, Event, InputRegion, KeyboardMode, Layer, LayerConfig, OutputId, Platform,
    SurfaceId,
};
use ui::{Canvas, TextEngine};

use crate::{
    bar::{Bar, BarModel},
    theme::UI_SCALE,
};

struct BarSurface {
    surface: SurfaceId,
    view: Option<(Bar, Canvas)>,
}

struct App {
    platform: Platform,
    engine: TextEngine,
    model: BarModel,
    bars: HashMap<OutputId, BarSurface>,
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
                    let mut bar = Bar::new(width, height);
                    bar.rebuild(&mut self.engine, &self.model);
                    entry.view = Some((bar, canvas));
                }
                if let Some((bar, canvas)) = entry.view.as_mut() {
                    let mut damage = bar.tree.collect_damage();
                    if resized {
                        damage = Damage::full(width as i32, height as i32);
                    }
                    canvas.render(&mut bar.tree, &mut self.engine, &damage);
                    self.platform
                        .present(surface, canvas.data(), &damage, false);
                }
            }
            Event::Closed(surface) => {
                self.bars.retain(|_, bar| bar.surface != surface);
            }
            Event::Frame(_) => {}
        }
    }

    fn process(&mut self) {
        while self.platform.has_events() {
            for event in self.platform.take_events() {
                self.handle(event);
            }
        }
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
    let mut app = App {
        platform,
        engine: TextEngine::new(theme::FONT_FAMILIES),
        model: BarModel::placeholder(),
        bars: HashMap::new(),
        running: true,
    };
    while app.running {
        event_loop.dispatch(None, &mut app)?;
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
