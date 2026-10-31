use platform::{
    Damage, Edges, InputRegion, KeyboardMode, Layer, LayerConfig, OutputId, Platform, SurfaceId,
};
use ui::{
    Canvas, Color, Node, NodeId, Paint, RectPaint, TextEngine, TextPaint, TextSpec, TransformOp,
    Tree,
    anim::{Easing, Millis, Sequence, Step, Tween},
    widgets::SwapText,
};

use crate::{
    state::{ShellState, osd_caption, osd_has_bar, osd_icon, osd_level},
    theme::{BG, ELEVATED, FONT, GREEN, ICONS, LINE, MUTED, NORMAL, TEXT, UI_SCALE, YELLOW},
};

const WIDTH: f32 = 300.0;
const HEIGHT: f32 = 60.0;
const ROW_X: f32 = 20.0;
const ROW_WIDTH: f32 = WIDTH - 40.0;
const SPACING: f32 = 14.0;
const ICON_WIDTH: f32 = 24.0;
const TRACK_WIDTH: f32 = ROW_WIDTH - 24.0 - 44.0 - 28.0;

pub fn layer_config(output: OutputId) -> LayerConfig {
    LayerConfig {
        namespace: "material-osd".into(),
        layer: Layer::Overlay,
        anchor: Edges {
            top: false,
            bottom: true,
            left: false,
            right: false,
        },
        margin_top: 0,
        margin_right: 0,
        margin_bottom: 72,
        margin_left: 0,
        width: (WIDTH * UI_SCALE) as u32,
        height: (HEIGHT * UI_SCALE) as u32,
        exclusive_zone: -1,
        keyboard: KeyboardMode::None,
        input: InputRegion::Empty,
        output: Some(output),
    }
}

pub struct Osd {
    pub surface: Option<SurfaceId>,
    pub output: Option<OutputId>,
    canvas: Option<Canvas>,
    tree: Tree,
    pill: NodeId,
    row: NodeId,
    icon: NodeId,
    track: NodeId,
    fill: NodeId,
    caption: SwapText,
    open: bool,
    reveal: Tween<f32>,
    hide_at: Option<Millis>,
    pulse: Sequence,
    icon_color: Tween<Color>,
    fill_width: Tween<f32>,
    fill_color: Tween<Color>,
    icon_text: String,
    icon_metrics: ui::TextMetrics,
    has_bar: bool,
    last_serial: u64,
    full: bool,
}

fn bump() -> Sequence {
    Sequence::new(
        1.0,
        vec![
            Step::To {
                value: 1.18,
                duration: 80.0,
                easing: Easing::OutQuad,
            },
            Step::Spring {
                value: 1.0,
                spring: 5.0,
                damping: 0.3,
            },
        ],
    )
}

fn icon_spec(text: &str, color: Color) -> TextSpec {
    TextSpec::plain(text, ICONS, 22.0, color)
}

