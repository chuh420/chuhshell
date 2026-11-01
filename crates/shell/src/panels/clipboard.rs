use std::{collections::HashMap, sync::Arc};

use platform::{CursorShape, KeyEvent, MouseButton, keys};
use services::clipboard::ClipEntry;
use ui::{
    Node, NodeId, Paint, TextEngine, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, linear},
    textfield::KeyResult,
    tiny_skia::Pixmap,
    widgets::{Chip, Label},
};

use super::{
    PanelAction, PanelData,
    kit::{chip, icon_label, label, place_chip, place_label, rect},
    picker::{PickList, PickStyle, SEARCH_HEIGHT, SearchBox, eyebrow},
};
use crate::theme::{ELEVATED, FAST, GREEN, MUTED, NORMAL, RED, SLOW, TEXT};

const TEXT_HEIGHT: f32 = 48.0;
const IMAGE_HEIGHT: f32 = 84.0;
const CACHE_BUFFER: f32 = 320.0;

struct Row {
    entry: ClipEntry,
    thumb_bg: NodeId,
    thumb: NodeId,
    thumb_opacity: Tween<f32>,
    thumb_set: bool,
    text: Label,
    meta: Label,
    delete: Label,
    delete_area: AreaId,
    delete_opacity: Tween<f32>,
    arrow: Label,
}

