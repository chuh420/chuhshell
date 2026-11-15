use platform::MouseButton;
use ui::{
    ArcPaint, Color, Node, NodeId, Paint, RectPaint, TextEngine, TextSpec, Tree, Weight,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Buttons, Input, InputEvent, MouseArea},
    layout::{Cell, Slot, center, linear},
    widgets::{Chip, ChipStyle, Label, Ripple, Slider, SliderStyle, SwapText},
};

use super::{PanelAction, PanelData, TooltipSpot};
use crate::theme::{self, FAST, NORMAL};

fn chip_style() -> ChipStyle {
    ChipStyle {
        family: crate::theme::icons(),
        hover: theme::elevated(),
        rest: ui::Color::TRANSPARENT,
    }
}

pub(crate) fn slider_style() -> SliderStyle {
    SliderStyle {
        track: theme::elevated(),
        handle: theme::text(),
    }
}

fn text(value: &str, family: &str, size: f32, color: Color) -> TextSpec {
    TextSpec::plain(value, family, size, color)
}

fn label(tree: &mut Tree, engine: &mut TextEngine, parent: NodeId, spec: TextSpec) -> Label {
    Label::new(tree, engine, parent, spec)
}

fn card(tree: &mut Tree, parent: NodeId, color: Color, radius: f32) -> NodeId {
    tree.add(
        parent,
        Node::rect(0.0, 0.0, 0.0, 0.0, RectPaint::solid(color, radius)),
    )
}

#[allow(clippy::too_many_arguments)]
fn chip(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    value: &str,
    ink: Color,
    size: Option<f32>,
    shortcut: &str,
) -> Chip {
    let mut chip = Chip::new(
        tree,
        input,
        engine,
        parent,
        &chip_style(),
        value,
        ink,
        false,
        None,
    );
    if let Some(size) = size {
        chip.set_font_size(engine, size, size);
    }
    chip.shortcut = shortcut.to_string();
    chip
}

fn fit(slot: Slot, size: f32) -> (f32, f32) {
    let size = size.min(slot.size);
    (slot.pos + center(slot.size, size), size)
}

fn place_label(tree: &mut Tree, label: &mut Label, x: f32, slot: Slot, width: f32) {
    let (y, height) = fit(slot, label.implicit_height());
    label.place(tree, x, y, width, height);
}

fn place_chip(tree: &mut Tree, chip: &mut Chip, x: f32, slot: Slot) {
    let (y, height) = fit(slot, chip.implicit_height().ceil());
    let width = chip.implicit_width().ceil();
    chip.place_sized(tree, x, y, width, height);
}

const RING: f32 = 50.0;
const RING_GAP: f32 = 8.0;

struct Ring {
    root: NodeId,
    arc: NodeId,
    icon: Label,
    caption: Label,
    level: Tween<f32>,
    target: f32,
    color: Color,
    started: bool,
}

impl Ring {
    fn new(
        tree: &mut Tree,
        engine: &mut TextEngine,
        parent: NodeId,
        glyph: &str,
        color: Color,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, RING, RING));
        let arc = tree.add(
            root,
            Node::new(0.0, 0.0, RING, RING).with_paint(Paint::Arc(ArcPaint {
                start: -135.0,
                sweep: 0.0,
                span: 270.0,
                width: 4.5,
                color,
                track: theme::elevated(),
            })),
        );
        let icon = label(
            tree,
            engine,
            root,
            text(glyph, crate::theme::icons(), 17.0, color),
        );
        let caption = label(
            tree,
            engine,
            root,
            text("", crate::theme::icons(), 8.5, theme::muted()),
        );
        Self {
            root,
            arc,
            icon,
            caption,
            level: Tween::new(0.0, 900.0, Easing::OutCubic),
            target: 0.0,
            color,
            started: false,
        }
    }

    fn set(&mut self, engine: &mut TextEngine, level: f32, caption: &str, color: Color) {
        self.target = level.clamp(0.0, 1.0);
        self.color = color;
        self.caption.set_text(engine, caption);
        self.icon.set_color(color);
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32) {
        tree.set_pos(self.root, x, y);
        let (width, height) = (
            self.icon.implicit_width().ceil(),
            self.icon.implicit_height().ceil(),
        );
        self.icon.place(
            tree,
            center(RING, width),
            center(RING, height) - 3.0,
            width,
            height,
        );
        let (width, height) = (
            self.caption.implicit_width().ceil(),
            self.caption.implicit_height().ceil(),
        );
        self.caption.place(
            tree,
            center(RING, width),
            RING - height - 1.0,
            width,
            height,
        );
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis, delay: Millis) -> bool {
        if !self.started {
            if now < delay {
                return true;
            }
            self.started = true;
            self.level.start(now, 0.0, self.target);
        } else {
            self.level.set_target(now, self.target);
        }
        let running = self.level.tick(now);
        tree.set_arc(self.arc, 270.0 * self.level.value(), self.color);
        self.icon.sync(tree);
        self.caption.sync(tree);
        running
    }
}

const TILE: f32 = 62.0;
const BADGE: f32 = 36.0;

struct Tile {
    root: NodeId,
    area: AreaId,
    badge: NodeId,
    icon: Label,
    state: Label,
    name: Label,
    status: Label,
    wifi: bool,
    active: bool,
    color: Tween<Color>,
    border: Tween<Color>,
    badge_color: Tween<Color>,
    ink: Tween<Color>,
    scale: Tween<f32>,
    ripple: Ripple,
}

