use platform::MouseButton;
use services::upower::Battery;
use ui::{
    Node, NodeId, TextEngine, TextSpec, Tree, Weight,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent},
    layout::{Cell, Slot, linear},
    widgets::{Chip, Label, Slider, Sparkline},
};

use super::{
    PanelAction, PanelData, TooltipSpot,
    control::SLIDER_STYLE,
    kit::{chip, fit, label, place_label, rect},
};
use crate::{
    history::LIMIT,
    theme::{ELEVATED, FONT, GREEN, LINE, MUTED, RED, SURFACE, TEXT, YELLOW},
};

const PROFILES: [(&str, &str, &str); 3] = [
    ("power-saver", "󰾆  saver", "force minimum power use"),
    ("balanced", "󰾅  balanced", "default on battery"),
    ("performance", "󰓅  performance", "default on AC"),
];
const ROWS: [&str; 5] = [
    "remaining",
    "power draw",
    "health",
    "capacity",
    "charge cycles",
];

struct Graph {
    card: NodeId,
    title: Label,
    range: Label,
    spark: Sparkline,
}

pub struct BatteryPanel {
    pub root: NodeId,
    title: Label,
    state: Label,
    level: Label,
    track: NodeId,
    fill: NodeId,
    shimmer: NodeId,
    fill_width: Tween<f32>,
    shimmer_start: Millis,
    rows: Vec<(Label, Label)>,
    graphs: [Graph; 2],
    profile_title: Label,
    profiles: Vec<Chip>,
    auto: Label,
    brightness_title: Label,
    slider: Slider,
    data: PanelData,
    width: f32,
}

pub fn duration(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    let minutes = (seconds as f64 / 60.0 + 0.5).floor() as i64;
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

pub fn state_text(battery: Option<&Battery>, plugged: bool) -> &'static str {
    let Some(battery) = battery else {
        return "no battery";
    };
    match battery.state {
        1 => "charging",
        2 => "on battery",
        4 => "fully charged",
        5 => "not charging",
        3 => "empty",
        _ => {
            if plugged {
                "plugged in"
            } else {
                "on battery"
            }
        }
    }
}

pub fn remaining(battery: Option<&Battery>) -> String {
    let Some(battery) = battery else {
        return "—".into();
    };
    match battery.state {
        4 => "Fully charged".into(),
        1 => match duration(battery.time_to_full) {
            text if text.is_empty() => "Calculating…".into(),
            text => format!("{text} until full"),
        },
        2 => match duration(battery.time_to_empty) {
            text if text.is_empty() => "Calculating…".into(),
            text => format!("{text} left"),
        },
        _ => "—".into(),
    }
}

fn round_js(value: f64) -> i64 {
    (value + 0.5).floor() as i64
}

fn values(data: &PanelData) -> [String; 5] {
    let battery = data.power.battery.as_ref();
    let power = battery.map_or(0.0, |battery| battery.energy_rate.abs());
    let stats = data.battery_stats.unwrap_or_default();
    let health = match battery.filter(|battery| battery.capacity > 0.0) {
        Some(battery) => format!("{}%", round_js(battery.capacity)),
        None if stats.health != 0 => format!("{}%", stats.health),
        None => "—".into(),
    };
    let capacity = if stats.design != 0.0 {
        format!("{:.1} / {:.1} Wh", stats.full, stats.design)
    } else {
        "—".into()
    };
    [
        remaining(battery),
        if power != 0.0 {
            format!("{power:.1} W")
        } else {
            "—".into()
        },
        health,
        capacity,
        if stats.cycles > 0 {
            stats.cycles.to_string()
        } else {
            "not reported".into()
        },
    ]
}

fn level(data: &PanelData) -> u32 {
    data.power
        .battery
        .as_ref()
        .map_or(0, |battery| round_js(battery.percentage) as u32)
}

