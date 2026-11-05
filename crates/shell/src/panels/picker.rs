use ui::{
    Color, Node, NodeId, RectPaint, TextEngine, TextSpec, TransformOp, Tree,
    anim::{Easing, Flick, Millis, Tween},
    input::{AreaId, Input, MouseArea},
    textfield::TextField,
    widgets::Label,
};

use super::kit::{icon_label, label, rect, set_border};
use crate::theme::{self, FAST, NORMAL, SLOW};

pub const SEARCH_HEIGHT: f32 = 52.0;
const SPACING: f32 = 5.0;
const INTRO: Millis = 420.0;
const MOVE: Millis = 170.0;
const SCROLLBAR: Color = Color::rgb(0xb8b8b8);

pub fn eyebrow(tree: &mut Tree, engine: &mut TextEngine, parent: NodeId, text: &str) -> Label {
    let mut spec = TextSpec::plain(text, crate::theme::font(), 9.0, theme::green());
    if let Some(span) = spec.spans.first_mut() {
        span.letter_spacing = 1.0;
    }
    Label::new(tree, engine, parent, spec)
}

pub struct SearchBox {
    pub rect: NodeId,
    icon: Label,
    pub field: TextField,
    placeholder: Label,
    border: Tween<Color>,
    icon_color: Tween<Color>,
    animate_icon: bool,
}

impl SearchBox {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        glyph: &str,
        placeholder: &str,
        animate_icon: bool,
        now: Millis,
    ) -> Self {
        let rect = rect(tree, parent, theme::surface(), 12.0);
        set_border(tree, rect, theme::line(), 1.0);
        let icon = icon_label(tree, engine, rect, glyph, 18.0, theme::muted());
        let placeholder = label(tree, engine, rect, placeholder, 16.0, theme::muted());
        let mut field = TextField::new(
            tree,
            input,
            engine,
            rect,
            crate::theme::font(),
            16.0,
            theme::text(),
            false,
        );
        field.set_selection_color(tree, theme::line());
        field.set_focus(true, now);
        let mut border = Tween::new(theme::line(), NORMAL, Easing::Linear);
        border.start(now, theme::line(), theme::green());
        Self {
            rect,
            icon,
            field,
            placeholder,
            border,
            icon_color: Tween::new(theme::muted(), NORMAL, Easing::Linear),
            animate_icon,
        }
    }

    pub fn text(&self) -> &str {
        self.field.text()
    }

    pub fn set_placeholder(&mut self, engine: &mut TextEngine, text: &str) {
        self.placeholder.set_text(engine, text);
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, y: f32, width: f32) {
        tree.set_pos(self.rect, 0.0, y);
        tree.set_size(self.rect, width, SEARCH_HEIGHT);
        let iw = self.icon.implicit_width();
        let ih = self.icon.implicit_height().ceil();
        self.icon
            .place(tree, 16.0, (SEARCH_HEIGHT - ih) / 2.0, iw, ih);
        let fx = 16.0 + iw + 12.0;
        let fw = (width - fx - 16.0).max(0.0);
        let fh = self.field.implicit_height();
        self.field
            .place(tree, engine, fx, (SEARCH_HEIGHT - fh) / 2.0, fw, fh);
        let ph = self.placeholder.implicit_height().ceil();
        let pw = self.placeholder.implicit_width().min(fw);
        self.placeholder
            .place(tree, fx, (SEARCH_HEIGHT - ph) / 2.0, pw, ph);
    }

    pub fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let mut running = self.border.tick(now);
        set_border(tree, self.rect, self.border.value(), 1.0);
        let target = if self.field.text().is_empty() {
            theme::muted()
        } else {
            theme::green()
        };
        if self.animate_icon {
            if self.icon_color.target() != target {
                self.icon_color.set_target(now, target);
            }
            running |= self.icon_color.tick(now);
        } else {
            self.icon_color.jump(target);
        }
        self.icon.set_color(self.icon_color.value());
        self.icon.sync(tree);
        tree.set_visible(self.placeholder.node, self.field.text().is_empty());
        self.field.tick(tree, now);
        running
    }
}

#[derive(Clone, Copy)]
pub struct PickStyle {
    pub resize: Millis,
    pub intro_easing: Easing,
    pub scrollbar: bool,
    pub hit_right_margin: f32,
}

pub struct PickRow {
    pub root: NodeId,
    pub content: NodeId,
    pub area: AreaId,
    pub height: f32,
    pub y: f32,
    hover_bg: NodeId,
    hit: NodeId,
    hover: Tween<f32>,
    margin: Tween<f32>,
    arrow: Tween<f32>,
    intro: Option<Millis>,
}

