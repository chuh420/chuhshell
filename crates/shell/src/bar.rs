use std::sync::Arc;

use platform::{MouseButton, PopupPlacement};
use ui::{
    Color, Node, NodeId, Paint, RectPaint, SvgPaint, TextEngine, TextPaint, TextSpec, TransformOp,
    Tree,
    anim::{Easing, Millis, Sequence, Step, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Item, center, plain_row, row_layout},
    widgets::{Chip, ChipStyle, SwapText},
};

use crate::{
    model::{BarModel, battery_icon, bluetooth_shortcut, network_icon},
    theme::{self, FAST, NORMAL, SLOW},
};

fn chip_style() -> ChipStyle {
    ChipStyle {
        family: crate::theme::icons(),
        hover: theme::elevated(),
        rest: ui::Color::TRANSPARENT,
    }
}
const KEYBOARD_ICON: &str = include_str!("../assets/keyboard-bar.svg");
const SLOT: f32 = 25.0;
const PAD: f32 = 3.0;
const GROUP: f32 = 5.0;
const EDGE: f32 = 3.0;
const MARK: f32 = 14.0;
const GAP: f32 = 3.0;
const ICON: f32 = 13.0;
const BIG_ICON: f32 = 15.0;

#[derive(Clone, Debug, PartialEq)]
pub enum BarAction {
    Open {
        panel: &'static str,
        anchor: Option<f64>,
    },
    Workspace(i32),
    Tray {
        key: String,
        right: bool,
        anchor: Option<f64>,
    },
    TrayScroll {
        key: String,
        delta: i32,
    },
    KeyboardNext,
    ToggleMute,
    Volume(i32),
    ToggleDnd,
    ToggleVpn,
    Media(services::media::Command),
}

pub struct TooltipTarget {
    pub key: AreaId,
    pub text: String,
    pub anchor: PopupPlacement,
}

fn chip(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    label: &str,
    ink: Color,
    shortcut: &str,
) -> Chip {
    sized_chip(tree, input, engine, parent, label, ink, shortcut, ICON)
}

#[allow(clippy::too_many_arguments)]
fn sized_chip(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    label: &str,
    ink: Color,
    shortcut: &str,
    icon_size: f32,
) -> Chip {
    let mut chip = Chip::new(
        tree,
        input,
        engine,
        parent,
        &chip_style(),
        label,
        ink,
        true,
        Some(icon_size),
    );
    chip.shortcut = shortcut.to_string();
    chip
}

fn separator(tree: &mut Tree, parent: NodeId) -> NodeId {
    tree.add(
        parent,
        Node::rect(
            0.0,
            0.0,
            1.0,
            12.0,
            RectPaint {
                color: theme::line(),
                ..RectPaint::default()
            },
        ),
    )
}

struct WorkspaceChip {
    id: i32,
    chip: Chip,
    active: bool,
    pop: Sequence,
}

fn pop_sequence() -> Sequence {
    Sequence::new(
        1.0,
        vec![
            Step::To {
                value: 0.8,
                duration: 70.0,
                easing: Easing::OutQuad,
            },
            Step::Spring {
                value: 1.0,
                spring: 5.0,
                damping: 0.28,
            },
        ],
    )
}

const PILL: f32 = 18.0;

struct Workspaces {
    root: NodeId,
    well: NodeId,
    highlight: NodeId,
    row: NodeId,
    chips: Vec<WorkspaceChip>,
    ids: Vec<i32>,
    active_id: i32,
    occupied: Vec<i32>,
    ready: bool,
    edge_left: Tween<f32>,
    edge_right: Tween<f32>,
    highlight_opacity: Tween<f32>,
    implicit_width: Tween<f32>,
}

