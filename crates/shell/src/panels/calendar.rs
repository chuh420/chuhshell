use std::collections::HashMap;

use jiff::{ToSpan, civil::Date};
use platform::{KeyEvent, MouseButton, keys};
use ui::{
    Color, Node, NodeId, TextEngine, TextSpec, Tree, Weight,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, linear},
    widgets::{Chip, Label, SwapText},
};

use super::{
    PanelAction, PanelData,
    kit::{chip, fit, label, place_chip, place_label, rect},
};
use crate::theme::{BG, ELEVATED, FAST, FONT, GREEN, ICONS, LINE, MUTED, TEXT, YELLOW};

const BASE: i32 = 600;
const COUNT: i32 = 1201;
const MOVE: Millis = 320.0;
const MIN_FLICK: f32 = 75.0;
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

struct Cell7 {
    date: Date,
    bg: NodeId,
    label: Label,
    color: Tween<Color>,
}

#[derive(Clone, Copy)]
struct Drag {
    start: f32,
    x: f32,
    last: (f32, std::time::Instant),
    velocity: f32,
}

pub fn drag_step(moved: f32, velocity: f32, width: f32) -> i32 {
    if velocity.abs() > MIN_FLICK {
        -(velocity.signum() as i32)
    } else if moved.abs() > width / 2.0 {
        moved.signum() as i32
    } else {
        0
    }
}

struct Page {
    root: NodeId,
    cells: Vec<Cell7>,
}

pub struct CalendarPanel {
    pub root: NodeId,
    eyebrow: Label,
    time: Label,
    date: Label,
    month: SwapText,
    today: Chip,
    prev: Chip,
    next: Chip,
    weekdays: Vec<Label>,
    view: NodeId,
    view_area: AreaId,
    content: NodeId,
    pages: HashMap<i32, Page>,
    footer: Label,
    current: i32,
    position: Tween<f32>,
    drag: Option<Drag>,
    hover: Option<(f32, f32)>,
    today_date: Date,
    width: f32,
    height: f32,
}

pub fn month_start(today: Date, offset: i32) -> Date {
    let months = today.year() as i32 * 12 + today.month() as i32 - 1 + offset;
    let year = months.div_euclid(12) as i16;
    let month = (months.rem_euclid(12) + 1) as i8;
    Date::new(year, month, 1).unwrap_or(today)
}

pub fn lead(first: Date) -> i64 {
    first.weekday().to_monday_zero_offset() as i64
}

fn month_title(date: Date) -> String {
    date.strftime("%B %Y").to_string().to_lowercase()
}

