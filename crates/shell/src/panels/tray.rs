use platform::{MouseButton, keys};
use services::tray::MenuEntry;
use ui::{
    Color, Node, NodeId, RectPaint, TextEngine, TextSpec, Tree,
    anim::Millis,
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, center, linear},
    widgets::{Chip, ChipStyle, Label},
};

use super::{PanelAction, PanelData};
use crate::theme::{self};

fn chip_style() -> ChipStyle {
    ChipStyle {
        family: crate::theme::icons(),
        hover: theme::elevated(),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Open,
    Menu,
    Quit,
    Entry(MenuEntry),
}

#[derive(Clone, Debug, PartialEq)]
struct RowData {
    kind: Kind,
    label: String,
    right: String,
    separator: bool,
    enabled: bool,
}

struct Row {
    root: NodeId,
    area: AreaId,
    line: NodeId,
    label: Label,
    right: Label,
    data: RowData,
}

pub struct TrayPanel {
    pub root: NodeId,
    back: Chip,
    title: Label,
    list: NodeId,
    empty: Label,
    rows: Vec<Row>,
    model: Vec<RowData>,
    show_menu: bool,
    stack: Vec<MenuEntry>,
    current: usize,
    serial: u64,
    data: PanelData,
}

fn strip_ampersands(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl TrayPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
    ) -> (Self, Vec<PanelAction>) {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let back = Chip::new(
            tree,
            input,
            engine,
            root,
            &chip_style(),
            "←",
            theme::text(),
            false,
            None,
        );
        let title = Label::new(
            tree,
            engine,
            root,
            TextSpec::plain("", crate::theme::font(), 14.0, theme::text()),
        );
        let mut list_node = Node::new(0.0, 0.0, 0.0, 0.0);
        list_node.clip = true;
        let list = tree.add(root, list_node);
        let empty = Label::new(
            tree,
            engine,
            list,
            TextSpec::plain(
                "no menu entries",
                crate::theme::font(),
                12.0,
                theme::muted(),
            ),
        );
        let mut panel = Self {
            root,
            back,
            title,
            list,
            empty,
            rows: Vec::new(),
            model: Vec::new(),
            show_menu: false,
            stack: Vec::new(),
            current: 0,
            serial: u64::MAX,
            data: PanelData::default(),
        };
        let actions = panel.sync(tree, input, engine, data);
        (panel, actions)
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.back.area];
        areas.extend(self.rows.iter().map(|row| row.area));
        areas
    }

    fn set_show_menu(&mut self, show: bool, actions: &mut Vec<PanelAction>) {
        if self.show_menu == show {
            return;
        }
        self.show_menu = show;
        actions.push(PanelAction::TrayExpanded(show));
        if show {
            actions.push(PanelAction::TrayLoadMenu);
        } else {
            actions.push(PanelAction::TrayEvent(0, "closed"));
        }
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        self.data = data.clone();
        if data.tray_serial != self.serial {
            self.serial = data.tray_serial;
            self.stack.clear();
            self.show_menu = data.tray_only_menu;
            if self.show_menu {
                actions.push(PanelAction::TrayLoadMenu);
            }
        }
        let title = if data.tray_title.is_empty() {
            "application"
        } else {
            data.tray_title.as_str()
        };
        self.title.set_text(engine, title);
        self.rebuild(tree, input, engine);
        self.update_areas(input);
        actions
    }

    fn wanted_rows(&self) -> Vec<RowData> {
        if !self.show_menu {
            return [
                (Kind::Open, "open application"),
                (Kind::Menu, "application menu"),
                (Kind::Quit, "quit application"),
            ]
            .into_iter()
            .map(|(kind, label)| RowData {
                kind,
                label: label.into(),
                right: String::new(),
                separator: false,
                enabled: true,
            })
            .collect();
        }
        let entries = match self.stack.last() {
            Some(entry) => entry.children.clone(),
            None => self.data.tray_menu.clone().unwrap_or_default(),
        };
        entries
            .into_iter()
            .map(|entry| RowData {
                label: strip_ampersands(&entry.label),
                right: if entry.has_children {
                    "›".into()
                } else if entry.checked {
                    "✓".into()
                } else {
                    String::new()
                },
                separator: entry.separator,
                enabled: entry.enabled,
                kind: Kind::Entry(entry),
            })
            .collect()
    }

    fn rebuild(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine) {
        let wanted = self.wanted_rows();
        if wanted == self.model {
            return;
        }
        for row in self.rows.drain(..) {
            input.remove(row.area);
            tree.remove(row.root);
        }
        self.current = 0;
        for data in &wanted {
            let root = tree.add(
                self.list,
                Node::rect(
                    0.0,
                    0.0,
                    0.0,
                    if data.separator { 9.0 } else { 30.0 },
                    RectPaint {
                        radius: 7.0,
                        ..RectPaint::default()
                    },
                ),
            );
            let line = tree.add(
                root,
                Node::rect(
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    RectPaint {
                        color: theme::line(),
                        ..RectPaint::default()
                    },
                ),
            );
            let color = if data.kind == Kind::Quit {
                theme::red()
            } else {
                theme::text()
            };
            let label = Label::new(
                tree,
                engine,
                root,
                TextSpec::plain(&data.label, crate::theme::font(), 12.0, color),
            );
            let right = Label::new(
                tree,
                engine,
                root,
                TextSpec::plain(&data.right, crate::theme::font(), 12.0, theme::green()),
            );
            let area = input.add(MouseArea::new(root));
            self.rows.push(Row {
                root,
                area,
                line,
                label,
                right,
                data: data.clone(),
            });
        }
        self.model = wanted;
    }

    fn row_enabled(&self, row: &RowData) -> bool {
        !row.separator && (!self.show_menu || row.enabled)
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        tree.set_size(self.root, width, height);
        let back_visible = self.show_menu && (!self.data.tray_only_menu || !self.stack.is_empty());
        let title_h = self.title.implicit_height().ceil();
        let header_h = if back_visible {
            title_h.max(self.back.implicit_height())
        } else {
            title_h
        };
        let rows = linear(&[Cell::implicit(header_h), Cell::fill(0.0)], 8.0, height);
        let mut cells = Vec::new();
        if back_visible {
            cells.push(Cell::implicit(self.back.implicit_width()));
        }
        cells.push(Cell::fill(self.title.implicit_width().ceil()));
        let columns = linear(&cells, 5.0, width);
        tree.set_visible(self.back.root, back_visible);
        let header = rows[0];
        let mut index = 0;
        if back_visible {
            let y = header.pos + center(header.size, self.back.implicit_height());
            let w = self.back.implicit_width().ceil();
            self.back.place(tree, columns[0].pos, y, w);
            index = 1;
        }
        let slot = columns[index];
        self.title.place(
            tree,
            slot.pos,
            header.pos + center(header.size, title_h),
            slot.size,
            title_h,
        );
        let list = rows[1];
        tree.set_pos(self.list, 0.0, list.pos);
        tree.set_size(self.list, width, list.size);
        let mut y = 0.0;
        for (i, row) in self.rows.iter_mut().enumerate() {
            if i > 0 {
                y += 4.0;
            }
            let h = if row.data.separator { 9.0 } else { 30.0 };
            tree.set_pos(row.root, 0.0, y);
            tree.set_size(row.root, width, h);
            tree.set_visible(row.line, row.data.separator);
            tree.set_pos(row.line, 6.0, center(h, 1.0));
            tree.set_size(row.line, width - 12.0, 1.0);
            let inner = width - 16.0;
            let parts = linear(
                &[
                    Cell::fill(row.label.implicit_width().ceil()),
                    Cell::implicit(row.right.implicit_width()),
                ],
                5.0,
                inner,
            );
            let lh = row.label.implicit_height().ceil();
            let rh = row.right.implicit_height().ceil();
            tree.set_visible(row.label.node, !row.data.separator);
            tree.set_visible(row.right.node, !row.data.separator);
            row.label
                .place(tree, 8.0 + parts[0].pos, center(h, lh), parts[0].size, lh);
            row.right
                .place(tree, 8.0 + parts[1].pos, center(h, rh), parts[1].size, rh);
            y += h;
        }
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        tree.set_visible(self.empty.node, self.rows.is_empty());
        self.empty
            .place(tree, center(width, ew), center(list.size, eh), ew, eh);
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        self.back.sync(now, input);
        running |= self.back.tick(tree, now);
        let show_menu = self.show_menu;
        for (i, row) in self.rows.iter().enumerate() {
            let enabled = !row.data.separator && (!show_menu || row.data.enabled);
            let highlight = enabled && (i == self.current || input.contains_mouse(row.area));
            tree.set_fill(
                row.root,
                if highlight {
                    theme::elevated()
                } else {
                    Color::TRANSPARENT
                },
            );
            let dim = show_menu && !row.data.enabled && !row.data.separator;
            tree.set_opacity(row.label.node, if dim { 0.45 } else { 1.0 });
            tree.set_opacity(row.right.node, if dim { 0.45 } else { 1.0 });
            row.label.sync(tree);
            row.right.sync(tree);
        }
        for label in [&self.title, &self.empty] {
            label.sync(tree);
        }
        running
    }

    pub fn update_areas(&self, input: &mut Input) {
        for row in &self.rows {
            let enabled = self.row_enabled(&row.data);
            if let Some(area) = input.get_mut(row.area) {
                area.enabled = enabled;
            }
        }
    }

    fn activate(&mut self, index: usize, actions: &mut Vec<PanelAction>) {
        let Some(row) = self.model.get(index).cloned() else {
            return;
        };
        if row.separator || (self.show_menu && !row.enabled) {
            return;
        }
        match row.kind {
            Kind::Open => {
                actions.push(PanelAction::TrayActivate);
                actions.push(PanelAction::Close);
            }
            Kind::Menu => self.set_show_menu(true, actions),
            Kind::Quit => {
                actions.push(PanelAction::TrayQuit);
                actions.push(PanelAction::Close);
            }
            Kind::Entry(entry) => {
                if entry.has_children {
                    actions.push(PanelAction::TrayEvent(entry.id, "opened"));
                    self.stack.push(entry);
                } else {
                    actions.push(PanelAction::TrayEvent(entry.id, "clicked"));
                    actions.push(PanelAction::Close);
                }
            }
        }
    }

    pub fn closing(&mut self) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        if self.show_menu {
            actions.push(PanelAction::TrayEvent(0, "closed"));
        }
        while let Some(entry) = self.stack.pop() {
            actions.push(PanelAction::TrayEvent(entry.id, "closed"));
        }
        actions
    }

    fn back(&mut self, actions: &mut Vec<PanelAction>) {
        if let Some(entry) = self.stack.pop() {
            actions.push(PanelAction::TrayEvent(entry.id, "closed"));
        } else {
            self.set_show_menu(false, actions);
        }
    }

    pub fn handle(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        events: &[InputEvent],
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            if let InputEvent::Click(area, MouseButton::Left) = *event {
                if area == self.back.area {
                    self.back(&mut actions);
                } else if let Some(index) = self.rows.iter().position(|row| row.area == area) {
                    self.current = index;
                    self.activate(index, &mut actions);
                }
            }
        }
        self.rebuild(tree, input, engine);
        self.update_areas(input);
        actions
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        keysym: u32,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        match keysym {
            keys::UP => self.current = self.current.saturating_sub(1),
            keys::DOWN => {
                if self.current + 1 < self.model.len() {
                    self.current += 1;
                }
            }
            keys::RETURN | keys::KP_ENTER => self.activate(self.current, &mut actions),
            keys::RIGHT => {
                if let Some(RowData {
                    kind: Kind::Entry(entry),
                    ..
                }) = self.model.get(self.current)
                    && entry.has_children
                {
                    self.activate(self.current, &mut actions);
                }
            }
            keys::LEFT => self.back(&mut actions),
            _ => {}
        }
        self.rebuild(tree, input, engine);
        self.update_areas(input);
        actions
    }
}