impl Tile {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        wifi: bool,
    ) -> Self {
        let root = tree.add(
            parent,
            Node::rect(
                0.0,
                0.0,
                0.0,
                TILE,
                RectPaint::solid(theme::surface(), 16.0).bordered(theme::line(), 0.8),
            ),
        );
        let badge = tree.add(
            root,
            Node::rect(
                13.0,
                center(TILE, BADGE),
                BADGE,
                BADGE,
                RectPaint::solid(theme::elevated(), BADGE / 2.0),
            ),
        );
        let glyph = if wifi { "󰤨" } else { "󰂯" };
        let icon = label(
            tree,
            engine,
            badge,
            text(glyph, crate::theme::icons(), 18.0, theme::muted()),
        );
        let state = label(
            tree,
            engine,
            root,
            text("", crate::theme::icons(), 10.0, theme::muted()),
        );
        let mut name_spec = text("", crate::theme::font(), 13.0, theme::text());
        if let Some(span) = name_spec.spans.first_mut() {
            span.weight = Weight::Medium;
        }
        let name = label(tree, engine, root, name_spec);
        let status = label(
            tree,
            engine,
            root,
            text("", crate::theme::font(), 10.0, theme::muted()),
        );
        let mut area = MouseArea::new(root);
        area.buttons = Buttons::LEFT_RIGHT;
        let area = input.add(area);
        let ripple = Ripple::new(tree, root, area, 16.0, theme::text());
        Self {
            ripple,
            root,
            area,
            badge,
            icon,
            state,
            name,
            status,
            wifi,
            active: false,
            color: Tween::new(theme::surface(), FAST, Easing::Linear),
            border: Tween::new(theme::line(), NORMAL, Easing::Linear),
            badge_color: Tween::new(theme::elevated(), NORMAL, Easing::Linear),
            ink: Tween::new(theme::muted(), NORMAL, Easing::Linear),
            scale: Tween::new(1.0, 110.0, Easing::OutCubic),
        }
    }

    fn accent(&self) -> Color {
        if self.wifi {
            theme::green()
        } else {
            theme::yellow()
        }
    }

    fn sync(&mut self, engine: &mut TextEngine, data: &PanelData) {
        if self.wifi {
            self.active = data.network_connected;
            self.state.set_text(engine, "󰁔");
            let name = if data.network_name.is_empty() {
                "wi-fi"
            } else {
                data.network_name.as_str()
            };
            self.name.set_text(engine, name);
            self.status.set_text(
                engine,
                if self.active {
                    "connected"
                } else {
                    "disconnected"
                },
            );
        } else {
            self.active = data.bluetooth_powered;
            self.state
                .set_text(engine, if self.active { "on" } else { "off" });
            self.name.set_text(engine, "bluetooth");
            let name = if data.bluetooth_name.is_empty() {
                "no adapter found"
            } else {
                data.bluetooth_name.as_str()
            };
            self.status.set_text(engine, name);
        }
    }

    fn fill(&self, hovered: bool) -> Color {
        match (self.active, hovered) {
            (true, true) => theme::tint(self.accent(), 0.24),
            (true, false) => theme::tint(self.accent(), 0.15),
            (false, true) => theme::elevated(),
            (false, false) => theme::surface(),
        }
    }

    fn border_target(&self) -> Color {
        if self.active {
            theme::tint(self.accent(), 0.55)
        } else {
            theme::line()
        }
    }

    fn badge_target(&self) -> (Color, Color) {
        if self.active {
            (self.accent(), theme::dim())
        } else {
            (theme::elevated(), theme::muted())
        }
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32, height: f32) {
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, height);
        if let Some(node) = tree.get_mut(self.root) {
            node.origin = Some((width / 2.0, height / 2.0));
        }
        tree.set_pos(self.badge, 13.0, center(height, BADGE));
        let (icon_width, icon_height) = (
            self.icon.implicit_width().ceil(),
            self.icon.implicit_height().ceil(),
        );
        self.icon.place(
            tree,
            center(BADGE, icon_width),
            center(BADGE, icon_height),
            icon_width,
            icon_height,
        );
        let state_width = self.state.implicit_width().ceil();
        let state_height = self.state.implicit_height().ceil();
        self.state.place(
            tree,
            width - 14.0 - state_width,
            center(height, state_height),
            state_width,
            state_height,
        );
        let text_x = 13.0 + BADGE + 11.0;
        let text_width = (width - text_x - state_width - 22.0).max(0.0);
        let name_height = self.name.implicit_height().ceil();
        let status_height = self.status.implicit_height().ceil();
        let top = center(height, name_height + 1.0 + status_height);
        self.name.place(tree, text_x, top, text_width, name_height);
        self.status.place(
            tree,
            text_x,
            top + name_height + 1.0,
            text_width,
            status_height,
        );
    }

    fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let hovered = input.contains_mouse(self.area);
        self.ripple.sync(input);
        let rippling = self.ripple.tick(tree, now);
        self.color.set_target(now, self.fill(hovered));
        self.border.set_target(now, self.border_target());
        let (badge, ink) = self.badge_target();
        self.badge_color.set_target(now, badge);
        self.ink.set_target(now, ink);
        self.scale
            .set_target(now, if input.pressed(self.area) { 0.97 } else { 1.0 });
        let mut running = self.color.tick(now);
        running |= self.border.tick(now);
        running |= self.badge_color.tick(now);
        running |= self.ink.tick(now);
        running |= self.scale.tick(now);
        running |= rippling;
        tree.set_scale(self.root, self.scale.value());
        tree.set_fill(self.root, self.color.value());
        tree.set_border(self.root, self.border.value(), 0.8);
        tree.set_fill(self.badge, self.badge_color.value());
        self.icon.set_color(self.ink.value());
        self.state.set_color(if self.active {
            self.accent()
        } else {
            theme::muted()
        });
        for label in [&self.icon, &self.state, &self.name, &self.status] {
            label.sync(tree);
        }
        running
    }

    fn jump(&mut self, input: &Input) {
        let hovered = input.contains_mouse(self.area);
        self.color.jump(self.fill(hovered));
        self.border.jump(self.border_target());
        let (badge, ink) = self.badge_target();
        self.badge_color.jump(badge);
        self.ink.jump(ink);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Switch {
    Awake,
    Dnd,
    Mic,
    Theme,
}

const SWITCH: f32 = 38.0;
const KNOB: f32 = 12.0;

struct Toggle {
    kind: Switch,
    root: NodeId,
    area: AreaId,
    icon: Label,
    label: Label,
    track: NodeId,
    knob: NodeId,
    on: bool,
    shown: bool,
    hint: &'static str,
    color: Tween<Color>,
    ink: Tween<Color>,
    track_color: Tween<Color>,
    knob_color: Tween<Color>,
    knob_x: Tween<f32>,
    scale: Tween<f32>,
    width: f32,
    ripple: Ripple,
}

impl Toggle {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        kind: Switch,
    ) -> Self {
        let (glyph, caption, hint) = match kind {
            Switch::Awake => (
                "󰅶",
                "keep awake",
                "prevent automatic locking and screen sleep",
            ),
            Switch::Dnd => ("󰂛", "do not disturb", "hide notification popups"),
            Switch::Mic => ("󰍬", "microphone", ""),
            Switch::Theme => ("󰏘", "auto theme", "colors follow the wallpaper"),
        };
        let root = card(tree, parent, theme::surface(), 13.0);
        let icon = label(
            tree,
            engine,
            root,
            text(glyph, crate::theme::icons(), 16.0, theme::muted()),
        );
        let label = label(
            tree,
            engine,
            root,
            text(caption, crate::theme::font(), 11.5, theme::text()),
        );
        let track = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                28.0,
                16.0,
                RectPaint::solid(theme::elevated(), 8.0),
            ),
        );
        let knob = tree.add(
            track,
            Node::rect(
                2.0,
                2.0,
                KNOB,
                KNOB,
                RectPaint::solid(theme::muted(), KNOB / 2.0),
            ),
        );
        let area = input.add(MouseArea::new(root));
        let ripple = Ripple::new(tree, root, area, 13.0, theme::text());
        Self {
            ripple,
            kind,
            root,
            area,
            icon,
            label,
            track,
            knob,
            on: false,
            shown: true,
            hint,
            color: Tween::new(theme::surface(), FAST, Easing::Linear),
            ink: Tween::new(theme::muted(), NORMAL, Easing::Linear),
            track_color: Tween::new(theme::elevated(), NORMAL, Easing::Linear),
            knob_color: Tween::new(theme::muted(), NORMAL, Easing::Linear),
            knob_x: Tween::new(2.0, 320.0, Easing::SPATIAL_FAST),
            scale: Tween::new(1.0, 110.0, Easing::OutCubic),
            width: 0.0,
        }
    }

    fn sync(&mut self, engine: &mut TextEngine, data: &PanelData) {
        let mic = data.audio.source().filter(|source| source.has_audio);
        match self.kind {
            Switch::Awake => self.on = data.keep_awake,
            Switch::Dnd => self.on = data.dnd,
            Switch::Mic => {
                self.on = mic.is_some_and(|source| !source.muted);
                self.shown = mic.is_some();
                self.icon.set_text(engine, if self.on { "󰍬" } else { "󰍭" });
            }
            Switch::Theme => {
                self.on = data.theme_auto;
                self.shown = data.theme_available;
            }
        }
    }

    fn fill(&self, hovered: bool) -> Color {
        match (self.on, hovered) {
            (true, true) => theme::tint(theme::green(), 0.22),
            (true, false) => theme::tint(theme::green(), 0.13),
            (false, true) => theme::elevated(),
            (false, false) => theme::surface(),
        }
    }

    fn targets(&self) -> (Color, Color, Color, f32) {
        if self.on {
            (
                theme::green(),
                theme::green(),
                theme::dim(),
                28.0 - KNOB - 2.0,
            )
        } else {
            (theme::muted(), theme::line(), theme::muted(), 2.0)
        }
    }

    fn jump(&mut self, input: &Input) {
        self.color.jump(self.fill(input.contains_mouse(self.area)));
        let (ink, track, knob, x) = self.targets();
        self.ink.jump(ink);
        self.track_color.jump(track);
        self.knob_color.jump(knob);
        self.knob_x.jump(x);
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32) {
        self.width = width;
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, SWITCH);
        if let Some(node) = tree.get_mut(self.root) {
            node.origin = Some((width / 2.0, SWITCH / 2.0));
        }
        let icon_height = self.icon.implicit_height().ceil();
        let icon_width = self.icon.implicit_width().ceil();
        self.icon.place(
            tree,
            12.0,
            center(SWITCH, icon_height),
            icon_width,
            icon_height,
        );
        let label_height = self.label.implicit_height().ceil();
        self.label.place(
            tree,
            12.0 + icon_width + 9.0,
            center(SWITCH, label_height),
            (width - icon_width - 21.0 - 28.0 - 18.0).max(0.0),
            label_height,
        );
        tree.set_pos(self.track, width - 28.0 - 11.0, center(SWITCH, 16.0));
    }

    fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        self.color
            .set_target(now, self.fill(input.contains_mouse(self.area)));
        let (ink, track, knob, x) = self.targets();
        self.ink.set_target(now, ink);
        self.track_color.set_target(now, track);
        self.knob_color.set_target(now, knob);
        self.knob_x.set_target(now, x);
        self.scale
            .set_target(now, if input.pressed(self.area) { 0.97 } else { 1.0 });
        self.ripple.sync(input);
        let mut running = self.ripple.tick(tree, now);
        running |= self.color.tick(now);
        running |= self.ink.tick(now);
        running |= self.track_color.tick(now);
        running |= self.knob_color.tick(now);
        running |= self.knob_x.tick(now);
        running |= self.scale.tick(now);
        tree.set_scale(self.root, self.scale.value());
        tree.set_fill(self.root, self.color.value());
        tree.set_fill(self.track, self.track_color.value());
        tree.set_fill(self.knob, self.knob_color.value());
        tree.set_pos(self.knob, self.knob_x.value(), 2.0);
        self.icon.set_color(self.ink.value());
        self.icon.sync(tree);
        self.label.sync(tree);
        running
    }
}

