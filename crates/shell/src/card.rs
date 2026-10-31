use std::sync::Arc;

use platform::{CursorShape, MouseButton};
use services::notifications::{Notification, Urgency};
use ui::{
    Color, Node, NodeId, Paint, RectPaint, TextEngine, TextSpec, Tree, Weight,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Buttons, Input, InputEvent, MouseArea},
    layout::{Cell, center, linear},
    rich::styled_spans,
    tiny_skia::Pixmap,
    widgets::{Chip, ChipStyle, Label},
};

use crate::theme::{
    BG, ELEVATED, FAST, FONT, GREEN, ICONS, LINE, MUTED, NORMAL, RED, SLOW, SURFACE, TEXT,
    UI_SCALE, YELLOW,
};

const CHIP_STYLE: ChipStyle = ChipStyle {
    family: ICONS,
    hover: ELEVATED,
};

#[derive(Clone, Debug, PartialEq)]
pub enum CardAction {
    Invoke(u32, String),
    Dismiss(u32),
    Finished(u32),
    OpenLink(String),
}

pub fn timeout(notification: &Notification) -> f64 {
    if notification.urgency == Urgency::Critical {
        return 0.0;
    }
    if notification.expire_timeout > 0 {
        return (notification.expire_timeout as f64).max(1500.0);
    }
    if notification.urgency == Urgency::Low {
        4000.0
    } else {
        6000.0
    }
}

pub fn age(time: Option<i64>, now_minute: i64) -> String {
    let Some(time) = time else {
        return String::new();
    };
    let minutes = ((now_minute - time) as f64 / 60000.0).floor() as i64;
    if minutes < 1 {
        "now".into()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h", minutes / 60)
    }
}