impl Osd {
    pub fn new(engine: &mut TextEngine) -> Self {
        let mut tree = Tree::new();
        let root = tree.root();
        let mut pill = Node::rect(
            0.0,
            0.0,
            WIDTH,
            HEIGHT,
            RectPaint {
                color: BG,
                radius: 18.0,
                border_color: LINE,
                border_width: 1.0,
            },
        );
        pill.scale = UI_SCALE;
        pill.origin = Some((0.0, 0.0));
        pill.opacity = 0.0;
        let pill = tree.add(root, pill);
        let row = tree.add(pill, Node::new(ROW_X, 0.0, ROW_WIDTH, 0.0));
        let icon_metrics = engine.measure(&icon_spec("󰌌", GREEN), 1.0);
        let mut icon_node = Node::new(0.0, 0.0, ICON_WIDTH, icon_metrics.height);
        icon_node.origin = Some((ICON_WIDTH / 2.0, icon_metrics.height / 2.0));
        let icon = tree.add(row, icon_node);
        tree.add(
            icon,
            Node::new(0.0, 0.0, icon_metrics.width, icon_metrics.height)
                .with_paint(Paint::Text(TextPaint::new(icon_spec("󰌌", GREEN)))),
        );
        let track = tree.add(
            row,
            Node::rect(
                0.0,
                0.0,
                TRACK_WIDTH,
                6.0,
                RectPaint {
                    color: ELEVATED,
                    radius: 3.0,
                    ..RectPaint::default()
                },
            ),
        );
        let fill = tree.add(
            track,
            Node::rect(
                0.0,
                0.0,
                0.0,
                6.0,
                RectPaint {
                    color: GREEN,
                    radius: 3.0,
                    ..RectPaint::default()
                },
            ),
        );
        let mut caption = SwapText::new(&mut tree, engine, row, "", "", FONT, 14.0, TEXT, 100000.0);
        caption.set_distance(5.0);
        Self {
            surface: None,
            output: None,
            canvas: None,
            tree,
            pill,
            row,
            icon,
            track,
            fill,
            caption,
            open: false,
            reveal: Tween::new(0.0, 260.0, Easing::OUT_BACK),
            hide_at: None,
            pulse: bump(),
            icon_color: Tween::new(GREEN, NORMAL, Easing::Linear),
            fill_width: Tween::new(0.0, NORMAL, Easing::OutCubic),
            fill_color: Tween::new(GREEN, NORMAL, Easing::Linear),
            icon_text: "󰌌".into(),
            icon_metrics,
            has_bar: false,
            last_serial: 0,
            full: true,
        }
    }

    fn set_reveal(&mut self, now: Millis, to: f32) {
        let (duration, easing) = if self.open {
            (260.0, Easing::OUT_BACK)
        } else {
            (180.0, Easing::InCubic)
        };
        if self.reveal.is_running() && self.reveal.target() == to {
            return;
        }
        let current = self.reveal.value();
        let mut tween = Tween::new(current, duration, easing);
        tween.set_target(now, to);
        self.reveal = tween;
    }