struct Bar {
    node: NodeId,
    area: AreaId,
    opacity: Tween<f32>,
    hide_at: Option<Millis>,
}

pub struct PickList {
    pub view: NodeId,
    pub view_area: AreaId,
    content: NodeId,
    highlight: NodeId,
    bar: Option<Bar>,
    pub rows: Vec<PickRow>,
    current: i32,
    highlight_y: Tween<f32>,
    highlight_h: Tween<f32>,
    scroll: Flick,
    tracking: bool,
    moving: bool,
    opened: Millis,
    width: f32,
    view_height: f32,
    style: PickStyle,
}

impl PickList {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        parent: NodeId,
        style: PickStyle,
        now: Millis,
    ) -> Self {
        let mut view_node = Node::new(0.0, 0.0, 0.0, 0.0);
        view_node.clip = true;
        let view = tree.add(parent, view_node);
        let mut view_mouse = MouseArea::hover_handler(view);
        view_mouse.wheel = true;
        let view_area = input.add(view_mouse);
        let content = tree.add(view, Node::new(0.0, 0.0, 0.0, 0.0));
        let highlight = rect(tree, content, theme::elevated(), 11.0);
        let accent = tree.add(
            highlight,
            Node::rect(
                0.0,
                0.0,
                3.0,
                20.0,
                RectPaint {
                    color: theme::green(),
                    radius: 2.0,
                    ..RectPaint::default()
                },
            ),
        );
        let _ = accent;
        let bar = style.scrollbar.then(|| {
            let node = tree.add(
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
            let area = input.add(MouseArea::hover_handler(node));
            Bar {
                node,
                area,
                opacity: Tween::new(0.0, 200.0, Easing::Linear),
                hide_at: None,
            }
        });
        Self {
            view,
            view_area,
            content,
            highlight,
            bar,
            rows: Vec::new(),
            current: -1,
            highlight_y: Tween::new(0.0, MOVE, Easing::InOutQuad),
            highlight_h: Tween::new(0.0, style.resize.max(1.0), Easing::InOutQuad),
            scroll: Flick::new(0.0),
            tracking: false,
            moving: false,
            opened: now,
            width: 0.0,
            view_height: 0.0,
            style,
        }
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.view_area];
        if let Some(bar) = &self.bar {
            areas.push(bar.area);
        }
        areas.extend(self.rows.iter().map(|row| row.area));
        areas
    }

    pub fn clear(&mut self, tree: &mut Tree, input: &mut Input) {
        for row in self.rows.drain(..) {
            input.remove(row.area);
            tree.remove(row.root);
        }
    }

    pub fn push(&mut self, tree: &mut Tree, input: &mut Input, height: f32, now: Millis) -> usize {
        let index = self.rows.len();
        let y = self
            .rows
            .last()
            .map_or(0.0, |row| row.y + row.height + SPACING);
        let root = tree.add(self.content, Node::new(0.0, y, 0.0, height));
        let hover_bg = rect(tree, root, theme::surface(), 11.0);
        let content = tree.add(root, Node::new(0.0, 10.0, 0.0, height - 20.0));
        let hit = tree.add(root, Node::new(0.0, 0.0, 0.0, height));
        let area = input.add(MouseArea::new(hit));
        let intro = (now < self.opened + INTRO && index < 10).then_some(now + index as f64 * 24.0);
        self.rows.push(PickRow {
            root,
            content,
            area,
            height,
            y,
            hover_bg,
            hit,
            hover: Tween::new(0.0, FAST, Easing::Linear),
            margin: Tween::new(10.0, NORMAL, Easing::OutCubic),
            arrow: Tween::new(0.0, FAST, Easing::Linear),
            intro,
        });
        index
    }

    pub fn reset_selection(&mut self) {
        self.current = if self.rows.is_empty() { -1 } else { 0 };
        if let Some(row) = self.rows.first_mut() {
            row.margin.jump(16.0);
            row.arrow.jump(1.0);
            self.highlight_h.jump(row.height);
        }
        self.highlight_y.jump(0.0);
        self.scroll.jump(0.0);
        self.tracking = false;
        self.moving = false;
    }

    pub fn current(&self) -> i32 {
        self.current
    }

    pub fn current_index(&self) -> Option<usize> {
        usize::try_from(self.current)
            .ok()
            .filter(|index| *index < self.rows.len())
    }

    pub fn is_current(&self, index: usize) -> bool {
        index as i32 == self.current
    }

    pub fn set_current(&mut self, index: i32, now: Millis) {
        if index == self.current {
            return;
        }
        self.current = index;
        if let Some(row) = self.current_index().map(|index| &self.rows[index]) {
            let (y, height) = (row.y, row.height);
            self.highlight_y.set_target(now, y);
            if self.style.resize > 0.0 {
                self.highlight_h.set_target(now, height);
            } else {
                self.highlight_h.jump(height);
            }
            self.tracking = true;
        }
    }

    pub fn step(&mut self, step: i32, now: Millis) {
        let count = self.rows.len() as i32;
        if count > 0 {
            self.set_current((self.current + step).clamp(0, count - 1), now);
        }
    }

    pub fn row_at(&self, area: AreaId) -> Option<usize> {
        self.rows.iter().position(|row| row.area == area)
    }

    pub fn hovered(&self, input: &Input, index: usize) -> bool {
        self.rows
            .get(index)
            .is_some_and(|row| input.contains_mouse(row.area))
    }

    pub fn content_width(&self, index: usize) -> f32 {
        self.rows
            .get(index)
            .map_or(0.0, |row| (self.width - row.margin.value() - 10.0).max(0.0))
    }

    pub fn arrow_opacity(&self, index: usize) -> f32 {
        self.rows.get(index).map_or(0.0, |row| row.arrow.value())
    }

    fn content_height(&self) -> f32 {
        self.rows.last().map_or(0.0, |row| row.y + row.height)
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height() - self.view_height).max(0.0)
    }

    pub fn visible(&self, buffer: f32) -> Vec<usize> {
        let top = self.scroll.value() - buffer;
        let bottom = self.scroll.value() + self.view_height + buffer;
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.y + row.height > top && row.y < bottom)
            .map(|(index, _)| index)
            .collect()
    }

    pub fn wheel(&mut self, delta: i32, now: Millis) {
        self.tracking = false;
        let max = self.max_scroll();
        self.scroll.wheel(now, delta, 0.0, max);
        self.moving = self.scroll.is_running();
    }

    pub fn layout(&mut self, tree: &mut Tree, y: f32, width: f32, height: f32) {
        self.width = width;
        self.view_height = height;
        tree.set_pos(self.view, 0.0, y);
        tree.set_size(self.view, width, height);
        tree.set_size(self.content, width, self.content_height());
        let right = self.style.hit_right_margin;
        for row in &mut self.rows {
            tree.set_pos(row.root, 0.0, row.y);
            tree.set_size(row.root, width, row.height);
            tree.set_size(row.hover_bg, width, row.height);
            tree.set_size(row.hit, (width - right).max(0.0), row.height);
            let margin = row.margin.value();
            tree.set_pos(row.content, margin, 10.0);
            tree.set_size(
                row.content,
                (width - margin - 10.0).max(0.0),
                row.height - 20.0,
            );
        }
    }

    fn follow(&mut self) {
        let Some(current) = self.current_index() else {
            return;
        };
        if !self.tracking {
            return;
        }
        let size = self.view_height;
        let view_pos = self.scroll.value();
        let tracked_pos = self.highlight_y.value();
        let tracked_size = self.highlight_h.value();
        let tracked_end = tracked_pos + tracked_size;
        let row = &self.rows[current];
        let current_pos = row.y;
        let current_end = row.y + row.height;
        let mut pos = view_pos;
        if tracked_pos < view_pos && current_pos < view_pos {
            pos = tracked_pos.min(current_pos);
        } else if tracked_end >= view_pos + size && current_end >= view_pos + size {
            if tracked_end <= current_end {
                pos = tracked_end - size;
                if tracked_size > size {
                    pos = tracked_pos;
                }
            } else {
                pos = current_end - size;
                if row.height > size {
                    pos = current_pos;
                }
            }
        }
        if pos != view_pos {
            self.scroll.jump(pos.clamp(0.0, self.max_scroll()));
        }
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = self.highlight_y.tick(now);
        running |= self.highlight_h.tick(now);
        let scrolling = self.scroll.tick(now);
        running |= scrolling;
        self.follow();
        tree.set_pos(self.highlight, 0.0, self.highlight_y.value());
        tree.set_size(self.highlight, self.width, self.highlight_h.value());
        if let Some(accent) = tree
            .get(self.highlight)
            .and_then(|node| node.children().first().copied())
        {
            tree.set_pos(accent, 0.0, (self.highlight_h.value() - 20.0) / 2.0);
        }
        tree.set_opacity(self.highlight, if self.rows.is_empty() { 0.0 } else { 1.0 });
        tree.set_pos(self.content, 0.0, -self.scroll.value().round());
        running |= self.tick_bar(tree, input, now, scrolling);
        let width = self.width;
        for (index, row) in self.rows.iter_mut().enumerate() {
            let current = index as i32 == self.current;
            let hover = if input.contains_mouse(row.area) && !current {
                1.0
            } else {
                0.0
            };
            if row.hover.target() != hover {
                row.hover.set_target(now, hover);
            }
            let margin = if current { 16.0 } else { 10.0 };
            if row.margin.target() != margin {
                row.margin.set_target(now, margin);
            }
            let arrow = if current { 1.0 } else { 0.0 };
            if row.arrow.target() != arrow {
                row.arrow.set_target(now, arrow);
            }
            running |= row.hover.tick(now);
            running |= row.margin.tick(now);
            running |= row.arrow.tick(now);
            tree.set_opacity(row.hover_bg, row.hover.value());
            let margin = row.margin.value();
            tree.set_pos(row.content, margin, 10.0);
            tree.set_size(
                row.content,
                (width - margin - 10.0).max(0.0),
                row.height - 20.0,
            );
            let (opacity, lift) = match row.intro {
                Some(start) if now < start + SLOW.max(NORMAL) => {
                    running = true;
                    intro_values(self.style.intro_easing, start, now)
                }
                Some(_) => {
                    row.intro = None;
                    (1.0, 0.0)
                }
                None => (1.0, 0.0),
            };
            tree.set_opacity(row.root, opacity);
            tree.set_transforms(row.root, &[TransformOp::Translate { x: 0.0, y: lift }]);
        }
        running
    }

    fn tick_bar(&mut self, tree: &mut Tree, input: &Input, now: Millis, scrolling: bool) -> bool {
        let content_height = self.content_height();
        let scrollable = content_height > self.view_height;
        let scroll = self.scroll.value();
        let (width, height) = (self.width, self.view_height);
        let moving = self.moving && scrolling;
        if !scrolling {
            self.moving = false;
        }
        let Some(bar) = self.bar.as_mut() else {
            return false;
        };
        let mut running = false;
        if scrollable && (moving || input.contains_mouse(bar.area)) {
            bar.opacity.jump(0.75);
            bar.hide_at = Some(now + 450.0);
        } else if let Some(at) = bar.hide_at
            && at <= now
        {
            bar.hide_at = None;
            let from = bar.opacity.value();
            bar.opacity.start(now, from, 0.0);
        }
        running |= bar.hide_at.is_some();
        running |= bar.opacity.tick(now);
        if scrollable {
            let available = height - 4.0;
            let size = (height / content_height).clamp(0.0, 1.0);
            let position = scroll / content_height;
            tree.set_pos(bar.node, width - 8.0, 2.0 + position * available);
            tree.set_size(bar.node, 6.0, (size * available).max(6.0));
        }
        tree.set_opacity(bar.node, if scrollable { bar.opacity.value() } else { 0.0 });
        running
    }
}

