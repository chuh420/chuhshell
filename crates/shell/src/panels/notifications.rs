use std::{collections::HashMap, sync::Arc};

use platform::MouseButton;
use services::notifications::Notification;
use ui::{
    Color, Node, NodeId, RectPaint, TextEngine, TextSpec, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, center, linear},
    tiny_skia::Pixmap,
    widgets::{Chip, ChipStyle, Label},
};

use super::{PanelAction, PanelData, TooltipSpot};
use crate::{
    card::Card,
    theme::{ELEVATED, FONT, ICONS, LINE, MUTED, RED, SLOW, TEXT, YELLOW},
};

const CHIP_STYLE: ChipStyle = ChipStyle {
    family: ICONS,
    hover: ELEVATED,
};
const SCROLL_STEP: f32 = 70.0;
const SCROLLBAR: Color = Color::rgb(0xb8b8b8);

pub struct NotificationsPanel {
    pub root: NodeId,
    title: Label,
    dnd: Chip,
    clear: Chip,
    count: Label,
    list: NodeId,
    list_area: AreaId,
    scroller: NodeId,
    cards: Vec<Card>,
    ids: Vec<u32>,
    empty: NodeId,
    empty_icon: Label,
    empty_label: Label,
    empty_opacity: Tween<f32>,
    footer: Label,
    bar: NodeId,
    bar_area: AreaId,
    bar_opacity: Tween<f32>,
    bar_hide_at: Option<Millis>,
    scroll: Tween<f32>,
    list_height: f32,
    content_height: f32,
    width: f32,
    data: PanelData,
}

