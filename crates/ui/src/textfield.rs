use std::process::Command;

use platform::{CursorShape, KeyEvent, keys};

use crate::{
    Color, Node, NodeId, Paint, RectPaint, Span, TextEngine, TextPaint, TextSpec, Tree,
    anim::Millis,
    input::{AreaId, Input, MouseArea},
};

const BLINK: Millis = 500.0;
const HIGHLIGHT: Color = Color::rgb(0x000080);
const HIGHLIGHTED_TEXT: Color = Color::rgb(0xffffff);
const PASSWORD: char = '●';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyResult {
    Handled,
    Accepted,
    Escape,
    Ignored,
}

pub struct TextField {
    pub root: NodeId,
    text_node: NodeId,
    selection: NodeId,
    caret: NodeId,
    pub area: AreaId,
    text: String,
    cursor: usize,
    anchor: usize,
    focus: bool,
    password: bool,
    family: &'static str,
    size: f32,
    color: Color,
    carets: Vec<(usize, f32)>,
    height: f32,
    width: f32,
    scroll: f32,
    blink_start: Millis,
    dragging: bool,
}

fn word_left(text: &str, from: usize) -> usize {
    let before = &text[..from];
    let trimmed = before.trim_end();
    trimmed
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(index, c)| index + c.len_utf8())
}

fn word_right(text: &str, from: usize) -> usize {
    let after = &text[from..];
    let skip = after.len() - after.trim_start().len();
    let rest = &after[skip..];
    from + skip
        + rest
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map_or(rest.len(), |(index, _)| index)
}

