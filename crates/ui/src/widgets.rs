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
    implicit_height: f32,
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
            wrap_width: None,
            max_lines: None,
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
            implicit_height: height,
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
        self.implicit_height
    }

    pub fn height(&self) -> f32 {
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
            wrap_width: None,
            max_lines: None,
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

    pub fn set_font_size(&mut self, engine: &mut TextEngine, font_size: f32, icon_size: f32) {
        if self.font_size == font_size && self.icon_size == icon_size {
            return;
        }
        self.font_size = font_size;
        self.icon_size = icon_size;
        self.metrics = engine.measure(&self.spec(), 1.0);
        self.width = self.implicit_width();
    }

    pub fn place_sized(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32, height: f32) {
        self.height = height;
        self.place(tree, x, y, width);
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

pub struct Label {
    pub node: NodeId,
    spec: TextSpec,
    metrics: TextMetrics,
    width: f32,
}

impl Label {
    pub fn new(tree: &mut Tree, engine: &mut TextEngine, parent: NodeId, spec: TextSpec) -> Self {
        let metrics = engine.measure(&spec, 1.0);
        let node = tree.add(
            parent,
            Node::new(0.0, 0.0, metrics.width, metrics.height)
                .with_paint(Paint::Text(TextPaint::new(spec.clone()))),
        );
        Self {
            node,
            spec,
            metrics,
            width: metrics.width,
        }
    }

    pub fn implicit_width(&self) -> f32 {
        self.metrics.width
    }

    pub fn implicit_height(&self) -> f32 {
        self.metrics.height
    }

    pub fn text(&self) -> String {
        self.spec
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect()
    }

    pub fn set_text(&mut self, engine: &mut TextEngine, text: &str) {
        if self.spec.spans.len() == 1 && self.spec.spans[0].text == text {
            return;
        }
        if let Some(span) = self.spec.spans.first_mut() {
            span.text = text.to_string();
        }
        self.spec.spans.truncate(1);
        self.metrics = engine.measure(&self.spec, 1.0);
    }

    pub fn set_color(&mut self, color: Color) {
        self.spec.color = color;
    }

    pub fn set_spec(&mut self, engine: &mut TextEngine, spec: TextSpec) {
        if self.spec == spec {
            return;
        }
        self.spec = spec;
        self.metrics = engine.measure(&self.spec, 1.0);
    }

    pub fn spec(&self) -> &TextSpec {
        &self.spec
    }

    pub fn wrap(&mut self, engine: &mut TextEngine, width: f32, max_lines: usize) -> f32 {
        let width = width.max(1.0);
        if self.spec.wrap_width != Some(width) || self.spec.max_lines != Some(max_lines) {
            self.spec.wrap_width = Some(width);
            self.spec.max_lines = Some(max_lines);
            self.metrics = engine.measure(&self.spec, 1.0);
        }
        self.metrics.height
    }

    pub fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32, height: f32) {
        self.width = width;
        self.spec.elide_width =
            (self.spec.wrap_width.is_none() && width + 0.01 < self.metrics.width).then_some(width);
        tree.set_pos(self.node, x, y);
        tree.set_size(self.node, width.min(self.metrics.width), height);
        tree.set_text(self.node, self.spec.clone());
    }

    pub fn sync(&self, tree: &mut Tree) {
        tree.set_text(self.node, self.spec.clone());
    }
}

pub struct SliderStyle {
    pub track: Color,
    pub handle: Color,
}

pub struct Slider {
    pub root: NodeId,
    track: NodeId,
    fill: NodeId,
    handle: NodeId,
    pub area: AreaId,
    pub from: f64,
    pub to: f64,
    value: f64,
    width: f32,
    height: f32,
    handle_color: Color,
    accent: Color,
    fill_width: Tween<f32>,
    fill_color: Tween<Color>,
    handle_x: Tween<f32>,
    size: Tween<f32>,
    knob_color: Tween<Color>,
    press_x: Option<f32>,
    dragging: bool,
    pub shortcut: String,
}

const SLIDER_PADDING: f32 = 7.0;

impl Slider {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        parent: NodeId,
        style: &SliderStyle,
        accent: Color,
        from: f64,
        to: f64,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 200.0, 32.0));
        let track = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                0.0,
                6.0,
                RectPaint {
                    color: style.track,
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
                    color: accent,
                    radius: 3.0,
                    ..RectPaint::default()
                },
            ),
        );
        let handle = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                14.0,
                14.0,
                RectPaint {
                    color: style.handle,
                    radius: 14.0 / 2.3,
                    ..RectPaint::default()
                },
            ),
        );
        let mut area = MouseArea::new(root);
        area.cursor = platform::CursorShape::Default;
        let area = input.add(area);
        Self {
            root,
            track,
            fill,
            handle,
            area,
            from,
            to,
            value: from,
            width: 200.0,
            height: 32.0,
            handle_color: style.handle,
            accent,
            fill_width: Tween::new(0.0, NORMAL, Easing::OutCubic),
            fill_color: Tween::new(accent, NORMAL, Easing::Linear),
            handle_x: Tween::new(SLIDER_PADDING, NORMAL, Easing::OutCubic),
            size: Tween::new(14.0, FAST, Easing::OutCubic),
            knob_color: Tween::new(style.handle, FAST, Easing::Linear),
            press_x: None,
            dragging: false,
            shortcut: String::new(),
        }
    }

    pub fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32, height: f32) {
        self.width = width;
        self.height = height;
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, height);
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn set_value(&mut self, value: f64) {
        if self.press_x.is_none() {
            self.value = value.clamp(self.from.min(self.to), self.from.max(self.to));
        }
    }

    pub fn set_accent(&mut self, accent: Color) {
        self.accent = accent;
    }

    fn available(&self) -> f32 {
        self.width - SLIDER_PADDING * 2.0
    }

    fn position(&self) -> f32 {
        let range = self.to - self.from;
        if range == 0.0 {
            return 0.0;
        }
        ((self.value - self.from) / range).clamp(0.0, 1.0) as f32
    }

    fn position_at(&self, x: f32) -> f64 {
        let handle = self.size.value();
        let available = self.available() - handle;
        if available <= 0.0 {
            return 0.0;
        }
        ((x - SLIDER_PADDING - handle / 2.0) / available).clamp(0.0, 1.0) as f64
    }

    fn local_x(&self, tree: &Tree, x: f32, y: f32) -> Option<f32> {
        let inverse = tree.device_transform(self.root).invert()?;
        let mut point = tiny_skia::Point::from_xy(x, y);
        inverse.map_point(&mut point);
        Some(point.x)
    }

    fn move_to(&mut self, local: f32) -> Option<f64> {
        let old = self.position();
        let pos = self.position_at(local);
        self.value = self.from + (self.to - self.from) * pos;
        ((self.position() - old).abs() > f32::EPSILON).then_some(self.value)
    }

    pub fn press(&mut self, tree: &Tree, x: f32, y: f32) {
        self.press_x = self.local_x(tree, x, y);
        self.dragging = false;
    }

    pub fn motion(&mut self, tree: &Tree, x: f32, y: f32) -> Option<f64> {
        let start = self.press_x?;
        let local = self.local_x(tree, x, y)?;
        if !self.dragging {
            self.dragging = (local - start).abs() > 10.0;
        }
        if self.dragging {
            self.move_to(local)
        } else {
            None
        }
    }

    pub fn release(&mut self, tree: &Tree, x: f32, y: f32) -> Option<f64> {
        self.press_x?;
        let local = self.local_x(tree, x, y);
        let moved = local.and_then(|local| self.move_to(local));
        self.press_x = None;
        self.dragging = false;
        moved
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let pressed = input.pressed(self.area);
        let hovered = input.contains_mouse(self.area);
        let size = if pressed {
            18.0
        } else if hovered {
            16.0
        } else {
            14.0
        };
        self.size.set_target(now, size);
        self.fill_color.set_target(now, self.accent);
        self.knob_color.set_target(
            now,
            if pressed {
                self.accent
            } else {
                self.handle_color
            },
        );
        let mut running = self.size.tick(now);
        running |= self.fill_color.tick(now);
        running |= self.knob_color.tick(now);
        let available = self.available();
        let position = self.position();
        let size = self.size.value();
        let fill_target = available * position;
        let x_target = SLIDER_PADDING + position * (available - 14.0) - (size - 14.0) / 2.0;
        if pressed {
            self.fill_width.jump(fill_target);
            self.handle_x.jump(x_target);
        } else {
            self.fill_width.set_target(now, fill_target);
            self.handle_x.set_target(now, x_target);
        }
        running |= self.fill_width.tick(now);
        running |= self.handle_x.tick(now);
        tree.set_pos(self.track, SLIDER_PADDING, (self.height - 6.0) / 2.0);
        tree.set_size(self.track, available, 6.0);
        tree.set_size(self.fill, self.fill_width.value(), 6.0);
        tree.set_fill(self.fill, self.fill_color.value());
        tree.set_pos(
            self.handle,
            self.handle_x.value(),
            (self.height - size) / 2.0,
        );
        tree.set_size(self.handle, size, size);
        tree.set_fill(self.handle, self.knob_color.value());
        if let Some(node) = tree.get(self.handle)
            && let Paint::Rect(rect) = &node.paint
            && rect.radius != size / 2.3
            && let Some(node) = tree.get_mut(self.handle)
            && let Paint::Rect(rect) = &mut node.paint
        {
            rect.radius = size / 2.3;
        }
        running
    }

    pub fn jump(&mut self, tree: &mut Tree, input: &Input) {
        self.tick(tree, input, 0.0);
        let available = self.available();
        let position = self.position();
        let size = self.size.target();
        self.size.jump(size);
        self.fill_width.jump(available * position);
        self.handle_x
            .jump(SLIDER_PADDING + position * (available - 14.0) - (size - 14.0) / 2.0);
        self.fill_color.jump(self.accent);
        self.tick(tree, input, 0.0);
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

    pub fn set_distance(&mut self, distance: f32) {
        self.distance = distance;
    }

    pub fn set_font(&mut self, engine: &mut TextEngine, family: &'static str, pixel_size: f32) {
        if self.family == family && self.pixel_size == pixel_size {
            return;
        }
        self.family = family;
        self.pixel_size = pixel_size;
        let measure = |engine: &mut TextEngine, text: &str| {
            engine.measure(&TextSpec::plain(text, family, pixel_size, self.color), 1.0)
        };
        self.current_metrics = measure(engine, &self.shown);
        self.previous_metrics = measure(engine, &self.previous_text);
        let target = self.max_width.min(self.current_metrics.width);
        self.implicit_width.jump(target);
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

    pub fn advance(&mut self, now: Millis) -> bool {
        let mut running = self.implicit_width.tick(now);
        running |= self.previous_opacity.tick(now);
        running |= self.previous_offset.tick(now);
        running |= self.current_opacity.tick(now);
        running |= self.current_offset.tick(now);
        running
    }

    pub fn apply(&self, tree: &mut Tree) {
        let width = self.width;
        let spec = |text: &str| TextSpec {
            spans: vec![Span::new(text, self.family, self.pixel_size)],
            color: self.color,
            elide_width: Some(width),
            wrap_width: None,
            max_lines: None,
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
    }

    pub fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.advance(now);
        self.apply(tree);
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

pub struct Sparkline {
    pub root: NodeId,
    baseline: NodeId,
    second: NodeId,
    first: NodeId,
    pub capacity: usize,
    pub minimum: f64,
    pub maximum: f64,
    pub stroke: Color,
    pub second_stroke: Color,
    pub line: Color,
}

impl Sparkline {
    pub fn new(
        tree: &mut Tree,
        parent: NodeId,
        stroke: Color,
        second_stroke: Color,
        line: Color,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 44.0));
        let baseline = tree.add(
            root,
            Node::new(0.0, 0.0, 0.0, 0.0).with_paint(Paint::Path(crate::PathPaint::default())),
        );
        let second = tree.add(
            root,
            Node::new(0.0, 0.0, 0.0, 0.0).with_paint(Paint::Path(crate::PathPaint::default())),
        );
        let first = tree.add(
            root,
            Node::new(0.0, 0.0, 0.0, 0.0).with_paint(Paint::Path(crate::PathPaint::default())),
        );
        Self {
            root,
            baseline,
            second,
            first,
            capacity: 60,
            minimum: 0.0,
            maximum: 0.0,
            stroke,
            second_stroke,
            line,
        }
    }

    fn set(tree: &mut Tree, node: NodeId, width: f32, height: f32, path: crate::PathPaint) {
        tree.set_size(node, width, height);
        let changed = tree
            .get(node)
            .is_some_and(|node| !matches!(&node.paint, Paint::Path(old) if *old == path));
        if changed && let Some(node) = tree.get_mut(node) {
            node.paint = Paint::Path(path);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn place(
        &mut self,
        tree: &mut Tree,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        values: &[f64],
        second: &[f64],
    ) {
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, height);
        let ceiling = if self.maximum > 0.0 {
            self.maximum
        } else {
            (values.iter().chain(second).copied().fold(0.0, f64::max) * 1.15).max(1.0)
        };
        let span = (self.capacity.max(2) - 1) as f64;
        let range = (ceiling - self.minimum).max(0.0001);
        let trace = |data: &[f64], color: Color| -> crate::PathPaint {
            if data.is_empty() {
                return crate::PathPaint::default();
            }
            let offset = self.capacity as f64 - data.len() as f64;
            let points = data
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    let px = width as f64 * (i as f64 + offset) / span;
                    let level = ((value - self.minimum) / range).clamp(0.0, 1.0);
                    let py = height as f64 - 1.0 - (height as f64 - 2.0) * level;
                    (px as f32, py as f32)
                })
                .collect();
            crate::PathPaint {
                points,
                stroke: color,
                width: 1.4,
                fill: color.with_alpha(0.14),
                fill_to: Some(height),
            }
        };
        Self::set(
            tree,
            self.baseline,
            width,
            height,
            crate::PathPaint {
                points: vec![(0.0, height - 0.5), (width, height - 0.5)],
                stroke: self.line,
                width: 1.0,
                fill: Color::TRANSPARENT,
                fill_to: None,
            },
        );
        Self::set(
            tree,
            self.second,
            width,
            height,
            trace(second, self.second_stroke),
        );
        Self::set(tree, self.first, width, height, trace(values, self.stroke));
    }
}