impl CalendarPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let mut eyebrow_spec = TextSpec::plain("time / calendar", ICONS, 9.0, GREEN);
        if let Some(span) = eyebrow_spec.spans.first_mut() {
            span.letter_spacing = 1.0;
        }
        let eyebrow = Label::new(tree, engine, root, eyebrow_spec);
        let mut time_spec = TextSpec::plain(&data.time, FONT, 40.0, TEXT);
        if let Some(span) = time_spec.spans.first_mut() {
            span.weight = Weight::Light;
        }
        let time = Label::new(tree, engine, root, time_spec);
        let date = label(tree, engine, root, &data.date, 12.0, MUTED);
        let today_date = services::clock::now().date();
        let month = SwapText::new(
            tree,
            engine,
            root,
            &month_title(month_start(today_date, 0)),
            "",
            FONT,
            15.0,
            GREEN,
            100000.0,
        );
        let today = chip(tree, input, engine, root, "today", YELLOW, true, None);
        let prev = chip(tree, input, engine, root, "‹", TEXT, true, None);
        let next = chip(tree, input, engine, root, "›", TEXT, true, None);
        let weekdays = DAYS
            .iter()
            .enumerate()
            .map(|(index, day)| {
                let weekend = index > 4;
                let label = label(
                    tree,
                    engine,
                    root,
                    day,
                    10.0,
                    if weekend { YELLOW } else { MUTED },
                );
                if weekend {
                    tree.set_opacity(label.node, 0.8);
                }
                label
            })
            .collect();
        let mut view_node = Node::new(0.0, 0.0, 0.0, 0.0);
        view_node.clip = true;
        let view = tree.add(root, view_node);
        let mut mouse = MouseArea::new(view);
        mouse.wheel = true;
        mouse.cursor = platform::CursorShape::Default;
        let view_area = input.add(mouse);
        let content = tree.add(view, Node::new(0.0, 0.0, 0.0, 0.0));
        let footer = label(
            tree,
            engine,
            root,
            "← →  month    ↑ ↓  year    t  today",
            9.0,
            MUTED,
        );
        Self {
            root,
            eyebrow,
            time,
            date,
            month,
            today,
            prev,
            next,
            weekdays,
            view,
            view_area,
            content,
            pages: HashMap::new(),
            footer,
            current: BASE,
            position: Tween::new(0.0, MOVE, Easing::InOutQuad),
            drag: None,
            hover: None,
            today_date,
            width: 0.0,
            height: 0.0,
        }
    }

    pub fn areas(&self) -> Vec<AreaId> {
        vec![
            self.today.area,
            self.prev.area,
            self.next.area,
            self.view_area,
        ]
    }

    pub fn sync(&mut self, engine: &mut TextEngine, data: &PanelData) {
        self.time.set_text(engine, &data.time);
        self.date.set_text(engine, &data.date);
        let today = services::clock::now().date();
        if today != self.today_date {
            self.today_date = today;
            self.pages.clear();
        }
    }

    fn set_current(&mut self, index: i32, now: Millis) {
        let index = index.clamp(0, COUNT - 1);
        self.current = index;
        let target = (index - BASE) as f32 * self.width;
        let from = self.position.value();
        self.position.start(now, from, target);
    }

    fn shift(&mut self, step: i32, now: Millis) {
        let current = self.current;
        self.set_current(current + step, now);
    }

    fn make_page(&self, tree: &mut Tree, engine: &mut TextEngine, index: i32) -> Page {
        let root = tree.add(self.content, Node::new(0.0, 0.0, 0.0, 0.0));
        let first = month_start(self.today_date, index - BASE);
        let lead = lead(first);
        let cells = (0..42)
            .map(|cell| {
                let date = first
                    .checked_add((cell as i64 - lead).days())
                    .unwrap_or(first);
                let inside = date.month() == first.month();
                let today = date == self.today_date;
                let bg = rect(
                    tree,
                    root,
                    if today { GREEN } else { Color::TRANSPARENT },
                    9.0,
                );
                let color = if today {
                    BG
                } else if !inside {
                    LINE
                } else if cell % 7 > 4 {
                    YELLOW
                } else {
                    TEXT
                };
                let mut spec = TextSpec::plain(date.day().to_string(), FONT, 12.0, color);
                if today && let Some(span) = spec.spans.first_mut() {
                    span.weight = Weight::DemiBold;
                }
                let label = Label::new(tree, engine, root, spec);
                Cell7 {
                    date,
                    bg,
                    label,
                    color: Tween::new(
                        if today { GREEN } else { Color::TRANSPARENT },
                        FAST,
                        Easing::Linear,
                    ),
                }
            })
            .collect();
        Page { root, cells }
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        let h = |label: &Label| label.implicit_height().ceil();
        let chip_h = self.prev.implicit_height().ceil();
        let row_h = 22f32.max(chip_h);
        let slots = linear(
            &[
                Cell::implicit(h(&self.eyebrow)),
                Cell::implicit(h(&self.time)).margins(2.0, 0.0),
                Cell::implicit(h(&self.date)).margins(2.0, 0.0),
                Cell::implicit(row_h).margins(16.0, 0.0),
                Cell::implicit(h(&self.weekdays[0])).margins(12.0, 0.0),
                Cell::fill(0.0).margins(6.0, 0.0),
                Cell::implicit(h(&self.footer)).margins(6.0, 0.0),
            ],
            0.0,
            height,
        );
        for (label, slot) in [
            (&mut self.eyebrow, slots[0]),
            (&mut self.time, slots[1]),
            (&mut self.date, slots[2]),
            (&mut self.footer, slots[6]),
        ] {
            let lw = label.implicit_width().min(width);
            place_label(tree, label, 0.0, slot, lw);
        }
        let offset = self.current != BASE;
        let mut cells = vec![Cell::fill(0.0)];
        if offset {
            cells.push(Cell::implicit(self.today.implicit_width()));
        }
        cells.push(Cell::implicit(self.prev.implicit_width()));
        cells.push(Cell::implicit(self.next.implicit_width()));
        let columns = linear(&cells, 5.0, width);
        let (my, mh) = fit(slots[3], 22.0);
        self.month.place(tree, 0.0, my, columns[0].size, mh);
        tree.set_visible(self.today.root, offset);
        let mut column = 1;
        if offset {
            place_chip(tree, &mut self.today, columns[1].pos, slots[3]);
            column += 1;
        }
        place_chip(tree, &mut self.prev, columns[column].pos, slots[3]);
        place_chip(tree, &mut self.next, columns[column + 1].pos, slots[3]);
        let day_w = width / 7.0;
        for (index, label) in self.weekdays.iter_mut().enumerate() {
            let lw = label.implicit_width();
            place_label(
                tree,
                label,
                index as f32 * day_w + (day_w - lw) / 2.0,
                slots[4],
                lw,
            );
        }
        let list = slots[5];
        tree.set_pos(self.view, 0.0, list.pos);
        tree.set_size(self.view, width, list.size);
        if self.width != width {
            self.width = width;
            self.position.jump((self.current - BASE) as f32 * width);
        }
        self.height = list.size;
        self.layout_pages(tree, engine);
    }

    fn visible_pages(&self) -> Vec<i32> {
        if self.width <= 0.0 {
            return Vec::new();
        }
        let x = self.position.value() + self.drag_offset();
        let first = (x / self.width).floor() as i32 + BASE;
        let last = ((x + self.width - 0.5) / self.width).floor() as i32 + BASE;
        (first..=last)
            .filter(|index| (0..COUNT).contains(index))
            .collect()
    }

    fn drag_offset(&self) -> f32 {
        self.drag.map_or(0.0, |drag| drag.start - drag.x)
    }

    fn layout_pages(&mut self, tree: &mut Tree, engine: &mut TextEngine) {
        let visible = self.visible_pages();
        let stale: Vec<i32> = self
            .pages
            .keys()
            .copied()
            .filter(|index| !visible.contains(index))
            .collect();
        for index in stale {
            if let Some(page) = self.pages.remove(&index) {
                tree.remove(page.root);
            }
        }
        for index in visible {
            if !self.pages.contains_key(&index) {
                let page = self.make_page(tree, engine, index);
                self.pages.insert(index, page);
            }
        }
        let base_x = self.position.value() + self.drag_offset();
        tree.set_pos(self.content, -base_x.round(), 0.0);
        let (cw, ch) = (self.width / 7.0, self.height / 6.0);
        let (bw, bh) = ((cw - 6.0).min(38.0), (ch - 4.0).min(30.0));
        for (index, page) in &mut self.pages {
            tree.set_pos(page.root, (*index - BASE) as f32 * self.width, 0.0);
            tree.set_size(page.root, self.width, self.height);
            for (cell_index, cell) in page.cells.iter_mut().enumerate() {
                let x = (cell_index % 7) as f32 * cw;
                let y = (cell_index / 7) as f32 * ch;
                tree.set_pos(cell.bg, x + (cw - bw) / 2.0, y + (ch - bh) / 2.0);
                tree.set_size(cell.bg, bw, bh);
                let lw = cell.label.implicit_width();
                let lh = cell.label.implicit_height().ceil();
                cell.label
                    .place(tree, x + (cw - lw) / 2.0, y + (ch - lh) / 2.0, lw, lh);
            }
        }
    }

    fn hovered_cell(&self) -> Option<(i32, usize)> {
        let (x, y) = self.hover?;
        if self.drag.is_some() || self.width <= 0.0 || y < 0.0 || y >= self.height || x < 0.0 {
            return None;
        }
        let content_x = x + self.position.value();
        let page = (content_x / self.width).floor() as i32 + BASE;
        let local = content_x - (page - BASE) as f32 * self.width;
        let column = ((local / (self.width / 7.0)).floor() as usize).min(6);
        let row = ((y / (self.height / 6.0)).floor() as usize).min(5);
        let (cw, ch) = (self.width / 7.0, self.height / 6.0);
        let (bw, bh) = ((cw - 6.0).min(38.0), (ch - 4.0).min(30.0));
        let cx = local - column as f32 * cw - (cw - bw) / 2.0;
        let cy = y - row as f32 * ch - (ch - bh) / 2.0;
        (cx >= 0.0 && cx < bw && cy >= 0.0 && cy < bh).then_some((page, row * 7 + column))
    }

    pub fn tick(
        &mut self,
        tree: &mut Tree,
        input: &Input,
        engine: &mut TextEngine,
        now: Millis,
    ) -> bool {
        let mut running = false;
        for chip in [&mut self.today, &mut self.prev, &mut self.next] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        let shown = month_start(self.today_date, self.current - BASE);
        self.month.set(&month_title(shown), "");
        self.month.flush(engine, now);
        running |= self.month.tick(tree, now);
        running |= self.position.tick(now);
        self.layout_pages(tree, engine);
        let hovered = self.hovered_cell();
        let today = self.today_date;
        for (index, page) in &mut self.pages {
            for (cell_index, cell) in page.cells.iter_mut().enumerate() {
                let target = if cell.date == today {
                    GREEN
                } else if hovered == Some((*index, cell_index)) {
                    ELEVATED
                } else {
                    Color::TRANSPARENT
                };
                if cell.color.target() != target {
                    cell.color.set_target(now, target);
                }
                running |= cell.color.tick(now);
                tree.set_fill(cell.bg, cell.color.value());
            }
        }
        running
    }

    pub fn handle(&mut self, events: &[InputEvent], pointer: (f32, f32), tree: &Tree, now: Millis) {
        for event in events {
            match *event {
                InputEvent::Wheel(area, delta) if area == self.view_area => {
                    self.shift(if delta > 0 { -1 } else { 1 }, now);
                }
                InputEvent::Press(area, MouseButton::Left) if area == self.view_area => {
                    let x = self.local_x(tree, pointer);
                    self.drag = Some(Drag {
                        start: x,
                        x,
                        last: (x, std::time::Instant::now()),
                        velocity: 0.0,
                    });
                }
                InputEvent::Release(_, MouseButton::Left) => self.end_drag(now),
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.today.area {
                        self.set_current(BASE, now);
                    } else if area == self.prev.area {
                        self.shift(-1, now);
                    } else if area == self.next.area {
                        self.shift(1, now);
                    }
                }
                _ => {}
            }
        }
    }

    fn local_x(&self, tree: &Tree, pointer: (f32, f32)) -> f32 {
        self.local(tree, pointer).0
    }

    fn local(&self, tree: &Tree, pointer: (f32, f32)) -> (f32, f32) {
        let Some(inverse) = tree.device_transform(self.view).invert() else {
            return (-1.0, -1.0);
        };
        let mut point = ui::tiny_skia::Point::from_xy(pointer.0, pointer.1);
        inverse.map_point(&mut point);
        (point.x, point.y)
    }

    fn end_drag(&mut self, now: Millis) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let moved = drag.start - drag.x;
        let velocity = if drag.last.1.elapsed().as_millis() > 100 {
            0.0
        } else {
            drag.velocity
        };
        let step = drag_step(moved, velocity, self.width);
        let from = self.position.value() + moved;
        self.position.jump(from);
        let current = self.current;
        self.set_current(current + step, now);
    }

    pub fn motion(&mut self, tree: &Tree, input: &Input, x: f32, y: f32) {
        let local = self.local(tree, (x, y));
        self.hover = Some(local);
        if let Some(drag) = self.drag.as_mut() {
            if input.pressed(self.view_area) {
                let min = -(self.current as f32) * self.width;
                let max = (COUNT - 1 - self.current) as f32 * self.width;
                drag.x = drag.start - (drag.start - local.0).clamp(min, max);
                let elapsed = drag.last.1.elapsed().as_secs_f32();
                if elapsed > 0.0 {
                    drag.velocity = (drag.x - drag.last.0) / elapsed;
                }
                drag.last = (drag.x, std::time::Instant::now());
            } else {
                self.drag = None;
            }
        }
    }

    pub fn key(&mut self, event: &KeyEvent, now: Millis) -> (bool, Vec<PanelAction>) {
        match event.keysym {
            keys::LEFT => self.shift(-1, now),
            keys::RIGHT => self.shift(1, now),
            keys::UP => self.shift(-12, now),
            keys::DOWN => self.shift(12, now),
            keys::HOME | 0x74 | 0x54 => self.set_current(BASE, now),
            _ => return (false, Vec::new()),
        }
        (true, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_wrap_years() {
        let today = Date::new(2026, 11, 1).unwrap_or_default();
        assert_eq!(
            month_start(today, 2),
            Date::new(2027, 1, 1).unwrap_or_default()
        );
        assert_eq!(
            month_start(today, -11),
            Date::new(2025, 12, 1).unwrap_or_default()
        );
    }

    #[test]
    fn drag_snaps_like_snap_one_item() {
        assert_eq!(drag_step(30.0, -400.0, 380.0), 1);
        assert_eq!(drag_step(-30.0, 400.0, 380.0), -1);
        assert_eq!(drag_step(200.0, 0.0, 380.0), 1);
        assert_eq!(drag_step(100.0, 10.0, 380.0), 0);
    }

    #[test]
    fn monday_first_lead() {
        let first = Date::new(2026, 11, 1).unwrap_or_default();
        assert_eq!(lead(first), 6);
        assert_eq!(month_title(first), "november 2026");
    }
}
