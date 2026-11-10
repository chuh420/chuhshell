use std::sync::Arc;

use platform::{MouseButton, PopupPlacement};
use ui::{
    Color, Node, NodeId, Paint, RectPaint, SvgPaint, TextEngine, TextPaint, TextSpec, TransformOp,
    Tree,
    anim::{Easing, Millis, Sequence, Step, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Item, center, plain_row, row_layout},
    widgets::{Chip, ChipStyle, SwapText},
};

use crate::{
    model::{BarModel, battery_icon, bluetooth_shortcut, network_icon},
    theme::{self, FAST, NORMAL, SLOW},
};

fn chip_style() -> ChipStyle {
    ChipStyle {
        family: crate::theme::icons(),
        hover: theme::elevated(),
    }
}
const KEYBOARD_ICON: &str = include_str!("../assets/keyboard-bar.svg");
const SLOT: f32 = 25.0;
const GAP: f32 = 3.0;

#[derive(Clone, Debug, PartialEq)]
pub enum BarAction {
    Open {
        panel: &'static str,
        anchor: Option<f64>,
    },
    Workspace(i32),
    Tray {
        key: String,
        right: bool,
        anchor: Option<f64>,
    },
    TrayScroll {
        key: String,
        delta: i32,
    },
    KeyboardNext,
    ToggleMute,
    Volume(i32),
    ToggleDnd,
    ToggleVpn,
}

pub struct TooltipTarget {
    pub key: AreaId,
    pub text: String,
    pub anchor: PopupPlacement,
}

fn chip(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    label: &str,
    ink: Color,
    shortcut: &str,
) -> Chip {
    let mut chip = Chip::new(
        tree,
        input,
        engine,
        parent,
        &chip_style(),
        label,
        ink,
        true,
        Some(13.0),
    );
    chip.shortcut = shortcut.to_string();
    chip
}

fn separator(tree: &mut Tree, parent: NodeId) -> NodeId {
    tree.add(
        parent,
        Node::rect(
            0.0,
            0.0,
            1.0,
            12.0,
            RectPaint {
                color: theme::line(),
                ..RectPaint::default()
            },
        ),
    )
}

struct WorkspaceChip {
    id: i32,
    chip: Chip,
    active: bool,
    pop: Sequence,
}

fn pop_sequence() -> Sequence {
    Sequence::new(
        1.0,
        vec![
            Step::To {
                value: 0.8,
                duration: 70.0,
                easing: Easing::OutQuad,
            },
            Step::Spring {
                value: 1.0,
                spring: 5.0,
                damping: 0.28,
            },
        ],
    )
}

struct Workspaces {
    root: NodeId,
    highlight: NodeId,
    marker: NodeId,
    row: NodeId,
    chips: Vec<WorkspaceChip>,
    ids: Vec<i32>,
    active_id: i32,
    occupied: Vec<i32>,
    ready: bool,
    edge_left: Tween<f32>,
    edge_right: Tween<f32>,
    highlight_opacity: Tween<f32>,
    implicit_width: Tween<f32>,
}

