use std::{path::PathBuf, sync::Arc};

use ui::{
    Color, Node, NodeId, Paint, RectPaint, Span, SvgPaint, TextEngine, TextPaint, TextSpec,
    TransformOp, Tree,
    layout::{Item, center, plain_row, row_layout},
    rich::chip_spans,
};

use crate::theme::{BG, ELEVATED, FONT, GREEN, ICONS, LINE, MUTED, RED, TEXT, UI_SCALE, YELLOW};

pub struct BarModel {
    pub title: Option<String>,
    pub workspaces: Vec<i32>,
    pub occupied: Vec<i32>,
    pub active_workspace: i32,
    pub clock: String,
    pub cpu: Option<u32>,
    pub ram_used: f64,
    pub ram_total: f64,
    pub keyboard_label: String,
    pub network_connected: bool,
    pub wifi_connected: bool,
    pub wifi_strength: u32,
    pub bluetooth_powered: bool,
    pub bluetooth_connected: bool,
    pub muted: bool,
    pub volume: u32,
    pub battery_present: bool,
    pub battery_level: u32,
    pub battery_plugged: bool,
    pub hotkeys_paused: bool,
    pub dnd: bool,
    pub unread_notifications: u32,
}

impl Default for BarModel {
    fn default() -> Self {
        Self {
            title: None,
            workspaces: vec![1, 2, 3, 4, 5],
            occupied: vec![1],
            active_workspace: 1,
            clock: format_clock(&jiff::Zoned::now()),
            cpu: None,
            ram_used: 0.0,
            ram_total: 0.0,
            keyboard_label: String::new(),
            network_connected: false,
            wifi_connected: false,
            wifi_strength: 0,
            bluetooth_powered: false,
            bluetooth_connected: false,
            muted: false,
            volume: 0,
            battery_present: false,
            battery_level: 0,
            battery_plugged: false,
            hotkeys_paused: false,
            dnd: false,
            unread_notifications: 0,
        }
    }
}

impl BarModel {
    pub fn placeholder() -> Self {
        Self {
            keyboard_label: "RU".into(),
            network_connected: true,
            wifi_connected: true,
            wifi_strength: 100,
            bluetooth_powered: true,
            volume: 50,
            battery_present: true,
            battery_level: 100,
            battery_plugged: true,
            ..Self::default()
        }
    }
}

pub fn format_clock(time: &jiff::Zoned) -> String {
    time.strftime("%a, %-d %b  ·  %H:%M").to_string()
}

pub fn battery_icon(level: u32, plugged: bool) -> &'static str {
    const DISCHARGE: [&str; 11] = ["󰂎", "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰂀", "󰂁", "󰂂", "󰁹"];
    const CHARGE: [&str; 11] = ["󰢟", "󰢜", "󰂆", "󰂇", "󰂈", "󰢝", "󰂉", "󰢞", "󰂊", "󰂋", "󰂅"];
    let index = ((level as f64 / 10.0).round() as usize).min(10);
    if plugged {
        CHARGE[index]
    } else {
        DISCHARGE[index]
    }
}

pub fn network_icon(model: &BarModel) -> &'static str {
    if !model.network_connected {
        return "󰤭";
    }
    if !model.wifi_connected {
        return "󰈀";
    }
    match model.wifi_strength {
        s if s > 75 => "󰤨",
        s if s > 50 => "󰤥",
        s if s > 25 => "󰤢",
        _ => "󰤟",
    }
}

fn rice_path(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config/rice").join(name)
}

struct Chip {
    label: String,
    ink: Color,
    font_size: f32,
    icon_size: f32,
    width: Option<f32>,
    selected: bool,
    color: Option<Color>,
}

impl Chip {
    fn compact(label: impl Into<String>, ink: Color) -> Self {
        Self {
            label: label.into(),
            ink,
            font_size: 11.0,
            icon_size: 13.0,
            width: None,
            selected: false,
            color: None,
        }
    }

    fn spec(&self) -> TextSpec {
        TextSpec {
            spans: chip_spans(&self.label, ICONS, self.font_size, self.icon_size),
            color: self.ink,
            elide_width: None,
        }
    }
}

pub struct Bar {
    pub tree: Tree,
    body: NodeId,
    width: f32,
    height: f32,
    keyboard_icon: Option<Arc<ui::SvgTree>>,
}