const LEVEL: f32 = 40.0;

struct Level {
    chip: Chip,
    slider: Slider,
    value: Label,
    ink: Tween<Color>,
    width: f32,
}

impl Level {
    #[allow(clippy::too_many_arguments)]
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        glyph: &str,
        accent: Color,
        from: f64,
        shortcut: &str,
    ) -> Self {
        let mut chip = chip(tree, input, engine, parent, glyph, accent, Some(18.0), "");
        chip.color_override = Some(theme::surface());
        chip.jump_color(input);
        let style = SliderStyle {
            track: theme::surface(),
            handle: theme::text(),
        };
        let mut slider = Slider::new(tree, input, parent, &style, accent, from, 100.0).thick(tree);
        slider.shortcut = shortcut.to_string();
        let value = label(
            tree,
            engine,
            parent,
            text("", crate::theme::icons(), 11.0, theme::text()),
        );
        Self {
            chip,
            slider,
            value,
            ink: Tween::new(theme::text(), FAST, Easing::Linear),
            width: 0.0,
        }
    }

    fn place(&mut self, tree: &mut Tree, y: f32, width: f32) {
        self.width = width;
        self.chip.place_sized(tree, 0.0, y, LEVEL, LEVEL);
        self.slider
            .place(tree, LEVEL + 8.0, y, width - LEVEL - 8.0, LEVEL);
        let value_width = self.value.implicit_width().ceil();
        let value_height = self.value.implicit_height().ceil();
        self.value.place(
            tree,
            width - 20.0 - value_width,
            y + center(LEVEL, value_height),
            value_width,
            value_height,
        );
    }

    fn set_visible(&self, tree: &mut Tree, visible: bool) {
        tree.set_visible(self.chip.root, visible);
        tree.set_visible(self.slider.root, visible);
        tree.set_visible(self.value.node, visible);
    }

    fn covered(&self) -> bool {
        let edge = self.width - LEVEL - 8.0 - 26.0 - self.value.implicit_width();
        self.slider.fill_end() > edge
    }

    fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = self.slider.tick(tree, input, now);
        self.ink.set_target(
            now,
            if self.covered() {
                theme::dim()
            } else {
                theme::text()
            },
        );
        running |= self.ink.tick(now);
        self.value.set_color(self.ink.value());
        self.value.sync(tree);
        running
    }

    fn settle(&mut self, tree: &mut Tree, input: &Input) {
        self.slider.jump(tree, input);
        self.ink.jump(if self.covered() {
            theme::dim()
        } else {
            theme::text()
        });
    }
}

