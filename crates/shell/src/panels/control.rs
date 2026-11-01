use platform::MouseButton;
use ui::{
    Color, Node, NodeId, RectPaint, TextEngine, TextSpec, Tree, Weight,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Buttons, Input, InputEvent, MouseArea},
    layout::{Cell, Slot, center, linear},
    widgets::{Chip, ChipStyle, Label, Slider, SliderStyle},
};

use super::{PanelAction, PanelData, TooltipSpot};
use crate::theme::{
    ELEVATED, FAST, FONT, GREEN, ICONS, LINE, MUTED, NORMAL, RED, SURFACE, TEXT, YELLOW,
};

const CHIP_STYLE: ChipStyle = ChipStyle {
    family: ICONS,
    hover: ELEVATED,
};

pub(crate) const SLIDER_STYLE: SliderStyle = SliderStyle {
    track: ELEVATED,
    handle: TEXT,
};

fn text(value: &str, family: &str, size: f32, color: Color) -> TextSpec {
    TextSpec::plain(value, family, size, color)
}

fn label(tree: &mut Tree, engine: &mut TextEngine, parent: NodeId, spec: TextSpec) -> Label {
    Label::new(tree, engine, parent, spec)
}

fn card(tree: &mut Tree, parent: NodeId, color: Color, radius: f32) -> NodeId {
    tree.add(
        parent,
        Node::rect(
            0.0,
            0.0,
            0.0,
            0.0,
            RectPaint {
                color,
                radius,
                ..RectPaint::default()
            },
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn chip(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    value: &str,
    ink: Color,
    size: Option<f32>,
    shortcut: &str,
) -> Chip {
    let mut chip = Chip::new(
        tree,
        input,
        engine,
        parent,
        &CHIP_STYLE,
        value,
        ink,
        false,
        None,
    );
    if let Some(size) = size {
        chip.set_font_size(engine, size, size);
    }
    chip.shortcut = shortcut.to_string();
    chip
}

fn fit(slot: Slot, size: f32) -> (f32, f32) {
    let size = size.min(slot.size);
    (slot.pos + center(slot.size, size), size)
}

fn place_label(tree: &mut Tree, label: &mut Label, x: f32, slot: Slot, width: f32) {
    let (y, height) = fit(slot, label.implicit_height());
    label.place(tree, x, y, width, height);
}

fn place_chip(tree: &mut Tree, chip: &mut Chip, x: f32, slot: Slot) {
    let (y, height) = fit(slot, chip.implicit_height().ceil());
    let width = chip.implicit_width().ceil();
    chip.place_sized(tree, x, y, width, height);
}

struct Tile {
    root: NodeId,
    area: AreaId,
    icon: Label,
    state: Label,
    name: Label,
    status: Label,
    wifi: bool,
    active: bool,
    color: Tween<Color>,
    border: Tween<Color>,
    scale: Tween<f32>,
    width: f32,
}

impl Tile {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        wifi: bool,
    ) -> Self {
        let root = tree.add(
            parent,
            Node::rect(
                0.0,
                0.0,
                0.0,
                95.0,
                RectPaint {
                    color: SURFACE,
                    radius: 16.0,
                    border_color: LINE,
                    border_width: 0.7,
                },
            ),
        );
        let (glyph, ink) = if wifi {
            ("󰤨", GREEN)
        } else {
            ("󰂯", YELLOW)
        };
        let icon = label(tree, engine, root, text(glyph, ICONS, 22.0, ink));
        let state = label(tree, engine, root, text("", FONT, 12.0, MUTED));
        let name = label(tree, engine, root, text("", FONT, 14.0, TEXT));
        let status = label(tree, engine, root, text("", FONT, 10.0, MUTED));
        let mut area = MouseArea::new(root);
        area.buttons = Buttons::LEFT_RIGHT;
        let area = input.add(area);
        Self {
            root,
            area,
            icon,
            state,
            name,
            status,
            wifi,
            active: false,
            color: Tween::new(SURFACE, FAST, Easing::Linear),
            border: Tween::new(LINE, NORMAL, Easing::Linear),
            scale: Tween::new(1.0, 110.0, Easing::OutCubic),
            width: 0.0,
        }
    }

    fn sync(&mut self, engine: &mut TextEngine, data: &PanelData) {
        if self.wifi {
            self.active = data.network_connected;
            self.state.set_text(engine, "↗");
            self.name.set_text(engine, &data.network_name);
            self.status.set_text(
                engine,
                if self.active {
                    "connected"
                } else {
                    "disconnected"
                },
            );
        } else {
            self.active = data.bluetooth_powered;
            self.state
                .set_text(engine, if self.active { "on" } else { "off" });
            self.name.set_text(engine, "bluetooth");
            let name = if data.bluetooth_name.is_empty() {
                "no adapter found"
            } else {
                data.bluetooth_name.as_str()
            };
            self.status.set_text(engine, name);
        }
    }

    fn border_target(&self) -> Color {
        match (self.active, self.wifi) {
            (false, _) => LINE,
            (true, true) => GREEN,
            (true, false) => YELLOW,
        }
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32, width: f32, height: f32) {
        self.width = width;
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, width, height);
        if let Some(node) = tree.get_mut(self.root) {
            node.origin = Some((width / 2.0, height / 2.0));
        }
        let inner_width = width - 30.0;
        let row_height = self
            .icon
            .implicit_height()
            .ceil()
            .max(self.state.implicit_height().ceil());
        let rows = linear(
            &[
                Cell::layout(row_height),
                Cell::fill(0.0),
                Cell::implicit(self.name.implicit_height()),
                Cell::implicit(self.status.implicit_height()),
            ],
            4.0,
            height - 30.0,
        );
        let offset = |slot: Slot| Slot {
            pos: slot.pos + 15.0,
            size: slot.size,
        };
        let row = offset(rows[0]);
        let row = Slot {
            pos: row.pos,
            size: row.size.max(row_height),
        };
        let columns = linear(
            &[
                Cell::fill(self.icon.implicit_width().ceil()),
                Cell::implicit(self.state.implicit_width()),
            ],
            5.0,
            inner_width,
        );
        place_label(
            tree,
            &mut self.icon,
            15.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.state,
            15.0 + columns[1].pos,
            row,
            columns[1].size,
        );
        place_label(tree, &mut self.name, 15.0, offset(rows[2]), inner_width);
        place_label(tree, &mut self.status, 15.0, offset(rows[3]), inner_width);
    }

    fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let hovered = input.contains_mouse(self.area);
        self.color
            .set_target(now, if hovered { ELEVATED } else { SURFACE });
        self.border.set_target(now, self.border_target());
        self.scale
            .set_target(now, if input.pressed(self.area) { 0.97 } else { 1.0 });
        let mut running = self.color.tick(now);
        running |= self.border.tick(now);
        running |= self.scale.tick(now);
        tree.set_scale(self.root, self.scale.value());
        let color = self.color.value();
        let border = self.border.value();
        let changed = tree.get(self.root).is_some_and(|node| {
            matches!(&node.paint, ui::Paint::Rect(rect) if rect.color != color || rect.border_color != border)
        });
        if changed
            && let Some(node) = tree.get_mut(self.root)
            && let ui::Paint::Rect(rect) = &mut node.paint
        {
            rect.color = color;
            rect.border_color = border;
        }
        for label in [&self.icon, &self.state, &self.name, &self.status] {
            label.sync(tree);
        }
        running
    }

    fn jump(&mut self, input: &Input) {
        let hovered = input.contains_mouse(self.area);
        self.color.jump(if hovered { ELEVATED } else { SURFACE });
        self.border.jump(self.border_target());
    }
}

