use std::{collections::HashMap, sync::Arc};

use platform::{KeyEvent, MouseButton, keys};
use services::apps::AppEntry;
use ui::{
    Color, Node, NodeId, Paint, TextEngine, TextSpec, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent},
    layout::{Cell, linear},
    textfield::KeyResult,
    widgets::{Chip, Label},
};

use super::{
    AppIcon, PanelAction, PanelData,
    kit::{chip, icon_label, label, place_chip, place_label, rect},
    picker::{PickList, PickStyle, SEARCH_HEIGHT, SearchBox, eyebrow},
};
use crate::theme::{self, FONT, ICON_FALLBACK, NORMAL};

const ROW_HEIGHT: f32 = 56.0;
const CACHE_BUFFER: f32 = 320.0;

struct Row {
    app: AppEntry,
    icon_bg: NodeId,
    image: NodeId,
    letter: Label,
    name: Label,
    sub: Label,
    eye: Label,
    arrow: Label,
    hidden: bool,
    icon_set: bool,
}

pub struct LauncherPanel {
    pub root: NodeId,
    eyebrow: Label,
    title: Label,
    close: Chip,
    search: SearchBox,
    list: PickList,
    rows: Vec<Row>,
    empty: Label,
    empty_opacity: Tween<f32>,
    footer: Label,
    apps: Arc<Vec<AppEntry>>,
    hidden: Vec<String>,
    show_hidden: bool,
    restore: Option<(String, i32)>,
    icons: HashMap<String, AppIcon>,
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
        let eyebrow = eyebrow(tree, engine, root, "workspace / launch");
        let title = label(tree, engine, root, "applications", 26.0, theme::text());
        let close = chip(tree, input, engine, root, "×", theme::text(), false, None);
        let search = SearchBox::new(
            tree,
            input,
            engine,
            root,
            "󰍉",
            "find an application…",
            true,
            now,
        );
        let style = PickStyle {
            resize: 0.0,
            intro_easing: Easing::OutCubic,
            scrollbar: false,
            hit_right_margin: 0.0,
        };
        let list = PickList::new(tree, input, root, style, now);
        let empty = label(
            tree,
            engine,
            list.view,
            "No results found",
            12.0,
            theme::muted(),
        );
        let footer = label(tree, engine, root, "", 10.0, theme::muted());
        let mut panel = Self {
            root,
            eyebrow,
            title,
            close,
            search,
            list,
            rows: Vec::new(),
            empty,
            empty_opacity: Tween::new(0.0, NORMAL, Easing::Linear),
            footer,
            apps: data.apps.clone(),
            hidden: data.hidden.clone(),
            show_hidden: false,
            restore: None,
            icons: HashMap::new(),
        };
        panel.reset(tree, input, engine, now);
        panel
            .empty_opacity
            .jump(if panel.rows.is_empty() { 1.0 } else { 0.0 });
        panel.update_footer(engine);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.close.area, self.search.field.area];
        areas.extend(self.list.areas());
        areas
    }

    fn model(&self) -> Vec<AppEntry> {
        self.apps
            .iter()
            .filter(|app| self.show_hidden || !self.hidden.contains(&app.id))
            .cloned()
            .collect()
    }

    fn make_row(
        &self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        content: NodeId,
        app: AppEntry,
    ) -> Row {
        let icon_bg = rect(tree, content, theme::elevated(), 7.0);
        let image = tree.add(icon_bg, Node::new(0.0, 0.0, 28.0, 28.0));
        let initial: String = app.name.chars().take(1).collect();
        let letter = label(tree, engine, icon_bg, &initial, 16.0, theme::green());
        tree.set_visible(letter.node, false);
        let name = label(tree, engine, content, &app.name, 14.0, theme::text());
        let subtitle = if !app.generic.is_empty() {
            app.generic.as_str()
        } else if !app.comment.is_empty() {
            app.comment.as_str()
        } else {
            "Application"
        };
        let sub = label(tree, engine, content, subtitle, 10.0, theme::muted());
        let eye = icon_label(tree, engine, content, "󰈈", 18.0, theme::muted());
        let arrow = label(tree, engine, content, "↵", 12.0, theme::green());
        let hidden = self.hidden.contains(&app.id);
        tree.set_opacity(content, if hidden { 0.55 } else { 1.0 });
        Row {
            app,
            icon_bg,
            image,
            letter,
            name,
            sub,
            eye,
            arrow,
            hidden,
            icon_set: false,
        }
    }

    fn reset(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, now: Millis) {
        self.list.clear(tree, input);
        self.rows.clear();
        for app in self.model() {
            let index = self.list.push(tree, input, ROW_HEIGHT, now);
            let content = self.list.rows[index].content;
            let row = self.make_row(tree, engine, content, app);
            self.rows.push(row);
        }
        self.list.reset_selection();
        let icons = std::mem::take(&mut self.icons);
        self.apply_icons(tree, &icons);
        self.icons = icons;
        if let Some((id, index)) = self.restore.take() {
            let target = match self.rows.iter().position(|row| row.app.id == id) {
                Some(found) => found as i32,
                None => index.min(self.rows.len() as i32 - 1),
            };
            self.list.set_current(target, now);
        }
        self.layout_rows(tree);
    }

    fn current_row(&self) -> Option<&Row> {
        self.list
            .current_index()
            .and_then(|index| self.rows.get(index))
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
            .set_spec(engine, fallback_spec(&text, 10.0, theme::muted()));
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
        self.list
            .visible(CACHE_BUFFER)
            .into_iter()
            .filter_map(|index| self.rows.get(index))
            .filter(|row| !row.icon_set)
            .map(|row| (row.app.id.clone(), row.app.icon.clone()))
            .collect()
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
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
        self.search.layout(tree, engine, slots[2].pos, width);
        let list = slots[3];
        self.list.layout(tree, list.pos, width, list.size);
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
            Self::layout_row(tree, row, cw);
            tree.set_opacity(row.arrow.node, self.list.arrow_opacity(index));
        }
    }

    fn layout_row(tree: &mut Tree, row: &mut Row, cw: f32) {
        let ch = ROW_HEIGHT - 20.0;
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
        let y = (ch - (nh + 2.0 + sh)) / 2.0;
        let nw = row.name.implicit_width().min(available);
        let sw = row.sub.implicit_width().min(390.0).min(available);
        row.name.place(tree, x, y, nw, nh);
        row.sub.place(tree, x, y + nh + 2.0, sw, sh);
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        self.close.sync(now, input);
        running |= self.close.tick(tree, now);
        running |= self.search.tick(tree, now);
        running |= self.list.tick(tree, input, now);
        self.layout_rows(tree);
        let empty_target = if self.rows.is_empty() { 1.0 } else { 0.0 };
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
                    } else if let Some(row) = self
                        .list
                        .row_at(area)
                        .and_then(|index| self.rows.get(index))
                    {
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
        self.search.field.drag(tree, engine, input, x, y);
    }

    fn toggle_hidden(&mut self, actions: &mut Vec<PanelAction>) {
        let Some(row) = self.current_row() else {
            return;
        };
        let id = row.app.id.clone();
        self.restore = Some((id.clone(), self.list.current()));
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
        let index = self.list.current().max(0);
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
            keys::PAGE_DOWN => self.list.step(6, now),
            keys::PAGE_UP => self.list.step(-6, now),
            keys::TAB | keys::DOWN => self.list.step(1, now),
            keys::ISO_LEFT_TAB | keys::UP => self.list.step(-1, now),
            keys::ESCAPE => actions.push(PanelAction::Close),
            keys::RETURN => {
                if let Some(row) = self.current_row() {
                    actions.push(PanelAction::Launch(row.app.id.clone()));
                    actions.push(PanelAction::Close);
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
                    if !self.rows.is_empty() {
                        self.list.set_current(0, now);
                    }
                    actions.push(PanelAction::Search(self.search.text().to_string()));
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
        let spec = fallback_spec("a 󰈉 b", 10.0, theme::muted());
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
}