impl Workspaces {
    fn new(tree: &mut Tree, parent: NodeId) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 22.0));
        let highlight = tree.add(
            root,
            Node::rect(
                0.0,
                0.0,
                SLOT,
                22.0,
                RectPaint {
                    color: theme::elevated(),
                    radius: 6.0,
                    ..RectPaint::default()
                },
            ),
        );
        let marker = tree.add(
            highlight,
            Node::rect(
                0.0,
                0.0,
                0.0,
                2.0 / crate::theme::scale(),
                RectPaint {
                    color: theme::green(),
                    radius: 1.0,
                    ..RectPaint::default()
                },
            ),
        );
        let row = tree.add(root, Node::new(0.0, 0.0, 0.0, 22.0));
        Self {
            root,
            highlight,
            marker,
            row,
            chips: Vec::new(),
            ids: Vec::new(),
            active_id: 1,
            occupied: Vec::new(),
            ready: false,
            edge_left: Tween::new(0.0, 0.0, Easing::OutCubic),
            edge_right: Tween::new(SLOT, 0.0, Easing::OutCubic),
            highlight_opacity: Tween::new(1.0, FAST, Easing::Linear),
            implicit_width: Tween::new(0.0, NORMAL, Easing::OutCubic),
        }
    }

    fn width_for(count: usize) -> f32 {
        let count = count as f32;
        (count * SLOT + (count - 1.0) * GAP).max(0.0)
    }

    fn active_index(&self) -> Option<usize> {
        self.ids.iter().position(|id| *id == self.active_id)
    }

    fn place(&mut self, now: Millis) {
        let Some(index) = self.active_index() else {
            return;
        };
        let target = index as f32 * (SLOT + GAP);
        let forward = target > self.edge_left.value();
        self.edge_left
            .set_duration(if forward { 360.0 } else { 190.0 });
        self.edge_right
            .set_duration(if forward { 190.0 } else { 360.0 });
        if self.ready {
            self.edge_left.set_target(now, target);
            self.edge_right.set_target(now, target + SLOT);
        } else {
            self.edge_left.jump(target);
            self.edge_right.jump(target + SLOT);
        }
    }

    fn label(&self, id: i32) -> (String, f32, Color) {
        let active = id == self.active_id;
        let occupied = self.occupied.contains(&id);
        let label = if active {
            format!("{id:02}")
        } else if occupied {
            "●".to_string()
        } else {
            "○".to_string()
        };
        let ink = if active {
            theme::green()
        } else if occupied {
            theme::text()
        } else {
            theme::muted()
        };
        (label, if active { 11.0 } else { 13.0 }, ink)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        computed: Vec<i32>,
        active_id: i32,
        occupied: Vec<i32>,
        now: Millis,
    ) {
        let previous_index = self.active_index();
        self.occupied = occupied;
        self.active_id = active_id;
        if computed != self.ids {
            self.ids = computed;
            let width = Self::width_for(self.ids.len());
            if self.ready {
                self.implicit_width.set_target(now, width);
            } else {
                self.implicit_width.jump(width);
            }
            let mut kept: Vec<WorkspaceChip> = Vec::new();
            for existing in self.chips.drain(..) {
                if self.ids.contains(&existing.id) {
                    kept.push(existing);
                } else {
                    input.remove(existing.chip.area);
                    tree.remove(existing.chip.root);
                }
            }
            for id in self.ids.clone() {
                if !kept.iter().any(|chip| chip.id == id) {
                    let (label, font_size, ink) = self.label(id);
                    let mut chip = chip(tree, input, engine, self.row, &label, ink, "");
                    chip.set_label(engine, &label, font_size);
                    chip.shortcut = format!("Super + {id}");
                    chip.set_fixed_width(Some(SLOT));
                    chip.color_override = Some(Color::TRANSPARENT);
                    chip.jump_color(input);
                    kept.push(WorkspaceChip {
                        id,
                        chip,
                        active: false,
                        pop: pop_sequence(),
                    });
                }
            }
            kept.sort_by_key(|chip| self.ids.iter().position(|id| *id == chip.id));
            self.chips = kept;
        }
        for index in 0..self.chips.len() {
            let id = self.chips[index].id;
            let (label, font_size, ink) = self.label(id);
            let ready = self.ready;
            let entry = &mut self.chips[index];
            entry.chip.set_label(engine, &label, font_size);
            if ready {
                entry.chip.set_ink(now, ink);
            } else {
                entry.chip.jump_ink(ink);
            }
            let active = id == active_id;
            if active && !entry.active && ready {
                entry.pop.restart(now);
            }
            entry.active = active;
        }
        if self.active_index() != previous_index || !self.ready {
            self.place(now);
        }
        if !self.ready {
            self.ready = true;
        }
    }

    fn sync(&mut self, now: Millis, input: &Input) {
        for entry in &mut self.chips {
            let hovered = entry.chip.hovered(input);
            entry.chip.color_override = Some(if hovered && !entry.active {
                theme::surface()
            } else {
                Color::TRANSPARENT
            });
            entry.chip.sync(now, input);
        }
        let opacity = if self.active_index().is_some() {
            1.0
        } else {
            0.0
        };
        self.highlight_opacity.set_target(now, opacity);
    }

    fn implicit_width(&self) -> f32 {
        self.implicit_width.value()
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let mut running = self.edge_left.tick(now);
        running |= self.edge_right.tick(now);
        running |= self.highlight_opacity.tick(now);
        running |= self.implicit_width.tick(now);
        let left = self.edge_left.value();
        let width = self.edge_right.value() - left;
        tree.set_pos(self.highlight, left, 0.0);
        tree.set_size(self.highlight, width, 22.0);
        tree.set_opacity(self.highlight, self.highlight_opacity.value());
        let marker_width = (10.0 / crate::theme::scale())
            .max(14.0 / crate::theme::scale() - (width - SLOT) * 0.15);
        let marker_height = 2.0 / crate::theme::scale();
        tree.set_pos(
            self.marker,
            center(width, marker_width),
            22.0 - marker_height,
        );
        tree.set_size(self.marker, marker_width, marker_height);
        for (index, entry) in self.chips.iter_mut().enumerate() {
            running |= entry.pop.tick(now);
            let pulse = entry.pop.value() as f32;
            entry.chip.extra = vec![TransformOp::Scale {
                x: pulse,
                y: pulse,
                origin_x: SLOT / 2.0,
                origin_y: 11.0,
            }];
            entry
                .chip
                .place(tree, index as f32 * (SLOT + GAP), 0.0, SLOT);
            running |= entry.chip.tick(tree, now);
        }
        running
    }
}

struct HoverRect {
    root: NodeId,
    area: AreaId,
    color: Tween<Color>,
    scale: Tween<f32>,
    pressed_scale: f32,
}

impl HoverRect {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        parent: NodeId,
        width: f32,
        pressed_scale: f32,
    ) -> Self {
        let root = tree.add(
            parent,
            Node::rect(
                0.0,
                0.0,
                width,
                22.0,
                RectPaint {
                    radius: 6.0,
                    ..RectPaint::default()
                },
            ),
        );
        let area = input.add(MouseArea::new(root));
        Self {
            root,
            area,
            color: Tween::new(Color::TRANSPARENT, 140.0, Easing::Linear),
            scale: Tween::new(1.0, 110.0, Easing::OutCubic),
            pressed_scale,
        }
    }

    fn sync(&mut self, now: Millis, input: &Input, selected: bool) {
        let color = if input.contains_mouse(self.area) || selected {
            theme::elevated()
        } else {
            Color::TRANSPARENT
        };
        self.color.set_target(now, color);
        let scale = if input.pressed(self.area) {
            self.pressed_scale
        } else {
            1.0
        };
        self.scale.set_target(now, scale);
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.color.tick(now) | self.scale.tick(now);
        tree.set_fill(self.root, self.color.value());
        tree.set_scale(self.root, self.scale.value());
        running
    }
}