impl Workspaces {
    fn new(tree: &mut Tree, parent: NodeId) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 22.0));
        let well = tree.add(
            root,
            Node::rect(-3.0, 0.0, 0.0, 22.0, RectPaint::solid(theme::dim(), 8.0)),
        );
        let highlight = tree.add(
            root,
            Node::rect(
                0.0,
                center(22.0, PILL),
                SLOT,
                PILL,
                RectPaint::solid(theme::green(), 6.0),
            ),
        );
        let row = tree.add(root, Node::new(0.0, 0.0, 0.0, 22.0));
        Self {
            root,
            well,
            highlight,
            row,
            chips: Vec::new(),
            ids: Vec::new(),
            active_id: 1,
            occupied: Vec::new(),
            ready: false,
            edge_left: Tween::new(0.0, 0.0, Easing::OutCubic),
            edge_right: Tween::new(SLOT, 0.0, Easing::OutCubic),
            highlight_opacity: Tween::new(1.0, FAST, Easing::Linear),
            implicit_width: Tween::new(0.0, NORMAL, Easing::OutCubic),
        }
    }

    fn width_for(count: usize) -> f32 {
        let count = count as f32;
        (count * SLOT + (count - 1.0) * GAP).max(0.0)
    }

    fn active_index(&self) -> Option<usize> {
        self.ids.iter().position(|id| *id == self.active_id)
    }

    fn place(&mut self, now: Millis) {
        let Some(index) = self.active_index() else {
            return;
        };
        let target = index as f32 * (SLOT + GAP);
        let forward = target > self.edge_left.value();
        self.edge_left
            .set_duration(if forward { 360.0 } else { 190.0 });
        self.edge_right
            .set_duration(if forward { 190.0 } else { 360.0 });
        if self.ready {
            self.edge_left.set_target(now, target);
            self.edge_right.set_target(now, target + SLOT);
        } else {
            self.edge_left.jump(target);
            self.edge_right.jump(target + SLOT);
        }
    }

    fn label(&self, id: i32) -> (String, f32, Color) {
        let active = id == self.active_id;
        let occupied = self.occupied.contains(&id);
        let label = if active {
            format!("{id:02}")
        } else if occupied {
            "●".to_string()
        } else {
            "○".to_string()
        };
        let ink = if active {
            theme::dim()
        } else if occupied {
            theme::text()
        } else {
            theme::muted()
        };
        (label, if active { 11.0 } else { 13.0 }, ink)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        computed: Vec<i32>,
        active_id: i32,
        occupied: Vec<i32>,
        now: Millis,
    ) {
        let previous_index = self.active_index();
        self.occupied = occupied;
        self.active_id = active_id;
        if computed != self.ids {
            self.ids = computed;
            let width = Self::width_for(self.ids.len());
            if self.ready {
                self.implicit_width.set_target(now, width);
            } else {
                self.implicit_width.jump(width);
            }
            let mut kept: Vec<WorkspaceChip> = Vec::new();
            for existing in self.chips.drain(..) {
                if self.ids.contains(&existing.id) {
                    kept.push(existing);
                } else {
                    input.remove(existing.chip.area);
                    tree.remove(existing.chip.root);
                }
            }
            for id in self.ids.clone() {
                if !kept.iter().any(|chip| chip.id == id) {
                    let (label, font_size, ink) = self.label(id);
                    let mut chip = chip(tree, input, engine, self.row, &label, ink, "");
                    chip.set_label(engine, &label, font_size);
                    chip.shortcut = format!("Super + {id}");
                    chip.set_fixed_width(Some(SLOT));
                    chip.color_override = Some(Color::TRANSPARENT);
                    chip.jump_color(input);
                    kept.push(WorkspaceChip {
                        id,
                        chip,
                        active: false,
                        pop: pop_sequence(),
                    });
                }
            }
            kept.sort_by_key(|chip| self.ids.iter().position(|id| *id == chip.id));
            self.chips = kept;
        }
        for index in 0..self.chips.len() {
            let id = self.chips[index].id;
            let (label, font_size, ink) = self.label(id);
            let ready = self.ready;
            let entry = &mut self.chips[index];
            entry.chip.set_label(engine, &label, font_size);
            if ready {
                entry.chip.set_ink(now, ink);
            } else {
                entry.chip.jump_ink(ink);
            }
            let active = id == active_id;
            if active && !entry.active && ready {
                entry.pop.restart(now);
            }
            entry.active = active;
        }
        if self.active_index() != previous_index || !self.ready {
            self.place(now);
        }
        if !self.ready {
            self.ready = true;
        }
    }

    fn sync(&mut self, now: Millis, input: &Input) {
        for entry in &mut self.chips {
            let hovered = entry.chip.hovered(input);
            entry.chip.color_override = Some(if hovered && !entry.active {
                theme::elevated()
            } else {
                Color::TRANSPARENT
            });
            entry.chip.sync(now, input);
        }
        let opacity = if self.active_index().is_some() {
            1.0
        } else {
            0.0
        };
        self.highlight_opacity.set_target(now, opacity);
    }

    fn implicit_width(&self) -> f32 {
        self.implicit_width.value()
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let mut running = self.edge_left.tick(now);
        running |= self.edge_right.tick(now);
        running |= self.highlight_opacity.tick(now);
        running |= self.implicit_width.tick(now);
        let left = self.edge_left.value();
        let width = self.edge_right.value() - left;
        tree.set_pos(self.highlight, left, center(22.0, PILL));
        tree.set_size(self.highlight, width, PILL);
        tree.set_opacity(self.highlight, self.highlight_opacity.value());
        tree.set_size(self.well, self.implicit_width.value() + 6.0, 22.0);
        tree.set_visible(self.well, false);
        for (index, entry) in self.chips.iter_mut().enumerate() {
            running |= entry.pop.tick(now);
            let pulse = entry.pop.value() as f32;
            entry.chip.extra = vec![TransformOp::Scale {
                x: pulse,
                y: pulse,
                origin_x: SLOT / 2.0,
                origin_y: 11.0,
            }];
            entry
                .chip
                .place(tree, index as f32 * (SLOT + GAP), 0.0, SLOT);
            running |= entry.chip.tick(tree, now);
        }
        running
    }
}

struct HoverRect {
    root: NodeId,
    area: AreaId,
    color: Tween<Color>,
    scale: Tween<f32>,
    pressed_scale: f32,
    base: Color,
    ripple: ui::widgets::Ripple,
}

impl HoverRect {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        parent: NodeId,
        width: f32,
        pressed_scale: f32,
    ) -> Self {
        let root = tree.add(
            parent,
            Node::rect(
                0.0,
                0.0,
                width,
                22.0,
                RectPaint {
                    radius: 6.0,
                    ..RectPaint::default()
                },
            ),
        );
        let area = input.add(MouseArea::new(root));
        let ripple = ui::widgets::Ripple::new(tree, root, area, 9.0, theme::text());
        Self {
            ripple,
            root,
            area,
            color: Tween::new(Color::TRANSPARENT, 140.0, Easing::Linear),
            scale: Tween::new(1.0, 110.0, Easing::OutCubic),
            pressed_scale,
            base: Color::TRANSPARENT,
        }
    }

    fn set_base(&mut self, tree: &mut Tree, base: Color, radius: f32) {
        self.base = base;
        self.color.jump(base);
        tree.set_fill(self.root, base);
        tree.set_radius(self.root, radius);
    }

    fn sync(&mut self, now: Millis, input: &Input, selected: bool) {
        self.ripple.sync(input);
        let color = if input.contains_mouse(self.area) || selected {
            theme::elevated()
        } else {
            self.base
        };
        self.color.set_target(now, color);
        let scale = if input.pressed(self.area) {
            self.pressed_scale
        } else {
            1.0
        };
        self.scale.set_target(now, scale);
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.color.tick(now) | self.scale.tick(now) | self.ripple.tick(tree, now);
        tree.set_fill(self.root, self.color.value());
        tree.set_scale(self.root, self.scale.value());
        running
    }
}

struct TrayIcon {
    key: String,
    rect: HoverRect,
    image: NodeId,
    glyph: NodeId,
    opacity: Tween<f32>,
    icon: Option<Arc<ui::tiny_skia::Pixmap>>,
}