impl Bar {
    pub fn new(width: u32, height: u32) -> Self {
        let mut tree = Tree::new();
        let body_width = width as f32 / UI_SCALE;
        let body_height = height as f32 / UI_SCALE;
        let mut body = Node::rect(
            0.0,
            0.0,
            body_width,
            body_height,
            RectPaint {
                color: BG,
                radius: 9.0,
                border_color: LINE,
                border_width: 1.0,
            },
        );
        body.scale = UI_SCALE;
        body.origin = Some((0.0, 0.0));
        body.transforms = vec![TransformOp::Translate { x: 0.0, y: 0.0 }];
        let root = tree.root();
        let body = tree.add(root, body);
        Self {
            tree,
            body,
            width: body_width,
            height: body_height,
            keyboard_icon: ui::image::load_svg(&rice_path("icons/keyboard-bar.svg")),
        }
    }

    pub fn rebuild(&mut self, engine: &mut TextEngine, model: &BarModel) {
        self.tree.clear_children(self.body);
        self.build_left(engine, model);
        self.build_clock(engine, model);
        self.build_right(engine, model);
    }

    fn measure(engine: &mut TextEngine, spec: &TextSpec) -> ui::TextMetrics {
        engine.measure(spec, 1.0)
    }

    fn add_text(&mut self, parent: NodeId, spec: TextSpec, x: f32, y: f32, w: f32, h: f32) {
        self.tree.add(
            parent,
            Node::new(x, y, w, h).with_paint(Paint::Text(TextPaint::new(spec))),
        );
    }

    fn chip_size(engine: &mut TextEngine, chip: &Chip) -> (f32, f32, ui::TextMetrics) {
        let metrics = Self::measure(engine, &chip.spec());
        (chip.width.unwrap_or(metrics.width + 16.0), 22.0, metrics)
    }

    fn add_chip(
        &mut self,
        engine: &mut TextEngine,
        parent: NodeId,
        chip: &Chip,
        x: f32,
        y: f32,
        width: f32,
    ) {
        let (_, height, metrics) = Self::chip_size(engine, chip);
        let color = chip.color.unwrap_or(if chip.selected {
            ELEVATED
        } else {
            Color::TRANSPARENT
        });
        let node = self.tree.add(
            parent,
            Node::rect(
                x,
                y,
                width,
                height,
                RectPaint {
                    color,
                    radius: 6.0,
                    ..RectPaint::default()
                },
            ),
        );
        self.add_text(
            node,
            chip.spec(),
            center(width, metrics.width),
            center(height, metrics.height),
            metrics.width,
            metrics.height,
        );
    }

    fn separator(&mut self, parent: NodeId, x: f32, y: f32) {
        self.tree.add(
            parent,
            Node::rect(
                x,
                y,
                1.0,
                12.0,
                RectPaint {
                    color: LINE,
                    ..RectPaint::default()
                },
            ),
        );
    }