enum Footer {
    Toggle(&'static str),
    Run(&'static str),
    Suspend,
}

pub struct ControlPanel {
    pub root: NodeId,
    header: Label,
    time: Label,
    lock: Chip,
    date: Label,
    tiles: [Tile; 2],
    volume_card: NodeId,
    volume_icon: Label,
    volume_label: Label,
    volume_chip: Chip,
    volume_slider: Slider,
    bright_card: NodeId,
    bright_icon: Label,
    bright_label: Label,
    bright_value: Label,
    bright_slider: Slider,
    awake_row: NodeId,
    awake_color: Tween<Color>,
    awake_icon: Label,
    awake_label: Label,
    awake_chip: Chip,
    line: NodeId,
    footer: Vec<(Chip, Footer)>,
    data: PanelData,
    width: f32,
    height: f32,
}

impl ControlPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let mut header_spec = text("desktop / control", ICONS, 9.0, GREEN);
        if let Some(span) = header_spec.spans.first_mut() {
            span.letter_spacing = 1.0;
        }
        let header = label(tree, engine, root, header_spec);
        let mut time_spec = text("", FONT, 40.0, TEXT);
        if let Some(span) = time_spec.spans.first_mut() {
            span.weight = Weight::Light;
        }
        let time = label(tree, engine, root, time_spec);
        let lock = chip(
            tree,
            input,
            engine,
            root,
            "󰌾",
            YELLOW,
            Some(22.0),
            "Super + Ctrl + L",
        );
        let date = label(tree, engine, root, text("", FONT, 12.0, MUTED));
        let tiles = [
            Tile::new(tree, input, engine, root, true),
            Tile::new(tree, input, engine, root, false),
        ];
        let volume_card = card(tree, root, SURFACE, 16.0);
        let volume_icon = label(tree, engine, volume_card, text("󰕾", ICONS, 18.0, GREEN));
        let volume_label = label(tree, engine, volume_card, text("volume", FONT, 12.0, TEXT));
        let volume_chip = chip(
            tree,
            input,
            engine,
            volume_card,
            "",
            GREEN,
            None,
            "XF86AudioMute",
        );
        let mut volume_slider =
            Slider::new(tree, input, volume_card, &SLIDER_STYLE, GREEN, 0.0, 100.0);
        volume_slider.shortcut = "XF86AudioRaiseVolume / XF86AudioLowerVolume".into();
        let bright_card = card(tree, root, SURFACE, 16.0);
        let bright_icon = label(tree, engine, bright_card, text("󰃟", ICONS, 18.0, YELLOW));
        let bright_label = label(
            tree,
            engine,
            bright_card,
            text("brightness", FONT, 12.0, TEXT),
        );
        let bright_value = label(tree, engine, bright_card, text("", ICONS, 12.0, YELLOW));
        let mut bright_slider =
            Slider::new(tree, input, bright_card, &SLIDER_STYLE, YELLOW, 5.0, 100.0);
        bright_slider.shortcut = "XF86MonBrightnessUp / XF86MonBrightnessDown".into();
        let awake_row = card(tree, root, SURFACE, 13.0);
        let awake_icon = label(tree, engine, awake_row, text("󰅶", ICONS, 18.0, MUTED));
        let awake_label = label(
            tree,
            engine,
            awake_row,
            text("keep awake", FONT, 12.0, TEXT),
        );
        let awake_chip = chip(
            tree,
            input,
            engine,
            awake_row,
            "off",
            MUTED,
            None,
            "prevent automatic locking and screen sleep",
        );
        let line = card(tree, root, LINE, 0.0);
        let footer_spec: [(&str, Color, &str, Footer); 6] = [
            ("󰂚", TEXT, "Super + N", Footer::Toggle("notifications")),
            ("󰅍", TEXT, "Super + C", Footer::Toggle("clipboard")),
            ("󰄀", TEXT, "Print", Footer::Run("screenshot")),
            ("󰸉", GREEN, "Super + B", Footer::Toggle("wallpapers")),
            ("󰒲", TEXT, "", Footer::Suspend),
            ("󰐥", RED, "", Footer::Toggle("power")),
        ];
        let footer = footer_spec
            .into_iter()
            .map(|(glyph, ink, shortcut, action)| {
                (
                    chip(tree, input, engine, root, glyph, ink, Some(18.0), shortcut),
                    action,
                )
            })
            .collect();
        let mut panel = Self {
            root,
            header,
            time,
            lock,
            date,
            tiles,
            volume_card,
            volume_icon,
            volume_label,
            volume_chip,
            volume_slider,
            bright_card,
            bright_icon,
            bright_label,
            bright_value,
            bright_slider,
            awake_row,
            awake_color: Tween::new(SURFACE, NORMAL, Easing::Linear),
            awake_icon,
            awake_label,
            awake_chip,
            line,
            footer,
            data: PanelData::default(),
            width: 0.0,
            height: 0.0,
        };
        panel.sync(engine, data, 0.0);
        panel.jump(input);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.lock.area,
            self.tiles[0].area,
            self.tiles[1].area,
            self.volume_chip.area,
            self.volume_slider.area,
            self.bright_slider.area,
            self.awake_chip.area,
        ];
        areas.extend(self.footer.iter().map(|(chip, _)| chip.area));
        areas
    }

    pub fn sync(&mut self, engine: &mut TextEngine, data: &PanelData, now: Millis) {
        self.data = data.clone();
        self.time.set_text(engine, &data.time);
        self.date.set_text(engine, &data.date);
        for tile in &mut self.tiles {
            tile.sync(engine, data);
        }
        let volume = if data.muted {
            "mute".to_string()
        } else {
            format!("{}%", data.volume)
        };
        self.volume_chip.set_label(engine, &volume, 12.0);
        self.volume_slider.set_value(data.volume as f64);
        self.bright_value
            .set_text(engine, &format!("{}%", data.brightness));
        self.bright_slider.set_value(data.brightness as f64);
        self.awake_icon
            .set_color(if data.keep_awake { GREEN } else { MUTED });
        self.awake_chip
            .set_label(engine, if data.keep_awake { "on" } else { "off" }, 12.0);
        self.awake_chip
            .set_ink(now, if data.keep_awake { GREEN } else { MUTED });
        self.awake_chip.selected = data.keep_awake;
        self.awake_color
            .set_target(now, if data.keep_awake { ELEVATED } else { SURFACE });
    }

    fn jump(&mut self, input: &Input) {
        for tile in &mut self.tiles {
            tile.jump(input);
        }
        self.awake_color.jump(if self.data.keep_awake {
            ELEVATED
        } else {
            SURFACE
        });
        self.awake_chip
            .jump_ink(if self.data.keep_awake { GREEN } else { MUTED });
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        self.width = width;
        self.height = height;
        tree.set_size(self.root, width, height);
        let row_height = self
            .time
            .implicit_height()
            .ceil()
            .max(self.lock.implicit_height());
        let slots = linear(
            &[
                Cell::implicit(self.header.implicit_height()),
                Cell::layout(row_height).margins(8.0, 0.0),
                Cell::implicit(self.date.implicit_height()).margins(4.0, 0.0),
                Cell::layout(95.0).margins(17.0, 0.0),
                Cell::preferred(89.0).margins(18.0, 0.0),
                Cell::preferred(89.0).margins(10.0, 0.0),
                Cell::preferred(44.0).margins(15.0, 0.0),
                Cell::fill(0.0),
                Cell::preferred(1.0).margins(0.0, 12.0),
                Cell::layout(30.0),
            ],
            0.0,
            height,
        );
        let header_width = self.header.implicit_width().ceil().min(width);
        place_label(tree, &mut self.header, 0.0, slots[0], header_width);
        let top = linear(
            &[
                Cell::fill(self.time.implicit_width().ceil()),
                Cell::implicit(self.lock.implicit_width()),
            ],
            5.0,
            width,
        );
        place_label(tree, &mut self.time, top[0].pos, slots[1], top[0].size);
        place_chip(tree, &mut self.lock, top[1].pos, slots[1]);
        let date_width = self.date.implicit_width().ceil().min(width);
        place_label(tree, &mut self.date, 0.0, slots[2], date_width);
        let tiles = linear(&[Cell::fill(0.0), Cell::fill(0.0)], 10.0, width);
        let (tile_y, tile_height) = fit(slots[3], 95.0);
        for (tile, slot) in self.tiles.iter_mut().zip(&tiles) {
            tile.place(tree, slot.pos, tile_y, slot.size, tile_height);
        }
        let (volume_y, volume_height) = fit(slots[4], 89.0);
        tree.set_pos(self.volume_card, 0.0, volume_y);
        tree.set_size(self.volume_card, width, volume_height);
        self.layout_volume(tree, width, volume_height);
        let (bright_y, bright_height) = fit(slots[5], 89.0);
        tree.set_pos(self.bright_card, 0.0, bright_y);
        tree.set_size(self.bright_card, width, bright_height);
        self.layout_brightness(tree, width, bright_height);
        let (awake_y, awake_height) = fit(slots[6], 44.0);
        tree.set_pos(self.awake_row, 0.0, awake_y);
        tree.set_size(self.awake_row, width, awake_height);
        self.layout_awake(tree, width, awake_height);
        let (line_y, line_height) = fit(slots[8], 1.0);
        tree.set_pos(self.line, 0.0, line_y);
        tree.set_size(self.line, width, line_height);
        let mut cells: Vec<Cell> = self.footer[..4]
            .iter()
            .map(|(chip, _)| Cell::implicit(chip.implicit_width()))
            .collect();
        cells.push(Cell::fill(0.0));
        cells.extend(
            self.footer[4..]
                .iter()
                .map(|(chip, _)| Cell::implicit(chip.implicit_width())),
        );
        let columns = linear(&cells, 7.0, width);
        let positions: Vec<f32> = columns
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 4)
            .map(|(_, slot)| slot.pos)
            .collect();
        for ((chip, _), x) in self.footer.iter_mut().zip(positions) {
            place_chip(tree, chip, x, slots[9]);
        }
    }

    fn inner_rows(row_height: f32, height: f32) -> (Slot, Slot) {
        let rows = linear(
            &[Cell::layout(row_height), Cell::implicit(32.0)],
            5.0,
            height - 24.0,
        );
        let shift = |slot: Slot| Slot {
            pos: slot.pos + 12.0,
            size: slot.size,
        };
        (shift(rows[0]), shift(rows[1]))
    }

    fn layout_volume(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let row_height = self
            .volume_icon
            .implicit_height()
            .ceil()
            .max(self.volume_label.implicit_height().ceil())
            .max(self.volume_chip.implicit_height());
        let (row, slider) = Self::inner_rows(row_height, height);
        let inner = width - 24.0;
        let columns = linear(
            &[
                Cell::implicit(self.volume_icon.implicit_width()),
                Cell::fill(self.volume_label.implicit_width().ceil()),
                Cell::implicit(self.volume_chip.implicit_width()),
            ],
            5.0,
            inner,
        );
        let inside = Slot {
            pos: row.pos,
            size: row.size.max(row_height),
        };
        place_label(
            tree,
            &mut self.volume_icon,
            12.0 + columns[0].pos,
            inside,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.volume_label,
            12.0 + columns[1].pos,
            inside,
            columns[1].size,
        );
        place_chip(tree, &mut self.volume_chip, 12.0 + columns[2].pos, inside);
        let (slider_y, slider_height) = fit(slider, 32.0);
        self.volume_slider
            .place(tree, 12.0, slider_y, inner, slider_height);
    }

    fn layout_brightness(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let row_height = self
            .bright_icon
            .implicit_height()
            .ceil()
            .max(self.bright_label.implicit_height().ceil())
            .max(self.bright_value.implicit_height().ceil());
        let (row, slider) = Self::inner_rows(row_height, height);
        let inner = width - 24.0;
        let columns = linear(
            &[
                Cell::implicit(self.bright_icon.implicit_width()),
                Cell::fill(self.bright_label.implicit_width().ceil()),
                Cell::implicit(self.bright_value.implicit_width()),
            ],
            5.0,
            inner,
        );
        let inside = Slot {
            pos: row.pos,
            size: row.size.max(row_height),
        };
        place_label(
            tree,
            &mut self.bright_icon,
            12.0 + columns[0].pos,
            inside,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.bright_label,
            12.0 + columns[1].pos,
            inside,
            columns[1].size,
        );
        place_label(
            tree,
            &mut self.bright_value,
            12.0 + columns[2].pos,
            inside,
            columns[2].size,
        );
        let (slider_y, slider_height) = fit(slider, 32.0);
        self.bright_slider
            .place(tree, 12.0, slider_y, inner, slider_height);
    }

    fn layout_awake(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let tallest = self
            .awake_icon
            .implicit_height()
            .ceil()
            .max(self.awake_label.implicit_height().ceil())
            .max(self.awake_chip.implicit_height());
        let inner = Slot {
            pos: 8.0,
            size: (height - 16.0).max(tallest),
        };
        let columns = linear(
            &[
                Cell::implicit(self.awake_icon.implicit_width()),
                Cell::fill(self.awake_label.implicit_width().ceil()),
                Cell::implicit(self.awake_chip.implicit_width()),
            ],
            10.0,
            width - 16.0,
        );
        place_label(
            tree,
            &mut self.awake_icon,
            8.0 + columns[0].pos,
            inner,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.awake_label,
            8.0 + columns[1].pos,
            inner,
            columns[1].size,
        );
        place_chip(tree, &mut self.awake_chip, 8.0 + columns[2].pos, inner);
    }

    fn chips(&self) -> Vec<&Chip> {
        let mut chips = vec![&self.lock, &self.volume_chip, &self.awake_chip];
        chips.extend(self.footer.iter().map(|(chip, _)| chip));
        chips
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for tile in &mut self.tiles {
            running |= tile.tick(tree, input, now);
        }
        let mut chips: Vec<&mut Chip> =
            vec![&mut self.lock, &mut self.volume_chip, &mut self.awake_chip];
        chips.extend(self.footer.iter_mut().map(|(chip, _)| chip));
        for chip in chips {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.volume_slider.tick(tree, input, now);
        running |= self.bright_slider.tick(tree, input, now);
        running |= self.awake_color.tick(now);
        tree.set_fill(self.awake_row, self.awake_color.value());
        for label in [
            &self.header,
            &self.time,
            &self.date,
            &self.volume_icon,
            &self.volume_label,
            &self.bright_icon,
            &self.bright_label,
            &self.bright_value,
            &self.awake_icon,
            &self.awake_label,
        ] {
            label.sync(tree);
        }
        running
    }

    pub fn settle(&mut self, tree: &mut Tree, input: &Input) {
        self.volume_slider.jump(tree, input);
        self.bright_slider.jump(tree, input);
        let mut chips: Vec<&mut Chip> =
            vec![&mut self.lock, &mut self.volume_chip, &mut self.awake_chip];
        chips.extend(self.footer.iter_mut().map(|(chip, _)| chip));
        for chip in chips {
            chip.jump_color(input);
        }
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
                InputEvent::Press(area, MouseButton::Left) => {
                    if area == self.volume_slider.area {
                        self.volume_slider.press(tree, pointer.0, pointer.1);
                    } else if area == self.bright_slider.area {
                        self.bright_slider.press(tree, pointer.0, pointer.1);
                    }
                }
                InputEvent::Release(area, MouseButton::Left) => {
                    if area == self.volume_slider.area
                        && let Some(value) = self.volume_slider.release(tree, pointer.0, pointer.1)
                    {
                        actions.push(PanelAction::SetVolume(value));
                    } else if area == self.bright_slider.area
                        && let Some(value) = self.bright_slider.release(tree, pointer.0, pointer.1)
                    {
                        actions.push(PanelAction::SetBrightness(value));
                    }
                }
                InputEvent::Click(area, button) => {
                    if let Some(action) = self.click(area, button) {
                        actions.push(action);
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn motion(&mut self, tree: &Tree, input: &Input, x: f32, y: f32) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        if input.pressed(self.volume_slider.area)
            && let Some(value) = self.volume_slider.motion(tree, x, y)
        {
            actions.push(PanelAction::SetVolume(value));
        }
        if input.pressed(self.bright_slider.area)
            && let Some(value) = self.bright_slider.motion(tree, x, y)
        {
            actions.push(PanelAction::SetBrightness(value));
        }
        actions
    }

    fn click(&self, area: AreaId, button: MouseButton) -> Option<PanelAction> {
        let right = button == MouseButton::Right;
        if area == self.lock.area {
            return (!right).then_some(PanelAction::Run("lock"));
        }
        if area == self.tiles[0].area {
            return Some(PanelAction::Toggle("wifi"));
        }
        if area == self.tiles[1].area {
            return Some(if right {
                PanelAction::BluetoothPower
            } else {
                PanelAction::Toggle("bluetooth")
            });
        }
        if area == self.volume_chip.area {
            return Some(if right {
                PanelAction::Toggle("audio")
            } else {
                PanelAction::ToggleMute
            });
        }
        if area == self.awake_chip.area {
            return (!right).then_some(PanelAction::ToggleKeepAwake);
        }
        let (_, action) = self.footer.iter().find(|(chip, _)| chip.area == area)?;
        if right {
            return None;
        }
        Some(match action {
            Footer::Toggle(name) => PanelAction::Toggle(name),
            Footer::Run(name) => PanelAction::Run(name),
            Footer::Suspend => PanelAction::Suspend,
        })
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        for chip in self.chips() {
            if !chip.shortcut.is_empty() && chip.hovered(input) {
                return Some(TooltipSpot {
                    key: chip.area,
                    text: chip.shortcut.clone(),
                    node: chip.root,
                    width: chip.width(),
                    height: chip.height(),
                });
            }
        }
        for slider in [&self.volume_slider, &self.bright_slider] {
            if !slider.shortcut.is_empty() && input.contains_mouse(slider.area) {
                return Some(TooltipSpot {
                    key: slider.area,
                    text: slider.shortcut.clone(),
                    node: slider.root,
                    width: slider.width(),
                    height: slider.height(),
                });
            }
        }
        None
    }
}