struct TrayIcon {
    key: String,
    rect: HoverRect,
    image: NodeId,
    glyph: NodeId,
    opacity: Tween<f32>,
    icon: Option<Arc<ui::tiny_skia::Pixmap>>,
}

impl TrayIcon {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        key: &str,
        now: Millis,
    ) -> Self {
        let rect = HoverRect::new(tree, input, parent, 23.0, 0.9);
        if let Some(area) = input.get_mut(rect.area) {
            area.buttons = ui::input::Buttons::LEFT_RIGHT;
            area.wheel = true;
        }
        let image = tree.add(
            rect.root,
            Node::new(center(23.0, 17.0), center(22.0, 17.0), 17.0, 17.0),
        );
        let spec = TextSpec::plain("󰀻", crate::theme::icons(), 16.0, theme::muted());
        let metrics = engine.measure(&spec, 1.0);
        let glyph = tree.add(
            rect.root,
            Node::new(
                center(23.0, metrics.width),
                center(22.0, metrics.height),
                metrics.width,
                metrics.height,
            )
            .with_paint(Paint::Text(TextPaint::new(spec))),
        );
        let mut opacity = Tween::new(0.0, NORMAL, Easing::Linear);
        opacity.start(now, 0.0, 1.0);
        tree.set_opacity(rect.root, 0.0);
        Self {
            key: key.to_string(),
            rect,
            image,
            glyph,
            opacity,
            icon: None,
        }
    }

    fn set_icon(&mut self, tree: &mut Tree, icon: Option<Arc<ui::tiny_skia::Pixmap>>) {
        let same = match (&self.icon, &icon) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.icon = icon;
        if let Some(node) = tree.get_mut(self.image) {
            node.paint = match &self.icon {
                Some(pixmap) => Paint::Image(pixmap.clone()),
                None => Paint::None,
            };
        }
        tree.set_visible(self.glyph, self.icon.is_none());
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.rect.tick(tree, now) | self.opacity.tick(now);
        tree.set_opacity(self.rect.root, self.opacity.value());
        running
    }
}

struct Resources {
    rect: HoverRect,
    cpu: NodeId,
    ram: NodeId,
    cpu_text: String,
    ram_text: String,
    cpu_color: Tween<Color>,
    width: f32,
}

fn resource_spec(icon: &str, value: &str, color: Color) -> TextSpec {
    TextSpec {
        spans: vec![
            ui::Span::new(icon, crate::theme::icons(), 12.0),
            ui::Span::new(format!(" {value}"), crate::theme::icons(), 10.0),
        ],
        color,
        elide_width: None,
        wrap_width: None,
        max_lines: None,
    }
}

impl Resources {
    fn new(tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, parent: NodeId) -> Self {
        let cpu_spec = resource_spec("󰻠", "…", theme::text());
        let ram_spec = resource_spec("󰍛", "…", theme::text());
        let cpu_metrics = engine.measure(&cpu_spec, 1.0);
        let ram_metrics = engine.measure(&ram_spec, 1.0);
        let layout = plain_row(
            &[
                Item::new(53.0, cpu_metrics.height),
                Item::new(74.0, ram_metrics.height),
            ],
            0.0,
        );
        let width = layout.width + 8.0;
        let rect = HoverRect::new(tree, input, parent, width, 0.96);
        let row = tree.add(
            rect.root,
            Node::new(
                4.0,
                center(22.0, layout.height),
                layout.width,
                layout.height,
            ),
        );
        let cpu = tree.add(
            row,
            Node::new(0.0, 0.0, 53.0, cpu_metrics.height)
                .with_paint(Paint::Text(TextPaint::new(cpu_spec))),
        );
        let ram = tree.add(
            row,
            Node::new(53.0, 0.0, 74.0, ram_metrics.height)
                .with_paint(Paint::Text(TextPaint::new(ram_spec))),
        );
        Self {
            rect,
            cpu,
            ram,
            cpu_text: "…".into(),
            ram_text: "…".into(),
            cpu_color: Tween::new(theme::text(), SLOW, Easing::Linear),
            width,
        }
    }

    fn apply(&mut self, model: &BarModel, now: Millis) {
        self.cpu_text = model
            .cpu
            .map_or_else(|| "…".to_string(), |cpu| format!("{cpu}%"));
        self.ram_text = if model.ram_total > 0.0 {
            format!("{:.1} GiB", model.ram_used)
        } else {
            "…".to_string()
        };
        let color = if model.cpu.is_some_and(|cpu| cpu >= 85) {
            theme::yellow()
        } else {
            theme::text()
        };
        self.cpu_color.set_target(now, color);
    }