    fn title_spec(&self, model: &BarModel, max_width: f32) -> TextSpec {
        let title = model
            .title
            .clone()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "still / material".into());
        TextSpec {
            spans: vec![Span::new(title, FONT, 10.0)],
            color: MUTED,
            elide_width: Some(max_width),
        }
    }

    fn build_left(&mut self, engine: &mut TextEngine, model: &BarModel) {
        let launcher = Chip::compact("󰣇", GREEN);
        let (launcher_w, _, _) = Self::chip_size(engine, &launcher);
        let slot = 25.0;
        let gap = 3.0;
        let count = model.workspaces.len() as f32;
        let workspaces_w = count * slot + (count - 1.0) * gap;
        let max_width = 70.0f32.max(self.width * 0.16);
        let title_spec = self.title_spec(model, max_width);
        let title_metrics = Self::measure(engine, &title_spec);
        let title_w = max_width.min(title_metrics.width);
        let items = [
            Item::new(launcher_w, 22.0),
            Item::new(1.0, 12.0).margins(4.0, 6.0),
            Item::new(workspaces_w, 22.0),
            Item::preferred(title_w, 16.0).margins(12.0, 0.0),
        ];
        let layout = row_layout(&items, 3.0);
        let row = self.tree.add(
            self.body,
            Node::new(
                5.0,
                center(self.height, layout.height),
                layout.width,
                layout.height,
            ),
        );
        let p = &layout.positions;
        self.add_chip(
            engine,
            row,
            &launcher,
            p[0].0,
            p[0].1,
            items[0].layout_width(),
        );
        self.separator(row, p[1].0, p[1].1);
        self.build_workspaces(engine, model, row, p[2].0, p[2].1);
        let mut title = Node::new(p[3].0, p[3].1, title_w, 16.0);
        title.clip = true;
        let title = self.tree.add(row, title);
        self.add_text(
            title,
            title_spec,
            0.0,
            center(16.0, title_metrics.height),
            title_w,
            title_metrics.height,
        );
    }

    fn build_workspaces(
        &mut self,
        engine: &mut TextEngine,
        model: &BarModel,
        parent: NodeId,
        x: f32,
        y: f32,
    ) {
        let slot = 25.0;
        let gap = 3.0;
        let count = model.workspaces.len() as f32;
        let width = count * slot + (count - 1.0) * gap;
        let node = self.tree.add(parent, Node::new(x, y, width, 22.0));
        let active_index = model
            .workspaces
            .iter()
            .position(|id| *id == model.active_workspace);
        if let Some(index) = active_index {
            let left = index as f32 * (slot + gap);
            let highlight = self.tree.add(
                node,
                Node::rect(
                    left,
                    0.0,
                    slot,
                    22.0,
                    RectPaint {
                        color: ELEVATED,
                        radius: 6.0,
                        ..RectPaint::default()
                    },
                ),
            );
            let bar_w = (10.0 / UI_SCALE).max(14.0 / UI_SCALE);
            let bar_h = 2.0 / UI_SCALE;
            self.tree.add(
                highlight,
                Node::rect(
                    center(slot, bar_w),
                    22.0 - bar_h,
                    bar_w,
                    bar_h,
                    RectPaint {
                        color: GREEN,
                        radius: 1.0,
                        ..RectPaint::default()
                    },
                ),
            );
        }
        for (index, id) in model.workspaces.iter().enumerate() {
            let active = *id == model.active_workspace;
            let occupied = model.occupied.contains(id);
            let label = if active {
                format!("{id:02}")
            } else if occupied {
                "●".into()
            } else {
                "○".into()
            };
            let ink = if active {
                GREEN
            } else if occupied {
                TEXT
            } else {
                MUTED
            };
            let mut chip = Chip::compact(label, ink);
            chip.width = Some(slot);
            chip.font_size = if active { 11.0 } else { 13.0 };
            chip.color = Some(Color::TRANSPARENT);
            self.add_chip(engine, node, &chip, index as f32 * (slot + gap), 0.0, slot);
        }
    }

    fn build_clock(&mut self, engine: &mut TextEngine, model: &BarModel) {
        let chip = Chip::compact(model.clock.clone(), TEXT);
        let (width, height, _) = Self::chip_size(engine, &chip);
        let x = center(self.width, width);
        let y = center(self.height, height);
        self.add_chip(engine, self.body, &chip, x, y, width);
    }

    fn resource_spec(icon: &str, value: String, color: Color) -> TextSpec {
        TextSpec {
            spans: vec![
                Span::new(icon, ICONS, 12.0),
                Span::new(format!(" {value}"), ICONS, 10.0),
            ],
            color,
            elide_width: None,
        }
    }

    fn build_right(&mut self, engine: &mut TextEngine, model: &BarModel) {
        let cpu_text = model
            .cpu
            .map_or_else(|| "…".to_string(), |cpu| format!("{cpu}%"));
        let cpu_color = if model.cpu.is_some_and(|cpu| cpu >= 85) {
            YELLOW
        } else {
            TEXT
        };
        let ram_text = if model.ram_total > 0.0 {
            format!("{:.1} GiB", model.ram_used)
        } else {
            "…".to_string()
        };
        let cpu_spec = Self::resource_spec("󰻠", cpu_text, cpu_color);
        let ram_spec = Self::resource_spec("󰍛", ram_text, TEXT);
        let cpu_metrics = Self::measure(engine, &cpu_spec);
        let ram_metrics = Self::measure(engine, &ram_spec);
        let resource_row = plain_row(
            &[
                Item::new(53.0, cpu_metrics.height),
                Item::new(74.0, ram_metrics.height),
            ],
            0.0,
        );
        let resource_w = resource_row.width + 8.0;

        let network = Chip::compact(
            network_icon(model),
            if model.network_connected {
                GREEN
            } else {
                MUTED
            },
        );
        let bluetooth = Chip::compact(
            if model.bluetooth_connected {
                "󰂱"
            } else if model.bluetooth_powered {
                "󰂯"
            } else {
                "󰂲"
            },
            if model.bluetooth_powered {
                GREEN
            } else {
                MUTED
            },
        );
        let audio = Chip::compact(
            if model.muted {
                "󰖁"
            } else if model.volume > 66 {
                "󰕾"
            } else if model.volume > 33 {
                "󰖀"
            } else {
                "󰕿"
            },
            if model.muted { MUTED } else { TEXT },
        );
        let battery = Chip::compact(
            format!(
                "{} {}%",
                battery_icon(model.battery_level, model.battery_plugged),
                model.battery_level
            ),
            if model.battery_plugged {
                GREEN
            } else if model.battery_level < 20 {
                RED
            } else {
                TEXT
            },
        );
        let paused = Chip::compact("󰌐", YELLOW);
        let bell = Chip::compact(
            if model.dnd {
                "󰂛"
            } else if model.unread_notifications > 0 {
                "󱅫"
            } else {
                "󰂚"
            },
            if model.dnd { MUTED } else { YELLOW },
        );
        let control = Chip::compact("󰒓", GREEN);

        enum Slot<'a> {
            Resources,
            Keyboard,
            Chip(&'a Chip),
            Separator,
        }
        let mut slots: Vec<(Slot, Item)> = vec![
            (
                Slot::Resources,
                Item::new(resource_w, 22.0).margins(2.0, 0.0),
            ),
            (Slot::Keyboard, Item::new(49.0, 22.0)),
        ];
        let chips: Vec<&Chip> = {
            let mut list = vec![&network, &bluetooth, &audio];
            if model.battery_present {
                list.push(&battery);
            }
            list
        };
        for chip in chips {
            let (w, h, _) = Self::chip_size(engine, chip);
            slots.push((Slot::Chip(chip), Item::new(w, h)));
        }
        slots.push((Slot::Separator, Item::new(1.0, 12.0).margins(5.0, 5.0)));
        let tail: Vec<&Chip> = if model.hotkeys_paused {
            vec![&paused, &bell, &control]
        } else {
            vec![&bell, &control]
        };
        for chip in tail {
            let (w, h, _) = Self::chip_size(engine, chip);
            slots.push((Slot::Chip(chip), Item::new(w, h)));
        }
        let items: Vec<Item> = slots.iter().map(|(_, item)| *item).collect();
        let layout = row_layout(&items, 2.0);
        let row = self.tree.add(
            self.body,
            Node::new(
                self.width - 5.0 - layout.width,
                center(self.height, layout.height),
                layout.width,
                layout.height,
            ),
        );
        for ((slot, item), (x, y)) in slots.iter().zip(layout.positions.iter().copied()) {
            match slot {
                Slot::Resources => {
                    let chip = self.tree.add(
                        row,
                        Node::rect(
                            x,
                            y,
                            resource_w,
                            22.0,
                            RectPaint {
                                radius: 6.0,
                                ..RectPaint::default()
                            },
                        ),
                    );
                    let inner = self.tree.add(
                        chip,
                        Node::new(
                            4.0,
                            center(22.0, resource_row.height),
                            resource_row.width,
                            resource_row.height,
                        ),
                    );
                    self.add_text(inner, cpu_spec.clone(), 0.0, 0.0, 53.0, cpu_metrics.height);
                    self.add_text(inner, ram_spec.clone(), 53.0, 0.0, 74.0, ram_metrics.height);
                }
                Slot::Keyboard => self.build_keyboard(engine, model, row, x, y),
                Slot::Chip(chip) => self.add_chip(engine, row, chip, x, y, item.layout_width()),
                Slot::Separator => self.separator(row, x, y),
            }
        }
    }

    fn build_keyboard(
        &mut self,
        engine: &mut TextEngine,
        model: &BarModel,
        parent: NodeId,
        x: f32,
        y: f32,
    ) {
        let node = self.tree.add(
            parent,
            Node::rect(
                x,
                y,
                49.0,
                22.0,
                RectPaint {
                    radius: 6.0,
                    ..RectPaint::default()
                },
            ),
        );
        let inner_row = plain_row(&[Item::new(16.0, 12.0), Item::new(16.0, 14.0)], 5.0);
        let inner = self.tree.add(
            node,
            Node::new(
                center(49.0, inner_row.width),
                center(22.0, inner_row.height),
                inner_row.width,
                inner_row.height,
            ),
        );
        if let Some(icon) = &self.keyboard_icon {
            self.tree.add(
                inner,
                Node::new(0.0, center(inner_row.height, 12.0), 16.0, 12.0)
                    .with_paint(Paint::Svg(SvgPaint::new(icon.clone()))),
            );
        }
        let spec = TextSpec::plain(model.keyboard_label.clone(), ICONS, 10.0, TEXT);
        let metrics = Self::measure(engine, &spec);
        let mut swap = Node::new(21.0, center(inner_row.height, 14.0), 16.0, 14.0);
        swap.clip = true;
        let swap = self.tree.add(inner, swap);
        let spec = TextSpec {
            elide_width: Some(16.0),
            ..spec
        };
        self.add_text(
            swap,
            spec,
            0.0,
            center(14.0, metrics.height),
            16.0,
            metrics.height,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_matches_qml_format() {
        let time: jiff::Zoned = "2026-10-31T14:05:00+03:00[Europe/Moscow]"
            .parse()
            .expect("valid time");
        assert_eq!(format_clock(&time), "Sat, 31 Oct  ·  14:05");
    }

    #[test]
    fn battery_icons_round_like_qml() {
        assert_eq!(battery_icon(100, true), "󰂅");
        assert_eq!(battery_icon(4, false), "󰂎");
        assert_eq!(battery_icon(5, false), "󰁺");
    }
}
