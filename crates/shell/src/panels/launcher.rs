use std::{collections::HashMap, sync::Arc};

use platform::{KeyEvent, MouseButton, keys};
use services::apps::AppEntry;
use ui::{
    Color, Node, NodeId, Paint, RectPaint, TextEngine, TextSpec, TransformOp, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, linear},
    textfield::{KeyResult, TextField},
    widgets::{Chip, Label},
};

use super::{
    AppIcon, PanelAction, PanelData,
    kit::{chip, icon_label, label, place_chip, place_label, rect, set_border},
};
use crate::theme::{
    ELEVATED, FAST, FONT, GREEN, ICON_FALLBACK, LINE, MUTED, NORMAL, SLOW, SURFACE, TEXT,
};

const ROW_HEIGHT: f32 = 56.0;
const ROW_SPACING: f32 = 5.0;
const SEARCH_HEIGHT: f32 = 52.0;
const SCROLL_STEP: f32 = 70.0;
const CACHE_BUFFER: f32 = 320.0;
const INTRO: Millis = 420.0;
const MOVE: Millis = 170.0;

struct Row {
    app: AppEntry,
    root: NodeId,
    hover_bg: NodeId,
    content: NodeId,
    icon_bg: NodeId,
    image: NodeId,
    letter: Label,
    name: Label,
    sub: Label,
    eye: Label,
    arrow: Label,
    area: AreaId,
    hidden: bool,
    hover: Tween<f32>,
    margin: Tween<f32>,
    arrow_opacity: Tween<f32>,
    intro: Option<Millis>,
    icon_set: bool,
}

pub struct LauncherPanel {
    pub root: NodeId,
    eyebrow: Label,
    title: Label,
    close: Chip,
    search: NodeId,
    search_icon: Label,
    field: TextField,
    placeholder: Label,
    border: Tween<Color>,
    icon_color: Tween<Color>,
    view: NodeId,
    view_area: AreaId,
    content: NodeId,
    highlight: NodeId,
    rows: Vec<Row>,
    empty: Label,
    empty_opacity: Tween<f32>,
    footer: Label,
    apps: Arc<Vec<AppEntry>>,
    hidden: Vec<String>,
    show_hidden: bool,
    current: i32,
    highlight_y: Tween<f32>,
    scroll: Tween<f32>,
    tracking: bool,
    restore: Option<(String, i32)>,
    opened: Millis,
    icons: HashMap<String, AppIcon>,
    width: f32,
    view_height: f32,
}

fn private_use(c: char) -> bool {
    ('\u{e000}'..='\u{f8ff}').contains(&c) || c >= '\u{f0000}'
}

pub fn fallback_spec(text: &str, size: f32, color: Color) -> TextSpec {
    let mut spec = TextSpec::plain("", FONT, size, color);
    let mut spans: Vec<ui::Span> = Vec::new();
    for c in text.chars() {
        let family = if private_use(c) { ICON_FALLBACK } else { FONT };
        match spans.last_mut() {
            Some(span) if span.family == family => span.text.push(c),
            _ => spans.push(ui::Span::new(c.to_string(), family, size)),
        }
    }
    if !spans.is_empty() {
        spec.spans = spans;
    }
    spec
}

fn row_y(index: usize) -> f32 {
    index as f32 * (ROW_HEIGHT + ROW_SPACING)
}

fn intro_values(start: Millis, now: Millis) -> (f32, f32) {
    if now < start {
        return (0.0, 14.0);
    }
    let t = now - start;
    let opacity = Easing::OutCubic.value((t / NORMAL).min(1.0)) as f32;
    let lift = 14.0 * (1.0 - Easing::OutCubic.value((t / SLOW).min(1.0)) as f32);
    (opacity, lift)
}

