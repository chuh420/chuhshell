use crate::{
    Color, Node, NodeId, Paint, RectPaint, TextEngine, TextPaint, TextSpec, TransformOp, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Buttons, Input, MouseArea},
    layout::center,
    rich::chip_spans,
    text::{Span, TextMetrics},
};

pub const UI_SCALE: f32 = 1.22;
const FAST: Millis = 140.0;
const NORMAL: Millis = 220.0;
const SLOW: Millis = 340.0;

pub struct ChipStyle {
    pub family: &'static str,
    pub hover: Color,
}

pub struct Chip {
    pub root: NodeId,
    text: NodeId,
    pub area: AreaId,
    family: &'static str,
    hover_color: Color,
    label: String,
    pub compact: bool,
    font_size: f32,
    icon_size: f32,
    fixed_width: Option<f32>,
    metrics: TextMetrics,
    width: f32,
    height: f32,
    ink: Tween<Color>,
    color: Tween<Color>,
    scale: Tween<f32>,
    pub selected: bool,
    pub color_override: Option<Color>,
    pub shortcut: String,
    pub extra: Vec<TransformOp>,
}

impl Chip {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        style: &ChipStyle,
        label: &str,
        ink: Color,
        compact: bool,
        icon_size: Option<f32>,
    ) -> Self {
        let font_size = if compact { 11.0 } else { 12.0 };
        let height = if compact { 22.0 } else { 30.0 };
        let icon_size = icon_size.unwrap_or(font_size);
        let root = tree.add(
            parent,
            Node::rect(
                0.0,
                0.0,
                0.0,
                height,
                RectPaint {
                    radius: if compact { 6.0 } else { 10.0 },
                    ..RectPaint::default()
                },
            ),
        );
        let spec = TextSpec {
            spans: chip_spans(label, style.family, font_size, icon_size),
            color: ink,
            elide_width: None,
        };
        let metrics = engine.measure(&spec, 1.0);
        let text = tree.add(
            root,
            Node::new(0.0, 0.0, metrics.width, metrics.height)
                .with_paint(Paint::Text(TextPaint::new(spec))),
        );
        let mut area = MouseArea::new(root);
        area.buttons = Buttons::LEFT_RIGHT;
        area.wheel = true;
        let area = input.add(area);
        let mut chip = Self {
            root,
            text,
            area,
            family: style.family,
            hover_color: style.hover,
            label: label.to_string(),
            compact,
            font_size,
            icon_size,
            fixed_width: None,
            metrics,
            width: 0.0,
            height,
            ink: Tween::new(ink, NORMAL, Easing::Linear),
            color: Tween::new(Color::TRANSPARENT, 140.0, Easing::Linear),
            scale: Tween::new(1.0, 110.0, Easing::OutCubic),
            selected: false,
            color_override: None,
            shortcut: String::new(),
            extra: Vec::new(),
        };
        chip.width = chip.implicit_width();
        chip
    }

    pub fn implicit_width(&self) -> f32 {
        self.fixed_width
            .unwrap_or(self.metrics.width + if self.compact { 16.0 } else { 24.0 })
    }

    pub fn implicit_height(&self) -> f32 {
        self.height
    }

    pub fn set_fixed_width(&mut self, width: Option<f32>) {
        self.fixed_width = width;
    }

    fn spec(&self) -> TextSpec {
        TextSpec {
            spans: chip_spans(&self.label, self.family, self.font_size, self.icon_size),
            color: self.ink.value(),
            elide_width: None,
        }
    }

    pub fn set_label(&mut self, engine: &mut TextEngine, label: &str, font_size: f32) {
        if self.label == label && self.font_size == font_size {
            return;
        }
        self.label = label.to_string();
        self.font_size = font_size;
        self.metrics = engine.measure(&self.spec(), 1.0);
    }

    pub fn set_ink(&mut self, now: Millis, ink: Color) {
        self.ink.set_target(now, ink);
    }

    pub fn jump_ink(&mut self, ink: Color) {
        self.ink.jump(ink);
    }

    pub fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32) {
        self.width = width;
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, self.height);
        tree.set_pos(
            self.text,
            center(width, self.metrics.width),
            center(self.height, self.metrics.height),
        );
        tree.set_size(self.text, self.metrics.width, self.metrics.height);
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn hovered(&self, input: &Input) -> bool {
        input.contains_mouse(self.area)
    }

    pub fn sync(&mut self, now: Millis, input: &Input) {
        let hovered = input.contains_mouse(self.area);
        let color = self.color_override.unwrap_or(if self.selected || hovered {
            self.hover_color
        } else {
            Color::TRANSPARENT
        });
        self.color.set_target(now, color);
        let scale = if input.pressed(self.area) { 0.94 } else { 1.0 };
        self.scale.set_target(now, scale);
    }

    pub fn jump_color(&mut self, input: &Input) {
        let hovered = input.contains_mouse(self.area);
        let color = self.color_override.unwrap_or(if self.selected || hovered {
            self.hover_color
        } else {
            Color::TRANSPARENT
        });
        self.color.jump(color);
    }

    pub fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let mut running = self.ink.tick(now);
        running |= self.color.tick(now);
        running |= self.scale.tick(now);
        tree.set_fill(self.root, self.color.value());
        tree.set_scale(self.root, self.scale.value());
        tree.set_transforms(self.root, &self.extra);
        tree.set_text(self.text, self.spec());
        running
    }
}

