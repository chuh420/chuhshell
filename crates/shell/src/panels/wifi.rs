use platform::{KeyEvent, MouseButton};
use services::network::WifiNetwork;
use ui::{
    Color, Node, NodeId, TextEngine, Tree,
    anim::Millis,
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, Slot, center, linear},
    textfield::{KeyResult, TextField},
    widgets::{Chip, Label, Sparkline},
};

use super::{
    NetAction, PanelAction, PanelData, TooltipSpot,
    kit::{Scroller, chip, fit, icon_label, label, place_chip, place_label, rect, set_border},
};
use crate::theme::{self};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Available,
    Saved,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Ssid,
    Password,
}

pub fn speed(bytes: Option<f64>) -> String {
    let Some(bytes) = bytes else {
        return "…".into();
    };
    if bytes >= 1048576.0 {
        format!("{:.1} MiB/s", bytes / 1048576.0)
    } else if bytes >= 1024.0 {
        format!("{} KiB/s", (bytes / 1024.0 + 0.5).floor())
    } else {
        format!("{} B/s", (bytes + 0.5).floor())
    }
}

fn quality(rssi: i64) -> &'static str {
    if rssi >= -55 {
        "excellent"
    } else if rssi >= -67 {
        "good"
    } else if rssi >= -75 {
        "fair"
    } else {
        "weak"
    }
}

fn network_icon(network: &WifiNetwork) -> &'static str {
    if network.strength > 65.0 {
        "󰤨"
    } else if network.strength > 35.0 {
        "󰤥"
    } else if network.available {
        "󰤟"
    } else {
        "󰤮"
    }
}

fn network_sub(network: &WifiNetwork) -> String {
    if network.connected {
        "connected · click to disconnect".into()
    } else if !network.available {
        format!(
            "{}out of range",
            if network.hidden { "hidden · " } else { "" }
        )
    } else if network.security == "open" {
        "open network".into()
    } else {
        format!(
            "{}{}",
            if network.saved { "saved · " } else { "" },
            network.security.to_uppercase()
        )
    }
}

struct Row {
    data: WifiNetwork,
    root: NodeId,
    hit: NodeId,
    area: AreaId,
    icon: Label,
    name: Label,
    sub: Label,
    auto: Chip,
    forget: Chip,
}

struct InputBox {
    frame: NodeId,
    field: TextField,
    placeholder: Option<Label>,
}

pub struct WifiPanel {
    pub root: NodeId,
    title: Label,
    airplane: Chip,
    power: Chip,
    status: Label,
    card: NodeId,
    card_icon: Label,
    band: Label,
    rssi: Label,
    ip: Label,
    link: Label,
    spark: Sparkline,
    down: Label,
    up: Label,
    available: Chip,
    saved: Chip,
    hidden: Chip,
    scan: Chip,
    hidden_box: InputBox,
    hidden_cancel: Chip,
    hidden_connect: Chip,
    list: Scroller,
    rows: Vec<Row>,
    empty: Label,
    prompt_label: Label,
    prompt_box: InputBox,
    prompt_cancel: Chip,
    prompt_connect: Chip,
    message: Label,
    section: Section,
    hidden_open: bool,
    focus: Option<Focus>,
    data: PanelData,
    prompt: String,
}

fn input_box(
    tree: &mut Tree,
    input: &mut Input,
    engine: &mut TextEngine,
    parent: NodeId,
    password: bool,
    placeholder: Option<&str>,
) -> InputBox {
    let frame = rect(tree, parent, theme::surface(), 8.0);
    set_border(tree, frame, theme::green(), 1.0);
    let placeholder =
        placeholder.map(|text| label(tree, engine, frame, text, 12.0, theme::muted()));
    let field = TextField::new(
        tree,
        input,
        engine,
        frame,
        crate::theme::font(),
        12.0,
        theme::text(),
        password,
    );
    InputBox {
        frame,
        field,
        placeholder,
    }
}

