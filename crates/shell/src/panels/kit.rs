use ui::{
    Color, Node, NodeId, RectPaint, TextEngine, TextSpec, Tree,
    anim::{Easing, Flick, Millis, Tween},
    input::{AreaId, Input, MouseArea},
    layout::{Slot, center},
    widgets::{Chip, ChipStyle, Label},
};

use crate::theme::{self};

pub fn chip_style() -> ChipStyle {
    ChipStyle {
        family: crate::theme::icons(),
        hover: theme::elevated(),
    }
}

const SCROLLBAR: Color = Color::rgb(0xb8b8b8);

pub fn label(
    tree: &mut Tree,
    engine: &mut TextEngine,
    parent: NodeId,
    text: &str,
    size: f32,
    color: Color,
) -> Label {
    Label::new(
        tree,
        engine,
        parent,
        TextSpec::plain(text, crate::theme::font(), size, color),
    )
}

pub fn icon_label(
    tree: &mut Tree,
    engine: &mut TextEngine,
    parent: NodeId,
    text: &str,
    size: f32,
    color: Color,
) -> Label {
    Label::new(
        tree,
        engine,
        parent,
        TextSpec::plain(text, crate::theme::icons(), size, color),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn chip(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    text: &str,
    ink: Color,
    compact: bool,
    icon_size: Option<f32>,
) -> Chip {
    Chip::new(
        tree,
        input,
        engine,
        parent,
        &chip_style(),
        text,
        ink,
        compact,
        icon_size,
    )
}

pub fn fit(slot: Slot, size: f32) -> (f32, f32) {
    let size = size.min(slot.size);
    (slot.pos + center(slot.size, size), size)
}

pub fn place_label(tree: &mut Tree, label: &mut Label, x: f32, slot: Slot, width: f32) {
    let (y, height) = fit(slot, label.implicit_height().ceil());
    label.place(tree, x, y, width, height);
}

pub fn place_chip(tree: &mut Tree, chip: &mut Chip, x: f32, slot: Slot) {
    let (y, height) = fit(slot, chip.implicit_height().ceil());
    let width = chip.implicit_width().ceil();
    chip.place_sized(tree, x, y, width, height);
}

pub fn rect(tree: &mut Tree, parent: NodeId, color: Color, radius: f32) -> NodeId {
    tree.add(
        parent,
        Node::rect(
            0.0,
            0.0,
            0.0,
            0.0,
            RectPaint {
                color,
                radius,
                ..RectPaint::default()
            },
        ),
    )
}

pub fn set_border(tree: &mut Tree, node: NodeId, color: Color, width: f32) {
    let changed = tree.get(node).is_some_and(|node| {
        matches!(&node.paint, ui::Paint::Rect(rect) if rect.border_color != color || rect.border_width != width)
    });
    if changed
        && let Some(node) = tree.get_mut(node)
        && let ui::Paint::Rect(rect) = &mut node.paint
    {
        rect.border_color = color;
        rect.border_width = width;
    }
}

pub struct Scroller {
    pub view: NodeId,
    pub content: NodeId,
    pub area: AreaId,
    bar: NodeId,
    bar_area: AreaId,
    scroll: Flick,
    bar_opacity: Tween<f32>,
    hide_at: Option<Millis>,
    has_bar: bool,
    height: f32,
    width: f32,
    content_height: f32,
}

impl Scroller {
    pub fn new(tree: &mut Tree, input: &mut Input, parent: NodeId, has_bar: bool) -> Self {
        let mut view_node = Node::new(0.0, 0.0, 0.0, 0.0);
        view_node.clip = true;
        let view = tree.add(parent, view_node);
        let mut mouse = MouseArea::hover_handler(view);
        mouse.wheel = true;
        let area = input.add(mouse);
        let content = tree.add(view, Node::new(0.0, 0.0, 0.0, 0.0));
        let bar = tree.add(
            view,
            Node::rect(
                0.0,
                0.0,
                6.0,
                0.0,
                RectPaint {
                    color: SCROLLBAR,
                    radius: 3.0,
                    ..RectPaint::default()
                },
            ),
        );
        let bar_area = input.add(MouseArea::hover_handler(bar));
        Self {
            view,
            content,
            area,
            bar,
            bar_area,
            scroll: Flick::new(0.0),
            bar_opacity: Tween::new(0.0, 200.0, Easing::Linear),
            hide_at: None,
            has_bar,
            height: 0.0,
            width: 0.0,
            content_height: 0.0,
        }
    }

    pub fn areas(&self) -> [AreaId; 2] {
        [self.area, self.bar_area]
    }

    pub fn place(
        &mut self,
        tree: &mut Tree,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        content_height: f32,
    ) {
        self.width = width;
        self.height = height;
        self.content_height = content_height;
        tree.set_pos(self.view, x, y);
        tree.set_size(self.view, width, height);
        tree.set_size(self.content, width, content_height);
    }

    fn max(&self) -> f32 {
        (self.content_height - self.height).max(0.0)
    }

    pub fn value(&self) -> f32 {
        self.scroll.value()
    }

    pub fn jump(&mut self, pos: f32) {
        self.scroll.jump(pos.clamp(0.0, self.max()));
    }

    pub fn wheel(&mut self, delta: i32, now: Millis) {
        let max = self.max();
        self.scroll.wheel(now, delta, 0.0, max);
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let max = self.max();
        if self.scroll.target() > max {
            let value = self.scroll.value().min(max);
            self.scroll.jump(value);
        }
        let mut running = self.scroll.tick(now);
        tree.set_pos(self.content, 0.0, -self.scroll.value().round());
        let scrollable = self.has_bar && self.content_height > self.height;
        if scrollable && (self.scroll.is_running() || input.contains_mouse(self.bar_area)) {
            self.bar_opacity.jump(0.75);
            self.hide_at = Some(now + 450.0);
        } else if let Some(at) = self.hide_at
            && at <= now
        {
            self.hide_at = None;
            let from = self.bar_opacity.value();
            self.bar_opacity.start(now, from, 0.0);
        }
        running |= self.hide_at.is_some();
        running |= self.bar_opacity.tick(now);
        if scrollable {
            let available = self.height - 4.0;
            let size = (self.height / self.content_height).clamp(0.0, 1.0);
            let position = self.scroll.value() / self.content_height;
            tree.set_pos(self.bar, self.width - 8.0, 2.0 + position * available);
            tree.set_size(self.bar, 6.0, (size * available).max(6.0));
        }
        tree.set_opacity(
            self.bar,
            if scrollable {
                self.bar_opacity.value()
            } else {
                0.0
            },
        );
        running
    }
}