const MEDIA: f32 = 66.0;

struct MediaCard {
    root: NodeId,
    art_box: NodeId,
    art: NodeId,
    glyph: Label,
    title: SwapText,
    artist: Label,
    previous: Chip,
    toggle: Chip,
    next: Chip,
    shown: bool,
    can_previous: bool,
    can_next: bool,
    shown_art: super::Art,
}

impl MediaCard {
    fn new(tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, parent: NodeId) -> Self {
        let root = card(tree, parent, theme::surface(), 16.0);
        let art_size = crate::mediactl::ART;
        let art_box = tree.add(
            root,
            Node::rect(
                10.0,
                center(MEDIA, art_size),
                art_size,
                art_size,
                RectPaint::solid(theme::elevated(), 11.0),
            ),
        );
        let glyph = label(
            tree,
            engine,
            art_box,
            text("󰎈", crate::theme::icons(), 20.0, theme::green()),
        );
        let art = tree.add(art_box, Node::new(0.0, 0.0, art_size, art_size));
        let title = SwapText::new(
            tree,
            engine,
            root,
            "",
            "",
            crate::theme::font(),
            13.0,
            theme::text(),
            100000.0,
        );
        let artist = label(
            tree,
            engine,
            root,
            text("", crate::theme::font(), 10.5, theme::muted()),
        );
        let previous = chip(
            tree,
            input,
            engine,
            root,
            "󰒮",
            theme::text(),
            Some(16.0),
            "",
        );
        let mut toggle = chip(tree, input, engine, root, "󰐊", theme::dim(), Some(18.0), "");
        toggle.color_override = Some(theme::green());
        toggle.jump_color(input);
        let next = chip(
            tree,
            input,
            engine,
            root,
            "󰒭",
            theme::text(),
            Some(16.0),
            "",
        );
        Self {
            root,
            art_box,
            art,
            glyph,
            title,
            artist,
            previous,
            toggle,
            next,
            shown: false,
            can_previous: false,
            can_next: false,
            shown_art: super::Art(None),
        }
    }