pub struct SwapText {
    pub root: NodeId,
    previous: NodeId,
    current: NodeId,
    family: &'static str,
    pixel_size: f32,
    color: Color,
    pub max_width: f32,
    distance: f32,
    text: String,
    key: String,
    shown: String,
    shown_key: String,
    previous_text: String,
    pending: bool,
    ready: bool,
    current_metrics: TextMetrics,
    previous_metrics: TextMetrics,
    implicit_width: Tween<f32>,
    previous_opacity: Tween<f32>,
    previous_offset: Tween<f32>,
    current_opacity: Tween<f32>,
    current_offset: Tween<f32>,
    width: f32,
    height: f32,
}

impl SwapText {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tree: &mut Tree,
        engine: &mut TextEngine,
        parent: NodeId,
        text: &str,
        key: &str,
        family: &'static str,
        pixel_size: f32,
        color: Color,
        max_width: f32,
    ) -> Self {
        let mut root = Node::new(0.0, 0.0, 0.0, 0.0);
        root.clip = true;
        let root = tree.add(parent, root);
        let previous = tree.add(
            root,
            Node::new(0.0, 0.0, 0.0, 0.0).with_paint(Paint::Text(TextPaint::new(TextSpec::plain(
                "", family, pixel_size, color,
            )))),
        );
        let current = tree.add(
            root,
            Node::new(0.0, 0.0, 0.0, 0.0).with_paint(Paint::Text(TextPaint::new(TextSpec::plain(
                text, family, pixel_size, color,
            )))),
        );
        if let Some(node) = tree.get_mut(previous) {
            node.opacity = 0.0;
        }
        let current_metrics =
            engine.measure(&TextSpec::plain(text, family, pixel_size, color), 1.0);
        let previous_metrics = engine.measure(&TextSpec::plain("", family, pixel_size, color), 1.0);
        let implicit = max_width.min(current_metrics.width);
        Self {
            root,
            previous,
            current,
            family,
            pixel_size,
            color,
            max_width,
            distance: 7.0,
            text: text.to_string(),
            key: key.to_string(),
            shown: text.to_string(),
            shown_key: key.to_string(),
            previous_text: String::new(),
            pending: false,
            ready: true,
            current_metrics,
            previous_metrics,
            implicit_width: Tween::new(implicit, NORMAL, Easing::OutCubic),
            previous_opacity: Tween::new(0.0, FAST, Easing::InQuad),
            previous_offset: Tween::new(0.0, FAST, Easing::InQuad),
            current_opacity: Tween::new(1.0, NORMAL, Easing::OutCubic),
            current_offset: Tween::new(0.0, SLOW, Easing::OutCubic),
            width: implicit,
            height: current_metrics.height,
        }
    }

    pub fn implicit_width(&self) -> f32 {
        self.implicit_width.value()
    }

    pub fn implicit_height(&self) -> f32 {
        self.current_metrics.height
    }

    pub fn set(&mut self, text: &str, key: &str) {
        if self.text != text || self.key != key {
            self.text = text.to_string();
            self.key = key.to_string();
            self.pending = true;
        }
    }

    pub fn set_max_width(&mut self, now: Millis, max_width: f32) {
        if self.max_width != max_width {
            self.max_width = max_width;
            self.retarget_width(now);
        }
    }

    fn retarget_width(&mut self, now: Millis) {
        let target = self.max_width.min(self.current_metrics.width);
        if self.ready {
            self.implicit_width.set_target(now, target);
        } else {
            self.implicit_width.jump(target);
        }
    }

    pub fn flush(&mut self, engine: &mut TextEngine, now: Millis) {
        if !self.pending {
            return;
        }
        self.pending = false;
        if !self.ready || self.text == self.shown {
            return;
        }
        let measure = |engine: &mut TextEngine, text: &str| {
            engine.measure(
                &TextSpec::plain(text, self.family, self.pixel_size, self.color),
                1.0,
            )
        };
        if !self.key.is_empty() && self.key == self.shown_key {
            self.shown = self.text.clone();
            self.current_metrics = measure(engine, &self.shown);
            self.retarget_width(now);
            return;
        }
        self.shown_key = self.key.clone();
        self.previous_text = std::mem::take(&mut self.shown);
        self.previous_metrics = measure(engine, &self.previous_text);
        self.shown = self.text.clone();
        self.current_metrics = measure(engine, &self.shown);
        self.retarget_width(now);
        self.previous_opacity.start(now, 1.0, 0.0);
        self.previous_offset.start(now, 0.0, -self.distance);
        self.current_opacity.start(now, 0.0, 1.0);
        self.current_offset.start(now, self.distance, 0.0);
    }

    pub fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32, height: f32) {
        self.width = width;
        self.height = height;
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, height);
    }

    pub fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let mut running = self.implicit_width.tick(now);
        running |= self.previous_opacity.tick(now);
        running |= self.previous_offset.tick(now);
        running |= self.current_opacity.tick(now);
        running |= self.current_offset.tick(now);
        let width = self.width;
        let spec = |text: &str| TextSpec {
            spans: vec![Span::new(text, self.family, self.pixel_size)],
            color: self.color,
            elide_width: Some(width),
        };
        tree.set_text(self.previous, spec(&self.previous_text));
        tree.set_text(self.current, spec(&self.shown));
        let previous_y =
            center(self.height, self.previous_metrics.height) + self.previous_offset.value();
        let current_y =
            center(self.height, self.current_metrics.height) + self.current_offset.value();
        tree.set_pos(self.previous, 0.0, previous_y);
        tree.set_size(self.previous, width, self.previous_metrics.height);
        tree.set_opacity(self.previous, self.previous_opacity.value());
        tree.set_pos(self.current, 0.0, current_y);
        tree.set_size(self.current, width, self.current_metrics.height);
        tree.set_opacity(self.current, self.current_opacity.value());
        running
    }
}