impl TextField {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        family: &'static str,
        size: f32,
        color: Color,
        password: bool,
    ) -> Self {
        let mut root_node = Node::new(0.0, 0.0, 0.0, 0.0);
        root_node.clip = true;
        let root = tree.add(parent, root_node);
        let selection = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                0.0,
                0.0,
                RectPaint {
                    color: HIGHLIGHT,
                    ..RectPaint::default()
                },
            ),
        );
        let height = engine
            .measure(&TextSpec::plain("Ag", family, size, color), 1.0)
            .height;
        let spec = TextSpec::plain("", family, size, color);
        let text_node = tree.add(
            root,
            Node::new(0.0, 0.0, 0.0, height).with_paint(Paint::Text(TextPaint::new(spec))),
        );
        let caret = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                1.0,
                height,
                RectPaint {
                    color,
                    ..RectPaint::default()
                },
            ),
        );
        let mut area = MouseArea::new(root);
        area.cursor = CursorShape::Text;
        let area = input.add(area);
        let mut field = Self {
            root,
            text_node,
            selection,
            caret,
            area,
            text: String::new(),
            cursor: 0,
            anchor: 0,
            focus: false,
            password,
            family,
            size,
            color,
            carets: vec![(0, 0.0)],
            height,
            width: 0.0,
            scroll: 0.0,
            blink_start: 0.0,
            dragging: false,
        };
        field.refresh(tree, engine);
        field
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn has_focus(&self) -> bool {
        self.focus
    }

    pub fn implicit_height(&self) -> f32 {
        self.height
    }

    pub fn set_focus(&mut self, focus: bool, now: Millis) {
        self.focus = focus;
        self.blink_start = now;
    }

    pub fn set_text(&mut self, tree: &mut Tree, engine: &mut TextEngine, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
        self.anchor = self.cursor;
        self.refresh(tree, engine);
    }

    pub fn select_all(&mut self, tree: &mut Tree, engine: &mut TextEngine) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.refresh(tree, engine);
    }

    pub fn set_selection_color(&mut self, tree: &mut Tree, color: Color) {
        tree.set_fill(self.selection, color);
    }

    fn styled(&self, shown: String) -> TextSpec {
        let (start, end) = self.range();
        if start == end {
            return TextSpec::plain(shown, self.family, self.size, self.color);
        }
        let (start, end) = (self.shown_index(start), self.shown_index(end));
        let mut spec = TextSpec::plain("", self.family, self.size, self.color);
        spec.spans = [
            (&shown[..start], None),
            (&shown[start..end], Some(HIGHLIGHTED_TEXT)),
            (&shown[end..], None),
        ]
        .into_iter()
        .filter(|(text, _)| !text.is_empty())
        .map(|(text, color)| {
            let mut span = Span::new(text, self.family, self.size);
            span.color = color;
            span
        })
        .collect();
        spec
    }

    fn shown(&self) -> String {
        if self.password {
            std::iter::repeat_n(PASSWORD, self.text.chars().count()).collect()
        } else {
            self.text.clone()
        }
    }

    fn shown_index(&self, index: usize) -> usize {
        if self.password {
            self.text[..index].chars().count() * PASSWORD.len_utf8()
        } else {
            index
        }
    }

    fn x_of(&self, index: usize) -> f32 {
        let shown = self.shown_index(index);
        self.carets
            .iter()
            .find(|(byte, _)| *byte >= shown)
            .or(self.carets.last())
            .map_or(0.0, |(_, x)| *x)
    }

    fn index_at(&self, x: f32) -> usize {
        let target = x + self.scroll;
        let mut best = (0usize, f32::MAX);
        for (byte, caret) in &self.carets {
            let distance = (caret - target).abs();
            if distance < best.1 {
                best = (*byte, distance);
            }
        }
        if self.password {
            let chars = best.0 / PASSWORD.len_utf8();
            self.text
                .char_indices()
                .nth(chars)
                .map_or(self.text.len(), |(index, _)| index)
        } else {
            best.0
        }
    }

    fn refresh(&mut self, tree: &mut Tree, engine: &mut TextEngine) {
        let spec = self.styled(self.shown());
        self.carets = engine.carets(&spec);
        let width = engine.measure(&spec, 1.0).width;
        tree.set_text(self.text_node, spec);
        let cursor_x = self.x_of(self.cursor);
        if self.width > 0.0 {
            if cursor_x - self.scroll > self.width {
                self.scroll = cursor_x - self.width;
            } else if cursor_x < self.scroll {
                self.scroll = cursor_x;
            }
            self.scroll = self.scroll.min((width - self.width).max(0.0)).max(0.0);
        }
        tree.set_pos(self.text_node, -self.scroll, 0.0);
        tree.set_size(self.text_node, width, self.height);
        let (start, end) = if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        };
        let x0 = self.x_of(start) - self.scroll;
        let x1 = self.x_of(end) - self.scroll;
        tree.set_visible(self.selection, start != end);
        tree.set_pos(self.selection, x0, 0.0);
        tree.set_size(self.selection, (x1 - x0).max(0.0), self.height);
        tree.set_pos(self.caret, cursor_x - self.scroll, 0.0);
    }

    pub fn place(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, height);
        if self.width != width {
            self.width = width;
            self.refresh(tree, engine);
        }
    }

    pub fn tick(&mut self, tree: &mut Tree, now: Millis) {
        let visible = self.focus && ((now - self.blink_start) / BLINK).floor() as i64 % 2 == 0;
        tree.set_visible(self.caret, visible);
    }

    pub fn next_blink(&self, now: Millis) -> Option<Millis> {
        if !self.focus {
            return None;
        }
        let phase = ((now - self.blink_start) / BLINK).floor() + 1.0;
        Some(self.blink_start + phase * BLINK)
    }

    fn insert(&mut self, value: &str) {
        let (start, end) = self.range();
        self.text.replace_range(start..end, value);
        self.cursor = start + value.len();
        self.anchor = self.cursor;
    }

    fn range(&self) -> (usize, usize) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    fn prev(&self, from: usize) -> usize {
        self.text[..from]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next(&self, from: usize) -> usize {
        self.text[from..]
            .chars()
            .next()
            .map_or(from, |c| from + c.len_utf8())
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        event: &KeyEvent,
        now: Millis,
    ) -> KeyResult {
        if !self.focus || !event.pressed {
            return KeyResult::Ignored;
        }
        let ctrl = event.modifiers.ctrl;
        let shift = event.modifiers.shift;
        let (start, end) = self.range();
        let moved = |field: &mut Self, to: usize| {
            field.cursor = to;
            if !shift {
                field.anchor = to;
            }
        };
        match event.keysym {
            keys::ESCAPE => return KeyResult::Escape,
            keys::RETURN | keys::KP_ENTER => return KeyResult::Accepted,
            keys::BACKSPACE => {
                if start != end {
                    self.insert("");
                } else if self.cursor > 0 {
                    let from = if ctrl {
                        word_left(&self.text, self.cursor)
                    } else {
                        self.prev(self.cursor)
                    };
                    self.text.replace_range(from..self.cursor, "");
                    self.cursor = from;
                    self.anchor = from;
                }
            }
            keys::DELETE => {
                if start != end {
                    self.insert("");
                } else if self.cursor < self.text.len() {
                    let to = if ctrl {
                        word_right(&self.text, self.cursor)
                    } else {
                        self.next(self.cursor)
                    };
                    self.text.replace_range(self.cursor..to, "");
                }
            }
            keys::LEFT => {
                let to = if !shift && start != end {
                    start
                } else if ctrl {
                    word_left(&self.text, self.cursor)
                } else {
                    self.prev(self.cursor)
                };
                moved(self, to);
            }
            keys::RIGHT => {
                let to = if !shift && start != end {
                    end
                } else if ctrl {
                    word_right(&self.text, self.cursor)
                } else {
                    self.next(self.cursor)
                };
                moved(self, to);
            }
            keys::HOME => moved(self, 0),
            keys::END => {
                let len = self.text.len();
                moved(self, len);
            }
            _ => {
                let lower = event.utf8.as_deref().unwrap_or("").to_lowercase();
                if ctrl && (lower == "a" || event.keysym == 0x61) {
                    self.anchor = 0;
                    self.cursor = self.text.len();
                } else if ctrl && (event.keysym == 0x63 || event.keysym == 0x78) {
                    if start != end && !self.password {
                        let selected = self.text[start..end].to_string();
                        if let Ok(mut child) =
                            Command::new("wl-copy").arg("--").arg(&selected).spawn()
                        {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                        if event.keysym == 0x78 {
                            self.insert("");
                        }
                    }
                } else if ctrl && event.keysym == 0x76 {
                    if let Ok(output) = Command::new("wl-paste").arg("-n").output() {
                        let pasted =
                            String::from_utf8_lossy(&output.stdout).replace(['\n', '\r'], "");
                        self.insert(&pasted);
                    }
                } else if !ctrl
                    && !event.modifiers.alt
                    && let Some(text) = event.utf8.as_deref()
                    && !text.is_empty()
                    && !text.chars().any(char::is_control)
                {
                    self.insert(text);
                } else {
                    return KeyResult::Ignored;
                }
            }
        }
        self.blink_start = now;
        self.refresh(tree, engine);
        KeyResult::Handled
    }

    fn local_x(&self, tree: &Tree, x: f32, y: f32) -> Option<f32> {
        let inverse = tree.device_transform(self.root).invert()?;
        let mut point = tiny_skia::Point::from_xy(x, y);
        inverse.map_point(&mut point);
        Some(point.x)
    }

    pub fn press(&mut self, tree: &mut Tree, engine: &mut TextEngine, x: f32, y: f32, now: Millis) {
        let Some(local) = self.local_x(tree, x, y) else {
            return;
        };
        self.focus = true;
        self.blink_start = now;
        self.cursor = self.index_at(local);
        self.anchor = self.cursor;
        self.dragging = true;
        self.refresh(tree, engine);
    }

    pub fn drag(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        input: &Input,
        x: f32,
        y: f32,
    ) {
        if !self.dragging || !input.pressed(self.area) {
            self.dragging = false;
            return;
        }
        if let Some(local) = self.local_x(tree, x, y) {
            self.cursor = self.index_at(local);
            self.refresh(tree, engine);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_motion() {
        assert_eq!(word_left("hello big world", 15), 10);
        assert_eq!(word_left("hello big ", 10), 6);
        assert_eq!(word_right("hello big world", 0), 5);
        assert_eq!(word_right("hello big world", 5), 9);
    }

    #[test]
    fn empty_field_has_line_height() {
        let mut tree = Tree::new();
        let mut input = Input::new();
        let mut engine = TextEngine::new(&["Adwaita Sans"]);
        let root = tree.root();
        let field = TextField::new(
            &mut tree,
            &mut input,
            &mut engine,
            root,
            "Adwaita Sans",
            12.0,
            Color::rgb(0xffffff),
            false,
        );
        assert!(field.implicit_height() >= 12.0);
    }

    #[test]
    fn selection_splits_spans() {
        let mut tree = Tree::new();
        let mut input = Input::new();
        let mut engine = TextEngine::new(&["Adwaita Sans"]);
        let root = tree.root();
        let mut field = TextField::new(
            &mut tree,
            &mut input,
            &mut engine,
            root,
            "Adwaita Sans",
            12.0,
            Color::rgb(0xffffff),
            false,
        );
        field.set_text(&mut tree, &mut engine, "testnet");
        field.anchor = 4;
        let spec = field.styled(field.shown());
        let parts: Vec<_> = spec
            .spans
            .iter()
            .map(|span| (span.text.as_str(), span.color))
            .collect();
        assert_eq!(parts, vec![("test", None), ("net", Some(HIGHLIGHTED_TEXT))]);
    }
}