    fn sync(&mut self, tree: &mut Tree, engine: &mut TextEngine, data: &PanelData, now: Millis) {
        self.shown = data.media.is_some();
        let Some(track) = &data.media else {
            return;
        };
        self.title.set(&track.title, "");
        self.title.flush(engine, now);
        let artist = if track.artist.is_empty() {
            track.app().to_string()
        } else {
            track.artist.clone()
        };
        self.artist.set_text(engine, &artist);
        self.toggle
            .set_label(engine, if track.playing { "󰏤" } else { "󰐊" }, 18.0);
        self.can_previous = track.can_previous;
        self.can_next = track.can_next;
        if self.shown_art != data.media_art {
            self.shown_art = data.media_art.clone();
            if let Some(node) = tree.get_mut(self.art) {
                node.paint = match &self.shown_art.0 {
                    Some(pixmap) => Paint::Image(pixmap.clone()),
                    None => Paint::None,
                };
            }
            tree.set_visible(self.glyph.node, self.shown_art.0.is_none());
        }
    }

    fn areas(&self) -> [AreaId; 3] {
        [self.previous.area, self.toggle.area, self.next.area]
    }

    fn place(&mut self, tree: &mut Tree, y: f32, width: f32) {
        tree.set_visible(self.root, self.shown);
        tree.set_pos(self.root, 0.0, y);
        tree.set_size(self.root, width, MEDIA);
        let art_size = crate::mediactl::ART;
        tree.set_pos(self.art_box, 10.0, center(MEDIA, art_size));
        let (glyph_width, glyph_height) = (
            self.glyph.implicit_width().ceil(),
            self.glyph.implicit_height().ceil(),
        );
        self.glyph.place(
            tree,
            center(art_size, glyph_width),
            center(art_size, glyph_height),
            glyph_width,
            glyph_height,
        );
        let buttons = 30.0 + 36.0 + 30.0 + 8.0;
        let mut x = width - 12.0 - buttons;
        let slot = Slot {
            pos: 0.0,
            size: MEDIA,
        };
        self.previous
            .place_sized(tree, x, center(MEDIA, 30.0), 30.0, 30.0);
        x += 34.0;
        self.toggle
            .place_sized(tree, x, center(MEDIA, 36.0), 36.0, 36.0);
        x += 40.0;
        self.next
            .place_sized(tree, x, center(MEDIA, 30.0), 30.0, 30.0);
        let _ = slot;
        let text_x = 10.0 + art_size + 12.0;
        let text_width = (width - text_x - buttons - 24.0).max(0.0);
        let title_height = self.title.implicit_height().ceil();
        let artist_height = self.artist.implicit_height().ceil();
        let top = center(MEDIA, title_height + 2.0 + artist_height);
        self.title
            .place(tree, text_x, top, text_width, title_height);
        self.title.apply(tree);
        self.artist.place(
            tree,
            text_x,
            top + title_height + 2.0,
            text_width.min(self.artist.implicit_width().ceil()),
            artist_height,
        );
    }

    fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = self.title.advance(now);
        self.toggle.color_override = Some(if self.toggle.hovered(input) {
            theme::lift(theme::green(), 0.15)
        } else {
            theme::green()
        });
        self.previous.set_ink(
            now,
            if self.can_previous {
                theme::text()
            } else {
                theme::line()
            },
        );
        self.next.set_ink(
            now,
            if self.can_next {
                theme::text()
            } else {
                theme::line()
            },
        );
        for chip in [&mut self.previous, &mut self.toggle, &mut self.next] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        tree.set_radius(self.toggle.root, 18.0);
        self.glyph.sync(tree);
        self.artist.sync(tree);
        running
    }

    fn click(&self, area: AreaId) -> Option<PanelAction> {
        use services::media::Command;
        if area == self.toggle.area {
            Some(PanelAction::Media(Command::Toggle))
        } else if area == self.previous.area {
            self.can_previous
                .then_some(PanelAction::Media(Command::Previous))
        } else if area == self.next.area {
            self.can_next.then_some(PanelAction::Media(Command::Next))
        } else {
            None
        }
    }
}