impl InputBox {
    fn place(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        x: f32,
        y: f32,
        width: f32,
        now_empty: bool,
    ) {
        tree.set_pos(self.frame, x, y);
        tree.set_size(self.frame, width, 38.0);
        self.field
            .place(tree, engine, 10.0, 10.0, width - 20.0, 18.0);
        if let Some(placeholder) = self.placeholder.as_mut() {
            tree.set_visible(placeholder.node, now_empty);
            let h = placeholder.implicit_height().ceil();
            placeholder.place(tree, 10.0, 10.0, width - 20.0, h);
        }
    }
}

impl WifiPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "Wi-Fi", 18.0, theme::text());
        let mut airplane = chip(tree, input, engine, root, "󰀝", theme::muted(), false, None);
        airplane.set_font_size(engine, 15.0, 15.0);
        let power = chip(
            tree,
            input,
            engine,
            root,
            "off",
            theme::muted(),
            false,
            None,
        );
        let status = label(tree, engine, root, "", 12.0, theme::muted());
        let card = rect(tree, root, theme::surface(), 14.0);
        let card_icon = icon_label(tree, engine, card, "󰤨", 16.0, theme::green());
        let band = label(tree, engine, card, "", 12.0, theme::text());
        let rssi = label(tree, engine, card, "", 11.0, theme::green());
        let ip = label(tree, engine, card, "", 10.0, theme::muted());
        let link = label(tree, engine, card, "", 10.0, theme::muted());
        let spark = Sparkline::new(tree, card, theme::green(), theme::blue(), theme::line());
        let down = icon_label(tree, engine, card, "", 10.0, theme::green());
        let up = icon_label(tree, engine, card, "", 10.0, theme::blue());
        let available = chip(
            tree,
            input,
            engine,
            root,
            "available",
            theme::text(),
            false,
            None,
        );
        let saved = chip(
            tree,
            input,
            engine,
            root,
            "saved",
            theme::text(),
            false,
            None,
        );
        let hidden = chip(
            tree,
            input,
            engine,
            root,
            "+ hidden",
            theme::text(),
            false,
            None,
        );
        let scan = chip(tree, input, engine, root, "󰑐", theme::green(), false, None);
        let hidden_box = input_box(
            tree,
            input,
            engine,
            root,
            false,
            Some("hidden network name (SSID)"),
        );
        let hidden_cancel = chip(
            tree,
            input,
            engine,
            root,
            "cancel",
            theme::text(),
            false,
            None,
        );
        let hidden_connect = chip(
            tree,
            input,
            engine,
            root,
            "connect",
            theme::green(),
            false,
            None,
        );
        let list = Scroller::new(tree, input, root, false);
        let empty = label(tree, engine, list.view, "", 12.0, theme::muted());
        let prompt_label = label(tree, engine, root, "", 12.0, theme::text());
        let prompt_box = input_box(tree, input, engine, root, true, None);
        let prompt_cancel = chip(
            tree,
            input,
            engine,
            root,
            "cancel",
            theme::text(),
            false,
            None,
        );
        let prompt_connect = chip(
            tree,
            input,
            engine,
            root,
            "connect",
            theme::green(),
            false,
            None,
        );
        let mut message = label(tree, engine, root, "", 11.0, theme::red());
        message.set_color(theme::red());
        let mut panel = Self {
            root,
            title,
            airplane,
            power,
            status,
            card,
            card_icon,
            band,
            rssi,
            ip,
            link,
            spark,
            down,
            up,
            available,
            saved,
            hidden,
            scan,
            hidden_box,
            hidden_cancel,
            hidden_connect,
            list,
            rows: Vec::new(),
            empty,
            prompt_label,
            prompt_box,
            prompt_cancel,
            prompt_connect,
            message,
            section: Section::Available,
            hidden_open: false,
            focus: None,
            data: PanelData::default(),
            prompt: String::new(),
        };
        panel.sync(tree, input, engine, data, now);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.airplane.area,
            self.power.area,
            self.available.area,
            self.saved.area,
            self.hidden.area,
            self.scan.area,
            self.hidden_box.field.area,
            self.hidden_cancel.area,
            self.hidden_connect.area,
            self.prompt_box.field.area,
            self.prompt_cancel.area,
            self.prompt_connect.area,
        ];
        areas.extend(self.list.areas());
        for row in &self.rows {
            areas.extend([row.area, row.auto.area, row.forget.area]);
        }
        areas
    }

    fn set_focus(&mut self, focus: Option<Focus>, now: Millis) {
        self.focus = focus;
        self.hidden_box
            .field
            .set_focus(focus == Some(Focus::Ssid), now);
        self.prompt_box
            .field
            .set_focus(focus == Some(Focus::Password), now);
    }

    fn filtered(&self) -> Vec<WifiNetwork> {
        self.data
            .wifi
            .networks
            .iter()
            .filter(|network| match self.section {
                Section::Saved => network.saved,
                Section::Available => network.available,
            })
            .cloned()
            .collect()
    }

    fn rebuild(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine) {
        let wanted = self.filtered();
        let same = self.rows.len() == wanted.len()
            && self
                .rows
                .iter()
                .zip(&wanted)
                .all(|(row, network)| row.data == *network);
        if same {
            return;
        }
        for row in self.rows.drain(..) {
            for area in [row.area, row.auto.area, row.forget.area] {
                input.remove(area);
            }
            tree.remove(row.root);
        }
        for network in wanted {
            let root = rect(tree, self.list.content, Color::TRANSPARENT, 10.0);
            let hit = tree.add(root, Node::new(0.0, 0.0, 0.0, 58.0));
            let area = input.add(MouseArea::new(hit));
            let icon = icon_label(
                tree,
                engine,
                root,
                network_icon(&network),
                18.0,
                if network.connected {
                    theme::green()
                } else {
                    theme::muted()
                },
            );
            let name = label(tree, engine, root, &network.name, 12.0, theme::text());
            let sub = label(
                tree,
                engine,
                root,
                &network_sub(&network),
                10.0,
                theme::muted(),
            );
            let mut auto = chip(
                tree,
                input,
                engine,
                root,
                if network.autoconnect {
                    "auto"
                } else {
                    "manual"
                },
                if network.autoconnect {
                    theme::green()
                } else {
                    theme::muted()
                },
                false,
                None,
            );
            auto.shortcut = if network.autoconnect {
                "connects automatically · click to disable"
            } else {
                "connect only by hand · click to enable autoconnect"
            }
            .into();
            let forget = chip(tree, input, engine, root, "󰆴", theme::red(), false, None);
            self.rows.push(Row {
                data: network,
                root,
                hit,
                area,
                icon,
                name,
                sub,
                auto,
                forget,
            });
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
        let wifi = &data.wifi;
        if data.wifi_prompt != self.prompt {
            self.prompt = data.wifi_prompt.clone();
            self.prompt_box.field.set_text(tree, engine, "");
            if !self.prompt.is_empty() {
                self.set_focus(Some(Focus::Password), now);
            } else if self.focus == Some(Focus::Password) {
                self.set_focus(None, now);
            }
        }
        self.airplane.set_ink(
            now,
            if wifi.airplane {
                theme::yellow()
            } else {
                theme::muted()
            },
        );
        self.airplane.selected = wifi.airplane;
        self.airplane.shortcut = if wifi.airplane {
            "airplane mode on · click to turn off"
        } else {
            "airplane mode: turn off Wi-Fi and Bluetooth"
        }
        .into();
        self.power
            .set_label(engine, if wifi.powered { "on" } else { "off" }, 12.0);
        self.power.set_ink(
            now,
            if wifi.powered {
                theme::green()
            } else {
                theme::muted()
            },
        );
        let status = if wifi.airplane {
            "airplane mode is on".to_string()
        } else if wifi.state == "connected" {
            wifi.network.clone()
        } else if wifi.busy {
            "Connecting…".into()
        } else if wifi.powered {
            "select a network".into()
        } else {
            "wi-fi is turned off".into()
        };
        self.status.set_text(engine, &status);
        if let Some(details) = &data.wifi_details {
            self.band.set_text(
                engine,
                &format!(
                    "{} · ch {} · {}",
                    details.band, details.channel, details.mode
                ),
            );
            self.rssi.set_text(
                engine,
                &format!("{} dBm · {}", details.rssi, quality(details.rssi)),
            );
            self.rssi.set_color(if details.rssi < -75 {
                theme::yellow()
            } else {
                theme::green()
            });
            let ip = if details.ip.is_empty() {
                "no address".to_string()
            } else {
                details.ip.clone()
            };
            let gateway = if details.gateway.is_empty() {
                String::new()
            } else {
                format!("  ·  gw {}", details.gateway)
            };
            self.ip.set_text(engine, &format!("{ip}{gateway}"));
            self.link.set_text(
                engine,
                &format!("link {} Mbit/s", (details.rx_rate + 0.5).floor()),
            );
        }
        self.down.set_text(
            engine,
            &format!("↓ {}", speed(data.wifi_down.last().copied())),
        );
        self.up.set_text(
            engine,
            &format!("↑ {}", speed(data.wifi_up.last().copied())),
        );
        self.available.selected = self.section == Section::Available;
        self.saved.selected = self.section == Section::Saved;
        self.hidden.selected = self.hidden_open;
        if let Some(area) = input.get_mut(self.hidden.area) {
            area.enabled = wifi.powered;
        }
        self.scan
            .set_label(engine, if wifi.scanning { "scanning…" } else { "󰑐" }, 12.0);
        let empty = if wifi.airplane {
            "airplane mode"
        } else if wifi.powered {
            "no networks · try scanning"
        } else {
            "wireless disabled"
        };
        self.empty.set_text(engine, empty);
        self.prompt_label
            .set_text(engine, &format!("password for {}", self.prompt));
        self.message.set_text(engine, &wifi.message);
        self.rebuild(tree, input, engine);
        for row in &self.rows {
            if let Some(area) = input.get_mut(row.area) {
                area.enabled = !wifi.busy && row.data.available;
            }
        }
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        tree.set_size(self.root, width, height);
        let wifi = self.data.wifi.clone();
        let details_visible = self.data.wifi_details.is_some();
        let prompt_visible = !self.prompt.is_empty();
        let message_visible = !wifi.message.is_empty();
        let header_h = self
            .title
            .implicit_height()
            .ceil()
            .max(self.airplane.implicit_height());
        let prompt_h = self.prompt_label.implicit_height().ceil() + 5.0 + 38.0 + 5.0 + 30.0;
        let message_h = if message_visible {
            self.message.wrap(engine, width, 1000)
        } else {
            0.0
        };
        let mut cells = vec![
            Cell::layout(header_h),
            Cell::implicit(self.status.implicit_height()),
        ];
        if details_visible {
            cells.push(Cell::preferred(146.0));
        }
        cells.push(Cell::layout(30.0));
        if self.hidden_open {
            cells.push(Cell::layout(38.0 + 8.0 + 30.0));
        }
        cells.push(Cell::fill(0.0));
        if prompt_visible {
            cells.push(Cell::layout(prompt_h));
        }
        if message_visible {
            cells.push(Cell::implicit(message_h));
        }
        let slots = linear(&cells, 12.0, height);
        let mut next = slots.iter().copied();
        let header = next.next().unwrap_or_default();
        let mut header_cells = vec![
            Cell::fill(self.title.implicit_width().ceil()),
            Cell::implicit(self.airplane.implicit_width()),
        ];
        if !wifi.airplane {
            header_cells.push(Cell::implicit(self.power.implicit_width()));
        }
        let columns = linear(&header_cells, 5.0, width);
        place_label(
            tree,
            &mut self.title,
            columns[0].pos,
            header,
            columns[0].size,
        );
        place_chip(tree, &mut self.airplane, columns[1].pos, header);
        tree.set_visible(self.power.root, !wifi.airplane);
        if let Some(column) = columns.get(2) {
            place_chip(tree, &mut self.power, column.pos, header);
        }
        let status = next.next().unwrap_or_default();
        place_label(tree, &mut self.status, 0.0, status, width);
        tree.set_visible(self.card, details_visible);
        if details_visible {
            let slot = next.next().unwrap_or_default();
            let (y, h) = fit(slot, 146.0);
            tree.set_pos(self.card, 0.0, y);
            tree.set_size(self.card, width, h);
            self.layout_card(tree, width, h);
        }
        let tabs = next.next().unwrap_or_default();
        let tab_cells = [
            Cell::implicit(self.available.implicit_width()),
            Cell::implicit(self.saved.implicit_width()),
            Cell::fill(0.0),
            Cell::implicit(self.hidden.implicit_width()),
            Cell::implicit(self.scan.implicit_width()),
        ];
        let columns = linear(&tab_cells, 5.0, width);
        place_chip(tree, &mut self.available, columns[0].pos, tabs);
        place_chip(tree, &mut self.saved, columns[1].pos, tabs);
        place_chip(tree, &mut self.hidden, columns[3].pos, tabs);
        place_chip(tree, &mut self.scan, columns[4].pos, tabs);
        tree.set_opacity(self.hidden.root, if wifi.powered { 1.0 } else { 0.4 });
        let hidden_nodes = [
            self.hidden_box.frame,
            self.hidden_cancel.root,
            self.hidden_connect.root,
        ];
        for node in hidden_nodes {
            tree.set_visible(node, self.hidden_open);
        }
        if self.hidden_open {
            let slot = next.next().unwrap_or_default();
            let empty = self.hidden_box.field.text().is_empty();
            self.hidden_box
                .place(tree, engine, 0.0, slot.pos, width, empty);
            let row = Slot {
                pos: slot.pos + 38.0 + 8.0,
                size: 30.0,
            };
            let buttons = linear(
                &[
                    Cell::fill(0.0),
                    Cell::implicit(self.hidden_cancel.implicit_width()),
                    Cell::implicit(self.hidden_connect.implicit_width()),
                ],
                5.0,
                width,
            );
            place_chip(tree, &mut self.hidden_cancel, buttons[1].pos, row);
            place_chip(tree, &mut self.hidden_connect, buttons[2].pos, row);
        }
        let list = next.next().unwrap_or_default();
        let mut y = 0.0;
        let saved = self.section == Section::Saved;
        for (index, row) in self.rows.iter_mut().enumerate() {
            if index > 0 {
                y += 6.0;
            }
            tree.set_pos(row.root, 0.0, y);
            tree.set_size(row.root, width, 58.0);
            tree.set_size(row.hit, if saved { width - 120.0 } else { width }, 58.0);
            Self::layout_row(tree, row, width, saved);
            y += 58.0;
        }
        self.list.place(tree, 0.0, list.pos, width, list.size, y);
        tree.set_visible(self.empty.node, self.rows.is_empty());
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, center(width, ew), center(list.size, eh), ew, eh);
        for node in [
            self.prompt_label.node,
            self.prompt_box.frame,
            self.prompt_cancel.root,
            self.prompt_connect.root,
        ] {
            tree.set_visible(node, prompt_visible);
        }
        if prompt_visible {
            let slot = next.next().unwrap_or_default();
            let lh = self.prompt_label.implicit_height().ceil();
            self.prompt_label.place(tree, 0.0, slot.pos, width, lh);
            let empty = self.prompt_box.field.text().is_empty();
            self.prompt_box
                .place(tree, engine, 0.0, slot.pos + lh + 5.0, width, empty);
            let row = Slot {
                pos: slot.pos + lh + 5.0 + 38.0 + 5.0,
                size: 30.0,
            };
            let buttons = linear(
                &[
                    Cell::fill(0.0),
                    Cell::implicit(self.prompt_cancel.implicit_width()),
                    Cell::implicit(self.prompt_connect.implicit_width()),
                ],
                5.0,
                width,
            );
            place_chip(tree, &mut self.prompt_cancel, buttons[1].pos, row);
            place_chip(tree, &mut self.prompt_connect, buttons[2].pos, row);
        }
        tree.set_visible(self.message.node, message_visible);
        if message_visible {
            let slot = next.next().unwrap_or_default();
            self.message.place(tree, 0.0, slot.pos, width, message_h);
        }
    }

    fn layout_card(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let inner = width - 24.0;
        let row1_h = self
            .card_icon
            .implicit_height()
            .ceil()
            .max(self.band.implicit_height().ceil())
            .max(self.rssi.implicit_height().ceil());
        let row2_h = self
            .ip
            .implicit_height()
            .ceil()
            .max(self.link.implicit_height().ceil());
        let row4_h = self
            .down
            .implicit_height()
            .ceil()
            .max(self.up.implicit_height().ceil());
        let rows = linear(
            &[
                Cell::layout(row1_h),
                Cell::layout(row2_h),
                Cell::fill(44.0).margins(4.0, 0.0),
                Cell::layout(row4_h),
            ],
            5.0,
            height - 24.0,
        );
        let shift = |slot: Slot| Slot {
            pos: slot.pos + 12.0,
            size: slot.size,
        };
        let row = shift(rows[0]);
        let columns = linear(
            &[
                Cell::implicit(self.card_icon.implicit_width()),
                Cell::fill(self.band.implicit_width().ceil()),
                Cell::implicit(self.rssi.implicit_width()),
            ],
            5.0,
            inner,
        );
        place_label(
            tree,
            &mut self.card_icon,
            12.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.band,
            12.0 + columns[1].pos,
            row,
            columns[1].size,
        );
        place_label(
            tree,
            &mut self.rssi,
            12.0 + columns[2].pos,
            row,
            columns[2].size,
        );
        let row = shift(rows[1]);
        let columns = linear(
            &[
                Cell::fill(self.ip.implicit_width().ceil()),
                Cell::implicit(self.link.implicit_width()),
            ],
            5.0,
            inner,
        );
        place_label(
            tree,
            &mut self.ip,
            12.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.link,
            12.0 + columns[1].pos,
            row,
            columns[1].size,
        );
        let spark = shift(rows[2]);
        self.spark.place(
            tree,
            12.0,
            spark.pos,
            inner,
            spark.size,
            &self.data.wifi_down,
            &self.data.wifi_up,
        );
        let row = shift(rows[3]);
        let columns = linear(
            &[
                Cell::fill(self.down.implicit_width().ceil()),
                Cell::implicit(self.up.implicit_width()),
            ],
            5.0,
            inner,
        );
        place_label(
            tree,
            &mut self.down,
            12.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.up,
            12.0 + columns[1].pos,
            row,
            columns[1].size,
        );
    }

    fn layout_row(tree: &mut Tree, row: &mut Row, width: f32, saved: bool) {
        let inner = Slot {
            pos: 10.0,
            size: 38.0,
        };
        let text_h = row.name.implicit_height().ceil() + 3.0 + row.sub.implicit_height().ceil();
        let mut cells = vec![
            Cell::implicit(row.icon.implicit_width()),
            Cell::layout(
                row.name
                    .implicit_width()
                    .ceil()
                    .max(row.sub.implicit_width().ceil()),
            ),
        ];
        cells[1] = Cell::fill(cells[1].size);
        if saved {
            cells.push(Cell::implicit(row.auto.implicit_width()));
            cells.push(Cell::implicit(row.forget.implicit_width()));
        }
        let columns = linear(&cells, 10.0, width - 20.0);
        place_label(
            tree,
            &mut row.icon,
            10.0 + columns[0].pos,
            inner,
            columns[0].size,
        );
        let (ty, _) = fit(inner, text_h);
        let nh = row.name.implicit_height().ceil();
        row.name
            .place(tree, 10.0 + columns[1].pos, ty, columns[1].size, nh);
        let sh = row.sub.implicit_height().ceil();
        row.sub.place(
            tree,
            10.0 + columns[1].pos,
            ty + nh + 3.0,
            columns[1].size,
            sh,
        );
        tree.set_visible(row.auto.root, saved);
        tree.set_visible(row.forget.root, saved);
        if saved {
            place_chip(tree, &mut row.auto, 10.0 + columns[2].pos, inner);
            place_chip(tree, &mut row.forget, 10.0 + columns[3].pos, inner);
        }
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        let mut chips: Vec<&mut Chip> = vec![
            &mut self.airplane,
            &mut self.power,
            &mut self.available,
            &mut self.saved,
            &mut self.hidden,
            &mut self.scan,
            &mut self.hidden_cancel,
            &mut self.hidden_connect,
            &mut self.prompt_cancel,
            &mut self.prompt_connect,
        ];
        for row in &mut self.rows {
            chips.push(&mut row.auto);
            chips.push(&mut row.forget);
        }
        for chip in chips {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        for row in &self.rows {
            let color = if row.data.connected {
                theme::elevated()
            } else if input.contains_mouse(row.area) {
                theme::surface()
            } else {
                Color::TRANSPARENT
            };
            tree.set_fill(row.root, color);
            for label in [&row.icon, &row.name, &row.sub] {
                label.sync(tree);
            }
        }
        running |= self.list.tick(tree, input, now);
        self.hidden_box.field.tick(tree, now);
        self.prompt_box.field.tick(tree, now);
        for label in [
            &self.title,
            &self.status,
            &self.card_icon,
            &self.band,
            &self.rssi,
            &self.ip,
            &self.link,
            &self.down,
            &self.up,
            &self.empty,
            &self.prompt_label,
            &self.message,
        ] {
            label.sync(tree);
        }
        if let Some(placeholder) = &self.hidden_box.placeholder {
            placeholder.sync(tree);
        }
        running
    }

    pub fn next_deadline(&self, now: Millis) -> Option<Millis> {
        [
            self.hidden_box.field.next_blink(now),
            self.prompt_box.field.next_blink(now),
        ]
        .into_iter()
        .flatten()
        .reduce(f64::min)
    }

    fn submit_hidden(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        actions: &mut Vec<PanelAction>,
        now: Millis,
    ) {
        let text = self.hidden_box.field.text().to_string();
        if text.trim().is_empty() {
            return;
        }
        actions.push(PanelAction::Net(NetAction::ConnectHidden(text)));
        self.hidden_box.field.set_text(tree, engine, "");
        self.hidden_open = false;
        if self.focus == Some(Focus::Ssid) {
            self.set_focus(None, now);
        }
    }

    fn submit_password(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        actions: &mut Vec<PanelAction>,
    ) {
        let text = self.prompt_box.field.text().to_string();
        actions.push(PanelAction::Net(NetAction::Password(text)));
        self.prompt_box.field.set_text(tree, engine, "");
    }

    pub fn handle(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        events: &[InputEvent],
        pointer: (f32, f32),
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        let wifi = self.data.wifi.clone();
        for event in events {
            match *event {
                InputEvent::Press(area, MouseButton::Left)
                    if area == self.hidden_box.field.area =>
                {
                    self.set_focus(Some(Focus::Ssid), now);
                    self.hidden_box
                        .field
                        .press(tree, engine, pointer.0, pointer.1, now);
                }
                InputEvent::Press(area, MouseButton::Left)
                    if area == self.prompt_box.field.area =>
                {
                    self.set_focus(Some(Focus::Password), now);
                    self.prompt_box
                        .field
                        .press(tree, engine, pointer.0, pointer.1, now);
                }
                InputEvent::Wheel(area, delta) if area == self.list.area => {
                    self.list.wheel(delta, now);
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.airplane.area {
                        actions.push(PanelAction::Net(NetAction::Airplane(!wifi.airplane)));
                    } else if area == self.power.area {
                        actions.push(PanelAction::Net(NetAction::WifiPower(!wifi.powered)));
                    } else if area == self.available.area {
                        self.section = Section::Available;
                    } else if area == self.saved.area {
                        self.section = Section::Saved;
                    } else if area == self.hidden.area {
                        self.hidden_open = !self.hidden_open;
                        if self.hidden_open {
                            self.set_focus(Some(Focus::Ssid), now);
                        } else if self.focus == Some(Focus::Ssid) {
                            self.set_focus(None, now);
                        }
                    } else if area == self.scan.area {
                        if wifi.powered {
                            actions.push(PanelAction::Net(NetAction::Scan));
                        }
                    } else if area == self.hidden_cancel.area {
                        self.hidden_box.field.set_text(tree, engine, "");
                        self.hidden_open = false;
                        if self.focus == Some(Focus::Ssid) {
                            self.set_focus(None, now);
                        }
                    } else if area == self.hidden_connect.area {
                        self.submit_hidden(tree, engine, &mut actions, now);
                    } else if area == self.prompt_cancel.area {
                        self.prompt_box.field.set_text(tree, engine, "");
                        actions.push(PanelAction::Net(NetAction::Cancel));
                    } else if area == self.prompt_connect.area {
                        self.submit_password(tree, engine, &mut actions);
                    } else if let Some(row) = self.rows.iter().find(|row| row.area == area) {
                        actions.push(PanelAction::Net(if row.data.connected {
                            NetAction::Disconnect
                        } else {
                            NetAction::Connect(row.data.path.clone())
                        }));
                    } else if let Some(row) = self.rows.iter().find(|row| row.auto.area == area) {
                        actions.push(PanelAction::Net(NetAction::Autoconnect(
                            row.data.known.clone(),
                            !row.data.autoconnect,
                        )));
                    } else if let Some(row) = self.rows.iter().find(|row| row.forget.area == area) {
                        actions.push(PanelAction::Net(NetAction::Forget(row.data.known.clone())));
                    }
                }
                _ => {}
            }
        }
        let data = self.data.clone();
        self.sync(tree, input, engine, &data, now);
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
        self.hidden_box.field.drag(tree, engine, input, x, y);
        self.prompt_box.field.drag(tree, engine, input, x, y);
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        event: &KeyEvent,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut actions = Vec::new();
        match self.focus {
            Some(Focus::Ssid) => match self.hidden_box.field.key(tree, engine, event, now) {
                KeyResult::Accepted => self.submit_hidden(tree, engine, &mut actions, now),
                KeyResult::Escape => {
                    self.hidden_box.field.set_text(tree, engine, "");
                    self.hidden_open = false;
                    self.set_focus(None, now);
                }
                KeyResult::Handled => {}
                KeyResult::Ignored => return (false, actions),
            },
            Some(Focus::Password) => match self.prompt_box.field.key(tree, engine, event, now) {
                KeyResult::Accepted => self.submit_password(tree, engine, &mut actions),
                KeyResult::Escape | KeyResult::Ignored => return (false, actions),
                KeyResult::Handled => {}
            },
            None => return (false, actions),
        }
        (true, actions)
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        let mut chips = vec![&self.airplane];
        if self.section == Section::Saved {
            chips.extend(self.rows.iter().map(|row| &row.auto));
        }
        chips
            .into_iter()
            .find(|chip| !chip.shortcut.is_empty() && chip.hovered(input))
            .map(|chip| TooltipSpot {
                key: chip.area,
                text: chip.shortcut.clone(),
                node: chip.root,
                width: chip.width(),
                height: chip.height(),
            })
    }
}