impl NotificationsPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = Label::new(
            tree,
            engine,
            root,
            TextSpec::plain("notifications", FONT, 18.0, TEXT),
        );
        let mut dnd = Chip::new(
            tree,
            input,
            engine,
            root,
            &CHIP_STYLE,
            "",
            YELLOW,
            false,
            None,
        );
        dnd.shortcut = "do not disturb · right click the bell".into();
        let clear = Chip::new(
            tree,
            input,
            engine,
            root,
            &CHIP_STYLE,
            "clear all",
            RED,
            false,
            None,
        );
        let count = Label::new(tree, engine, root, TextSpec::plain("", FONT, 12.0, MUTED));
        let mut list_node = Node::new(0.0, 0.0, 0.0, 0.0);
        list_node.clip = true;
        let list = tree.add(root, list_node);
        let mut list_mouse = MouseArea::hover_handler(list);
        list_mouse.wheel = true;
        let list_area = input.add(list_mouse);
        let scroller = tree.add(list, Node::new(0.0, 0.0, 0.0, 0.0));
        let empty = tree.add(list, Node::new(0.0, 0.0, 0.0, 0.0));
        let empty_icon = Label::new(tree, engine, empty, TextSpec::plain("󰂚", ICONS, 46.0, LINE));
        let empty_label = Label::new(
            tree,
            engine,
            empty,
            TextSpec::plain("all caught up", FONT, 12.0, MUTED),
        );
        let bar = tree.add(
            list,
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
        let footer = Label::new(
            tree,
            engine,
            root,
            TextSpec::plain(
                "click to open  ·  swipe or right click to dismiss",
                FONT,
                10.0,
                MUTED,
            ),
        );
        let mut panel = Self {
            root,
            title,
            dnd,
            clear,
            count,
            list,
            list_area,
            scroller,
            cards: Vec::new(),
            ids: Vec::new(),
            empty,
            empty_icon,
            empty_label,
            empty_opacity: Tween::new(1.0, SLOW, Easing::Linear),
            footer,
            bar,
            bar_area,
            bar_opacity: Tween::new(0.0, 200.0, Easing::Linear),
            bar_hide_at: None,
            scroll: Tween::new(0.0, 110.0, Easing::OutCubic),
            list_height: 0.0,
            content_height: 0.0,
            width: 0.0,
            data: PanelData::default(),
        };
        panel.sync(tree, input, engine, data, now);
        let visible = panel.ids.is_empty();
        panel.empty_opacity.jump(if visible { 1.0 } else { 0.0 });
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.dnd.area,
            self.clear.area,
            self.list_area,
            self.bar_area,
        ];
        for card in &self.cards {
            areas.extend(card.areas());
        }
        areas
    }

    fn history(data: &PanelData) -> Vec<&Notification> {
        data.notifications
            .iter()
            .filter(|n| !n.transient)
            .rev()
            .collect()
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
        now: Millis,
    ) {
        self.data = data.clone();
        let history = Self::history(data);
        let ids: Vec<u32> = history.iter().map(|n| n.id).collect();
        let age = |id: u32| data.ages.get(&id).cloned().unwrap_or_default();
        if ids != self.ids {
            for card in self.cards.drain(..) {
                card.remove(tree, input);
            }
            for notification in &history {
                let mut card = Card::new(
                    tree,
                    input,
                    engine,
                    self.scroller,
                    notification,
                    false,
                    &age(notification.id),
                    now,
                );
                card.opacity = Tween::new(0.0, crate::theme::NORMAL, Easing::Linear);
                card.opacity.start(now, 0.0, 1.0);
                self.cards.push(card);
            }
            self.ids = ids;
            self.scroll.jump(0.0);
        } else {
            for (card, notification) in self.cards.iter_mut().zip(&history) {
                card.set_notification(tree, input, engine, notification, &age(notification.id));
            }
        }
        let count = self.ids.len();
        self.dnd.set_label(
            engine,
            if data.dnd {
                "󰂛  silent"
            } else {
                "󰂚  alerts"
            },
            12.0,
        );
        self.dnd.set_ink(now, if data.dnd { MUTED } else { YELLOW });
        self.dnd.selected = data.dnd;
        self.count.set_text(
            engine,
            &if count == 0 {
                "nothing new".to_string()
            } else {
                format!(
                    "{count}{}",
                    if count == 1 {
                        " notification"
                    } else {
                        " notifications"
                    }
                )
            },
        );
        self.empty_icon
            .set_text(engine, if data.dnd { "󰂛" } else { "󰂚" });
        self.empty_opacity
            .set_target(now, if count == 0 { 1.0 } else { 0.0 });
    }

    pub fn set_icons(&mut self, tree: &mut Tree, icons: &HashMap<u32, Option<Arc<Pixmap>>>) {
        for card in &mut self.cards {
            card.set_icon(tree, icons.get(&card.id).cloned().flatten());
        }
    }

    pub fn icon_requests(&self) -> Vec<(u32, u32)> {
        self.cards
            .iter()
            .map(|card| (card.id, card.icon_size()))
            .collect()
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        self.width = width;
        tree.set_size(self.root, width, height);
        let clear_visible = !self.ids.is_empty();
        let header_height = self
            .title
            .implicit_height()
            .ceil()
            .max(self.dnd.implicit_height());
        let rows = linear(
            &[
                Cell::implicit(header_height),
                Cell::implicit(self.count.implicit_height()),
                Cell::fill(0.0),
                Cell::implicit(self.footer.implicit_height()),
            ],
            12.0,
            height,
        );
        let mut cells = vec![
            Cell::fill(self.title.implicit_width().ceil()),
            Cell::implicit(self.dnd.implicit_width()),
        ];
        if clear_visible {
            cells.push(Cell::implicit(self.clear.implicit_width()));
        }
        let columns = linear(&cells, 5.0, width);
        let row = rows[0];
        let title_h = self.title.implicit_height().ceil();
        self.title.place(
            tree,
            columns[0].pos,
            row.pos + center(row.size, title_h),
            columns[0].size,
            title_h,
        );
        let chip_y = row.pos + center(row.size, self.dnd.implicit_height());
        let dnd_w = self.dnd.implicit_width().ceil();
        self.dnd.place(tree, columns[1].pos, chip_y, dnd_w);
        tree.set_visible(self.clear.root, clear_visible);
        if let Some(input_slot) = columns.get(2) {
            let clear_w = self.clear.implicit_width().ceil();
            self.clear.place(tree, input_slot.pos, chip_y, clear_w);
        }
        let count_w = self.count.implicit_width().ceil().min(width);
        let count_h = self.count.implicit_height().ceil();
        self.count.place(tree, 0.0, rows[1].pos, count_w, count_h);
        let list = rows[2];
        self.list_height = list.size;
        tree.set_pos(self.list, 0.0, list.pos);
        tree.set_size(self.list, width, list.size);
        let mut y = 0.0;
        for (index, card) in self.cards.iter_mut().enumerate() {
            if index > 0 {
                y += 8.0;
            }
            let h = card.layout(tree, engine, width);
            tree.set_pos(card.root, 0.0, y);
            y += h;
        }
        self.content_height = y;
        let footer_w = self.footer.implicit_width().ceil().min(width);
        let footer_h = self.footer.implicit_height().ceil();
        self.footer
            .place(tree, 0.0, rows[3].pos, footer_w, footer_h);
        let icon_w = self.empty_icon.implicit_width();
        let icon_h = self.empty_icon.implicit_height().ceil();
        let label_w = self.empty_label.implicit_width();
        let label_h = self.empty_label.implicit_height().ceil();
        let column_w = icon_w.max(label_w).ceil();
        let column_h = icon_h + 10.0 + label_h;
        tree.set_pos(
            self.empty,
            center(width, column_w),
            center(list.size, column_h),
        );
        tree.set_size(self.empty, column_w, column_h);
        self.empty_icon
            .place(tree, (column_w - icon_w) / 2.0, 0.0, icon_w, icon_h);
        self.empty_label.place(
            tree,
            (column_w - label_w) / 2.0,
            icon_h + 10.0,
            label_w,
            label_h,
        );
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height - self.list_height).max(0.0)
    }

    pub fn tick(
        &mut self,
        tree: &mut Tree,
        input: &Input,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut running = false;
        let mut actions = Vec::new();
        let mut card_actions = Vec::new();
        for card in &mut self.cards {
            running |= card.tick(tree, input, now, &mut card_actions);
        }
        actions.extend(card_actions.into_iter().map(PanelAction::Card));
        for chip in [&mut self.dnd, &mut self.clear] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.empty_opacity.tick(now);
        tree.set_opacity(self.empty, self.empty_opacity.value());
        let max = self.max_scroll();
        if self.scroll.target() > max {
            let value = self.scroll.value().min(max);
            self.scroll.jump(value);
        }
        running |= self.scroll.tick(now);
        tree.set_pos(self.scroller, 0.0, -self.scroll.value().round());
        let scrollable = self.content_height > self.list_height;
        let active =
            scrollable && (self.scroll.is_running() || input.contains_mouse(self.bar_area));
        if active {
            self.bar_opacity.jump(0.75);
            self.bar_hide_at = Some(now + 450.0);
        } else if let Some(at) = self.bar_hide_at
            && at <= now
        {
            self.bar_hide_at = None;
            self.bar_opacity.start(now, self.bar_opacity.value(), 0.0);
        }
        running |= self.bar_hide_at.is_some();
        running |= self.bar_opacity.tick(now);
        if scrollable {
            let available = self.list_height - 4.0;
            let size = (self.list_height / self.content_height).clamp(0.0, 1.0);
            let position = self.scroll.value() / self.content_height;
            let bar_h = (size * available).max(6.0);
            tree.set_pos(self.bar, self.width - 8.0, 2.0 + position * available);
            tree.set_size(self.bar, 6.0, bar_h);
        }
        tree.set_opacity(
            self.bar,
            if scrollable {
                self.bar_opacity.value()
            } else {
                0.0
            },
        );
        for label in [
            &self.title,
            &self.count,
            &self.footer,
            &self.empty_icon,
            &self.empty_label,
        ] {
            label.sync(tree);
        }
        (running, actions)
    }

    pub fn handle(
        &mut self,
        tree: &Tree,
        engine: &mut TextEngine,
        events: &[InputEvent],
        pointer: (f32, f32),
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Click(area, MouseButton::Left) if area == self.dnd.area => {
                    actions.push(PanelAction::ToggleDnd);
                }
                InputEvent::Click(area, MouseButton::Left) if area == self.clear.area => {
                    actions.push(PanelAction::ClearAll);
                }
                InputEvent::Wheel(area, delta) if area == self.list_area => {
                    let max = self.max_scroll();
                    let target =
                        (self.scroll.target() - delta as f32 / 120.0 * SCROLL_STEP).clamp(0.0, max);
                    if target != self.scroll.target() {
                        let from = self.scroll.value();
                        self.scroll.start(now, from, target);
                    }
                }
                _ => {}
            }
        }
        for card in &mut self.cards {
            actions.extend(
                card.handle(tree, engine, events, pointer, now)
                    .into_iter()
                    .map(PanelAction::Card),
            );
        }
        actions
    }

    pub fn motion(&mut self, input: &Input, x: f32) {
        for card in &mut self.cards {
            card.motion(input, x);
        }
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        if self.dnd.hovered(input) {
            return Some(TooltipSpot {
                key: self.dnd.area,
                text: self.dnd.shortcut.clone(),
                node: self.dnd.root,
                width: self.dnd.width(),
                height: self.dnd.height(),
            });
        }
        None
    }
}