impl LauncherPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let mut eyebrow_spec = TextSpec::plain("workspace / launch", FONT, 9.0, GREEN);
        if let Some(span) = eyebrow_spec.spans.first_mut() {
            span.letter_spacing = 1.0;
        }
        let eyebrow = Label::new(tree, engine, root, eyebrow_spec);
        let title = label(tree, engine, root, "applications", 26.0, TEXT);
        let close = chip(tree, input, engine, root, "×", TEXT, false, None);
        let search = rect(tree, root, SURFACE, 12.0);
        set_border(tree, search, LINE, 1.0);
        let search_icon = icon_label(tree, engine, search, "󰍉", 18.0, MUTED);
        let placeholder = label(tree, engine, search, "find an application…", 16.0, MUTED);
        let mut field = TextField::new(tree, input, engine, search, FONT, 16.0, TEXT, false);
        field.set_selection_color(tree, LINE);
        field.set_focus(true, now);
        let mut view_node = Node::new(0.0, 0.0, 0.0, 0.0);
        view_node.clip = true;
        let view = tree.add(root, view_node);
        let mut view_mouse = MouseArea::hover_handler(view);
        view_mouse.wheel = true;
        let view_area = input.add(view_mouse);
        let content = tree.add(view, Node::new(0.0, 0.0, 0.0, 0.0));
        let highlight = rect(tree, content, ELEVATED, 11.0);
        tree.add(
            highlight,
            Node::rect(
                0.0,
                (ROW_HEIGHT - 20.0) / 2.0,
                3.0,
                20.0,
                RectPaint {
                    color: GREEN,
                    radius: 2.0,
                    ..RectPaint::default()
                },
            ),
        );
        let empty = label(tree, engine, view, "No results found", 12.0, MUTED);
        let footer = label(tree, engine, root, "", 10.0, MUTED);
        let mut border = Tween::new(LINE, NORMAL, Easing::Linear);
        border.start(now, LINE, GREEN);
        let mut panel = Self {
            root,
            eyebrow,
            title,
            close,
            search,
            search_icon,
            field,
            placeholder,
            border,
            icon_color: Tween::new(MUTED, NORMAL, Easing::Linear),
            view,
            view_area,
            content,
            highlight,
            rows: Vec::new(),
            empty,
            empty_opacity: Tween::new(0.0, NORMAL, Easing::Linear),
            footer,
            apps: Arc::new(Vec::new()),
            hidden: Vec::new(),
            show_hidden: false,
            current: -1,
            highlight_y: Tween::new(0.0, MOVE, Easing::InOutQuad),
            scroll: Tween::new(0.0, 110.0, Easing::OutCubic),
            tracking: false,
            restore: None,
            opened: now,
            icons: HashMap::new(),
            width: 0.0,
            view_height: 0.0,
        };
        panel.apps = data.apps.clone();
        panel.hidden = data.hidden.clone();
        panel.reset(tree, input, engine, now);
        panel
            .empty_opacity
            .jump(if panel.rows.is_empty() { 1.0 } else { 0.0 });
        panel.update_footer(engine);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.close.area, self.field.area, self.view_area];
        areas.extend(self.rows.iter().map(|row| row.area));
        areas
    }

    fn model(&self) -> Vec<AppEntry> {
        self.apps
            .iter()
            .filter(|app| self.show_hidden || !self.hidden.contains(&app.id))
            .cloned()
            .collect()
    }

    fn content_height(&self) -> f32 {
        let n = self.rows.len();
        if n == 0 {
            0.0
        } else {
            n as f32 * ROW_HEIGHT + (n - 1) as f32 * ROW_SPACING
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height() - self.view_height).max(0.0)
    }

    fn make_row(
        &self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        app: AppEntry,
        index: usize,
        now: Millis,
    ) -> Row {
        let root = tree.add(self.content, Node::new(0.0, row_y(index), 0.0, ROW_HEIGHT));
        let hover_bg = rect(tree, root, SURFACE, 11.0);
        let content = tree.add(root, Node::new(0.0, 10.0, 0.0, ROW_HEIGHT - 20.0));
        let icon_bg = rect(tree, content, ELEVATED, 7.0);
        let image = tree.add(icon_bg, Node::new(0.0, 0.0, 28.0, 28.0));
        let initial: String = app.name.chars().take(1).collect();
        let letter = label(tree, engine, icon_bg, &initial, 16.0, GREEN);
        let name = label(tree, engine, content, &app.name, 14.0, TEXT);
        let subtitle = if !app.generic.is_empty() {
            app.generic.as_str()
        } else if !app.comment.is_empty() {
            app.comment.as_str()
        } else {
            "Application"
        };
        let sub = label(tree, engine, content, subtitle, 10.0, MUTED);
        let eye = icon_label(tree, engine, content, "󰈈", 18.0, MUTED);
        let arrow = label(tree, engine, content, "↵", 12.0, GREEN);
        let area = input.add(MouseArea::new(root));
        let hidden = self.hidden.contains(&app.id);
        let current = index as i32 == self.current;
        let intro = (now < self.opened + INTRO && index < 10).then_some(now + index as f64 * 24.0);
        tree.set_visible(letter.node, false);
        Row {
            app,
            root,
            hover_bg,
            content,
            icon_bg,
            image,
            letter,
            name,
            sub,
            eye,
            arrow,
            area,
            hidden,
            hover: Tween::new(0.0, FAST, Easing::Linear),
            margin: Tween::new(if current { 16.0 } else { 10.0 }, NORMAL, Easing::OutCubic),
            arrow_opacity: Tween::new(if current { 1.0 } else { 0.0 }, FAST, Easing::Linear),
            intro,
            icon_set: false,
        }
    }

    fn reset(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, now: Millis) {
        for row in self.rows.drain(..) {
            input.remove(row.area);
            tree.remove(row.root);
        }
        let model = self.model();
        self.current = if model.is_empty() { -1 } else { 0 };
        for (index, app) in model.into_iter().enumerate() {
            let row = self.make_row(tree, input, engine, app, index, now);
            self.rows.push(row);
        }
        self.highlight_y.jump(0.0);
        self.scroll.jump(0.0);
        self.tracking = false;
        let icons = std::mem::take(&mut self.icons);
        self.apply_icons(tree, &icons);
        self.icons = icons;
        if let Some((id, index)) = self.restore.take() {
            let found = self.rows.iter().position(|row| row.app.id == id);
            let target = match found {
                Some(found) => found as i32,
                None => index.min(self.rows.len() as i32 - 1),
            };
            self.set_current(target, now);
        }
        self.layout_rows(tree, engine);
    }

    fn set_current(&mut self, index: i32, now: Millis) {
        if index == self.current {
            return;
        }
        self.current = index;
        if index >= 0 {
            self.highlight_y.set_target(now, row_y(index as usize));
            self.tracking = true;
        }
    }

    fn step(&mut self, step: i32, now: Millis) {
        let count = self.rows.len() as i32;
        if count > 0 {
            self.set_current((self.current + step).clamp(0, count - 1), now);
        }
    }

    fn current_row(&self) -> Option<&Row> {
        usize::try_from(self.current)
            .ok()
            .and_then(|index| self.rows.get(index))
    }

    fn follow(&mut self) {
        if !self.tracking || self.current < 0 {
            return;
        }
        let size = self.view_height;
        let view_pos = self.scroll.value();
        let tracked_pos = self.highlight_y.value();
        let tracked_end = tracked_pos + ROW_HEIGHT;
        let current_pos = row_y(self.current as usize);
        let current_end = current_pos + ROW_HEIGHT;
        let mut pos = view_pos;
        if tracked_pos < view_pos && current_pos < view_pos {
            pos = tracked_pos.min(current_pos);
        } else if tracked_end >= view_pos + size && current_end >= view_pos + size {
            if tracked_end <= current_end {
                pos = tracked_end - size;
                if ROW_HEIGHT > size {
                    pos = tracked_pos;
                }
            } else {
                pos = current_end - size;
                if ROW_HEIGHT > size {
                    pos = current_pos;
                }
            }
        }
        if pos != view_pos {
            self.scroll.jump(pos.clamp(0.0, self.max_scroll()));
        }
    }

    fn update_footer(&mut self, engine: &mut TextEngine) {
        let hidden = self.current_row().is_some_and(|row| row.hidden);
        let text = format!(
            "↑ ↓  select       ↵  launch       󰈉  ctrl+h  {}       󰈈  ctrl+shift+h  {}       esc  close",
            if hidden { "unhide" } else { "hide" },
            if self.show_hidden {
                "hide hidden"
            } else {
                "show hidden"
            }
        );
        self.footer
            .set_spec(engine, fallback_spec(&text, 10.0, MUTED));
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
        now: Millis,
    ) {
        let changed = !Arc::ptr_eq(&self.apps, &data.apps) && *self.apps != *data.apps
            || self.hidden != data.hidden;
        self.apps = data.apps.clone();
        if changed {
            self.hidden = data.hidden.clone();
            self.reset(tree, input, engine, now);
            self.update_footer(engine);
        }
    }

    pub fn set_icons(&mut self, tree: &mut Tree, icons: &HashMap<String, AppIcon>) {
        self.apply_icons(tree, icons);
        self.icons = icons.clone();
    }

    fn apply_icons(&mut self, tree: &mut Tree, icons: &HashMap<String, AppIcon>) {
        for row in &mut self.rows {
            if row.icon_set {
                continue;
            }
            let Some(icon) = icons.get(&row.app.id) else {
                continue;
            };
            row.icon_set = true;
            match icon {
                AppIcon::Letter => tree.set_visible(row.letter.node, true),
                AppIcon::Image(Some(pixmap)) => {
                    if let Some(node) = tree.get_mut(row.image) {
                        node.paint = Paint::Image(pixmap.clone());
                    }
                    tree.set_size(row.image, 28.0, 28.0);
                }
                AppIcon::Image(None) => {}
            }
        }
    }

    pub fn icon_requests(&self) -> Vec<(String, String)> {
        let top = self.scroll.value() - CACHE_BUFFER;
        let bottom = self.scroll.value() + self.view_height + CACHE_BUFFER;
        self.rows
            .iter()
            .enumerate()
            .filter(|(index, row)| {
                let y = row_y(*index);
                !row.icon_set && y + ROW_HEIGHT > top && y < bottom
            })
            .map(|(_, row)| (row.app.id.clone(), row.app.icon.clone()))
            .collect()
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        self.width = width;
        let eyebrow_h = self.eyebrow.implicit_height().ceil();
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
        let columns = linear(
            &[
                Cell::fill(self.title.implicit_width().ceil()),
                Cell::implicit(self.close.implicit_width()),
            ],
            5.0,
            width,
        );
        place_label(
            tree,
            &mut self.title,
            columns[0].pos,
            slots[1],
            columns[0].size,
        );
        place_chip(tree, &mut self.close, columns[1].pos, slots[1]);
        tree.set_pos(self.search, 0.0, slots[2].pos);
        tree.set_size(self.search, width, SEARCH_HEIGHT);
        let iw = self.search_icon.implicit_width();
        let ih = self.search_icon.implicit_height().ceil();
        self.search_icon
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
        let list = slots[3];
        self.view_height = list.size;
        tree.set_pos(self.view, 0.0, list.pos);
        tree.set_size(self.view, width, list.size);
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, (width - ew) / 2.0, (list.size - eh) / 2.0, ew, eh);
        let fw = self.footer.implicit_width();
        place_label(tree, &mut self.footer, 0.0, slots[4], fw);
        self.layout_rows(tree, engine);
    }

    fn layout_rows(&mut self, tree: &mut Tree, _engine: &mut TextEngine) {
        let width = self.width;
        let content_height = self.content_height();
        tree.set_size(self.content, width, content_height);
        tree.set_size(self.highlight, width, ROW_HEIGHT);
        for (index, row) in self.rows.iter_mut().enumerate() {
            tree.set_pos(row.root, 0.0, row_y(index));
            tree.set_size(row.root, width, ROW_HEIGHT);
            tree.set_size(row.hover_bg, width, ROW_HEIGHT);
            Self::layout_row(tree, row, width);
        }
    }

    fn layout_row(tree: &mut Tree, row: &mut Row, width: f32) {
        let margin = row.margin.value();
        let cw = (width - margin - 10.0).max(0.0);
        let ch = ROW_HEIGHT - 20.0;
        tree.set_pos(row.content, margin, 10.0);
        tree.set_size(row.content, cw, ch);
        tree.set_pos(row.icon_bg, 0.0, (ch - 28.0) / 2.0);
        tree.set_size(row.icon_bg, 28.0, 28.0);
        let lw = row.letter.implicit_width();
        let lh = row.letter.implicit_height().ceil();
        row.letter
            .place(tree, (28.0 - lw) / 2.0, (28.0 - lh) / 2.0, lw, lh);
        let aw = row.arrow.implicit_width();
        let ah = row.arrow.implicit_height().ceil();
        let ax = cw - aw;
        row.arrow.place(tree, ax, (ch - ah) / 2.0, aw, ah);
        let mut right = ax;
        tree.set_visible(row.eye.node, row.hidden);
        if row.hidden {
            let ew = row.eye.implicit_width();
            let eh = row.eye.implicit_height().ceil();
            let ex = ax - 13.0 - ew;
            row.eye.place(tree, ex, (ch - eh) / 2.0, ew, eh);
            right = ex;
        }
        let x = 28.0 + 13.0;
        let available = (right - 13.0 - 13.0 - x).max(0.0);
        let nh = row.name.implicit_height().ceil();
        let sh = row.sub.implicit_height().ceil();
        let column_h = nh + 2.0 + sh;
        let y = (ch - column_h) / 2.0;
        let nw = row.name.implicit_width().min(available);
        let sw = row.sub.implicit_width().min(390.0).min(available);
        row.name.place(tree, x, y, nw, nh);
        row.sub.place(tree, x, y + nh + 2.0, sw, sh);
        tree.set_opacity(row.content, if row.hidden { 0.55 } else { 1.0 });
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        self.close.sync(now, input);
        running |= self.close.tick(tree, now);
        running |= self.border.tick(now);
        set_border(tree, self.search, self.border.value(), 1.0);
        let icon_target = if self.field.text().is_empty() {
            MUTED
        } else {
            GREEN
        };
        if self.icon_color.target() != icon_target {
            self.icon_color.set_target(now, icon_target);
        }
        running |= self.icon_color.tick(now);
        self.search_icon.set_color(self.icon_color.value());
        self.search_icon.sync(tree);
        tree.set_visible(self.placeholder.node, self.field.text().is_empty());
        self.field.tick(tree, now);
        running |= self.highlight_y.tick(now);
        running |= self.scroll.tick(now);
        self.follow();
        tree.set_pos(self.highlight, 0.0, self.highlight_y.value());
        tree.set_opacity(self.highlight, if self.rows.is_empty() { 0.0 } else { 1.0 });
        tree.set_pos(self.content, 0.0, -self.scroll.value().round());
        let empty_target = if self.rows.is_empty() { 1.0 } else { 0.0 };
        if self.empty_opacity.target() != empty_target {
            self.empty_opacity.set_target(now, empty_target);
        }
        running |= self.empty_opacity.tick(now);
        tree.set_opacity(self.empty.node, self.empty_opacity.value());
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
            if row.arrow_opacity.target() != arrow {
                row.arrow_opacity.set_target(now, arrow);
            }
            running |= row.hover.tick(now);
            let margin_running = row.margin.tick(now);
            running |= margin_running;
            running |= row.arrow_opacity.tick(now);
            tree.set_opacity(row.hover_bg, row.hover.value());
            tree.set_opacity(row.arrow.node, row.arrow_opacity.value());
            if margin_running || row.margin.value() != margin {
                Self::layout_row(tree, row, width);
            } else {
                tree.set_pos(row.content, row.margin.value(), 10.0);
            }
            let (opacity, lift) = match row.intro {
                Some(start) => {
                    if now >= start + SLOW.max(NORMAL) {
                        row.intro = None;
                        (1.0, 0.0)
                    } else {
                        running = true;
                        intro_values(start, now)
                    }
                }
                None => (1.0, 0.0),
            };
            tree.set_opacity(row.root, opacity);
            tree.set_transforms(row.root, &[TransformOp::Translate { x: 0.0, y: lift }]);
        }
        running
    }

    pub fn next_deadline(&self, now: Millis) -> Option<Millis> {
        self.field.next_blink(now)
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
                InputEvent::Press(area, MouseButton::Left) if area == self.field.area => {
                    self.field.press(tree, engine, pointer.0, pointer.1, now);
                }
                InputEvent::Wheel(area, delta) if area == self.view_area => {
                    self.tracking = false;
                    let target = (self.scroll.target() - delta as f32 / 120.0 * SCROLL_STEP)
                        .clamp(0.0, self.max_scroll());
                    if target != self.scroll.target() {
                        let from = self.scroll.value();
                        self.scroll.start(now, from, target);
                    }
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.close.area {
                        actions.push(PanelAction::Close);
                    } else if let Some(row) = self.rows.iter().find(|row| row.area == area) {
                        actions.push(PanelAction::Launch(row.app.id.clone()));
                        actions.push(PanelAction::Close);
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
        self.field.drag(tree, engine, input, x, y);
    }

    fn toggle_hidden(&mut self, actions: &mut Vec<PanelAction>) {
        let Some(row) = self.current_row() else {
            return;
        };
        let id = row.app.id.clone();
        self.restore = Some((id.clone(), self.current));
        actions.push(PanelAction::ToggleHidden(id));
    }

    fn toggle_show_hidden(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        now: Millis,
    ) {
        let id = self
            .current_row()
            .map(|row| row.app.id.clone())
            .unwrap_or_default();
        let index = self.current.max(0);
        self.show_hidden = !self.show_hidden;
        self.restore = Some((id, index));
        self.reset(tree, input, engine, now);
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
        let mods = event.modifiers;
        let ctrl_only = mods.ctrl && !mods.alt && !mods.logo;
        match event.keysym {
            0x68 | 0x48 if ctrl_only => {
                if !event.repeat {
                    if mods.shift {
                        self.toggle_show_hidden(tree, input, engine, now);
                    } else {
                        self.toggle_hidden(&mut actions);
                    }
                }
            }
            keys::PAGE_DOWN => self.step(6, now),
            keys::PAGE_UP => self.step(-6, now),
            keys::TAB => self.step(1, now),
            keys::ISO_LEFT_TAB => self.step(-1, now),
            keys::DOWN => self.step(1, now),
            keys::UP => self.step(-1, now),
            keys::ESCAPE => actions.push(PanelAction::Close),
            keys::RETURN => {
                if let Some(row) = self.current_row() {
                    actions.push(PanelAction::Launch(row.app.id.clone()));
                    actions.push(PanelAction::Close);
                }
            }
            keys::KP_ENTER => {}
            _ => {
                let before = self.field.text().to_string();
                match self.field.key(tree, engine, event, now) {
                    KeyResult::Ignored => return (false, actions),
                    KeyResult::Escape => actions.push(PanelAction::Close),
                    KeyResult::Accepted | KeyResult::Handled => {}
                }
                if self.field.text() != before {
                    if !self.rows.is_empty() {
                        self.set_current(0, now);
                    }
                    actions.push(PanelAction::Search(self.field.text().to_string()));
                }
            }
        }
        self.update_footer(engine);
        (true, actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_icons_use_fallback_font() {
        let spec = fallback_spec("a 󰈉 b", 10.0, MUTED);
        let parts: Vec<_> = spec
            .spans
            .iter()
            .map(|span| (span.text.as_str(), span.family.as_str()))
            .collect();
        assert_eq!(
            parts,
            vec![("a ", FONT), ("󰈉", ICON_FALLBACK), (" b", FONT)]
        );
    }

    #[test]
    fn rows_are_spaced_like_list_view() {
        assert_eq!(row_y(0), 0.0);
        assert_eq!(row_y(3), 3.0 * 61.0);
    }
}