impl TrayIcon {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        key: &str,
        now: Millis,
    ) -> Self {
        let rect = HoverRect::new(tree, input, parent, 23.0, 0.9);
        if let Some(area) = input.get_mut(rect.area) {
            area.buttons = ui::input::Buttons::LEFT_RIGHT;
            area.wheel = true;
        }
        let image = tree.add(
            rect.root,
            Node::new(center(23.0, 17.0), center(22.0, 17.0), 17.0, 17.0),
        );
        let spec = TextSpec::plain("󰀻", crate::theme::icons(), 16.0, theme::muted());
        let metrics = engine.measure(&spec, 1.0);
        let glyph = tree.add(
            rect.root,
            Node::new(
                center(23.0, metrics.width),
                center(22.0, metrics.height),
                metrics.width,
                metrics.height,
            )
            .with_paint(Paint::Text(TextPaint::new(spec))),
        );
        let mut opacity = Tween::new(0.0, NORMAL, Easing::Linear);
        opacity.start(now, 0.0, 1.0);
        tree.set_opacity(rect.root, 0.0);
        Self {
            key: key.to_string(),
            rect,
            image,
            glyph,
            opacity,
            icon: None,
        }
    }

    fn set_icon(&mut self, tree: &mut Tree, icon: Option<Arc<ui::tiny_skia::Pixmap>>) {
        let same = match (&self.icon, &icon) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.icon = icon;
        if let Some(node) = tree.get_mut(self.image) {
            node.paint = match &self.icon {
                Some(pixmap) => Paint::Image(pixmap.clone()),
                None => Paint::None,
            };
        }
        tree.set_visible(self.glyph, self.icon.is_none());
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.rect.tick(tree, now) | self.opacity.tick(now);
        tree.set_opacity(self.rect.root, self.opacity.value());
        running
    }
}

const RING: f32 = 13.0;
const CPU_WIDTH: f32 = 50.0;
const RAM_WIDTH: f32 = 72.0;

struct Gauge {
    ring: NodeId,
    text: NodeId,
    value: String,
    level: f32,
    color: Tween<Color>,
}

fn gauge_spec(value: &str) -> TextSpec {
    TextSpec::plain(value, crate::theme::icons(), 10.0, theme::text())
}

impl Gauge {
    fn new(
        tree: &mut Tree,
        engine: &mut TextEngine,
        parent: NodeId,
        x: f32,
        width: f32,
        color: Color,
    ) -> Self {
        let ring = tree.add(
            parent,
            Node::new(x, center(22.0, RING), RING, RING).with_paint(Paint::Arc(ui::ArcPaint {
                start: 0.0,
                sweep: 0.0,
                span: 360.0,
                width: 2.4,
                color,
                track: theme::line(),
            })),
        );
        let spec = gauge_spec("…");
        let metrics = engine.measure(&spec, 1.0);
        let text = tree.add(
            parent,
            Node::new(
                x + RING + 6.0,
                center(22.0, metrics.height),
                width - RING - 6.0,
                metrics.height,
            )
            .with_paint(Paint::Text(TextPaint::new(spec))),
        );
        Self {
            ring,
            text,
            value: "…".into(),
            level: 0.0,
            color: Tween::new(color, SLOW, Easing::Linear),
        }
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.color.tick(now);
        tree.set_arc(self.ring, 360.0 * self.level, self.color.value());
        tree.set_text(self.text, gauge_spec(&self.value));
        running
    }
}

struct Resources {
    rect: HoverRect,
    cpu: Gauge,
    ram: Gauge,
    width: f32,
}

impl Resources {
    fn new(tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, parent: NodeId) -> Self {
        let width = 9.0 + CPU_WIDTH + RAM_WIDTH + 5.0;
        let mut rect = HoverRect::new(tree, input, parent, width, 0.96);
        rect.set_base(tree, Color::TRANSPARENT, 9.0);
        let cpu = Gauge::new(tree, engine, rect.root, 9.0, CPU_WIDTH, theme::green());
        let ram = Gauge::new(
            tree,
            engine,
            rect.root,
            9.0 + CPU_WIDTH,
            RAM_WIDTH,
            theme::blue(),
        );
        Self {
            rect,
            cpu,
            ram,
            width,
        }
    }

    fn apply(&mut self, model: &BarModel, now: Millis) {
        self.cpu.value = model
            .cpu
            .map_or_else(|| "…".to_string(), |cpu| format!("{cpu}%"));
        self.cpu.level = model.cpu.map_or(0.0, |cpu| cpu.min(100) as f32 / 100.0);
        if model.ram_total > 0.0 {
            self.ram.value = format!("{:.1} GiB", model.ram_used);
            self.ram.level = (model.ram_used / model.ram_total).clamp(0.0, 1.0) as f32;
        } else {
            self.ram.value = "…".to_string();
            self.ram.level = 0.0;
        }
        let color = if model.cpu.is_some_and(|cpu| cpu >= 85) {
            theme::yellow()
        } else {
            theme::green()
        };
        self.cpu.color.set_target(now, color);
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let mut running = self.rect.tick(tree, now);
        running |= self.cpu.tick(tree, now);
        running |= self.ram.tick(tree, now);
        running
    }
}

const MEDIA_MAX: f32 = 210.0;

struct MediaChip {
    rect: HoverRect,
    icon: NodeId,
    label: SwapText,
    playing: bool,
    shown: bool,
    opacity: Tween<f32>,
    ink: Tween<Color>,
}

fn media_icon(playing: bool, color: Color) -> TextSpec {
    TextSpec::plain(
        if playing { "󰎈" } else { "󰏤" },
        crate::theme::icons(),
        13.0,
        color,
    )
}

impl MediaChip {
    fn new(tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, parent: NodeId) -> Self {
        let mut rect = HoverRect::new(tree, input, parent, 0.0, 0.96);
        rect.set_base(tree, Color::TRANSPARENT, 9.0);
        if let Some(area) = input.get_mut(rect.area) {
            area.buttons = ui::input::Buttons::LEFT_RIGHT;
        }
        let spec = media_icon(true, theme::green());
        let metrics = engine.measure(&spec, 1.0);
        let icon = tree.add(
            rect.root,
            Node::new(
                9.0,
                center(22.0, metrics.height),
                metrics.width,
                metrics.height,
            )
            .with_paint(Paint::Text(TextPaint::new(spec))),
        );
        let label = SwapText::new(
            tree,
            engine,
            rect.root,
            "",
            "",
            crate::theme::font(),
            10.0,
            theme::text(),
            MEDIA_MAX,
        );
        tree.set_opacity(rect.root, 0.0);
        Self {
            rect,
            icon,
            label,
            playing: false,
            shown: false,
            opacity: Tween::new(0.0, NORMAL, Easing::Linear),
            ink: Tween::new(theme::muted(), NORMAL, Easing::Linear),
        }
    }