pub struct TooltipStyle {
    pub family: &'static str,
    pub text: Color,
    pub surface: Color,
    pub line: Color,
}

pub struct TooltipView {
    pub tree: Tree,
    item: NodeId,
    opacity: Tween<f32>,
    scale: Tween<f32>,
    pub width: f32,
    pub height: f32,
    closing: bool,
}

impl TooltipView {
    pub fn padding() -> (f32, f32) {
        ((10.0 * UI_SCALE).round(), (6.0 * UI_SCALE).round())
    }

    pub fn spec(text: &str, style: &TooltipStyle) -> TextSpec {
        TextSpec::plain(text, style.family, (11.0 * UI_SCALE).round(), style.text)
    }

    pub fn measure(engine: &mut TextEngine, text: &str, style: &TooltipStyle) -> (f32, f32) {
        let metrics = engine.measure(&Self::spec(text, style), 1.0);
        let (horizontal, vertical) = Self::padding();
        (
            metrics.width + horizontal * 2.0,
            metrics.height + vertical * 2.0,
        )
    }

    pub fn new(engine: &mut TextEngine, text: &str, style: &TooltipStyle) -> Self {
        let mut tree = Tree::new();
        let (width, height) = Self::measure(engine, text, style);
        let (horizontal, vertical) = Self::padding();
        let root = tree.root();
        let mut item = Node::new(0.0, 0.0, width, height);
        item.opacity = 0.0;
        item.scale = 0.94;
        let item = tree.add(root, item);
        tree.add(
            item,
            Node::rect(
                0.0,
                0.0,
                width,
                height,
                RectPaint {
                    color: style.surface,
                    radius: 8.0,
                    border_color: style.line,
                    border_width: 1.0,
                },
            ),
        );
        let spec = Self::spec(text, style);
        let metrics = engine.measure(&spec, 1.0);
        tree.add(
            item,
            Node::new(horizontal, vertical, metrics.width, metrics.height)
                .with_paint(Paint::Text(TextPaint::new(spec))),
        );
        Self {
            tree,
            item,
            opacity: Tween::new(0.0, FAST, Easing::OutCubic),
            scale: Tween::new(0.94, NORMAL, Easing::OUT_BACK),
            width,
            height,
            closing: false,
        }
    }

    pub fn open(&mut self, now: Millis) {
        self.closing = false;
        self.opacity = Tween::new(0.0, FAST, Easing::OutCubic);
        self.opacity.start(now, 0.0, 1.0);
        self.scale.start(now, 0.94, 1.0);
    }

    pub fn close(&mut self, now: Millis) {
        if self.closing {
            return;
        }
        self.closing = true;
        let from = self.opacity.value();
        self.opacity = Tween::new(from, 100.0, Easing::Linear);
        self.opacity.start(now, from, 0.0);
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    pub fn finished_closing(&self) -> bool {
        self.closing && !self.opacity.is_running()
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        let mut running = self.opacity.tick(now);
        if !self.closing {
            running |= self.scale.tick(now);
        }
        self.tree.set_opacity(self.item, self.opacity.value());
        self.tree.set_scale(self.item, self.scale.value());
        running
    }
}