fn intro_values(easing: Easing, start: Millis, now: Millis) -> (f32, f32) {
    if now < start {
        return (0.0, 14.0);
    }
    let t = now - start;
    let opacity = easing.value((t / NORMAL).min(1.0)) as f32;
    let lift = 14.0 * (1.0 - Easing::OutCubic.value((t / SLOW).min(1.0)) as f32);
    (opacity, lift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_stack_with_spacing() {
        let mut tree = Tree::new();
        let mut input = Input::new();
        let root = tree.root();
        let style = PickStyle {
            resize: 120.0,
            intro_easing: Easing::Linear,
            scrollbar: true,
            hit_right_margin: 60.0,
        };
        let mut list = PickList::new(&mut tree, &mut input, root, style, 0.0);
        for height in [48.0, 84.0, 48.0] {
            list.push(&mut tree, &mut input, height, 10_000.0);
        }
        let ys: Vec<f32> = list.rows.iter().map(|row| row.y).collect();
        assert_eq!(ys, vec![0.0, 53.0, 142.0]);
        list.reset_selection();
        assert_eq!(list.current(), 0);
        list.step(5, 10_000.0);
        assert_eq!(list.current(), 2);
    }

    #[test]
    fn intro_starts_hidden_and_settles() {
        assert_eq!(intro_values(Easing::Linear, 100.0, 50.0), (0.0, 14.0));
        assert_eq!(intro_values(Easing::Linear, 0.0, SLOW), (1.0, 0.0));
    }
}