    fn apply(&mut self, engine: &mut TextEngine, media: &Option<(String, bool)>, now: Millis) {
        self.shown = media.is_some();
        self.opacity
            .set_target(now, if self.shown { 1.0 } else { 0.0 });
        if let Some((text, playing)) = media {
            self.playing = *playing;
            self.label.set(text, "");
            self.label.flush(engine, now);
        }
        self.ink.set_target(
            now,
            if self.playing {
                theme::green()
            } else {
                theme::muted()
            },
        );
    }

    fn width(&self) -> f32 {
        9.0 + 14.0 + 6.0 + self.label.implicit_width().ceil() + 10.0
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis, x: f32, y: f32) -> bool {
        let mut running = self.rect.tick(tree, now);
        running |= self.opacity.tick(now);
        running |= self.ink.tick(now);
        running |= self.label.advance(now);
        let width = self.width();
        let visible = self.opacity.value() > 0.0;
        tree.set_visible(self.rect.root, visible);
        tree.set_opacity(self.rect.root, self.opacity.value());
        tree.set_pos(self.rect.root, x, y);
        tree.set_size(self.rect.root, width, 22.0);
        tree.set_text(self.icon, media_icon(self.playing, self.ink.value()));
        let height = self.label.implicit_height().ceil();
        self.label.place(
            tree,
            29.0,
            center(22.0, height),
            (width - 39.0).max(0.0),
            height,
        );
        self.label.apply(tree);
        running
    }
}

struct Clock {
    rect: HoverRect,
    label: SwapText,
}

impl Clock {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        text: &str,
    ) -> Self {
        let rect = HoverRect::new(tree, input, parent, 0.0, 0.96);
        let mut label = SwapText::new(
            tree,
            engine,
            rect.root,
            text,
            "",
            crate::theme::icons(),
            11.0,
            theme::text(),
            100000.0,
        );
        label.set_distance(6.0);
        Self { rect, label }
    }

    fn width(&self) -> f32 {
        self.label.implicit_width().ceil() + 20.0
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32) {
        let width = self.width();
        let height = self.label.implicit_height().ceil();
        tree.set_pos(self.rect.root, x, y);
        tree.set_size(self.rect.root, width, 22.0);
        self.label
            .place(tree, 10.0, center(22.0, height), width - 20.0, height);
        self.label.apply(tree);
    }
}

struct Keyboard {
    rect: HoverRect,
    label: SwapText,
}

impl Keyboard {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        icon: Option<Arc<ui::SvgTree>>,
        label: &str,
    ) -> Self {
        let rect = HoverRect::new(tree, input, parent, 49.0, 0.94);
        let layout = plain_row(&[Item::new(16.0, 12.0), Item::new(16.0, 14.0)], 5.0);
        let row = tree.add(
            rect.root,
            Node::new(
                center(49.0, layout.width),
                center(22.0, layout.height),
                layout.width,
                layout.height,
            ),
        );
        if let Some(icon) = icon {
            tree.add(
                row,
                Node::new(0.0, center(layout.height, 12.0), 16.0, 12.0)
                    .with_paint(Paint::Svg(SvgPaint::new(icon))),
            );
        }
        let mut label = SwapText::new(
            tree,
            engine,
            row,
            label,
            "",
            crate::theme::icons(),
            10.0,
            theme::text(),
            100000.0,
        );
        label.place(tree, 21.0, center(layout.height, 14.0), 16.0, 14.0);
        Self { rect, label }
    }
}

pub struct Bar {
    pub tree: Tree,
    pub input: Input,
    body: NodeId,
    enter: Tween<f32>,
    width: f32,
    height: f32,
    left_row: NodeId,
    launcher: Chip,
    left_separator: NodeId,
    workspaces: Workspaces,
    title: SwapText,
    clock: Clock,
    media: MediaChip,
    right_row: NodeId,
    islands: [NodeId; 8],
    tray: Vec<TrayIcon>,
    resources: Resources,
    keyboard: Keyboard,
    network: Chip,
    vpn: Chip,
    bluetooth: Chip,
    audio: Chip,
    battery: Chip,
    battery_visible: bool,
    network_visible: bool,
    vpn_visible: bool,
    bluetooth_visible: bool,
    battery_plugged: Option<bool>,
    battery_pulse: Sequence,
    right_separator: NodeId,
    paused: Chip,
    paused_visible: bool,
    bell: Chip,
    bell_ring: Sequence,
    control: Chip,
    panel: String,
}

