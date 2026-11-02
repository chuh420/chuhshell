use platform::MouseButton;
use services::system::{Detail, Proc};
use ui::{
    Color, Node, NodeId, TextEngine, TextSpec, Tree, Weight,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Buttons, Input, InputEvent, MouseArea},
    layout::{Cell, Slot, linear},
    widgets::{Chip, Label, Sparkline},
};

use super::{
    PanelAction, PanelData, TooltipSpot,
    kit::{chip, icon_label, label, place_chip, place_label, rect},
};
use crate::theme::{self, FONT, NORMAL};

struct Card {
    root: NodeId,
    icon: Label,
    name: Label,
    value: Label,
    sub: Label,
    spark: Sparkline,
}

struct Core {
    track: NodeId,
    fill: NodeId,
    width: Tween<f32>,
    label: Label,
}

struct Row {
    root: NodeId,
    area: AreaId,
    name: Label,
    cpu: Label,
    mem: Label,
    kill: Chip,
    pid: i32,
    own: bool,
}

pub struct SystemPanel {
    pub root: NodeId,
    title: Label,
    uptime: Label,
    cards: [Card; 2],
    cores: Vec<Core>,
    details: Vec<(Label, Label)>,
    separator: NodeId,
    processes: Label,
    sort_cpu: Chip,
    sort_mem: Chip,
    list: NodeId,
    rows: Vec<Row>,
    empty: Label,
    btop: Chip,
    sort: &'static str,
    data: PanelData,
    rows_key: Vec<(i32, String, String, String, bool)>,
}

pub fn uptime(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor() as i64;
    let days = minutes / 1440;
    let hours = minutes % 1440 / 60;
    let prefix = if days > 0 {
        format!("{days}d ")
    } else {
        String::new()
    };
    format!("{prefix}{hours}h {:02}m", minutes % 60)
}

pub fn size(gib: f64) -> String {
    if gib >= 100.0 {
        format!("{} GiB", (gib + 0.5).floor() as i64)
    } else {
        format!("{gib:.1} GiB")
    }
}

fn heat(value: i64) -> Color {
    if value >= 90 {
        theme::red()
    } else if value >= 75 {
        theme::yellow()
    } else {
        theme::text()
    }
}

fn mem_text(mem: i64) -> String {
    if mem >= 1024 {
        format!("{:.1} GiB", mem as f64 / 1024.0)
    } else {
        format!("{mem} MiB")
    }
}