    fn tick(&mut self, tree: &mut Tree, now: Millis) -> bool {
        let running = self.rect.tick(tree, now) | self.cpu_color.tick(now);
        tree.set_text(
            self.cpu,
            resource_spec("󰻠", &self.cpu_text, self.cpu_color.value()),
        );
        tree.set_text(self.ram, resource_spec("󰍛", &self.ram_text, theme::text()));
        running
    }
}

struct Keyboard {
    rect: HoverRect,
    label: SwapText,
}

impl Keyboard {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        icon: Option<Arc<ui::SvgTree>>,
        label: &str,
    ) -> Self {
        let rect = HoverRect::new(tree, input, parent, 49.0, 0.94);
        let layout = plain_row(&[Item::new(16.0, 12.0), Item::new(16.0, 14.0)], 5.0);
        let row = tree.add(
            rect.root,
            Node::new(
                center(49.0, layout.width),
                center(22.0, layout.height),
                layout.width,
                layout.height,
            ),
        );
        if let Some(icon) = icon {
            tree.add(
                row,
                Node::new(0.0, center(layout.height, 12.0), 16.0, 12.0)
                    .with_paint(Paint::Svg(SvgPaint::new(icon))),
            );
        }
        let mut label = SwapText::new(
            tree,
            engine,
            row,
            label,
            "",
            crate::theme::icons(),
            10.0,
            theme::text(),
            100000.0,
        );
        label.place(tree, 21.0, center(layout.height, 14.0), 16.0, 14.0);
        Self { rect, label }
    }
}

pub struct Bar {
    pub tree: Tree,
    pub input: Input,
    body: NodeId,
    enter: Tween<f32>,
    width: f32,
    height: f32,
    left_row: NodeId,
    launcher: Chip,
    left_separator: NodeId,
    workspaces: Workspaces,
    title: SwapText,
    clock: Chip,
    right_row: NodeId,
    tray: Vec<TrayIcon>,
    resources: Resources,
    keyboard: Keyboard,
    network: Chip,
    vpn: Chip,
    bluetooth: Chip,
    audio: Chip,
    battery: Chip,
    battery_visible: bool,
    network_visible: bool,
    vpn_visible: bool,
    bluetooth_visible: bool,
    battery_plugged: Option<bool>,
    battery_pulse: Sequence,
    right_separator: NodeId,
    paused: Chip,
    paused_visible: bool,
    bell: Chip,
    bell_ring: Sequence,
    control: Chip,
    panel: String,
}