impl Bar {
    pub fn new(
        engine: &mut TextEngine,
        width: u32,
        height: u32,
        model: &BarModel,
        now: Millis,
    ) -> Self {
        let mut tree = Tree::new();
        let mut input = Input::new();
        let body_width = width as f32 / crate::theme::scale();
        let body_height = height as f32 / crate::theme::scale();
        let mut body = Node::rect(
            0.0,
            0.0,
            body_width,
            body_height,
            RectPaint::solid(theme::bg(), 10.0),
        );
        body.scale = crate::theme::scale();
        body.origin = Some((0.0, 0.0));
        body.opacity = 0.0;
        let root = tree.root();
        let body = tree.add(root, body);
        let islands = std::array::from_fn(|_| {
            tree.add(
                body,
                Node::rect(
                    0.0,
                    0.0,
                    1.0,
                    MARK,
                    RectPaint {
                        color: theme::line(),
                        ..RectPaint::default()
                    },
                ),
            )
        });
        let left_row = tree.add(body, Node::new(5.0, 0.0, 0.0, 22.0));
        let launcher = sized_chip(
            &mut tree,
            &mut input,
            engine,
            left_row,
            "󰣇",
            theme::green(),
            "Super + D",
            BIG_ICON,
        );
        let left_separator = separator(&mut tree, left_row);
        let workspaces = Workspaces::new(&mut tree, left_row);
        let max_width = 70.0f32.max(body_width * 0.16);
        let title_text = title_text(model);
        let title = SwapText::new(
            &mut tree,
            engine,
            left_row,
            &title_text,
            "none",
            crate::theme::font(),
            10.0,
            theme::muted(),
            max_width,
        );
        let clock = Clock::new(&mut tree, &mut input, engine, body, &model.clock);
        let media = MediaChip::new(&mut tree, &mut input, engine, body);
        let right_row = tree.add(body, Node::new(0.0, 0.0, 0.0, 22.0));
        let resources = Resources::new(&mut tree, &mut input, engine, right_row);
        let keyboard = Keyboard::new(
            &mut tree,
            &mut input,
            engine,
            right_row,
            ui::image::svg_from_data(keyboard_icon().as_bytes(), "keyboard-bar.svg"),
            &model.keyboard_label,
        );
        let network = sized_chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            network_icon(model),
            theme::green(),
            "",
            BIG_ICON,
        );
        let vpn = sized_chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰒙",
            theme::muted(),
            "",
            BIG_ICON,
        );
        let bluetooth = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰂯",
            theme::green(),
            "",
        );
        let audio = sized_chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰕾",
            theme::text(),
            "XF86AudioRaiseVolume / XF86AudioLowerVolume / XF86AudioMute",
            BIG_ICON,
        );
        let battery = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "",
            theme::text(),
            "",
        );
        let right_separator = separator(&mut tree, right_row);
        let paused = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰌐",
            theme::yellow(),
            "hotkeys paused · Super + Escape",
        );
        let bell = sized_chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰂚",
            theme::yellow(),
            "Super + N",
            BIG_ICON,
        );
        let control = sized_chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰒓",
            theme::green(),
            "Super + Space",
            BIG_ICON,
        );
        let (mut network, mut vpn, mut bluetooth, mut audio, mut bell) =
            (network, vpn, bluetooth, audio, bell);
        for chip in [
            &mut network,
            &mut vpn,
            &mut bluetooth,
            &mut audio,
            &mut bell,
        ] {
            chip.pop_on_change = true;
        }
        let mut enter = Tween::new(0.0, 700.0, Easing::SPATIAL_SLOW);
        enter.start(now, 0.0, 1.0);
        let mut bar = Self {
            tree,
            input,
            body,
            enter,
            width: body_width,
            height: body_height,
            left_row,
            launcher,
            left_separator,
            workspaces,
            title,
            clock,
            media,
            right_row,
            islands,
            tray: Vec::new(),
            resources,
            keyboard,
            network,
            vpn,
            bluetooth,
            audio,
            battery,
            battery_visible: false,
            network_visible: true,
            vpn_visible: false,
            bluetooth_visible: true,
            battery_plugged: None,
            battery_pulse: Sequence::new(
                1.0,
                vec![
                    Step::To {
                        value: 1.18,
                        duration: 120.0,
                        easing: Easing::OutQuad,
                    },
                    Step::Spring {
                        value: 1.0,
                        spring: 4.0,
                        damping: 0.22,
                    },
                ],
            ),
            right_separator,
            paused,
            paused_visible: false,
            bell,
            bell_ring: Sequence::new(
                0.0,
                [
                    (16.0, 70.0),
                    (-12.0, 110.0),
                    (7.0, 100.0),
                    (-3.0, 90.0),
                    (0.0, 80.0),
                ]
                .into_iter()
                .map(|(value, duration)| Step::To {
                    value,
                    duration,
                    easing: Easing::Linear,
                })
                .collect(),
            ),
            control,
            panel: String::new(),
        };
        bar.apply(engine, model, now, true);
        bar
    }

    pub fn apply(&mut self, engine: &mut TextEngine, model: &BarModel, now: Millis, initial: bool) {
        self.panel = model.panel.clone();
        let computed = workspace_ids(services::compositor::kind(), model);
        self.workspaces.apply(
            &mut self.tree,
            &mut self.input,
            engine,
            computed,
            model.active_workspace,
            model.occupied.clone(),
            now,
        );
        self.title.set(&title_text(model), &model.title_key);
        self.title.flush(engine, now);
        self.clock.label.set(&model.clock, "");
        self.clock.label.flush(engine, now);
        self.media.apply(engine, &model.media, now);
        self.resources.apply(model, now);
        self.keyboard.label.set(&model.keyboard_label, "");
        self.keyboard.label.flush(engine, now);

        let set = |chip: &mut Chip, engine: &mut TextEngine, label: &str, ink: Color| {
            chip.set_label(engine, label, 11.0);
            if initial {
                chip.jump_ink(ink);
            } else {
                chip.set_ink(now, ink);
            }
        };
        set(
            &mut self.network,
            engine,
            network_icon(model),
            if model.network_connected {
                theme::green()
            } else {
                theme::muted()
            },
        );
        let (vpn_icon, vpn_ink) = match model.vpn {
            crate::vpnctl::VpnState::On => ("󰕥", theme::green()),
            crate::vpnctl::VpnState::Connecting | crate::vpnctl::VpnState::Disconnecting => {
                ("󰞀", theme::yellow())
            }
            crate::vpnctl::VpnState::Off => ("󰒙", theme::muted()),
        };
        set(&mut self.vpn, engine, vpn_icon, vpn_ink);
        self.vpn.shortcut = model.vpn_tooltip.clone();
        self.vpn_visible = model.available.vpn;
        let bluetooth_icon = if !model.bluetooth_connected.is_empty() {
            "󰂱"
        } else if model.bluetooth_powered {
            "󰂯"
        } else {
            "󰂲"
        };
        set(
            &mut self.bluetooth,
            engine,
            bluetooth_icon,
            if model.bluetooth_powered {
                theme::green()
            } else {
                theme::muted()
            },
        );
        self.bluetooth.shortcut = bluetooth_shortcut(&model.bluetooth_connected);
        let audio_icon = if model.muted {
            "󰖁"
        } else if model.volume > 66 {
            "󰕾"
        } else if model.volume > 33 {
            "󰖀"
        } else {
            "󰕿"
        };
        set(
            &mut self.audio,
            engine,
            audio_icon,
            if model.muted {
                theme::muted()
            } else {
                theme::text()
            },
        );
        let battery_label = format!(
            "{} {}%",
            battery_icon(model.battery_level, model.battery_plugged),
            model.battery_level
        );
        let battery_ink = if model.battery_plugged {
            theme::green()
        } else if model.battery_level < 20 {
            theme::red()
        } else {
            theme::text()
        };
        set(&mut self.battery, engine, &battery_label, battery_ink);
        self.battery_visible = model.battery_present;
        self.network_visible = model.available.wifi || model.network_connected;
        self.bluetooth_visible = model.available.bluetooth;
        self.sync_tray(engine, model, now);
        if let Some(previous) = self.battery_plugged
            && previous != model.battery_plugged
        {
            self.battery_pulse.restart(now);
        }
        self.battery_plugged = Some(model.battery_plugged);
        self.paused_visible = model.hotkeys_paused;
        let bell_icon = if model.dnd {
            "󰂛"
        } else if model.unread_notifications > 0 {
            "󱅫"
        } else {
            "󰂚"
        };
        set(
            &mut self.bell,
            engine,
            bell_icon,
            if model.dnd {
                theme::muted()
            } else {
                theme::yellow()
            },
        );
        self.launcher.selected = self.panel == "launcher";
        self.network.selected = self.panel == "wifi";
        self.vpn.selected = self.panel == "vpn";
        self.bluetooth.selected = self.panel == "bluetooth";
        self.audio.selected = self.panel == "audio";
        self.battery.selected = self.panel == "battery";
        self.bell.selected = self.panel == "notifications";
        self.control.selected = self.panel == "control";
        if initial {
            for chip in self.chips_mut() {
                chip.jump_color(&Input::new());
            }
        }
        self.sync(now);
    }

    pub fn ring_bell(&mut self, now: Millis) {
        self.bell_ring.restart(now);
    }

    fn chips_mut(&mut self) -> [&mut Chip; 9] {
        [
            &mut self.launcher,
            &mut self.network,
            &mut self.vpn,
            &mut self.bluetooth,
            &mut self.audio,
            &mut self.battery,
            &mut self.paused,
            &mut self.bell,
            &mut self.control,
        ]
    }

    fn sync_tray(&mut self, engine: &mut TextEngine, model: &BarModel, now: Millis) {
        let keys: Vec<&String> = model.tray.iter().map(|(key, _)| key).collect();
        let current: Vec<&String> = self.tray.iter().map(|icon| &icon.key).collect();
        if keys != current {
            let mut old = std::mem::take(&mut self.tray);
            for (key, _) in &model.tray {
                match old.iter().position(|icon| &icon.key == key) {
                    Some(index) => self.tray.push(old.remove(index)),
                    None => self.tray.push(TrayIcon::new(
                        &mut self.tree,
                        &mut self.input,
                        engine,
                        self.right_row,
                        key,
                        now,
                    )),
                }
            }
            for icon in old {
                self.input.remove(icon.rect.area);
                self.tree.remove(icon.rect.root);
            }
        }
        for (icon, (_, pixmap)) in self.tray.iter_mut().zip(&model.tray) {
            icon.set_icon(&mut self.tree, pixmap.clone());
        }
    }

    fn chips(&self) -> [&Chip; 9] {
        [
            &self.launcher,
            &self.network,
            &self.vpn,
            &self.bluetooth,
            &self.audio,
            &self.battery,
            &self.paused,
            &self.bell,
            &self.control,
        ]
    }

    fn sync(&mut self, now: Millis) {
        let input = &self.input;
        for chip in [
            &mut self.launcher,
            &mut self.network,
            &mut self.vpn,
            &mut self.bluetooth,
            &mut self.audio,
            &mut self.battery,
            &mut self.paused,
            &mut self.bell,
            &mut self.control,
        ] {
            chip.sync(now, input);
        }
        self.workspaces.sync(now, input);
        let system = self.panel == "system";
        self.resources.rect.sync(now, input, system);
        self.clock.rect.sync(now, input, self.panel == "calendar");
        self.media.rect.sync(now, input, false);
        self.keyboard.rect.sync(now, input, false);
        for icon in &mut self.tray {
            icon.rect.sync(now, input, false);
        }
    }

    pub fn handle(&mut self, events: &[InputEvent], now: Millis) -> Vec<BarAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Click(area, button) => {
                    if let Some(action) = self.click_action(area, button) {
                        actions.push(action);
                    }
                }
                InputEvent::Wheel(area, delta) if area == self.audio.area => {
                    actions.push(BarAction::Volume(delta));
                }
                InputEvent::Wheel(area, delta) => {
                    if let Some(icon) = self.tray.iter().find(|icon| icon.rect.area == area) {
                        actions.push(BarAction::TrayScroll {
                            key: icon.key.clone(),
                            delta,
                        });
                    }
                }
                _ => {}
            }
        }
        self.sync(now);
        actions
    }

    fn anchor_x(&self, area: AreaId) -> Option<f64> {
        let node = self.input.node(area)?;
        let width = self.tree.get(node)?.width;
        let mut point = ui::tiny_skia::Point::from_xy(width / 2.0, 0.0);
        self.tree.device_transform(node).map_point(&mut point);
        Some(point.x as f64 + (10.0 * crate::theme::scale()).round() as f64)
    }

    fn open(&self, panel: &'static str, area: AreaId) -> Option<BarAction> {
        Some(BarAction::Open {
            panel,
            anchor: self.anchor_x(area),
        })
    }

    fn click_action(&self, area: AreaId, button: MouseButton) -> Option<BarAction> {
        let right = button == MouseButton::Right;
        if area == self.launcher.area && !right {
            return Some(BarAction::Open {
                panel: "launcher",
                anchor: None,
            });
        }
        if let Some(icon) = self.tray.iter().find(|icon| icon.rect.area == area) {
            return Some(BarAction::Tray {
                key: icon.key.clone(),
                right,
                anchor: self.anchor_x(area),
            });
        }
        if let Some(entry) = self
            .workspaces
            .chips
            .iter()
            .find(|entry| entry.chip.area == area)
        {
            return (!right).then_some(BarAction::Workspace(entry.id));
        }
        if area == self.media.rect.area {
            return self.media.shown.then_some(BarAction::Media(if right {
                services::media::Command::Next
            } else {
                services::media::Command::Toggle
            }));
        }
        if area == self.clock.rect.area && !right {
            return self.open("calendar", area);
        }
        if area == self.resources.rect.area {
            return self.open("system", area);
        }
        if area == self.keyboard.rect.area {
            return Some(BarAction::KeyboardNext);
        }
        if area == self.network.area && !right {
            return self.open("wifi", area);
        }
        if area == self.vpn.area {
            return if right {
                Some(BarAction::ToggleVpn)
            } else {
                self.open("vpn", area)
            };
        }
        if area == self.bluetooth.area && !right {
            return self.open("bluetooth", area);
        }
        if area == self.audio.area {
            return if right {
                Some(BarAction::ToggleMute)
            } else {
                self.open("audio", area)
            };
        }
        if area == self.battery.area && !right {
            return self.open("battery", area);
        }
        if area == self.bell.area {
            return if right {
                Some(BarAction::ToggleDnd)
            } else {
                self.open("notifications", area)
            };
        }
        if area == self.control.area && !right {
            return Some(BarAction::Open {
                panel: "control",
                anchor: None,
            });
        }
        None
    }

    pub fn tooltip(&self) -> Option<TooltipTarget> {
        let candidates = self
            .chips()
            .into_iter()
            .chain(self.workspaces.chips.iter().map(|entry| &entry.chip));
        for chip in candidates {
            if chip.shortcut.is_empty() || !chip.hovered(&self.input) {
                continue;
            }
            let transform = self.tree.device_transform(chip.root);
            let mut origin = ui::tiny_skia::Point::from_xy(0.0, 0.0);
            transform.map_point(&mut origin);
            return Some(TooltipTarget {
                key: chip.area,
                text: chip.shortcut.clone(),
                anchor: PopupPlacement {
                    anchor_x: origin.x.floor() as i32,
                    anchor_y: origin.y.floor() as i32,
                    anchor_width: chip.width().ceil() as i32,
                    anchor_height: chip.implicit_height().ceil() as i32,
                    width: 0,
                    height: 0,
                },
            });
        }
        None
    }

    fn layout(&mut self) {
        let tree = &mut self.tree;
        let height = self.height;
        let mark = |tree: &mut Tree, node: NodeId, x: Option<f32>| {
            tree.set_visible(node, x.is_some());
            if let Some(x) = x {
                tree.set_pos(node, x.round(), center(height, MARK));
                tree.set_size(node, 1.0, MARK);
            }
        };
        let title_width = self.title.implicit_width();
        let left_items = [
            Item::new(self.launcher.implicit_width(), 22.0),
            Item::new(self.workspaces.implicit_width(), 22.0).margins(PAD + GROUP, 0.0),
            Item::preferred(title_width, 16.0).margins(PAD + GROUP + 11.0, 0.0),
        ];
        let left = row_layout(&left_items, 2.0);
        tree.set_pos(self.left_row, EDGE + PAD, center(height, left.height));
        tree.set_size(self.left_row, left.width, left.height);
        let p = &left.positions;
        self.launcher
            .place(tree, p[0].0, p[0].1, left_items[0].layout_width());
        tree.set_visible(self.left_separator, false);
        tree.set_pos(self.workspaces.root, p[1].0, p[1].1);
        tree.set_size(self.workspaces.root, left_items[1].layout_width(), 22.0);
        tree.set_size(self.workspaces.row, left_items[1].layout_width(), 22.0);
        self.title.place(tree, p[2].0, p[2].1, title_width, 16.0);
        let launcher_end = EDGE + PAD + p[0].0 + left_items[0].layout_width();
        mark(
            tree,
            self.islands[0],
            Some((launcher_end + EDGE + PAD + p[1].0) / 2.0),
        );
        let workspaces_end = EDGE + PAD + p[1].0 + left_items[1].layout_width();
        mark(
            tree,
            self.islands[1],
            (title_width > 0.0).then_some((workspaces_end + EDGE + PAD + p[2].0) / 2.0),
        );

        let clock_width = self.clock.width();
        let clock_x = center(self.width, clock_width).round();
        self.clock.place(tree, clock_x, center(height, 22.0));
        mark(
            tree,
            self.islands[2],
            Some(clock_x + clock_width + PAD + GROUP / 2.0),
        );
        mark(tree, self.islands[3], None);

        enum Slot {
            Tray(usize),
            Resources,
            Keyboard,
            Network,
            Vpn,
            Bluetooth,
            Audio,
            Battery,
            Paused,
            Bell,
            Control,
        }
        let gap = PAD * 2.0 + GROUP;
        let mut slots: Vec<(Slot, usize, Item)> = (0..self.tray.len())
            .map(|index| (Slot::Tray(index), 0, Item::new(23.0, 22.0)))
            .collect();
        let mut push = |slot: Slot, group: usize, width: f32| {
            let first = slots.last().is_some_and(|(_, last, _)| *last != group);
            slots.push((
                slot,
                group,
                Item::new(width, 22.0).margins(if first { gap - 2.0 } else { 0.0 }, 0.0),
            ));
        };
        push(Slot::Resources, 1, self.resources.width);
        push(Slot::Keyboard, 2, 49.0);
        if self.network_visible {
            push(Slot::Network, 2, self.network.implicit_width());
        }
        if self.vpn_visible {
            push(Slot::Vpn, 2, self.vpn.implicit_width());
        }
        if self.bluetooth_visible {
            push(Slot::Bluetooth, 2, self.bluetooth.implicit_width());
        }
        push(Slot::Audio, 2, self.audio.implicit_width());
        if self.battery_visible {
            push(Slot::Battery, 2, self.battery.implicit_width());
        }
        if self.paused_visible {
            push(Slot::Paused, 3, self.paused.implicit_width());
        }
        push(Slot::Bell, 3, self.bell.implicit_width());
        push(Slot::Control, 3, self.control.implicit_width());
        let items: Vec<Item> = slots.iter().map(|(_, _, item)| *item).collect();
        let right = row_layout(&items, 2.0);
        let right_x = self.width - EDGE - PAD - right.width;
        tree.set_pos(self.right_row, right_x, center(height, right.height));
        tree.set_size(self.right_row, right.width, right.height);
        tree.set_visible(self.battery.root, self.battery_visible);
        tree.set_visible(self.network.root, self.network_visible);
        tree.set_visible(self.vpn.root, self.vpn_visible);
        tree.set_visible(self.bluetooth.root, self.bluetooth_visible);
        tree.set_visible(self.paused.root, self.paused_visible);
        tree.set_visible(self.right_separator, false);
        let mut groups = [(f32::MAX, 0.0f32); 4];
        for ((slot, group, item), (x, y)) in slots.iter().zip(right.positions.iter().copied()) {
            let width = item.layout_width();
            groups[*group] = (groups[*group].0.min(x), groups[*group].1.max(x + width));
            match slot {
                Slot::Tray(index) => tree.set_pos(self.tray[*index].rect.root, x, y),
                Slot::Resources => tree.set_pos(self.resources.rect.root, x, y),
                Slot::Keyboard => tree.set_pos(self.keyboard.rect.root, x, y),
                Slot::Network => self.network.place(tree, x, y, width),
                Slot::Vpn => self.vpn.place(tree, x, y, width),
                Slot::Bluetooth => self.bluetooth.place(tree, x, y, width),
                Slot::Audio => self.audio.place(tree, x, y, width),
                Slot::Battery => self.battery.place(tree, x, y, width),
                Slot::Paused => self.paused.place(tree, x, y, width),
                Slot::Bell => self.bell.place(tree, x, y, width),
                Slot::Control => self.control.place(tree, x, y, width),
            }
        }
        let mut seen = false;
        for (index, (from, _)) in groups.into_iter().enumerate() {
            let present = from < f32::MAX;
            mark(
                tree,
                self.islands[4 + index],
                (present && seen).then_some(right_x + from - PAD - GROUP / 2.0),
            );
            seen |= present;
        }
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        let mut running = self.enter.tick(now);
        let enter = self.enter.value();
        self.tree
            .set_opacity(self.body, (enter * 2.0).clamp(0.0, 1.0));
        self.tree.set_transforms(
            self.body,
            &[TransformOp::Translate {
                x: 0.0,
                y: -26.0 * (1.0 - enter).max(0.0),
            }],
        );
        running |= self.workspaces.tick(&mut self.tree, now);
        running |= self.title.advance(now);
        running |= self.resources.tick(&mut self.tree, now);
        running |= self.clock.rect.tick(&mut self.tree, now);
        running |= self.clock.label.advance(now);
        let media_x = (center(self.width, self.clock.width()) + self.clock.width()).round()
            + PAD * 2.0
            + GROUP;
        running |= self
            .media
            .tick(&mut self.tree, now, media_x, center(self.height, 22.0));
        running |= self.keyboard.rect.tick(&mut self.tree, now);
        for icon in &mut self.tray {
            running |= icon.tick(&mut self.tree, now);
        }
        running |= self.keyboard.label.tick(&mut self.tree, now);
        running |= self.battery_pulse.tick(now);
        let pulse = self.battery_pulse.value() as f32;
        let battery_width = self.battery.width();
        self.battery.extra = vec![TransformOp::Scale {
            x: pulse,
            y: pulse,
            origin_x: battery_width / 2.0,
            origin_y: 11.0,
        }];
        running |= self.bell_ring.tick(now);
        let bell_width = self.bell.width();
        self.bell.extra = vec![TransformOp::Rotate {
            angle: self.bell_ring.value() as f32,
            origin_x: bell_width / 2.0,
            origin_y: 3.0,
        }];
        self.layout();
        self.title.apply(&mut self.tree);
        let tree = &mut self.tree;
        for chip in [
            &mut self.launcher,
            &mut self.network,
            &mut self.vpn,
            &mut self.bluetooth,
            &mut self.audio,
            &mut self.battery,
            &mut self.paused,
            &mut self.bell,
            &mut self.control,
        ] {
            running |= chip.tick(tree, now);
        }
        running
    }
}