fn trim(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

impl BatteryPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "battery", 18.0, TEXT);
        let state = label(tree, engine, root, "", 12.0, GREEN);
        let mut level_spec = TextSpec::plain("", FONT, 42.0, TEXT);
        if let Some(span) = level_spec.spans.first_mut() {
            span.weight = Weight::Light;
        }
        let level = Label::new(tree, engine, root, level_spec);
        let track = rect(tree, root, ELEVATED, 4.0);
        let fill = rect(tree, track, GREEN, 4.0);
        let shimmer = rect(tree, fill, TEXT, 4.0);
        tree.set_opacity(shimmer, 0.25);
        let rows = ROWS
            .iter()
            .map(|key| {
                (
                    label(tree, engine, root, key, 12.0, MUTED),
                    label(tree, engine, root, "", 12.0, TEXT),
                )
            })
            .collect();
        let graph = |tree: &mut Tree, engine: &mut TextEngine, name: &str, color| {
            let card = rect(tree, root, SURFACE, 12.0);
            Graph {
                card,
                title: label(tree, engine, card, &format!("{name} · 6h"), 10.0, MUTED),
                range: label(tree, engine, card, "", 10.0, color),
                spark: Sparkline::new(tree, card, color, YELLOW, LINE),
            }
        };
        let graphs = [
            graph(tree, engine, "level", GREEN),
            graph(tree, engine, "power", YELLOW),
        ];
        let profile_title = label(tree, engine, root, "power profile", 12.0, MUTED);
        let profiles = PROFILES
            .iter()
            .map(|(_, text, tip)| {
                let mut chip = chip(tree, input, engine, root, text, TEXT, false, None);
                chip.shortcut = (*tip).to_string();
                chip
            })
            .collect();
        let auto = label(
            tree,
            engine,
            root,
            "auto: performance on AC, balanced on battery",
            10.0,
            MUTED,
        );
        let brightness_title = label(tree, engine, root, "", 12.0, MUTED);
        let mut slider = Slider::new(tree, input, root, &SLIDER_STYLE, YELLOW, 5.0, 100.0);
        slider.shortcut = "XF86MonBrightnessUp / XF86MonBrightnessDown".into();
        let mut panel = Self {
            root,
            title,
            state,
            level,
            track,
            fill,
            shimmer,
            fill_width: Tween::new(0.0, 700.0, Easing::OutCubic),
            shimmer_start: now,
            rows,
            graphs,
            profile_title,
            profiles,
            auto,
            brightness_title,
            slider,
            data: PanelData::default(),
            width: 0.0,
        };
        panel.sync(engine, data, now);
        panel.slider.jump(tree, input);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.slider.area];
        areas.extend(self.profiles.iter().map(|chip| chip.area));
        areas
    }

    pub fn sync(&mut self, engine: &mut TextEngine, data: &PanelData, now: Millis) {
        self.data = data.clone();
        let battery = data.power.battery.as_ref();
        self.state
            .set_text(engine, state_text(battery, data.battery_plugged));
        self.level.set_text(engine, &format!("{}%", level(data)));
        for ((_, value), text) in self.rows.iter_mut().zip(values(data)) {
            value.set_text(engine, &text);
        }
        let level_points: Vec<f64> = data
            .battery_history
            .iter()
            .map(|point| point.level as f64)
            .collect();
        let power_points: Vec<f64> = data
            .battery_history
            .iter()
            .map(|point| point.power)
            .collect();
        let level_range = match (
            level_points.iter().cloned().reduce(f64::min),
            level_points.iter().cloned().reduce(f64::max),
        ) {
            (Some(min), Some(max)) => format!("{}–{}%", trim(min), trim(max)),
            _ => "—".into(),
        };
        let power_range = match power_points.iter().cloned().reduce(f64::max) {
            Some(max) => format!("max {max:.1} W"),
            None => "—".into(),
        };
        self.graphs[0].range.set_text(engine, &level_range);
        self.graphs[1].range.set_text(engine, &power_range);
        let capacity = level_points.len().clamp(60, LIMIT.max(60));
        for (graph, maximum) in self.graphs.iter_mut().zip([100.0, 0.0]) {
            graph.spark.capacity = capacity;
            graph.spark.maximum = maximum;
        }
        for (chip, (name, _, _)) in self.profiles.iter_mut().zip(PROFILES) {
            let selected = data.profiles.active == name;
            chip.selected = selected;
            chip.set_ink(now, if selected { GREEN } else { TEXT });
        }
        self.brightness_title
            .set_text(engine, &format!("Brightness · {}%", data.brightness));
        self.slider.set_value(data.brightness as f64);
    }

    fn charging(&self) -> bool {
        self.data
            .power
            .battery
            .as_ref()
            .is_some_and(|battery| battery.state == 1)
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        self.width = width;
        let h = |label: &Label| label.implicit_height().ceil();
        let row_h = h(&self.title).max(h(&self.state));
        let value_h = h(&self.rows[0].0);
        let chip_h = self.profiles[0].implicit_height().ceil();
        let mut cells = vec![
            Cell::implicit(row_h),
            Cell::implicit(h(&self.level)),
            Cell::layout(8.0),
        ];
        cells.extend((0..ROWS.len()).map(|_| Cell::implicit(value_h)));
        cells.extend([
            Cell::layout(84.0).margins(4.0, 0.0),
            Cell::implicit(h(&self.profile_title)).margins(4.0, 0.0),
            Cell::implicit(chip_h),
            Cell::implicit(h(&self.auto)),
            Cell::implicit(h(&self.brightness_title)).margins(4.0, 0.0),
            Cell::layout(32.0),
            Cell::fill(0.0),
        ]);
        let slots = linear(&cells, 12.0, height);
        let tw = self.title.implicit_width();
        let sw = self.state.implicit_width();
        place_label(
            tree,
            &mut self.title,
            0.0,
            slots[0],
            (width - sw - 5.0).max(tw.min(width)),
        );
        place_label(tree, &mut self.state, width - sw, slots[0], sw);
        let lw = self.level.implicit_width();
        place_label(tree, &mut self.level, 0.0, slots[1], lw);
        tree.set_pos(self.track, 0.0, slots[2].pos);
        tree.set_size(self.track, width, 8.0);
        for (index, (key, value)) in self.rows.iter_mut().enumerate() {
            let slot = slots[3 + index];
            let vw = value.implicit_width();
            place_label(tree, value, width - vw, slot, vw);
            place_label(tree, key, 0.0, slot, (width - vw - 5.0).max(0.0));
        }
        let graphs = slots[8];
        let columns = linear(&[Cell::fill(0.0), Cell::fill(0.0)], 10.0, width);
        for (graph, column) in self.graphs.iter_mut().zip(columns) {
            Self::layout_graph(
                tree,
                graph,
                column.pos,
                graphs.pos,
                column.size,
                graphs.size,
                &self.data,
            );
        }
        let pw = self.profile_title.implicit_width();
        place_label(tree, &mut self.profile_title, 0.0, slots[9], pw);
        let visible: Vec<usize> = (0..PROFILES.len())
            .filter(|index| *index != 2 || self.data.profiles.has_performance)
            .collect();
        let cells: Vec<Cell> = visible
            .iter()
            .map(|index| Cell::fill(self.profiles[*index].implicit_width()))
            .collect();
        let columns = linear(&cells, 6.0, width);
        for (index, chip) in self.profiles.iter_mut().enumerate() {
            tree.set_visible(chip.root, visible.contains(&index));
        }
        for (column, index) in columns.iter().zip(&visible) {
            let chip = &mut self.profiles[*index];
            chip.set_fixed_width(Some(column.size));
            let (y, h) = fit(slots[10], chip.implicit_height().ceil());
            chip.place_sized(tree, column.pos, y, column.size, h);
        }
        place_label(tree, &mut self.auto, 0.0, slots[11], width);
        let bw = self.brightness_title.implicit_width();
        place_label(tree, &mut self.brightness_title, 0.0, slots[12], bw);
        self.slider
            .place(tree, 0.0, slots[13].pos, width, slots[13].size);
    }

    fn layout_graph(
        tree: &mut Tree,
        graph: &mut Graph,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        data: &PanelData,
    ) {
        tree.set_pos(graph.card, x, y);
        tree.set_size(graph.card, width, height);
        let inner = width - 20.0;
        let th = graph.title.implicit_height().ceil();
        let rh = graph.range.implicit_height().ceil();
        let row = Slot {
            pos: 10.0,
            size: th.max(rh),
        };
        let rw = graph.range.implicit_width();
        place_label(tree, &mut graph.range, 10.0 + inner - rw, row, rw);
        place_label(
            tree,
            &mut graph.title,
            10.0,
            row,
            (inner - rw - 5.0).max(0.0),
        );
        let top = row.pos + row.size + 6.0;
        let points: Vec<f64> = data
            .battery_history
            .iter()
            .map(|point| {
                if graph.spark.maximum > 0.0 {
                    point.level as f64
                } else {
                    point.power
                }
            })
            .collect();
        graph.spark.place(
            tree,
            10.0,
            top,
            inner,
            (height - 10.0 - top).max(0.0),
            &points,
            &[],
        );
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for chip in &mut self.profiles {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.slider.tick(tree, input, now);
        let target = self.width * level(&self.data) as f32 / 100.0;
        if self.fill_width.target() != target {
            self.fill_width.set_target(now, target);
        }
        running |= self.fill_width.tick(now);
        let fill = self.fill_width.value();
        tree.set_size(self.fill, fill, 8.0);
        tree.set_fill(self.fill, if level(&self.data) < 20 { RED } else { GREEN });
        let charging = self.charging();
        tree.set_visible(self.shimmer, charging);
        if charging {
            let t = ((now - self.shimmer_start) % 1600.0) / 1600.0;
            let x = -28.0 + (fill + 28.0) * Easing::InOutSine.value(t) as f32;
            tree.set_pos(self.shimmer, x, 0.0);
            tree.set_size(self.shimmer, 28.0, 8.0);
            running = true;
        }
        running
    }

    pub fn handle(
        &mut self,
        tree: &Tree,
        events: &[InputEvent],
        pointer: (f32, f32),
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Press(area, MouseButton::Left) if area == self.slider.area => {
                    self.slider.press(tree, pointer.0, pointer.1);
                }
                InputEvent::Release(area, MouseButton::Left) if area == self.slider.area => {
                    if let Some(value) = self.slider.release(tree, pointer.0, pointer.1) {
                        actions.push(PanelAction::SetBrightness(value));
                    }
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if let Some(index) = self.profiles.iter().position(|chip| chip.area == area) {
                        actions.push(PanelAction::SetProfile(PROFILES[index].0));
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn motion(&mut self, tree: &Tree, input: &Input, x: f32, y: f32) -> Vec<PanelAction> {
        if input.pressed(self.slider.area)
            && let Some(value) = self.slider.motion(tree, x, y)
        {
            return vec![PanelAction::SetBrightness(value)];
        }
        Vec::new()
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        for chip in &self.profiles {
            if chip.hovered(input) {
                return Some(TooltipSpot {
                    key: chip.area,
                    text: chip.shortcut.clone(),
                    node: chip.root,
                    width: chip.width(),
                    height: chip.height(),
                });
            }
        }
        input.contains_mouse(self.slider.area).then(|| TooltipSpot {
            key: self.slider.area,
            text: self.slider.shortcut.clone(),
            node: self.slider.root,
            width: self.slider.width(),
            height: self.slider.height(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_match_qml() {
        assert_eq!(duration(0), "");
        assert_eq!(duration(3690), "1h 02m");
        assert_eq!(duration(89), "0h 01m");
    }

    #[test]
    fn states_match_qml() {
        let battery = |state| Battery {
            state,
            ..Battery::default()
        };
        assert_eq!(state_text(None, false), "no battery");
        assert_eq!(state_text(Some(&battery(1)), false), "charging");
        assert_eq!(state_text(Some(&battery(0)), true), "plugged in");
        assert_eq!(remaining(Some(&battery(4))), "Fully charged");
        assert_eq!(remaining(Some(&battery(1))), "Calculating…");
    }
}