impl Bar {
    pub fn new(
        engine: &mut TextEngine,
        width: u32,
        height: u32,
        model: &BarModel,
        now: Millis,
    ) -> Self {
        let mut tree = Tree::new();
        let mut input = Input::new();
        let body_width = width as f32 / crate::theme::scale();
        let body_height = height as f32 / crate::theme::scale();
        let mut body = Node::rect(
            0.0,
            0.0,
            body_width,
            body_height,
            RectPaint {
                color: theme::bg(),
                radius: 9.0,
                border_color: theme::line(),
                border_width: 1.0,
            },
        );
        body.scale = crate::theme::scale();
        body.origin = Some((0.0, 0.0));
        body.opacity = 0.0;
        let root = tree.root();
        let body = tree.add(root, body);
        let left_row = tree.add(body, Node::new(5.0, 0.0, 0.0, 22.0));
        let launcher = chip(
            &mut tree,
            &mut input,
            engine,
            left_row,
            "󰣇",
            theme::green(),
            "Super + D",
        );
        let left_separator = separator(&mut tree, left_row);
        let workspaces = Workspaces::new(&mut tree, left_row);
        let max_width = 70.0f32.max(body_width * 0.16);
        let title_text = title_text(model);
        let title = SwapText::new(
            &mut tree,
            engine,
            left_row,
            &title_text,
            "none",
            crate::theme::font(),
            10.0,
            theme::muted(),
            max_width,
        );
        let clock = chip(
            &mut tree,
            &mut input,
            engine,
            body,
            &model.clock,
            theme::text(),
            "",
        );
        let right_row = tree.add(body, Node::new(0.0, 0.0, 0.0, 22.0));
        let resources = Resources::new(&mut tree, &mut input, engine, right_row);
        let keyboard = Keyboard::new(
            &mut tree,
            &mut input,
            engine,
            right_row,
            ui::image::svg_from_data(keyboard_icon().as_bytes(), "keyboard-bar.svg"),
            &model.keyboard_label,
        );
        let network = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            network_icon(model),
            theme::green(),
            "",
        );
        let vpn = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰒙",
            theme::muted(),
            "",
        );
        let bluetooth = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰂯",
            theme::green(),
            "",
        );
        let audio = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰕾",
            theme::text(),
            "XF86AudioRaiseVolume / XF86AudioLowerVolume / XF86AudioMute",
        );
        let battery = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "",
            theme::text(),
            "",
        );
        let right_separator = separator(&mut tree, right_row);
        let paused = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰌐",
            theme::yellow(),
            "hotkeys paused · Super + Escape",
        );
        let bell = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰂚",
            theme::yellow(),
            "Super + N",
        );
        let control = chip(
            &mut tree,
            &mut input,
            engine,
            right_row,
            "󰒓",
            theme::green(),
            "Super + Space",
        );
        let mut enter = Tween::new(0.0, 620.0, Easing::OutCubic);
        enter.start(now, 0.0, 1.0);
        let mut bar = Self {
            tree,
            input,
            body,
            enter,
            width: body_width,
            height: body_height,
            left_row,
            launcher,
            left_separator,
            workspaces,
            title,
            clock,
            right_row,
            tray: Vec::new(),
            resources,
            keyboard,
            network,
            vpn,
            bluetooth,
            audio,
            battery,
            battery_visible: false,
            network_visible: true,
            vpn_visible: false,
            bluetooth_visible: true,
            battery_plugged: None,
            battery_pulse: Sequence::new(
                1.0,
                vec![
                    Step::To {
                        value: 1.18,
                        duration: 120.0,
                        easing: Easing::OutQuad,
                    },
                    Step::Spring {
                        value: 1.0,
                        spring: 4.0,
                        damping: 0.22,
                    },
                ],
            ),
            right_separator,
            paused,
            paused_visible: false,
            bell,
            bell_ring: Sequence::new(
                0.0,
                [
                    (16.0, 70.0),
                    (-12.0, 110.0),
                    (7.0, 100.0),
                    (-3.0, 90.0),
                    (0.0, 80.0),
                ]
                .into_iter()
                .map(|(value, duration)| Step::To {
                    value,
                    duration,
                    easing: Easing::Linear,
                })
                .collect(),
            ),
            control,
            panel: String::new(),
        };
        bar.apply(engine, model, now, true);
        bar
    }

    pub fn apply(&mut self, engine: &mut TextEngine, model: &BarModel, now: Millis, initial: bool) {
        self.panel = model.panel.clone();
        let computed = workspace_ids(services::compositor::kind(), model);
        self.workspaces.apply(
            &mut self.tree,
            &mut self.input,
            engine,
            computed,
            model.active_workspace,
            model.occupied.clone(),
            now,
        );
        self.title.set(&title_text(model), &model.title_key);
        self.title.flush(engine, now);
        self.clock.set_label(engine, &model.clock, 11.0);
        self.resources.apply(model, now);
        self.keyboard.label.set(&model.keyboard_label, "");
        self.keyboard.label.flush(engine, now);

        let set = |chip: &mut Chip, engine: &mut TextEngine, label: &str, ink: Color| {
            chip.set_label(engine, label, 11.0);
            if initial {
                chip.jump_ink(ink);
            } else {
                chip.set_ink(now, ink);
            }
        };
        set(
            &mut self.network,
            engine,
            network_icon(model),
            if model.network_connected {
                theme::green()
            } else {
                theme::muted()
            },
        );
        let (vpn_icon, vpn_ink) = match model.vpn {
            crate::vpnctl::VpnState::On => ("󰕥", theme::green()),
            crate::vpnctl::VpnState::Connecting | crate::vpnctl::VpnState::Disconnecting => {
                ("󰞀", theme::yellow())
            }
            crate::vpnctl::VpnState::Off => ("󰒙", theme::muted()),
        };
        set(&mut self.vpn, engine, vpn_icon, vpn_ink);
        self.vpn.shortcut = model.vpn_tooltip.clone();
        self.vpn_visible = model.available.vpn;
        let bluetooth_icon = if !model.bluetooth_connected.is_empty() {
            "󰂱"
        } else if model.bluetooth_powered {
            "󰂯"
        } else {
            "󰂲"
        };
        set(
            &mut self.bluetooth,
            engine,
            bluetooth_icon,
            if model.bluetooth_powered {
                theme::green()
            } else {
                theme::muted()
            },
        );
        self.bluetooth.shortcut = bluetooth_shortcut(&model.bluetooth_connected);
        let audio_icon = if model.muted {
            "󰖁"
        } else if model.volume > 66 {
            "󰕾"
        } else if model.volume > 33 {
            "󰖀"
        } else {
            "󰕿"
        };
        set(
            &mut self.audio,
            engine,
            audio_icon,
            if model.muted {
                theme::muted()
            } else {
                theme::text()
            },
        );
        let battery_label = format!(
            "{} {}%",
            battery_icon(model.battery_level, model.battery_plugged),
            model.battery_level
        );
        let battery_ink = if model.battery_plugged {
            theme::green()
        } else if model.battery_level < 20 {
            theme::red()
        } else {
            theme::text()
        };
        set(&mut self.battery, engine, &battery_label, battery_ink);
        self.battery_visible = model.battery_present;
        self.network_visible = model.available.wifi || model.network_connected;
        self.bluetooth_visible = model.available.bluetooth;
        self.sync_tray(engine, model, now);
        if let Some(previous) = self.battery_plugged
            && previous != model.battery_plugged
        {
            self.battery_pulse.restart(now);
        }
        self.battery_plugged = Some(model.battery_plugged);
        self.paused_visible = model.hotkeys_paused;
        let bell_icon = if model.dnd {
            "󰂛"
        } else if model.unread_notifications > 0 {
            "󱅫"
        } else {
            "󰂚"
        };
        set(
            &mut self.bell,
            engine,
            bell_icon,
            if model.dnd {
                theme::muted()
            } else {
                theme::yellow()
            },
        );
        self.launcher.selected = self.panel == "launcher";
        self.clock.selected = self.panel == "calendar";
        self.network.selected = self.panel == "wifi";
        self.vpn.selected = self.panel == "vpn";
        self.bluetooth.selected = self.panel == "bluetooth";
        self.audio.selected = self.panel == "audio";
        self.battery.selected = self.panel == "battery";
        self.bell.selected = self.panel == "notifications";
        self.control.selected = self.panel == "control";
        if initial {
            for chip in self.chips_mut() {
                chip.jump_color(&Input::new());
            }
        }
        self.sync(now);
    }

    #[allow(dead_code)]
    pub fn ring_bell(&mut self, now: Millis) {
        self.bell_ring.restart(now);
    }

    fn chips_mut(&mut self) -> [&mut Chip; 10] {
        [
            &mut self.launcher,
            &mut self.clock,
            &mut self.network,
            &mut self.vpn,
            &mut self.bluetooth,
            &mut self.audio,
            &mut self.battery,
            &mut self.paused,
            &mut self.bell,
            &mut self.control,
        ]
    }

    fn sync_tray(&mut self, engine: &mut TextEngine, model: &BarModel, now: Millis) {
        let keys: Vec<&String> = model.tray.iter().map(|(key, _)| key).collect();
        let current: Vec<&String> = self.tray.iter().map(|icon| &icon.key).collect();
        if keys != current {
            let mut old = std::mem::take(&mut self.tray);
            for (key, _) in &model.tray {
                match old.iter().position(|icon| &icon.key == key) {
                    Some(index) => self.tray.push(old.remove(index)),
                    None => self.tray.push(TrayIcon::new(
                        &mut self.tree,
                        &mut self.input,
                        engine,
                        self.right_row,
                        key,
                        now,
                    )),
                }
            }
            for icon in old {
                self.input.remove(icon.rect.area);
                self.tree.remove(icon.rect.root);
            }
        }
        for (icon, (_, pixmap)) in self.tray.iter_mut().zip(&model.tray) {
            icon.set_icon(&mut self.tree, pixmap.clone());
        }
    }

    fn chips(&self) -> [&Chip; 10] {
        [
            &self.launcher,
            &self.clock,
            &self.network,
            &self.vpn,
            &self.bluetooth,
            &self.audio,
            &self.battery,
            &self.paused,
            &self.bell,
            &self.control,
        ]
    }

    fn sync(&mut self, now: Millis) {
        let input = &self.input;
        for chip in [
            &mut self.launcher,
            &mut self.clock,
            &mut self.network,
            &mut self.vpn,
            &mut self.bluetooth,
            &mut self.audio,
            &mut self.battery,
            &mut self.paused,
            &mut self.bell,
            &mut self.control,
        ] {
            chip.sync(now, input);
        }
        self.workspaces.sync(now, input);
        let system = self.panel == "system";
        self.resources.rect.sync(now, input, system);
        self.keyboard.rect.sync(now, input, false);
        for icon in &mut self.tray {
            icon.rect.sync(now, input, false);
        }
    }

    pub fn handle(&mut self, events: &[InputEvent], now: Millis) -> Vec<BarAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Click(area, button) => {
                    if let Some(action) = self.click_action(area, button) {
                        actions.push(action);
                    }
                }
                InputEvent::Wheel(area, delta) if area == self.audio.area => {
                    actions.push(BarAction::Volume(delta));
                }
                InputEvent::Wheel(area, delta) => {
                    if let Some(icon) = self.tray.iter().find(|icon| icon.rect.area == area) {
                        actions.push(BarAction::TrayScroll {
                            key: icon.key.clone(),
                            delta,
                        });
                    }
                }
                _ => {}
            }
        }
        self.sync(now);
        actions
    }

    fn anchor_x(&self, area: AreaId) -> Option<f64> {
        let node = self.input.node(area)?;
        let width = self.tree.get(node)?.width;
        let mut point = ui::tiny_skia::Point::from_xy(width / 2.0, 0.0);
        self.tree.device_transform(node).map_point(&mut point);
        Some(point.x as f64 + (10.0 * crate::theme::scale()).round() as f64)
    }

    fn open(&self, panel: &'static str, area: AreaId) -> Option<BarAction> {
        Some(BarAction::Open {
            panel,
            anchor: self.anchor_x(area),
        })
    }

    fn click_action(&self, area: AreaId, button: MouseButton) -> Option<BarAction> {
        let right = button == MouseButton::Right;
        if area == self.launcher.area && !right {
            return Some(BarAction::Open {
                panel: "launcher",
                anchor: None,
            });
        }
        if let Some(icon) = self.tray.iter().find(|icon| icon.rect.area == area) {
            return Some(BarAction::Tray {
                key: icon.key.clone(),
                right,
                anchor: self.anchor_x(area),
            });
        }
        if let Some(entry) = self
            .workspaces
            .chips
            .iter()
            .find(|entry| entry.chip.area == area)
        {
            return (!right).then_some(BarAction::Workspace(entry.id));
        }
        if area == self.clock.area && !right {
            return self.open("calendar", area);
        }
        if area == self.resources.rect.area {
            return self.open("system", area);
        }
        if area == self.keyboard.rect.area {
            return Some(BarAction::KeyboardNext);
        }
        if area == self.network.area && !right {
            return self.open("wifi", area);
        }
        if area == self.vpn.area {
            return if right {
                Some(BarAction::ToggleVpn)
            } else {
                self.open("vpn", area)
            };
        }
        if area == self.bluetooth.area && !right {
            return self.open("bluetooth", area);
        }
        if area == self.audio.area {
            return if right {
                Some(BarAction::ToggleMute)
            } else {
                self.open("audio", area)
            };
        }
        if area == self.battery.area && !right {
            return self.open("battery", area);
        }
        if area == self.bell.area {
            return if right {
                Some(BarAction::ToggleDnd)
            } else {
                self.open("notifications", area)
            };
        }
        if area == self.control.area && !right {
            return Some(BarAction::Open {
                panel: "control",
                anchor: None,
            });
        }
        None
    }

    pub fn tooltip(&self) -> Option<TooltipTarget> {
        let candidates = self
            .chips()
            .into_iter()
            .chain(self.workspaces.chips.iter().map(|entry| &entry.chip));
        for chip in candidates {
            if chip.shortcut.is_empty() || !chip.hovered(&self.input) {
                continue;
            }
            let transform = self.tree.device_transform(chip.root);
            let mut origin = ui::tiny_skia::Point::from_xy(0.0, 0.0);
            transform.map_point(&mut origin);
            return Some(TooltipTarget {
                key: chip.area,
                text: chip.shortcut.clone(),
                anchor: PopupPlacement {
                    anchor_x: origin.x.floor() as i32,
                    anchor_y: origin.y.floor() as i32,
                    anchor_width: chip.width().ceil() as i32,
                    anchor_height: chip.implicit_height().ceil() as i32,
                    width: 0,
                    height: 0,
                },
            });
        }
        None
    }

    fn layout(&mut self) {
        let tree = &mut self.tree;
        let title_width = self.title.implicit_width();
        let left_items = [
            Item::new(self.launcher.implicit_width(), 22.0),
            Item::new(1.0, 12.0).margins(4.0, 6.0),
            Item::new(self.workspaces.implicit_width(), 22.0),
            Item::preferred(title_width, 16.0).margins(12.0, 0.0),
        ];
        let left = row_layout(&left_items, 3.0);
        tree.set_pos(self.left_row, 5.0, center(self.height, left.height));
        tree.set_size(self.left_row, left.width, left.height);
        let p = &left.positions;
        self.launcher
            .place(tree, p[0].0, p[0].1, left_items[0].layout_width());
        tree.set_pos(self.left_separator, p[1].0, p[1].1);
        tree.set_pos(self.workspaces.root, p[2].0, p[2].1);
        tree.set_size(self.workspaces.root, left_items[2].layout_width(), 22.0);
        tree.set_size(self.workspaces.row, left_items[2].layout_width(), 22.0);
        self.title.place(tree, p[3].0, p[3].1, title_width, 16.0);

        let clock_width = self.clock.implicit_width();
        self.clock.place(
            tree,
            center(self.width, clock_width),
            center(self.height, 22.0),
            clock_width,
        );

        enum Slot {
            Tray(usize),
            Resources,
            Keyboard,
            Network,
            Vpn,
            Bluetooth,
            Audio,
            Battery,
            Separator,
            Paused,
            Bell,
            Control,
        }
        let mut slots: Vec<(Slot, Item)> = (0..self.tray.len())
            .map(|index| (Slot::Tray(index), Item::new(23.0, 22.0)))
            .collect();
        slots.extend([
            (
                Slot::Resources,
                Item::new(self.resources.width, 22.0).margins(2.0, 0.0),
            ),
            (Slot::Keyboard, Item::new(49.0, 22.0)),
        ]);
        if self.network_visible {
            slots.push((
                Slot::Network,
                Item::new(self.network.implicit_width(), 22.0),
            ));
        }
        if self.vpn_visible {
            slots.push((Slot::Vpn, Item::new(self.vpn.implicit_width(), 22.0)));
        }
        if self.bluetooth_visible {
            slots.push((
                Slot::Bluetooth,
                Item::new(self.bluetooth.implicit_width(), 22.0),
            ));
        }
        slots.push((Slot::Audio, Item::new(self.audio.implicit_width(), 22.0)));
        if self.battery_visible {
            slots.push((
                Slot::Battery,
                Item::new(self.battery.implicit_width(), 22.0),
            ));
        }
        slots.push((Slot::Separator, Item::new(1.0, 12.0).margins(5.0, 5.0)));
        if self.paused_visible {
            slots.push((Slot::Paused, Item::new(self.paused.implicit_width(), 22.0)));
        }
        slots.push((Slot::Bell, Item::new(self.bell.implicit_width(), 22.0)));
        slots.push((
            Slot::Control,
            Item::new(self.control.implicit_width(), 22.0),
        ));
        let items: Vec<Item> = slots.iter().map(|(_, item)| *item).collect();
        let right = row_layout(&items, 2.0);
        tree.set_pos(
            self.right_row,
            self.width - 5.0 - right.width,
            center(self.height, right.height),
        );
        tree.set_size(self.right_row, right.width, right.height);
        tree.set_visible(self.battery.root, self.battery_visible);
        tree.set_visible(self.network.root, self.network_visible);
        tree.set_visible(self.vpn.root, self.vpn_visible);
        tree.set_visible(self.bluetooth.root, self.bluetooth_visible);
        tree.set_visible(self.paused.root, self.paused_visible);
        for ((slot, item), (x, y)) in slots.iter().zip(right.positions.iter().copied()) {
            let width = item.layout_width();
            match slot {
                Slot::Tray(index) => tree.set_pos(self.tray[*index].rect.root, x, y),
                Slot::Resources => tree.set_pos(self.resources.rect.root, x, y),
                Slot::Keyboard => tree.set_pos(self.keyboard.rect.root, x, y),
                Slot::Network => self.network.place(tree, x, y, width),
                Slot::Vpn => self.vpn.place(tree, x, y, width),
                Slot::Bluetooth => self.bluetooth.place(tree, x, y, width),
                Slot::Audio => self.audio.place(tree, x, y, width),
                Slot::Battery => self.battery.place(tree, x, y, width),
                Slot::Separator => tree.set_pos(self.right_separator, x, y),
                Slot::Paused => self.paused.place(tree, x, y, width),
                Slot::Bell => self.bell.place(tree, x, y, width),
                Slot::Control => self.control.place(tree, x, y, width),
            }
        }
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        let mut running = self.enter.tick(now);
        let enter = self.enter.value();
        self.tree.set_opacity(self.body, enter);
        self.tree.set_transforms(
            self.body,
            &[TransformOp::Translate {
                x: 0.0,
                y: -12.0 * (1.0 - enter),
            }],
        );
        running |= self.workspaces.tick(&mut self.tree, now);
        running |= self.title.advance(now);
        running |= self.resources.tick(&mut self.tree, now);
        running |= self.keyboard.rect.tick(&mut self.tree, now);
        for icon in &mut self.tray {
            running |= icon.tick(&mut self.tree, now);
        }
        running |= self.keyboard.label.tick(&mut self.tree, now);
        running |= self.battery_pulse.tick(now);
        let pulse = self.battery_pulse.value() as f32;
        let battery_width = self.battery.width();
        self.battery.extra = vec![TransformOp::Scale {
            x: pulse,
            y: pulse,
            origin_x: battery_width / 2.0,
            origin_y: 11.0,
        }];
        running |= self.bell_ring.tick(now);
        let bell_width = self.bell.width();
        self.bell.extra = vec![TransformOp::Rotate {
            angle: self.bell_ring.value() as f32,
            origin_x: bell_width / 2.0,
            origin_y: 3.0,
        }];
        self.layout();
        self.title.apply(&mut self.tree);
        let tree = &mut self.tree;
        for chip in [
            &mut self.launcher,
            &mut self.clock,
            &mut self.network,
            &mut self.vpn,
            &mut self.bluetooth,
            &mut self.audio,
            &mut self.battery,
            &mut self.paused,
            &mut self.bell,
            &mut self.control,
        ] {
            running |= chip.tick(tree, now);
        }
        running
    }
}