fn title_text(model: &BarModel) -> String {
    model
        .title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "still / material".into())
}

fn keyboard_icon() -> String {
    KEYBOARD_ICON
        .replace("#a9b665", &theme::hex(theme::green()))
        .replace("#d4be98", &theme::hex(theme::text()))
}

fn workspace_ids(kind: Option<services::compositor::Kind>, model: &BarModel) -> Vec<i32> {
    match kind {
        Some(kind) if kind.fixed_workspaces() => {
            let fixed = config::current().bar.workspaces as i32;
            let mut computed: Vec<i32> = (1..=fixed).collect();
            let mut extra: Vec<i32> = model
                .workspaces
                .iter()
                .copied()
                .filter(|id| *id > fixed)
                .collect();
            extra.sort_unstable();
            extra.dedup();
            computed.extend(extra);
            computed
        }
        Some(_) => {
            let mut ids = model.workspaces.clone();
            ids.sort_unstable();
            ids.dedup();
            ids
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::font_families;

    #[test]
    fn launcher_tooltip_anchor_matches_qt_positioner() {
        let mut engine = TextEngine::new(&font_families());
        let model = BarModel::placeholder();
        let mut bar = Bar::new(&mut engine, 1896, 35, &model, 0.0);
        bar.tick(1000.0);
        let events = bar.input.motion(&bar.tree, 20.0, 15.0);
        bar.handle(&events, 1000.0);
        let target = bar.tooltip().expect("launcher hovered");
        assert_eq!(target.text, "Super + D");
        assert_eq!(
            (
                target.anchor.anchor_x,
                target.anchor.anchor_y,
                target.anchor.anchor_width,
                target.anchor.anchor_height
            ),
            (7, 3, 23, 22)
        );
    }

    #[test]
    fn clock_has_no_tooltip() {
        let mut engine = TextEngine::new(&font_families());
        let model = BarModel::placeholder();
        let mut bar = Bar::new(&mut engine, 1896, 35, &model, 0.0);
        bar.tick(1000.0);
        let events = bar.input.motion(&bar.tree, 948.0, 17.0);
        bar.handle(&events, 1000.0);
        assert!(bar.tooltip().is_none());
    }
}