pub struct ClipboardPanel {
    pub root: NodeId,
    eyebrow: Label,
    title: Label,
    clear: Chip,
    close: Chip,
    search: SearchBox,
    list: PickList,
    rows: Vec<Row>,
    empty: Label,
    empty_opacity: Tween<f32>,
    footer: Label,
    entries: Option<Arc<Vec<ClipEntry>>>,
    restore: Option<i32>,
    thumbs: HashMap<String, Option<Arc<Pixmap>>>,
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn matches(entry: &ClipEntry, query: &str) -> bool {
    !entry.is_image() && entry.text.to_lowercase().contains(&query.to_lowercase())
}

impl ClipboardPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let eyebrow = eyebrow(tree, engine, root, "desktop / clipboard");
        let title = label(tree, engine, root, "clipboard", 26.0, TEXT);
        let clear = chip(tree, input, engine, root, "clear all", RED, false, None);
        let close = chip(tree, input, engine, root, "×", TEXT, false, None);
        let search = SearchBox::new(
            tree,
            input,
            engine,
            root,
            "󰅍",
            "search copied text…",
            false,
            now,
        );
        let style = PickStyle {
            resize: 120.0,
            intro_easing: Easing::Linear,
            scrollbar: true,
            hit_right_margin: 60.0,
        };
        let list = PickList::new(tree, input, root, style, now);
        let empty = label(tree, engine, list.view, "Clipboard is empty", 12.0, MUTED);
        let footer = label(
            tree,
            engine,
            root,
            "↑ ↓  select       ↵  copy       del  remove       esc  close",
            10.0,
            MUTED,
        );
        let mut panel = Self {
            root,
            eyebrow,
            title,
            clear,
            close,
            search,
            list,
            rows: Vec::new(),
            empty,
            empty_opacity: Tween::new(0.0, NORMAL, Easing::Linear),
            footer,
            entries: None,
            restore: None,
            thumbs: HashMap::new(),
        };
        panel.sync(tree, input, engine, data, now);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.clear.area, self.close.area, self.search.field.area];
        areas.extend(self.list.areas());
        areas.extend(self.rows.iter().map(|row| row.delete_area));
        areas
    }

    fn loading(&self) -> bool {
        self.entries.is_none()
    }

    fn filtered(&self) -> Vec<ClipEntry> {
        let Some(entries) = &self.entries else {
            return Vec::new();
        };
        let query = self.search.text();
        if query.is_empty() {
            entries.to_vec()
        } else {
            entries
                .iter()
                .filter(|entry| matches(entry, query))
                .cloned()
                .collect()
        }
    }

    fn make_row(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        content: NodeId,
        entry: ClipEntry,
    ) -> Row {
        let image = entry.is_image();
        let thumb_bg = rect(tree, content, ELEVATED, 8.0);
        if let Some(node) = tree.get_mut(thumb_bg) {
            node.clip = true;
        }
        tree.set_visible(thumb_bg, image);
        let thumb = tree.add(thumb_bg, Node::new(0.0, 0.0, 112.0, 64.0));
        let text = if image {
            "image".to_string()
        } else {
            one_line(&entry.text)
        };
        let text = label(tree, engine, content, &text, 13.0, TEXT);
        let meta_text = if image {
            format!(
                "{} · {}×{} · {}",
                entry.format, entry.width, entry.height, entry.size
            )
        } else {
            String::new()
        };
        let meta = label(tree, engine, content, &meta_text, 10.0, MUTED);
        tree.set_visible(meta.node, image);
        let delete = icon_label(tree, engine, content, "󰆴", 15.0, MUTED);
        let mut delete_mouse = MouseArea::new(delete.node);
        delete_mouse.cursor = CursorShape::Default;
        let delete_area = input.add(delete_mouse);
        let arrow = label(tree, engine, content, "↵", 12.0, GREEN);
        Row {
            entry,
            thumb_bg,
            thumb,
            thumb_opacity: Tween::new(0.0, SLOW, Easing::Linear),
            thumb_set: false,
            text,
            meta,
            delete,
            delete_area,
            delete_opacity: Tween::new(0.0, FAST, Easing::Linear),
            arrow,
        }
    }

    fn reset(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, now: Millis) {
        for row in self.rows.drain(..) {
            input.remove(row.delete_area);
        }
        self.list.clear(tree, input);
        for entry in self.filtered() {
            let height = if entry.is_image() {
                IMAGE_HEIGHT
            } else {
                TEXT_HEIGHT
            };
            let index = self.list.push(tree, input, height, now);
            let content = self.list.rows[index].content;
            let row = Self::make_row(tree, input, engine, content, entry);
            self.rows.push(row);
        }
        self.list.reset_selection();
        if let Some(row) = self.list.current_index().and_then(|i| self.rows.get_mut(i)) {
            row.delete_opacity.jump(1.0);
        }
        let thumbs = std::mem::take(&mut self.thumbs);
        self.apply_thumbs(tree, &thumbs, now);
        self.thumbs = thumbs;
        if let Some(index) = self.restore.take() {
            self.list
                .set_current(index.min(self.rows.len() as i32 - 1), now);
        }
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
        now: Millis,
    ) {
        let Some(entries) = &data.clipboard else {
            return;
        };
        let changed = match &self.entries {
            Some(current) => !Arc::ptr_eq(current, entries) && **current != **entries,
            None => true,
        };
        if changed {
            self.entries = Some(entries.clone());
            self.reset(tree, input, engine, now);
        }
    }

    pub fn set_thumbs(
        &mut self,
        tree: &mut Tree,
        thumbs: &HashMap<String, Option<Arc<Pixmap>>>,
        now: Millis,
    ) {
        self.apply_thumbs(tree, thumbs, now);
        self.thumbs = thumbs.clone();
    }

    fn apply_thumbs(
        &mut self,
        tree: &mut Tree,
        thumbs: &HashMap<String, Option<Arc<Pixmap>>>,
        now: Millis,
    ) {
        for row in &mut self.rows {
            if row.thumb_set || !row.entry.is_image() {
                continue;
            }
            let Some(thumb) = thumbs.get(&row.entry.id) else {
                continue;
            };
            row.thumb_set = true;
            if let Some(pixmap) = thumb {
                if let Some(node) = tree.get_mut(row.thumb) {
                    node.paint = Paint::Image(pixmap.clone());
                }
                row.thumb_opacity.start(now, 0.0, 1.0);
            }
        }
    }

    pub fn thumb_requests(&self) -> Vec<(String, String)> {
        self.list
            .visible(CACHE_BUFFER)
            .into_iter()
            .filter_map(|index| self.rows.get(index))
            .filter(|row| row.entry.is_image() && !row.thumb_set)
            .map(|row| {
                let format = if row.entry.format.is_empty() {
                    "png".to_string()
                } else {
                    row.entry.format.clone()
                };
                (row.entry.id.clone(), format)
            })
            .collect()
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        let eyebrow_h = self.eyebrow.implicit_height().ceil();
        let clear_visible = self.entries.as_ref().is_some_and(|e| !e.is_empty());
        let header_h = self
            .title
            .implicit_height()
            .ceil()
            .max(self.close.implicit_height().ceil());
        let footer_h = self.footer.implicit_height().ceil();
        let slots = linear(
            &[
                Cell::implicit(eyebrow_h),
                Cell::implicit(header_h),
                Cell::layout(SEARCH_HEIGHT),
                Cell::fill(0.0),
                Cell::implicit(footer_h),
            ],
            14.0,
            height,
        );
        let ew = self.eyebrow.implicit_width();
        place_label(tree, &mut self.eyebrow, 0.0, slots[0], ew);
        let mut cells = vec![Cell::fill(self.title.implicit_width().ceil())];
        if clear_visible {
            cells.push(Cell::implicit(self.clear.implicit_width()));
        }
        cells.push(Cell::implicit(self.close.implicit_width()));
        let columns = linear(&cells, 5.0, width);
        place_label(
            tree,
            &mut self.title,
            columns[0].pos,
            slots[1],
            columns[0].size,
        );
        tree.set_visible(self.clear.root, clear_visible);
        if clear_visible {
            place_chip(tree, &mut self.clear, columns[1].pos, slots[1]);
        }
        if let Some(column) = columns.last() {
            place_chip(tree, &mut self.close, column.pos, slots[1]);
        }
        self.search.layout(tree, engine, slots[2].pos, width);
        let list = slots[3];
        self.list.layout(tree, list.pos, width, list.size);
        let empty_text = if self.search.text().is_empty() {
            "Clipboard is empty"
        } else {
            "No matches"
        };
        self.empty.set_text(engine, empty_text);
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, (width - ew) / 2.0, (list.size - eh) / 2.0, ew, eh);
        let fw = self.footer.implicit_width();
        place_label(tree, &mut self.footer, 0.0, slots[4], fw);
        self.layout_rows(tree);
    }

    fn layout_rows(&mut self, tree: &mut Tree) {
        for (index, row) in self.rows.iter_mut().enumerate() {
            let cw = self.list.content_width(index);
            let ch = self.list.rows[index].height - 20.0;
            Self::layout_row(tree, row, cw, ch);
            tree.set_opacity(row.arrow.node, self.list.arrow_opacity(index));
        }
    }

    fn layout_row(tree: &mut Tree, row: &mut Row, cw: f32, ch: f32) {
        let image = row.entry.is_image();
        let mut x = 0.0;
        if image {
            tree.set_pos(row.thumb_bg, 0.0, (ch - 64.0) / 2.0);
            tree.set_size(row.thumb_bg, 112.0, 64.0);
            tree.set_size(row.thumb, 112.0, 64.0);
            x = 112.0 + 12.0;
        }
        let aw = row.arrow.implicit_width();
        let ah = row.arrow.implicit_height().ceil();
        let ax = cw - aw;
        row.arrow.place(tree, ax, (ch - ah) / 2.0, aw, ah);
        let dw = row.delete.implicit_width();
        let dh = row.delete.implicit_height().ceil();
        let dx = ax - 12.0 - dw;
        row.delete.place(tree, dx, (ch - dh) / 2.0, dw, dh);
        let column_w = (dx - 12.0 - x).max(0.0);
        let th = row.text.implicit_height().ceil();
        let mh = if image {
            row.meta.implicit_height().ceil()
        } else {
            0.0
        };
        let column_h = if image { th + 2.0 + mh } else { th };
        let y = (ch - column_h) / 2.0;
        row.text.place(tree, x, y, column_w, th);
        if image {
            let mw = row.meta.implicit_width().min(column_w);
            row.meta.place(tree, x, y + th + 2.0, mw, mh);
        }
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for chip in [&mut self.clear, &mut self.close] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.search.tick(tree, now);
        running |= self.list.tick(tree, input, now);
        for (index, row) in self.rows.iter_mut().enumerate() {
            let shown = self.list.is_current(index) || self.list.hovered(input, index);
            let target = if shown { 1.0 } else { 0.0 };
            if row.delete_opacity.target() != target {
                row.delete_opacity.set_target(now, target);
            }
            running |= row.delete_opacity.tick(now);
            running |= row.thumb_opacity.tick(now);
            tree.set_opacity(row.delete.node, row.delete_opacity.value());
            tree.set_opacity(row.thumb, row.thumb_opacity.value());
            let color = if input.contains_mouse(row.delete_area) {
                RED
            } else {
                MUTED
            };
            row.delete.set_color(color);
            row.delete.sync(tree);
        }
        self.layout_rows(tree);
        let empty_target = if self.rows.is_empty() && !self.loading() {
            1.0
        } else {
            0.0
        };
        if self.empty_opacity.target() != empty_target {
            self.empty_opacity.set_target(now, empty_target);
        }
        running |= self.empty_opacity.tick(now);
        tree.set_opacity(self.empty.node, self.empty_opacity.value());
        running
    }

    pub fn next_deadline(&self, now: Millis) -> Option<Millis> {
        self.search.field.next_blink(now)
    }

    fn remove(&mut self, index: usize, actions: &mut Vec<PanelAction>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        self.restore = Some(self.list.current());
        actions.push(PanelAction::ClipDelete(row.entry.id.clone()));
    }

    fn copy(&self, index: Option<usize>, actions: &mut Vec<PanelAction>) {
        let Some(row) = index.and_then(|index| self.rows.get(index)) else {
            return;
        };
        actions.push(PanelAction::ClipCopy(row.entry.id.clone()));
        actions.push(PanelAction::Close);
    }

    pub fn handle(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        events: &[InputEvent],
        pointer: (f32, f32),
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Press(area, MouseButton::Left) if area == self.search.field.area => {
                    self.search
                        .field
                        .press(tree, engine, pointer.0, pointer.1, now);
                }
                InputEvent::Wheel(area, delta) if area == self.list.view_area => {
                    self.list.wheel(delta, now);
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.close.area {
                        actions.push(PanelAction::Close);
                    } else if area == self.clear.area {
                        actions.push(PanelAction::ClipWipe);
                    } else if let Some(index) =
                        self.rows.iter().position(|row| row.delete_area == area)
                    {
                        self.remove(index, &mut actions);
                    } else if let Some(index) = self.list.row_at(area) {
                        self.copy(Some(index), &mut actions);
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn motion(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        input: &Input,
        x: f32,
        y: f32,
    ) {
        self.search.field.drag(tree, engine, input, x, y);
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        event: &KeyEvent,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut actions = Vec::new();
        match event.keysym {
            keys::ESCAPE => actions.push(PanelAction::Close),
            keys::DOWN | keys::TAB => self.list.step(1, now),
            keys::UP | keys::ISO_LEFT_TAB => self.list.step(-1, now),
            keys::PAGE_DOWN => self.list.step(6, now),
            keys::PAGE_UP => self.list.step(-6, now),
            keys::RETURN => self.copy(self.list.current_index(), &mut actions),
            keys::DELETE => {
                if let Some(index) = self.list.current_index() {
                    self.remove(index, &mut actions);
                }
            }
            keys::KP_ENTER => {}
            _ => {
                let before = self.search.text().to_string();
                match self.search.field.key(tree, engine, event, now) {
                    KeyResult::Ignored => return (false, actions),
                    KeyResult::Escape => actions.push(PanelAction::Close),
                    KeyResult::Accepted | KeyResult::Handled => {}
                }
                if self.search.text() != before {
                    self.reset(tree, input, engine, now);
                }
            }
        }
        (true, actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use services::clipboard::Kind;

    fn entry(kind: Kind, text: &str) -> ClipEntry {
        ClipEntry {
            id: "1".into(),
            kind,
            text: text.into(),
            size: String::new(),
            format: String::new(),
            width: 0,
            height: 0,
        }
    }

    #[test]
    fn search_skips_images_and_ignores_case() {
        assert!(matches(&entry(Kind::Text, "Hello World"), "world"));
        assert!(!matches(&entry(Kind::Image, ""), ""));
        assert!(!matches(&entry(Kind::Text, "abc"), "x"));
    }

    #[test]
    fn previews_collapse_whitespace() {
        assert_eq!(one_line("  a\n\tb   c "), "a b c");
    }
}