fn title_text(model: &BarModel) -> String {
    model
        .title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "still / material".into())
}

fn keyboard_icon() -> String {
    KEYBOARD_ICON
        .replace("#a9b665", &theme::hex(theme::green()))
        .replace("#d4be98", &theme::hex(theme::text()))
}

fn workspace_ids(kind: Option<services::compositor::Kind>, model: &BarModel) -> Vec<i32> {
    match kind {
        Some(kind) if kind.fixed_workspaces() => {
            let fixed = config::current().bar.workspaces as i32;
            let mut computed: Vec<i32> = (1..=fixed).collect();
            let mut extra: Vec<i32> = model
                .workspaces
                .iter()
                .copied()
                .filter(|id| *id > fixed)
                .collect();
            extra.sort_unstable();
            extra.dedup();
            computed.extend(extra);
            computed
        }
        Some(_) => {
            let mut ids = model.workspaces.clone();
            ids.sort_unstable();
            ids.dedup();
            ids
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::font_families;

    #[test]
    fn launcher_tooltip_anchor_matches_qt_positioner() {
        let mut engine = TextEngine::new(&font_families());
        let model = BarModel::placeholder();
        let mut bar = Bar::new(&mut engine, 1896, 35, &model, 0.0);
        bar.tick(1000.0);
        let events = bar.input.motion(&bar.tree, 20.0, 15.0);
        bar.handle(&events, 1000.0);
        let target = bar.tooltip().expect("launcher hovered");
        assert_eq!(target.text, "Super + D");
        assert_eq!(
            (
                target.anchor.anchor_x,
                target.anchor.anchor_y,
                target.anchor.anchor_width,
                target.anchor.anchor_height
            ),
            (6, 3, 23, 22)
        );
    }

    #[test]
    fn clock_has_no_tooltip() {
        let mut engine = TextEngine::new(&font_families());
        let model = BarModel::placeholder();
        let mut bar = Bar::new(&mut engine, 1896, 35, &model, 0.0);
        bar.tick(1000.0);
        let events = bar.input.motion(&bar.tree, 948.0, 17.0);
        bar.handle(&events, 1000.0);
        assert!(bar.tooltip().is_none());
    }
}