impl SystemPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "system", 18.0, theme::text());
        let uptime = label(tree, engine, root, "", 12.0, theme::green());
        let card = |tree: &mut Tree, engine: &mut TextEngine, cpu: bool| {
            let root = rect(tree, root, theme::surface(), 14.0);
            let color = if cpu { theme::green() } else { theme::blue() };
            let mut value_spec = TextSpec::plain("", FONT, 24.0, theme::text());
            if let Some(span) = value_spec.spans.first_mut() {
                span.weight = Weight::Light;
            }
            Card {
                root,
                icon: icon_label(tree, engine, root, if cpu { "󰻠" } else { "󰍛" }, 16.0, color),
                name: label(
                    tree,
                    engine,
                    root,
                    if cpu { "cpu" } else { "memory" },
                    12.0,
                    theme::muted(),
                ),
                value: Label::new(tree, engine, root, value_spec),
                sub: label(tree, engine, root, "", 10.0, theme::muted()),
                spark: Sparkline::new(tree, root, color, theme::yellow(), theme::line()),
            }
        };
        let cards = [card(tree, engine, true), card(tree, engine, false)];
        let details = ["swap", "disk /", "load average"]
            .iter()
            .map(|key| {
                (
                    label(tree, engine, root, key, 12.0, theme::muted()),
                    label(tree, engine, root, "", 12.0, theme::text()),
                )
            })
            .collect();
        let separator = rect(tree, root, theme::line(), 0.0);
        let processes = label(tree, engine, root, "processes", 12.0, theme::muted());
        let sort_cpu = chip(tree, input, engine, root, "cpu", theme::text(), true, None);
        let sort_mem = chip(
            tree,
            input,
            engine,
            root,
            "memory",
            theme::text(),
            true,
            None,
        );
        let mut list_node = Node::new(0.0, 0.0, 0.0, 0.0);
        list_node.clip = true;
        let list = tree.add(root, list_node);
        let empty = label(
            tree,
            engine,
            list,
            "reading processes…",
            12.0,
            theme::muted(),
        );
        let btop = chip(
            tree,
            input,
            engine,
            root,
            "btop ↗",
            theme::text(),
            false,
            None,
        );
        let mut panel = Self {
            root,
            title,
            uptime,
            cards,
            cores: Vec::new(),
            details,
            separator,
            processes,
            sort_cpu,
            sort_mem,
            list,
            rows: Vec::new(),
            empty,
            btop,
            sort: "cpu",
            data: PanelData::default(),
            rows_key: Vec::new(),
        };
        panel.sync(tree, input, engine, data, now);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.sort_cpu.area, self.sort_mem.area, self.btop.area];
        for row in &self.rows {
            areas.extend([row.area, row.kill.area]);
        }
        areas
    }

    fn processes_list(&self) -> Vec<Proc> {
        match &self.data.system_detail {
            Some(detail) if self.sort == "cpu" => detail.processes.cpu.clone(),
            Some(detail) => detail.processes.mem.clone(),
            None => Vec::new(),
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
        self.data = data.clone();
        let detail = data.system_detail.as_ref();
        self.uptime.set_text(
            engine,
            &detail.map_or(String::new(), |detail| {
                format!("up {}", uptime(detail.uptime))
            }),
        );
        let res = data.resources;
        let cpu_value = match res.cpu {
            Some(cpu) => format!("{cpu}%"),
            None => "…".into(),
        };
        self.cards[0].value.set_text(engine, &cpu_value);
        self.cards[0]
            .value
            .set_color(if res.cpu.is_some_and(|cpu| cpu >= 85) {
                theme::yellow()
            } else {
                theme::text()
            });
        let cpu_sub = match detail {
            Some(detail) => format!(
                "{:.2} GHz{}",
                detail.frequency,
                detail
                    .temperature
                    .map_or(String::new(), |t| format!(" · {t}°C"))
            ),
            None => "…".into(),
        };
        self.cards[0].sub.set_text(engine, &cpu_sub);
        self.cards[0].sub.set_color(
            detail
                .and_then(|detail| detail.temperature)
                .map_or(theme::muted(), heat),
        );
        self.cards[1]
            .value
            .set_text(engine, &format!("{:.1} GiB", res.ram_used));
        self.cards[1]
            .sub
            .set_text(engine, &format!("of {:.1} GiB", res.ram_total));
        self.cards[0].spark.maximum = 100.0;
        self.cards[1].spark.maximum = res.ram_total.max(1.0);
        self.sync_cores(tree, engine, detail, now);
        let values = detail.map(|detail: &Detail| {
            [
                format!("{} / {}", size(detail.swap_used), size(detail.swap_total)),
                format!("{} / {}", size(detail.disk_used), size(detail.disk_total)),
                detail
                    .load
                    .iter()
                    .map(|value| format!("{value:.2}"))
                    .collect::<Vec<_>>()
                    .join("  "),
            ]
        });
        for (index, (key, value)) in self.details.iter_mut().enumerate() {
            tree.set_visible(key.node, values.is_some());
            tree.set_visible(value.node, values.is_some());
            if let Some(values) = &values {
                value.set_text(engine, &values[index]);
            }
        }
        self.sort_cpu.selected = self.sort == "cpu";
        self.sort_mem.selected = self.sort == "mem";
        self.sync_rows(tree, input, engine);
    }

    fn sync_cores(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        detail: Option<&Detail>,
        now: Millis,
    ) {
        let cores = detail.map_or(&[][..], |detail| &detail.cores[..]);
        if cores.len() != self.cores.len() {
            for core in self.cores.drain(..) {
                tree.remove(core.track);
                tree.remove(core.label.node);
            }
            for _ in cores {
                let track = rect(tree, self.root, theme::elevated(), 3.0);
                let fill = rect(tree, track, theme::green(), 3.0);
                let label = label(tree, engine, self.root, "", 9.0, theme::muted());
                self.cores.push(Core {
                    track,
                    fill,
                    width: Tween::new(0.0, NORMAL, Easing::OutCubic),
                    label,
                });
            }
        }
        for (index, (core, value)) in self.cores.iter_mut().zip(cores).enumerate() {
            let text = match value {
                Some(value) => format!("core {index} · {value}%"),
                None => format!("core {index} · …"),
            };
            core.label.set_text(engine, &text);
            let percent = value.unwrap_or(0) as f32;
            if core.width.target() != percent {
                core.width.set_target(now, percent);
            }
            tree.set_fill(
                core.fill,
                if percent >= 85.0 {
                    theme::yellow()
                } else {
                    theme::green()
                },
            );
        }
    }

    fn sync_rows(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine) {
        let list = self.processes_list();
        let key: Vec<(i32, String, String, String, bool)> = list
            .iter()
            .map(|proc| {
                (
                    proc.pid,
                    proc.name.clone(),
                    format!("{:.1}%", proc.cpu),
                    mem_text(proc.mem),
                    proc.own,
                )
            })
            .collect();
        if key == self.rows_key {
            return;
        }
        for row in self.rows.drain(..) {
            input.remove(row.area);
            input.remove(row.kill.area);
            tree.remove(row.root);
        }
        for (pid, name, cpu, mem, own) in &key {
            let root = rect(tree, self.list, Color::TRANSPARENT, 8.0);
            let mut mouse = MouseArea::hover_handler(root);
            mouse.cursor = platform::CursorShape::Default;
            let area = input.add(mouse);
            let name = label(tree, engine, root, name, 12.0, theme::text());
            let cpu_color = if self.sort == "cpu" {
                theme::text()
            } else {
                theme::muted()
            };
            let mem_color = if self.sort == "mem" {
                theme::text()
            } else {
                theme::muted()
            };
            let cpu = icon_label(tree, engine, root, cpu, 10.0, cpu_color);
            let mem = icon_label(tree, engine, root, mem, 10.0, mem_color);
            let mut kill = chip(tree, input, engine, root, "󰅖", theme::red(), true, None);
            kill.shortcut = "end process · right click to force".into();
            if let Some(area) = input.get_mut(kill.area) {
                area.buttons = Buttons::LEFT_RIGHT;
                area.enabled = *own;
            }
            tree.set_opacity(kill.root, if *own { 1.0 } else { 0.3 });
            self.rows.push(Row {
                root,
                area,
                name,
                cpu,
                mem,
                kill,
                pid: *pid,
                own: *own,
            });
        }
        self.rows_key = key;
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let h = |label: &Label| label.implicit_height().ceil();
        let has_detail = self.data.system_detail.is_some();
        let row1 = h(&self.title).max(h(&self.uptime));
        let core_h = 5.0 + 4.0 + self.cores.first().map_or(0.0, |core| h(&core.label));
        let detail_h = h(&self.details[0].0);
        let chip_h = self.sort_cpu.implicit_height().ceil();
        let mut cells = vec![Cell::implicit(row1), Cell::layout(128.0)];
        if has_detail {
            cells.push(Cell::implicit(core_h));
            cells.extend((0..3).map(|_| Cell::implicit(detail_h)));
        }
        cells.extend([
            Cell::layout(1.0).margins(2.0, 0.0),
            Cell::implicit(h(&self.processes).max(chip_h)),
            Cell::fill(0.0),
            Cell::implicit(self.btop.implicit_height()),
        ]);
        let slots = linear(&cells, 12.0, height);
        let mut next = slots.iter().copied();
        let slot = next.next().unwrap_or_default();
        let uw = self.uptime.implicit_width();
        place_label(tree, &mut self.uptime, width - uw, slot, uw);
        place_label(
            tree,
            &mut self.title,
            0.0,
            slot,
            (width - uw - 5.0).max(0.0),
        );
        let slot = next.next().unwrap_or_default();
        let columns = linear(&[Cell::fill(1.0), Cell::fill(1.0)], 10.0, width);
        for (index, column) in columns.iter().enumerate() {
            let history = if index == 0 {
                self.data.cpu_history.clone()
            } else {
                self.data.ram_history.clone()
            };
            Self::layout_card(
                tree,
                &mut self.cards[index],
                column.pos,
                slot.pos,
                column.size,
                slot.size,
                &history,
            );
        }
        if has_detail {
            let slot = next.next().unwrap_or_default();
            let cells: Vec<Cell> = self.cores.iter().map(|_| Cell::fill(1.0)).collect();
            let columns = linear(&cells, 6.0, width);
            for (core, column) in self.cores.iter_mut().zip(columns) {
                tree.set_pos(core.track, column.pos, slot.pos);
                tree.set_size(core.track, column.size, 5.0);
                let lw = core.label.implicit_width().min(column.size);
                let lh = core.label.implicit_height().ceil();
                core.label.place(tree, column.pos, slot.pos + 9.0, lw, lh);
            }
            for (key, value) in &mut self.details {
                let slot = next.next().unwrap_or_default();
                let vw = value.implicit_width();
                place_label(tree, value, width - vw, slot, vw);
                place_label(tree, key, 0.0, slot, (width - vw - 5.0).max(0.0));
            }
        }
        for core in &self.cores {
            tree.set_visible(core.track, has_detail);
            tree.set_visible(core.label.node, has_detail);
        }
        let slot = next.next().unwrap_or_default();
        tree.set_pos(self.separator, 0.0, slot.pos);
        tree.set_size(self.separator, width, 1.0);
        let slot = next.next().unwrap_or_default();
        let mw = self.sort_mem.implicit_width().ceil();
        let cw = self.sort_cpu.implicit_width().ceil();
        place_chip(tree, &mut self.sort_mem, width - mw, slot);
        place_chip(tree, &mut self.sort_cpu, width - mw - 5.0 - cw, slot);
        place_label(
            tree,
            &mut self.processes,
            0.0,
            slot,
            (width - mw - cw - 10.0).max(0.0),
        );
        let slot = next.next().unwrap_or_default();
        tree.set_pos(self.list, 0.0, slot.pos);
        tree.set_size(self.list, width, slot.size);
        tree.set_visible(self.empty.node, self.rows.is_empty());
        let ew = self.empty.implicit_width();
        let eh = h(&self.empty);
        self.empty
            .place(tree, (width - ew) / 2.0, (slot.size - eh) / 2.0, ew, eh);
        for (index, row) in self.rows.iter_mut().enumerate() {
            let y = index as f32 * 32.0;
            tree.set_pos(row.root, 0.0, y);
            tree.set_size(row.root, width, 30.0);
            let line = Slot {
                pos: 0.0,
                size: 30.0,
            };
            let kw = row.kill.implicit_width().ceil();
            let kx = width - 2.0 - kw;
            place_chip(tree, &mut row.kill, kx, line);
            let mem_x = kx - 8.0 - 64.0;
            let cpu_x = mem_x - 8.0 - 48.0;
            let mw = row.mem.implicit_width().min(64.0);
            place_label(tree, &mut row.mem, mem_x + 64.0 - mw, line, mw);
            let cw = row.cpu.implicit_width().min(48.0);
            place_label(tree, &mut row.cpu, cpu_x + 48.0 - cw, line, cw);
            place_label(tree, &mut row.name, 8.0, line, (cpu_x - 8.0 - 8.0).max(0.0));
        }
        let slot = next.next().unwrap_or_default();
        place_chip(tree, &mut self.btop, 0.0, slot);
    }

    fn layout_card(
        tree: &mut Tree,
        card: &mut Card,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        history: &[f64],
    ) {
        tree.set_pos(card.root, x, y);
        tree.set_size(card.root, width, height);
        let inner = width - 24.0;
        let h = |label: &Label| label.implicit_height().ceil();
        let row = h(&card.icon).max(h(&card.name));
        let slots = linear(
            &[
                Cell::implicit(row),
                Cell::implicit(h(&card.value)),
                Cell::implicit(h(&card.sub)),
                Cell::fill(0.0).margins(6.0, 0.0),
            ],
            2.0,
            height - 24.0,
        );
        let shift = |slot: Slot| Slot {
            pos: slot.pos + 12.0,
            size: slot.size,
        };
        let iw = card.icon.implicit_width();
        place_label(tree, &mut card.icon, 12.0, shift(slots[0]), iw);
        place_label(
            tree,
            &mut card.name,
            12.0 + iw + 5.0,
            shift(slots[0]),
            (inner - iw - 5.0).max(0.0),
        );
        let vw = card.value.implicit_width().min(inner);
        place_label(tree, &mut card.value, 12.0, shift(slots[1]), vw);
        place_label(tree, &mut card.sub, 12.0, shift(slots[2]), inner);
        let spark = shift(slots[3]);
        card.spark
            .place(tree, 12.0, spark.pos, inner, spark.size, history, &[]);
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for chip in [&mut self.sort_cpu, &mut self.sort_mem, &mut self.btop]
            .into_iter()
            .chain(self.rows.iter_mut().map(|row| &mut row.kill))
        {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        for core in &mut self.cores {
            running |= core.width.tick(now);
            let track_w = tree.get(core.track).map_or(0.0, |node| node.width);
            tree.set_size(
                core.fill,
                track_w * core.width.value().max(0.0) / 100.0,
                5.0,
            );
        }
        for row in &self.rows {
            let color = if input.contains_mouse(row.area) {
                theme::surface()
            } else {
                Color::TRANSPARENT
            };
            tree.set_fill(row.root, color);
        }
        running
    }

    pub fn handle(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        events: &[InputEvent],
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            let InputEvent::Click(area, button) = *event else {
                continue;
            };
            if button == MouseButton::Left && area == self.sort_cpu.area {
                self.sort = "cpu";
                self.rows_key.clear();
            } else if button == MouseButton::Left && area == self.sort_mem.area {
                self.sort = "mem";
                self.rows_key.clear();
            } else if button == MouseButton::Left && area == self.btop.area {
                actions.push(PanelAction::Close);
                actions.push(PanelAction::Launch("btop.desktop".into()));
            } else if let Some(row) = self
                .rows
                .iter()
                .find(|row| row.kill.area == area && row.own)
            {
                actions.push(PanelAction::Kill(row.pid, button == MouseButton::Right));
            }
        }
        let data = self.data.clone();
        self.sync(tree, input, engine, &data, now);
        actions
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        self.rows
            .iter()
            .map(|row| &row.kill)
            .find(|chip| chip.hovered(input))
            .map(|chip| TooltipSpot {
                key: chip.area,
                text: chip.shortcut.clone(),
                node: chip.root,
                width: chip.width(),
                height: chip.height(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_like_qml() {
        assert_eq!(uptime(3_725.0), "1h 02m");
        assert_eq!(uptime(90_061.0), "1d 1h 01m");
    }

    #[test]
    fn sizes_like_qml() {
        assert_eq!(size(12.345), "12.3 GiB");
        assert_eq!(size(250.4), "250 GiB");
        assert_eq!(mem_text(2048), "2.0 GiB");
        assert_eq!(mem_text(300), "300 MiB");
    }
}