fn strip_images(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<img") {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

struct Countdown {
    duration: f64,
    elapsed: f64,
    last: Millis,
}

struct Drag {
    press_x: f32,
    start: f32,
    active: bool,
}

pub struct Card {
    pub id: u32,
    pub root: NodeId,
    body: NodeId,
    icon_box: NodeId,
    icon_image: NodeId,
    icon_glyph: Label,
    app: Label,
    age: Label,
    close: Label,
    summary: Label,
    text: Label,
    chips: Vec<(Chip, String)>,
    bar: NodeId,
    body_area: AreaId,
    close_area: AreaId,
    popup: bool,
    notification: Notification,
    width: f32,
    height: f32,
    progress: f64,
    countdown: Option<Countdown>,
    slide: f32,
    leave: Option<(Tween<f32>, bool)>,
    settle: Option<Tween<f32>>,
    leaving: bool,
    drag: Option<Drag>,
    suppress_click: bool,
    color: Tween<Color>,
    close_color: Tween<Color>,
    icon: Option<Arc<Pixmap>>,
    pub opacity: Tween<f32>,
}

fn rect(color: Color, radius: f32) -> RectPaint {
    RectPaint {
        color,
        radius,
        ..RectPaint::default()
    }
}

impl Card {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        notification: &Notification,
        popup: bool,
        age_text: &str,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let body = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                0.0,
                0.0,
                RectPaint {
                    color: BG,
                    radius: 14.0,
                    border_color: LINE,
                    border_width: 1.0,
                },
            ),
        );
        let mut icon_node = Node::rect(13.0, 13.0, 38.0, 38.0, rect(ELEVATED, 10.0));
        icon_node.clip = true;
        let icon_box = tree.add(body, icon_node);
        let icon_image = tree.add(icon_box, Node::new(0.0, 0.0, 0.0, 0.0));
        let icon_glyph = Label::new(
            tree,
            engine,
            icon_box,
            TextSpec::plain("󰂚", ICONS, 18.0, YELLOW),
        );
        let mut app_spec = TextSpec::plain("", FONT, 9.0, GREEN);
        if let Some(span) = app_spec.spans.first_mut() {
            span.letter_spacing = 0.6;
        }
        let app = Label::new(tree, engine, body, app_spec);
        let age = Label::new(tree, engine, body, TextSpec::plain("", FONT, 9.0, MUTED));
        let close = Label::new(tree, engine, body, TextSpec::plain("×", FONT, 13.0, MUTED));
        let mut summary_spec = TextSpec::plain("", FONT, 13.0, TEXT);
        if let Some(span) = summary_spec.spans.first_mut() {
            span.weight = Weight::Medium;
        }
        let summary = Label::new(tree, engine, body, summary_spec);
        let text = Label::new(tree, engine, body, TextSpec::plain("", FONT, 11.0, MUTED));
        let bar = tree.add(body, Node::rect(14.0, 0.0, 0.0, 2.0, rect(GREEN, 1.0)));
        let mut body_mouse = MouseArea::new(body);
        body_mouse.buttons = Buttons {
            left: true,
            right: true,
            middle: true,
        };
        let body_area = input.add(body_mouse);
        let mut close_mouse = MouseArea::new(close.node);
        close_mouse.cursor = CursorShape::Default;
        let close_area = input.add(close_mouse);
        let mut card = Self {
            id: notification.id,
            root,
            body,
            icon_box,
            icon_image,
            icon_glyph,
            app,
            age,
            close,
            summary,
            text,
            chips: Vec::new(),
            bar,
            body_area,
            close_area,
            popup,
            notification: notification.clone(),
            width: 0.0,
            height: 0.0,
            progress: 1.0,
            countdown: None,
            slide: 0.0,
            leave: None,
            settle: None,
            leaving: false,
            drag: None,
            suppress_click: false,
            color: Tween::new(BG, FAST, Easing::Linear),
            close_color: Tween::new(MUTED, 0.0, Easing::Linear),
            icon: None,
            opacity: Tween::new(1.0, NORMAL, Easing::Linear),
        };
        card.set_notification(tree, input, engine, notification, age_text);
        card.restart_timer(now);
        card
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.body_area, self.close_area];
        areas.extend(self.chips.iter().map(|(chip, _)| chip.area));
        areas
    }

    pub fn remove(self, tree: &mut Tree, input: &mut Input) {
        for area in self.areas() {
            input.remove(area);
        }
        tree.remove(self.root);
    }

    pub fn critical(&self) -> bool {
        self.notification.urgency == Urgency::Critical
    }

    pub fn set_notification(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        notification: &Notification,
        age_text: &str,
    ) {
        self.notification = notification.clone();
        let app = if notification.app_name.is_empty() {
            "notification".to_string()
        } else {
            notification.app_name.to_lowercase()
        };
        self.app.set_text(engine, &app);
        self.age.set_text(engine, age_text);
        self.summary.set_text(engine, &notification.summary);
        let body = strip_images(&notification.body);
        let spec = TextSpec {
            spans: styled_spans(&body, FONT, 11.0, GREEN),
            color: MUTED,
            elide_width: None,
            wrap_width: None,
            max_lines: None,
        };
        self.text.set_spec(engine, spec);
        let critical = self.critical();
        self.icon_glyph
            .set_text(engine, if critical { "󰀦" } else { "󰂚" });
        self.icon_glyph
            .set_color(if critical { RED } else { YELLOW });
        let wanted: Vec<(String, String)> = notification
            .actions
            .iter()
            .filter(|action| action.identifier != "default" && !action.text.is_empty())
            .map(|action| (action.identifier.clone(), action.text.clone()))
            .collect();
        let same = self.chips.len() == wanted.len()
            && self
                .chips
                .iter()
                .zip(&wanted)
                .all(|((_, key), (identifier, _))| key == identifier);
        if !same {
            for (chip, _) in self.chips.drain(..) {
                input.remove(chip.area);
                tree.remove(chip.root);
            }
            for (identifier, label) in wanted {
                let mut chip = Chip::new(
                    tree,
                    input,
                    engine,
                    self.body,
                    &CHIP_STYLE,
                    &label,
                    TEXT,
                    true,
                    None,
                );
                chip.color_override = Some(ELEVATED);
                chip.jump_color(input);
                self.chips.push((chip, identifier));
            }
        }
        let border = if critical { RED } else { LINE };
        if let Some(node) = tree.get_mut(self.body)
            && let Paint::Rect(rect) = &mut node.paint
        {
            rect.border_color = border;
        }
        tree.set_fill(self.bar, if critical { RED } else { GREEN });
    }

    pub fn set_icon(&mut self, tree: &mut Tree, icon: Option<Arc<Pixmap>>) {
        self.icon = icon;
        let has_image = matches!(
            self.notification.image,
            Some(services::notifications::Image::Pixels { .. })
                | Some(services::notifications::Image::Path(_))
        );
        let margin = if has_image { 0.0 } else { 6.0 };
        match &self.icon {
            Some(pixmap) => {
                if let Some(node) = tree.get_mut(self.icon_image) {
                    node.paint = Paint::Image(pixmap.clone());
                    node.x = margin;
                    node.y = margin;
                    node.width = 38.0 - margin * 2.0;
                    node.height = 38.0 - margin * 2.0;
                }
                tree.set_visible(self.icon_glyph.node, false);
            }
            None => {
                if let Some(node) = tree.get_mut(self.icon_image) {
                    node.paint = Paint::None;
                }
                tree.set_visible(self.icon_glyph.node, true);
            }
        }
    }

    pub fn icon_size(&self) -> u32 {
        let has_image = self.notification.image.is_some();
        let logical = if has_image { 38.0 } else { 26.0 };
        (logical * UI_SCALE).ceil() as u32
    }

    pub fn restart_timer(&mut self, now: Millis) {
        let timeout = timeout(&self.notification);
        if !self.popup || timeout <= 0.0 {
            return;
        }
        self.progress = 1.0;
        self.countdown = Some(Countdown {
            duration: timeout,
            elapsed: 0.0,
            last: now,
        });
    }

    fn paused(&self, input: &Input) -> bool {
        input.contains_mouse(self.body_area)
            || input.contains_mouse(self.close_area)
            || self.chips.iter().any(|(chip, _)| chip.hovered(input))
            || self.drag.as_ref().is_some_and(|drag| drag.active)
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32) -> f32 {
        self.width = width;
        let column_x = 13.0 + 38.0 + 12.0;
        let column_width = (width - 26.0 - 38.0 - 12.0).max(0.0);
        let row_height = self
            .app
            .implicit_height()
            .ceil()
            .max(self.age.implicit_height().ceil())
            .max(self.close.implicit_height().ceil());
        let mut cells = vec![Cell::implicit(row_height)];
        let has_summary = !self.notification.summary.is_empty();
        let has_text = !self.text.text().is_empty();
        let summary_height = if has_summary {
            self.summary.wrap(engine, column_width, 2)
        } else {
            0.0
        };
        let text_height = if has_text {
            self.text
                .wrap(engine, column_width, if self.popup { 3 } else { 6 })
        } else {
            0.0
        };
        if has_summary {
            cells.push(Cell::implicit(summary_height));
        }
        if has_text {
            cells.push(Cell::implicit(text_height));
        }
        let mut chip_rows: Vec<(f32, f32)> = Vec::new();
        if !self.chips.is_empty() {
            let mut x = 0.0;
            let mut y = 0.0;
            let mut line_height: f32 = 0.0;
            for (chip, _) in &self.chips {
                let w = chip.implicit_width();
                let h = chip.implicit_height();
                if x > 0.0 && x + w > column_width {
                    x = 0.0;
                    y += line_height + 6.0;
                    line_height = 0.0;
                }
                chip_rows.push((x, y));
                x += w + 6.0;
                line_height = line_height.max(h);
            }
            cells.push(Cell::implicit(y + line_height).margins(4.0, 0.0));
        }
        let column_height: f32 = cells
            .iter()
            .map(|cell| cell.size + cell.margin_start)
            .sum::<f32>()
            + 3.0 * (cells.len() - 1) as f32;
        let content_height = column_height.max(38.0);
        let column_y = 13.0 + center(content_height, column_height);
        let slots = linear(&cells, 3.0, column_height);
        let mut index = 0;
        let row = slots[index];
        index += 1;
        let columns = linear(
            &[
                Cell::fill(self.app.implicit_width().ceil()),
                Cell::implicit(self.age.implicit_width()),
                Cell::implicit(self.close.implicit_width()),
            ],
            5.0,
            column_width,
        );
        let place = |tree: &mut Tree, label: &mut Label, x: f32, width: f32| {
            let h = label.implicit_height().ceil();
            let y = column_y + row.pos + center(row.size, h);
            label.place(tree, column_x + x, y, width, h);
        };
        place(tree, &mut self.app, columns[0].pos, columns[0].size);
        place(tree, &mut self.age, columns[1].pos, columns[1].size);
        place(tree, &mut self.close, columns[2].pos, columns[2].size);
        tree.set_visible(self.summary.node, has_summary);
        if has_summary {
            let slot = slots[index];
            index += 1;
            self.summary.place(
                tree,
                column_x,
                column_y + slot.pos,
                column_width,
                summary_height,
            );
        }
        tree.set_visible(self.text.node, has_text);
        if has_text {
            let slot = slots[index];
            index += 1;
            self.text.place(
                tree,
                column_x,
                column_y + slot.pos,
                column_width,
                text_height,
            );
        }
        if !self.chips.is_empty() {
            let slot = slots[index];
            for ((chip, _), (x, y)) in self.chips.iter_mut().zip(&chip_rows) {
                let w = chip.implicit_width();
                chip.place(tree, column_x + x, column_y + slot.pos + y, w);
            }
        }
        let glyph_w = self.icon_glyph.implicit_width();
        let glyph_h = self.icon_glyph.implicit_height();
        self.icon_glyph.place(
            tree,
            center(38.0, glyph_w),
            center(38.0, glyph_h),
            glyph_w,
            glyph_h,
        );
        self.height = content_height + 26.0;
        tree.set_size(self.root, width, self.height);
        tree.set_size(self.body, width, self.height);
        tree.set_pos(self.icon_box, 13.0, 13.0);
        self.height
    }

    fn close_card(&mut self, now: Millis, dismiss: bool) {
        if self.leaving {
            return;
        }
        self.leaving = true;
        let to = if self.slide < 0.0 {
            -self.width - 40.0
        } else {
            self.width + 40.0
        };
        let mut tween = Tween::new(self.slide, NORMAL, Easing::InCubic);
        tween.start(now, self.slide, to);
        self.leave = Some((tween, dismiss));
        self.settle = None;
    }

    fn activate(&mut self, now: Millis, actions: &mut Vec<CardAction>) {
        let has_default = self
            .notification
            .actions
            .iter()
            .any(|action| action.identifier == "default");
        if has_default {
            actions.push(CardAction::Invoke(self.id, "default".into()));
            self.close_card(now, !self.notification.resident);
        } else if self.popup {
            self.close_card(now, false);
        }
    }

    fn local(&self, tree: &Tree, node: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        let inverse = tree.device_transform(node).invert()?;
        let mut point = ui::tiny_skia::Point::from_xy(x, y);
        inverse.map_point(&mut point);
        Some((point.x, point.y))
    }

    pub fn handle(
        &mut self,
        tree: &Tree,
        engine: &mut TextEngine,
        events: &[InputEvent],
        pointer: (f32, f32),
        now: Millis,
    ) -> Vec<CardAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Press(area, MouseButton::Left) if area == self.body_area => {
                    self.drag = Some(Drag {
                        press_x: pointer.0,
                        start: self.slide,
                        active: false,
                    });
                }
                InputEvent::Release(area, MouseButton::Left) if area == self.body_area => {
                    if let Some(drag) = self.drag.take()
                        && drag.active
                    {
                        self.suppress_click = true;
                        if self.slide.abs() > self.width * 0.3 {
                            self.close_card(now, true);
                        } else {
                            let mut tween = Tween::new(self.slide, SLOW, Easing::OUT_BACK);
                            tween.start(now, self.slide, 0.0);
                            self.settle = Some(tween);
                        }
                    }
                }
                InputEvent::Click(area, button) if area == self.body_area => {
                    if std::mem::take(&mut self.suppress_click) {
                        continue;
                    }
                    if button == MouseButton::Left {
                        if let Some((x, y)) = self.local(tree, self.text.node, pointer.0, pointer.1)
                            && let Some(link) = engine.link_at(self.text.spec(), x, y)
                        {
                            actions.push(CardAction::OpenLink(link));
                        }
                        self.activate(now, &mut actions);
                    } else {
                        self.close_card(now, button == MouseButton::Right || !self.popup);
                    }
                }
                InputEvent::Click(area, MouseButton::Left) if area == self.close_area => {
                    self.close_card(now, true);
                    self.activate(now, &mut actions);
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if let Some((_, key)) = self.chips.iter().find(|(chip, _)| chip.area == area) {
                        actions.push(CardAction::Invoke(self.id, key.clone()));
                        self.close_card(now, !self.notification.resident);
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn motion(&mut self, input: &Input, x: f32) {
        if !input.pressed(self.body_area) || self.leaving {
            return;
        }
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let delta = x - drag.press_x;
        if !drag.active && delta.abs() > 10.0 {
            drag.active = true;
            drag.start = self.slide;
            self.settle = None;
        }
        if drag.active {
            self.slide = drag.start + (x - drag.press_x);
        }
    }

    pub fn tick(
        &mut self,
        tree: &mut Tree,
        input: &Input,
        now: Millis,
        actions: &mut Vec<CardAction>,
    ) -> bool {
        let mut running = false;
        let paused = self.paused(input);
        if let Some(countdown) = self.countdown.as_mut() {
            if !paused && !self.leaving {
                countdown.elapsed += now - countdown.last;
            }
            countdown.last = now;
            self.progress = (1.0 - countdown.elapsed / countdown.duration).max(0.0);
            running = true;
            if countdown.elapsed >= countdown.duration {
                self.countdown = None;
                self.close_card(now, false);
            }
        }
        if let Some(settle) = self.settle.as_mut() {
            running |= settle.tick(now);
            self.slide = settle.value();
            if !settle.is_running() {
                self.settle = None;
            }
        }
        if let Some((leave, dismiss)) = self.leave.as_mut() {
            running |= leave.tick(now);
            self.slide = leave.value();
            if !leave.is_running() {
                let dismiss = *dismiss;
                self.leave = None;
                if dismiss {
                    actions.push(CardAction::Dismiss(self.id));
                }
                actions.push(CardAction::Finished(self.id));
            }
        }
        let hovered = input.contains_mouse(self.body_area)
            || input.contains_mouse(self.close_area)
            || self.chips.iter().any(|(chip, _)| chip.hovered(input));
        self.color
            .set_target(now, if hovered { SURFACE } else { BG });
        running |= self.color.tick(now);
        self.close_color.set_target(
            now,
            if input.contains_mouse(self.close_area) {
                TEXT
            } else {
                MUTED
            },
        );
        self.close_color.tick(now);
        self.close.set_color(self.close_color.value());
        running |= self.opacity.tick(now);
        tree.set_opacity(self.root, self.opacity.value());
        tree.set_pos(self.body, self.slide, 0.0);
        tree.set_fill(self.body, self.color.value());
        let fade = (1.0 - self.slide.abs() / (self.width * 0.9)).max(0.0);
        tree.set_opacity(self.body, fade);
        let show_bar = self.popup && timeout(&self.notification) > 0.0;
        tree.set_visible(self.bar, show_bar);
        tree.set_pos(self.bar, 14.0, self.height - 1.0 - 2.0);
        tree.set_size(self.bar, (self.width - 28.0) * self.progress as f32, 2.0);
        tree.set_opacity(self.bar, 0.7);
        for (chip, _) in &mut self.chips {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        for label in [
            &self.icon_glyph,
            &self.app,
            &self.age,
            &self.close,
            &self.summary,
            &self.text,
        ] {
            label.sync(tree);
        }
        running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_follow_qml() {
        let mut n = Notification {
            id: 1,
            app_name: String::new(),
            app_icon: String::new(),
            summary: String::new(),
            body: String::new(),
            actions: Vec::new(),
            urgency: Urgency::Normal,
            transient: false,
            resident: false,
            expire_timeout: -1,
            desktop_entry: String::new(),
            image: None,
            tag: String::new(),
        };
        assert_eq!(timeout(&n), 6000.0);
        n.expire_timeout = 500;
        assert_eq!(timeout(&n), 1500.0);
        n.urgency = Urgency::Critical;
        assert_eq!(timeout(&n), 0.0);
    }

    #[test]
    fn ages_like_qml() {
        assert_eq!(age(Some(0), 30_000), "now");
        assert_eq!(age(Some(0), 5 * 60_000), "5m");
        assert_eq!(age(Some(0), 125 * 60_000), "2h");
        assert_eq!(strip_images("a<img src=x/>b"), "ab");
    }
}