    pub fn sync(&mut self, state: &ShellState, engine: &mut TextEngine, now: Millis) -> bool {
        let mut triggered = false;
        if state.osd_serial != self.last_serial {
            self.last_serial = state.osd_serial;
            self.open = true;
            self.set_reveal(now, 1.0);
            self.hide_at = Some(now + 1400.0);
            self.pulse.restart(now);
            triggered = true;
        }
        let kind = state.osd_kind.as_str();
        self.has_bar = osd_has_bar(state);
        let icon_color = if kind == "brightness" || (kind == "hotkeys" && state.hotkeys_paused) {
            YELLOW
        } else if (kind == "volume" && state.muted()) || (kind == "mic" && state.mic_muted()) {
            MUTED
        } else {
            GREEN
        };
        self.icon_color.set_target(now, icon_color);
        let icon = osd_icon(state);
        if icon != self.icon_text {
            self.icon_text = icon.to_string();
            self.icon_metrics = engine.measure(&icon_spec(icon, GREEN), 1.0);
        }
        let level = osd_level(state).min(100) as f32;
        self.fill_width.set_target(now, TRACK_WIDTH * level / 100.0);
        self.fill_color
            .set_target(now, if kind == "brightness" { YELLOW } else { GREEN });
        if self.has_bar {
            self.caption.set_font(engine, ICONS, 12.0);
        } else {
            self.caption.set_font(engine, FONT, 14.0);
        }
        self.caption.set(&osd_caption(state), "");
        self.caption.flush(engine, now);
        triggered
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        if let Some(deadline) = self.hide_at
            && now >= deadline
        {
            self.hide_at = None;
            self.open = false;
            self.set_reveal(now, 0.0);
        }
        let mut running = self.reveal.tick(now);
        running |= self.pulse.tick(now);
        running |= self.icon_color.tick(now);
        running |= self.fill_width.tick(now);
        running |= self.fill_color.tick(now);
        running |= self.caption.advance(now);
        let reveal = self.reveal.value();
        let tree = &mut self.tree;
        tree.set_opacity(self.pill, reveal.clamp(0.0, 1.0));
        let scale = 0.9 + 0.1 * reveal;
        tree.set_transforms(
            self.pill,
            &[
                TransformOp::Translate {
                    x: 0.0,
                    y: 18.0 * (1.0 - reveal),
                },
                TransformOp::Scale {
                    x: scale,
                    y: scale,
                    origin_x: 150.0,
                    origin_y: 30.0,
                },
            ],
        );
        let icon_height = self.icon_metrics.height;
        let caption_x = if self.has_bar {
            ICON_WIDTH + SPACING + TRACK_WIDTH + SPACING
        } else {
            ICON_WIDTH + SPACING
        };
        let caption_width = if self.has_bar {
            44.0
        } else {
            ROW_WIDTH - ICON_WIDTH - SPACING
        };
        let row_height = icon_height
            .max(18.0)
            .max(if self.has_bar { 6.0 } else { 0.0 });
        tree.set_pos(self.row, ROW_X, ui::layout::center(HEIGHT, row_height));
        tree.set_size(self.row, ROW_WIDTH, row_height);
        tree.set_pos(self.icon, 0.0, ui::layout::center(row_height, icon_height));
        tree.set_size(self.icon, ICON_WIDTH, icon_height);
        if let Some(node) = tree.get(self.icon)
            && node.origin != Some((ICON_WIDTH / 2.0, icon_height / 2.0))
            && let Some(node) = tree.get_mut(self.icon)
        {
            node.origin = Some((ICON_WIDTH / 2.0, icon_height / 2.0));
        }
        tree.set_scale(self.icon, self.pulse.value() as f32);
        if let Some(text) = tree
            .get(self.icon)
            .and_then(|node| node.children().first().copied())
        {
            tree.set_pos(text, (ICON_WIDTH - self.icon_metrics.width) / 2.0, 0.0);
            tree.set_size(text, self.icon_metrics.width, icon_height);
            tree.set_text(text, icon_spec(&self.icon_text, self.icon_color.value()));
        }
        tree.set_visible(self.track, self.has_bar);
        tree.set_pos(
            self.track,
            ICON_WIDTH + SPACING,
            ui::layout::center(row_height, 6.0),
        );
        tree.set_size(self.fill, self.fill_width.value(), 6.0);
        tree.set_fill(self.fill, self.fill_color.value());
        self.caption.place(
            tree,
            caption_x,
            ui::layout::center(row_height, 18.0),
            caption_width,
            18.0,
        );
        self.caption.apply(tree);
        running
    }

    pub fn finished(&self) -> bool {
        !self.open && !self.reveal.is_running() && self.reveal.value() <= 0.0
    }

    pub fn next_deadline(&self) -> Option<Millis> {
        self.hide_at
    }

    pub fn attach(&mut self, surface: SurfaceId, output: OutputId) {
        self.surface = Some(surface);
        self.output = Some(output);
        self.canvas = None;
        self.full = true;
    }

    pub fn detach(&mut self) -> Option<SurfaceId> {
        self.canvas = None;
        self.output = None;
        self.surface.take()
    }

    pub fn configure(&mut self, width: u32, height: u32) {
        self.canvas = Canvas::new(width, height);
        self.tree.mark_all();
        self.full = true;
    }

    pub fn draw(&mut self, platform: &mut Platform, engine: &mut TextEngine, now: Millis) {
        let running = self.tick(now);
        let Some(surface) = self.surface else {
            return;
        };
        let Some(canvas) = self.canvas.as_mut() else {
            return;
        };
        let mut damage = self.tree.collect_damage();
        if self.full {
            self.full = false;
            damage = Damage::full(canvas.width() as i32, canvas.height() as i32);
        }
        if damage.is_empty() {
            if running {
                platform.schedule_frame(surface);
            }
            return;
        }
        canvas.render(&mut self.tree, engine, &damage);
        platform.present(surface, canvas.data(), &damage, running);
    }
}