enum Footer {
    Toggle(&'static str),
    Run(&'static str),
    Suspend,
}

pub struct ControlPanel {
    pub root: NodeId,
    header: Label,
    time: Label,
    date: Label,
    rings: Vec<Ring>,
    battery_ring: bool,
    tiles: [Tile; 2],
    toggles: Vec<Toggle>,
    volume: Level,
    brightness: Level,
    media: MediaCard,
    line: NodeId,
    footer: Vec<(Chip, Footer)>,
    data: PanelData,
    width: f32,
    height: f32,
}

impl ControlPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let header_spec =
            crate::decor::eyebrow(crate::decor::CONTROL, "control", crate::theme::icons());
        let header = label(tree, engine, root, header_spec);
        let mut time_spec = text("", crate::theme::font(), 40.0, theme::text());
        if let Some(span) = time_spec.spans.first_mut() {
            span.weight = Weight::Light;
        }
        let time = label(tree, engine, root, time_spec);
        let date = label(
            tree,
            engine,
            root,
            text("", crate::theme::font(), 12.0, theme::muted()),
        );
        let rings = vec![
            Ring::new(tree, engine, root, "󰻠", theme::green()),
            Ring::new(tree, engine, root, "󰍛", theme::blue()),
            Ring::new(tree, engine, root, "󰁹", theme::yellow()),
        ];
        let tiles = [
            Tile::new(tree, input, engine, root, true),
            Tile::new(tree, input, engine, root, false),
        ];
        let toggles = [Switch::Awake, Switch::Dnd, Switch::Mic, Switch::Theme]
            .into_iter()
            .map(|kind| Toggle::new(tree, input, engine, root, kind))
            .collect();
        let volume = Level::new(
            tree,
            input,
            engine,
            root,
            "󰕾",
            theme::green(),
            0.0,
            "XF86AudioRaiseVolume / XF86AudioLowerVolume",
        );
        let brightness = Level::new(
            tree,
            input,
            engine,
            root,
            "󰃟",
            theme::yellow(),
            5.0,
            "XF86MonBrightnessUp / XF86MonBrightnessDown",
        );
        let media = MediaCard::new(tree, input, engine, root);
        let line = card(tree, root, theme::line(), 0.0);
        let footer_spec: [(&str, Color, &str, Footer); 7] = [
            (
                "󰂚",
                theme::text(),
                "Super + N",
                Footer::Toggle("notifications"),
            ),
            ("󰅍", theme::text(), "Super + C", Footer::Toggle("clipboard")),
            ("󰄀", theme::text(), "Print", Footer::Run("screenshot")),
            (
                "󰸉",
                theme::green(),
                "Super + B",
                Footer::Toggle("wallpapers"),
            ),
            (
                "󰌾",
                theme::yellow(),
                "Super + Ctrl + L",
                Footer::Run("lock"),
            ),
            ("󰒲", theme::text(), "", Footer::Suspend),
            ("󰐥", theme::red(), "", Footer::Toggle("power")),
        ];
        let footer = footer_spec
            .into_iter()
            .map(|(glyph, ink, shortcut, action)| {
                (
                    chip(tree, input, engine, root, glyph, ink, Some(18.0), shortcut),
                    action,
                )
            })
            .collect();
        let mut panel = Self {
            root,
            header,
            time,
            date,
            rings,
            battery_ring: false,
            tiles,
            toggles,
            volume,
            brightness,
            media,
            line,
            footer,
            data: PanelData::default(),
            width: 0.0,
            height: 0.0,
        };
        panel.sync(tree, engine, data, 0.0);
        panel.jump(input);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.tiles[0].area,
            self.tiles[1].area,
            self.volume.chip.area,
            self.volume.slider.area,
            self.brightness.chip.area,
            self.brightness.slider.area,
        ];
        areas.extend(self.toggles.iter().map(|toggle| toggle.area));
        areas.extend(self.media.areas());
        areas.extend(self.footer.iter().map(|(chip, _)| chip.area));
        areas
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        data: &PanelData,
        now: Millis,
    ) {
        self.data = data.clone();
        self.media.sync(tree, engine, data, now);
        self.time.set_text(engine, &data.time);
        self.date.set_text(engine, &data.date);
        let cpu = data.resources.cpu.unwrap_or(0).min(100);
        self.rings[0].set(
            engine,
            cpu as f32 / 100.0,
            &data
                .resources
                .cpu
                .map_or_else(|| "…".to_string(), |cpu| format!("{cpu}%")),
            if cpu >= 85 {
                theme::red()
            } else {
                theme::green()
            },
        );
        let ram = if data.resources.ram_total > 0.0 {
            (data.resources.ram_used / data.resources.ram_total).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.rings[1].set(
            engine,
            ram as f32,
            &format!("{}%", (ram * 100.0).round() as i64),
            theme::blue(),
        );
        let battery = data
            .power
            .battery
            .as_ref()
            .filter(|battery| battery.present);
        self.battery_ring = battery.is_some();
        if let Some(battery) = battery {
            let level = battery.percentage.round().clamp(0.0, 100.0);
            self.rings[2].icon.set_text(
                engine,
                crate::model::battery_icon(level as u32, data.battery_plugged),
            );
            self.rings[2].set(
                engine,
                level as f32 / 100.0,
                &format!("{level}%"),
                if level < 20.0 && !data.battery_plugged {
                    theme::red()
                } else {
                    theme::yellow()
                },
            );
        }
        for tile in &mut self.tiles {
            tile.sync(engine, data);
        }
        for toggle in &mut self.toggles {
            toggle.sync(engine, data);
        }
        let (glyph, ink, value) = if data.muted {
            ("󰖁", theme::muted(), "mute".to_string())
        } else {
            (
                if data.volume > 66 {
                    "󰕾"
                } else if data.volume > 33 {
                    "󰖀"
                } else {
                    "󰕿"
                },
                theme::green(),
                format!("{}%", data.volume),
            )
        };
        self.volume.chip.set_label(engine, glyph, 18.0);
        self.volume.chip.set_ink(now, ink);
        self.volume.chip.shortcut = "XF86AudioMute".into();
        self.volume.value.set_text(engine, &value);
        self.volume.slider.set_value(data.volume as f64);
        self.volume.slider.set_accent(if data.muted {
            theme::muted()
        } else {
            theme::green()
        });
        self.brightness
            .value
            .set_text(engine, &format!("{}%", data.brightness));
        self.brightness.slider.set_value(data.brightness as f64);
    }

    fn jump(&mut self, input: &Input) {
        for tile in &mut self.tiles {
            tile.jump(input);
        }
        for toggle in &mut self.toggles {
            toggle.jump(input);
        }
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        self.width = width;
        self.height = height;
        tree.set_size(self.root, width, height);
        let available = self.data.available;
        let time_height = self.time.implicit_height().ceil();
        let date_height = self.date.implicit_height().ceil();
        let block = (time_height + 2.0 + date_height).max(RING);
        let shown: Vec<usize> = self
            .toggles
            .iter()
            .enumerate()
            .filter(|(_, toggle)| toggle.shown)
            .map(|(index, _)| index)
            .collect();
        let rows = shown.len().div_ceil(2) as f32;
        let grid = rows * SWITCH + (rows - 1.0).max(0.0) * 8.0;
        let bright = if available.brightness {
            Cell::preferred(LEVEL).margins(8.0, 0.0)
        } else {
            Cell::preferred(0.0)
        };
        let slots = linear(
            &[
                Cell::implicit(self.header.implicit_height()),
                Cell::layout(block).margins(8.0, 0.0),
                Cell::layout(TILE).margins(16.0, 0.0),
                Cell::layout(grid).margins(10.0, 0.0),
                Cell::preferred(LEVEL).margins(16.0, 0.0),
                bright,
                if self.media.shown {
                    Cell::preferred(MEDIA).margins(14.0, 0.0)
                } else {
                    Cell::preferred(0.0)
                },
                Cell::fill(0.0),
                Cell::preferred(1.0).margins(0.0, 12.0),
                Cell::layout(30.0),
            ],
            0.0,
            height,
        );
        let header_width = self.header.implicit_width().ceil().min(width);
        place_label(tree, &mut self.header, 0.0, slots[0], header_width);
        let top = slots[1].pos + center(slots[1].size, time_height + 2.0 + date_height);
        let rings = if self.battery_ring { 3 } else { 2 };
        let rings_width = rings as f32 * RING + (rings - 1) as f32 * RING_GAP;
        let time_width = self
            .time
            .implicit_width()
            .ceil()
            .min(width - rings_width - 10.0);
        self.time.place(tree, 0.0, top, time_width, time_height);
        let date_width = self
            .date
            .implicit_width()
            .ceil()
            .min(width - rings_width - 10.0);
        self.date
            .place(tree, 1.0, top + time_height + 2.0, date_width, date_height);
        let ring_y = slots[1].pos + center(slots[1].size, RING);
        for (index, ring) in self.rings.iter_mut().enumerate() {
            tree.set_visible(ring.root, index < rings);
            ring.place(
                tree,
                width - rings_width + index as f32 * (RING + RING_GAP),
                ring_y,
            );
        }
        let shown_tiles = [available.wifi, available.bluetooth];
        let visible_tiles = shown_tiles.iter().filter(|shown| **shown).count();
        let tiles = linear(&vec![Cell::fill(0.0); visible_tiles], 10.0, width);
        let (tile_y, tile_height) = fit(slots[2], TILE);
        let mut slots_iter = tiles.iter();
        for (tile, shown) in self.tiles.iter_mut().zip(shown_tiles) {
            tree.set_visible(tile.root, shown);
            if shown && let Some(slot) = slots_iter.next() {
                tile.place(tree, slot.pos, tile_y, slot.size, tile_height);
            }
        }
        let half = (width - 10.0) / 2.0;
        for toggle in &self.toggles {
            tree.set_visible(toggle.root, toggle.shown);
        }
        for (order, index) in shown.iter().enumerate() {
            let last_alone = order == shown.len() - 1 && shown.len() % 2 == 1;
            let column = (order % 2) as f32;
            let row = (order / 2) as f32;
            self.toggles[*index].place(
                tree,
                column * (half + 10.0),
                slots[3].pos + row * (SWITCH + 8.0),
                if last_alone { width } else { half },
            );
        }
        self.volume.place(tree, slots[4].pos, width);
        self.brightness.set_visible(tree, available.brightness);
        self.brightness.place(tree, slots[5].pos, width);
        self.media.place(tree, slots[6].pos, width);
        let (line_y, line_height) = fit(slots[8], 1.0);
        tree.set_pos(self.line, 0.0, line_y);
        tree.set_size(self.line, width, line_height);
        let footer_shown: Vec<bool> = self
            .footer
            .iter()
            .map(|(_, action)| match action {
                Footer::Toggle("clipboard") => available.clipboard,
                Footer::Run("screenshot") => available.screenshot,
                _ => true,
            })
            .collect();
        let mut cells: Vec<Cell> = Vec::new();
        let mut owners: Vec<Option<usize>> = Vec::new();
        for (index, ((chip, _), shown)) in self.footer.iter().zip(&footer_shown).enumerate() {
            if index == 4 {
                cells.push(Cell::fill(0.0));
                owners.push(None);
            }
            if *shown {
                cells.push(Cell::implicit(chip.implicit_width()));
                owners.push(Some(index));
            }
        }
        let columns = linear(&cells, 7.0, width);
        for (index, ((chip, _), shown)) in self.footer.iter_mut().zip(&footer_shown).enumerate() {
            tree.set_visible(chip.root, *shown);
            if let Some(column) = owners.iter().position(|owner| *owner == Some(index)) {
                place_chip(tree, chip, columns[column].pos, slots[9]);
            }
        }
    }

    fn chips(&self) -> Vec<&Chip> {
        let mut chips = vec![&self.volume.chip];
        chips.extend(self.footer.iter().map(|(chip, _)| chip));
        chips
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for (index, ring) in self.rings.iter_mut().enumerate() {
            running |= ring.tick(tree, now, 120.0 + index as f64 * 90.0);
        }
        for tile in &mut self.tiles {
            running |= tile.tick(tree, input, now);
        }
        for toggle in &mut self.toggles {
            running |= toggle.tick(tree, input, now);
        }
        self.volume.chip.color_override = Some(if self.volume.chip.hovered(input) {
            theme::elevated()
        } else {
            theme::surface()
        });
        let mut chips: Vec<&mut Chip> = vec![&mut self.volume.chip, &mut self.brightness.chip];
        chips.extend(self.footer.iter_mut().map(|(chip, _)| chip));
        for chip in chips {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.volume.tick(tree, input, now);
        running |= self.brightness.tick(tree, input, now);
        if self.media.shown {
            running |= self.media.tick(tree, input, now);
        }
        for label in [&self.header, &self.time, &self.date] {
            label.sync(tree);
        }
        running
    }

    pub fn settle(&mut self, tree: &mut Tree, input: &Input) {
        self.volume.settle(tree, input);
        self.brightness.settle(tree, input);
        let mut chips: Vec<&mut Chip> = vec![&mut self.volume.chip, &mut self.brightness.chip];
        chips.extend(self.footer.iter_mut().map(|(chip, _)| chip));
        for chip in chips {
            chip.jump_color(input);
        }
    }

    pub fn handle(
        &mut self,
        tree: &Tree,
        events: &[InputEvent],
        pointer: (f32, f32),
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Press(area, MouseButton::Left) => {
                    if area == self.volume.slider.area {
                        self.volume.slider.press(tree, pointer.0, pointer.1);
                    } else if area == self.brightness.slider.area {
                        self.brightness.slider.press(tree, pointer.0, pointer.1);
                    }
                }
                InputEvent::Release(area, MouseButton::Left) => {
                    if area == self.volume.slider.area
                        && let Some(value) = self.volume.slider.release(tree, pointer.0, pointer.1)
                    {
                        actions.push(PanelAction::SetVolume(value));
                    } else if area == self.brightness.slider.area
                        && let Some(value) =
                            self.brightness.slider.release(tree, pointer.0, pointer.1)
                    {
                        actions.push(PanelAction::SetBrightness(value));
                    }
                }
                InputEvent::Click(area, button) => {
                    if let Some(action) = self.click(area, button) {
                        actions.push(action);
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn motion(&mut self, tree: &Tree, input: &Input, x: f32, y: f32) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        if input.pressed(self.volume.slider.area)
            && let Some(value) = self.volume.slider.motion(tree, x, y)
        {
            actions.push(PanelAction::SetVolume(value));
        }
        if input.pressed(self.brightness.slider.area)
            && let Some(value) = self.brightness.slider.motion(tree, x, y)
        {
            actions.push(PanelAction::SetBrightness(value));
        }
        actions
    }

    fn click(&self, area: AreaId, button: MouseButton) -> Option<PanelAction> {
        let right = button == MouseButton::Right;
        if area == self.tiles[0].area {
            return Some(PanelAction::Toggle("wifi"));
        }
        if area == self.tiles[1].area {
            return Some(if right {
                PanelAction::BluetoothPower
            } else {
                PanelAction::Toggle("bluetooth")
            });
        }
        if area == self.volume.chip.area {
            return Some(if right {
                PanelAction::Toggle("audio")
            } else {
                PanelAction::ToggleMute
            });
        }
        if self.media.areas().contains(&area) {
            return (!right).then(|| self.media.click(area)).flatten();
        }
        if let Some(toggle) = self.toggles.iter().find(|toggle| toggle.area == area) {
            return (!right).then_some(match toggle.kind {
                Switch::Awake => PanelAction::ToggleKeepAwake,
                Switch::Dnd => PanelAction::ToggleDnd,
                Switch::Mic => PanelAction::ToggleMic,
                Switch::Theme => PanelAction::ToggleTheme,
            });
        }
        let (_, action) = self.footer.iter().find(|(chip, _)| chip.area == area)?;
        if right {
            return None;
        }
        Some(match action {
            Footer::Toggle(name) => PanelAction::Toggle(name),
            Footer::Run(name) => PanelAction::Run(name),
            Footer::Suspend => PanelAction::Suspend,
        })
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        for chip in self.chips() {
            if !chip.shortcut.is_empty() && chip.hovered(input) {
                return Some(TooltipSpot {
                    key: chip.area,
                    text: chip.shortcut.clone(),
                    node: chip.root,
                    width: chip.width(),
                    height: chip.height(),
                });
            }
        }
        for toggle in &self.toggles {
            if !toggle.hint.is_empty() && input.contains_mouse(toggle.area) {
                return Some(TooltipSpot {
                    key: toggle.area,
                    text: toggle.hint.to_string(),
                    node: toggle.root,
                    width: toggle.width,
                    height: SWITCH,
                });
            }
        }
        for slider in [&self.volume.slider, &self.brightness.slider] {
            if !slider.shortcut.is_empty() && input.contains_mouse(slider.area) {
                return Some(TooltipSpot {
                    key: slider.area,
                    text: slider.shortcut.clone(),
                    node: slider.root,
                    width: slider.width(),
                    height: slider.height(),
                });
            }
        }
        None
    }
}
