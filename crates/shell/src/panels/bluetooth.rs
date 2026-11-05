use platform::{KeyEvent, MouseButton};
use services::network::{AudioProfile, BluetoothDevice, DeviceProfiles};
use ui::{
    Node, NodeId, TextEngine, Tree,
    anim::Millis,
    input::{AreaId, Input, InputEvent},
    layout::{Cell, Slot, center, linear},
    textfield::{KeyResult, TextField},
    widgets::{Chip, Label},
};

use super::{
    NetAction, PanelAction, PanelData, TooltipSpot,
    kit::{Scroller, chip, fit, icon_label, label, place_chip, place_label, rect, set_border},
};
use crate::theme::{self};

pub fn device_icon(name: &str) -> &'static str {
    if name.starts_with("audio-head") {
        "󰋋"
    } else if name.starts_with("audio") {
        "󰓃"
    } else if name == "input-mouse" {
        "󰍽"
    } else if name == "input-keyboard" {
        "󰌌"
    } else if name == "input-gaming" {
        "󰊴"
    } else if name == "input-tablet" {
        "󰓶"
    } else if name.starts_with("phone") {
        "󰏲"
    } else if name == "computer" {
        "󰟀"
    } else if name.starts_with("camera") {
        "󰄀"
    } else {
        "󰂯"
    }
}

fn profile_label(profile: &AudioProfile) -> String {
    let codec = profile
        .description
        .find("codec ")
        .map(|start| &profile.description[start + 6..])
        .and_then(|rest| {
            rest.find(')')
                .map(|end| rest[..end].to_string())
                .filter(|c| !c.is_empty())
        });
    let kind = if profile.name.starts_with("a2dp") {
        "hi-fi".to_string()
    } else if profile.name.starts_with("headset") {
        "headset · mic".to_string()
    } else {
        profile.description.clone()
    };
    match codec {
        Some(codec) => format!("{kind} · {codec}"),
        None => kind,
    }
}

fn status(device: &BluetoothDevice) -> String {
    let base = if device.pairing {
        "pairing"
    } else if device.connected {
        "connected"
    } else if device.paired {
        "paired"
    } else {
        "available"
    };
    let mut text = base.to_string();
    if device.trusted {
        text.push_str(" · trusted");
    }
    if device.battery >= 0 {
        text.push_str(&format!(" · {}%", device.battery));
    }
    text
}

#[derive(Clone, PartialEq)]
struct CardKey {
    device: BluetoothDevice,
    profiles: Option<DeviceProfiles>,
    editing: bool,
}

struct Card {
    key: CardKey,
    root: NodeId,
    icon: Label,
    name: Label,
    editor: NodeId,
    alias: TextField,
    status: Label,
    rename: Chip,
    save: Chip,
    trust: Chip,
    action: Chip,
    forget: Chip,
    profiles: Vec<(Chip, AudioProfile)>,
}

impl Card {
    fn show_profiles(&self) -> bool {
        self.key.device.connected
            && self
                .key
                .profiles
                .as_ref()
                .is_some_and(|audio| audio.profiles.len() > 1)
    }

    fn height(&self) -> f32 {
        62.0 + if self.show_profiles() { 36.0 } else { 0.0 }
    }

    fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.alias.area,
            self.rename.area,
            self.save.area,
            self.trust.area,
            self.action.area,
            self.forget.area,
        ];
        areas.extend(self.profiles.iter().map(|(chip, _)| chip.area));
        areas
    }

    fn chips_mut(&mut self) -> Vec<&mut Chip> {
        let mut chips = vec![
            &mut self.rename,
            &mut self.save,
            &mut self.trust,
            &mut self.action,
            &mut self.forget,
        ];
        chips.extend(self.profiles.iter_mut().map(|(chip, _)| chip));
        chips
    }
}

pub struct BluetoothPanel {
    pub root: NodeId,
    title: Label,
    power: Chip,
    name: Label,
    scan: Chip,
    flick: Scroller,
    headers: [Label; 2],
    cards: Vec<Card>,
    empty: Label,
    prompt_title: Label,
    prompt_mode: Label,
    pin_frame: NodeId,
    pin: TextField,
    pin_placeholder: Label,
    cancel: Chip,
    confirm: Chip,
    error: Label,
    renaming: String,
    prompt: services::network::BluetoothPrompt,
    pin_focus: bool,
    alias_focus: bool,
    data: PanelData,
}

impl BluetoothPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "Bluetooth", 18.0, theme::text());
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
        let name = label(tree, engine, root, "", 12.0, theme::muted());
        let scan = chip(
            tree,
            input,
            engine,
            root,
            "scan",
            theme::green(),
            false,
            None,
        );
        let flick = Scroller::new(tree, input, root, true);
        let headers = [
            label(tree, engine, flick.content, "paired", 10.0, theme::muted()),
            label(
                tree,
                engine,
                flick.content,
                "available",
                10.0,
                theme::muted(),
            ),
        ];
        let empty = label(tree, engine, flick.view, "", 12.0, theme::muted());
        let prompt_title = label(tree, engine, root, "", 12.0, theme::text());
        let prompt_mode = label(tree, engine, root, "", 12.0, theme::green());
        let pin_frame = rect(tree, root, theme::surface(), 8.0);
        set_border(tree, pin_frame, theme::line(), 1.0);
        let pin_placeholder = label(tree, engine, pin_frame, "PIN", 12.0, theme::muted());
        let pin = TextField::new(
            tree,
            input,
            engine,
            pin_frame,
            crate::theme::font(),
            12.0,
            theme::text(),
            false,
        );
        let cancel = chip(
            tree,
            input,
            engine,
            root,
            "cancel",
            theme::text(),
            false,
            None,
        );
        let confirm = chip(
            tree,
            input,
            engine,
            root,
            "confirm",
            theme::green(),
            false,
            None,
        );
        let error = label(tree, engine, root, "", 12.0, theme::red());
        let mut panel = Self {
            root,
            title,
            power,
            name,
            scan,
            flick,
            headers,
            cards: Vec::new(),
            empty,
            prompt_title,
            prompt_mode,
            pin_frame,
            pin,
            pin_placeholder,
            cancel,
            confirm,
            error,
            renaming: String::new(),
            prompt: Default::default(),
            pin_focus: false,
            alias_focus: false,
            data: PanelData::default(),
        };
        panel.sync(tree, input, engine, data, now);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.power.area,
            self.scan.area,
            self.pin.area,
            self.cancel.area,
            self.confirm.area,
        ];
        areas.extend(self.flick.areas());
        for card in &self.cards {
            areas.extend(card.areas());
        }
        areas
    }

    fn pin_visible(&self) -> bool {
        matches!(self.prompt.mode.as_str(), "pin" | "passkey")
    }

    fn wanted(&self) -> Vec<CardKey> {
        let devices = &self.data.bluetooth.devices;
        let paired = devices.iter().filter(|d| d.paired || d.connected);
        let available = devices.iter().filter(|d| !d.paired && !d.connected);
        paired
            .chain(available)
            .map(|device| CardKey {
                device: device.clone(),
                profiles: self.data.bluetooth_profiles.get(&device.address).cloned(),
                editing: self.renaming == device.path,
            })
            .collect()
    }

    fn rebuild(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        now: Millis,
    ) {
        let wanted = self.wanted();
        let same = self.cards.len() == wanted.len()
            && self
                .cards
                .iter()
                .zip(&wanted)
                .all(|(card, key)| card.key == *key);
        if same {
            return;
        }
        for card in self.cards.drain(..) {
            for area in card.areas() {
                input.remove(area);
            }
            tree.remove(card.root);
        }
        self.alias_focus = false;
        for key in wanted {
            let device = &key.device;
            let root = rect(
                tree,
                self.flick.content,
                if device.connected {
                    theme::elevated()
                } else {
                    theme::surface()
                },
                10.0,
            );
            let icon = icon_label(
                tree,
                engine,
                root,
                device_icon(&device.icon),
                20.0,
                if device.connected {
                    theme::green()
                } else {
                    theme::muted()
                },
            );
            let name = label(tree, engine, root, &device.name, 12.0, theme::text());
            let editor = rect(tree, root, theme::bg(), 5.0);
            set_border(tree, editor, theme::green(), 1.0);
            let mut alias = TextField::new(
                tree,
                input,
                engine,
                editor,
                crate::theme::font(),
                12.0,
                theme::text(),
                false,
            );
            let status_label = label(
                tree,
                engine,
                root,
                &status(device),
                10.0,
                if device.connected {
                    theme::green()
                } else {
                    theme::muted()
                },
            );
            let mut rename = chip(
                tree,
                input,
                engine,
                root,
                "󰏫",
                theme::text(),
                true,
                Some(14.0),
            );
            rename.shortcut = "rename".into();
            let save = chip(
                tree,
                input,
                engine,
                root,
                "save",
                theme::green(),
                true,
                None,
            );
            let mut trust = chip(
                tree,
                input,
                engine,
                root,
                if device.trusted { "󰒘" } else { "󰦞" },
                if device.trusted {
                    theme::green()
                } else {
                    theme::muted()
                },
                true,
                Some(14.0),
            );
            trust.shortcut = if device.trusted {
                "trusted · reconnects on its own · click to untrust"
            } else {
                "not trusted · click to trust"
            }
            .into();
            let action_text = if device.pairing {
                "cancel"
            } else if device.connected {
                "disconnect"
            } else if device.paired {
                "connect"
            } else {
                "pair"
            };
            let action = chip(
                tree,
                input,
                engine,
                root,
                action_text,
                theme::text(),
                false,
                None,
            );
            let mut forget = chip(
                tree,
                input,
                engine,
                root,
                "󰆴",
                theme::red(),
                true,
                Some(14.0),
            );
            forget.shortcut = "forget".into();
            let mut profiles = Vec::new();
            if let Some(audio) = &key.profiles {
                for profile in &audio.profiles {
                    let selected = audio.current == Some(profile.index);
                    let mut profile_chip = chip(
                        tree,
                        input,
                        engine,
                        root,
                        &profile_label(profile),
                        if selected {
                            theme::green()
                        } else {
                            theme::text()
                        },
                        true,
                        None,
                    );
                    profile_chip.shortcut = profile.description.clone();
                    profile_chip.selected = selected;
                    profile_chip.jump_color(input);
                    profiles.push((profile_chip, profile.clone()));
                }
            }
            if key.editing {
                alias.set_text(tree, engine, &device.name);
                alias.select_all(tree, engine);
                alias.set_focus(true, now);
                self.alias_focus = true;
                self.pin_focus = false;
                self.pin.set_focus(false, now);
            }
            self.cards.push(Card {
                key,
                root,
                icon,
                name,
                editor,
                alias,
                status: status_label,
                rename,
                save,
                trust,
                action,
                forget,
                profiles,
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
        let bluetooth = &data.bluetooth;
        self.power
            .set_label(engine, if bluetooth.powered { "on" } else { "off" }, 12.0);
        self.power.set_ink(
            now,
            if bluetooth.powered {
                theme::green()
            } else {
                theme::muted()
            },
        );
        let name = if bluetooth.name.is_empty() {
            "no adapter found"
        } else {
            bluetooth.name.as_str()
        };
        self.name.set_text(engine, name);
        self.scan.set_label(
            engine,
            if bluetooth.discovering {
                "stop scan"
            } else {
                "scan"
            },
            12.0,
        );
        self.empty.set_text(
            engine,
            if bluetooth.powered {
                "scan to discover devices"
            } else {
                "bluetooth is turned off"
            },
        );
        if data.bluetooth_prompt != self.prompt {
            self.prompt = data.bluetooth_prompt.clone();
            self.pin.set_text(tree, engine, "");
            if self.pin_visible() {
                self.pin_focus = true;
                self.pin.set_focus(true, now);
            }
        }
        self.prompt_title
            .set_text(engine, &format!("pair with {}", self.prompt.name));
        let mode = match self.prompt.mode.as_str() {
            "display" => format!("enter on device: {}", self.prompt.value),
            "confirm" => format!("confirm code: {}", self.prompt.value),
            _ => "allow pairing / enter PIN".into(),
        };
        self.prompt_mode.set_text(engine, &mode);
        self.error.set_text(engine, &data.bluetooth_error);
        self.rebuild(tree, input, engine, now);
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        tree.set_size(self.root, width, height);
        let prompt_visible = !self.prompt.name.is_empty();
        let pin_visible = prompt_visible && self.pin_visible();
        let error_visible = !self.data.bluetooth_error.is_empty();
        let pin_h = self.pin.implicit_height() + 12.0;
        let prompt_h = if prompt_visible {
            let mut cells = vec![
                Cell::implicit(self.prompt_title.implicit_height()),
                Cell::implicit(self.prompt_mode.implicit_height()),
            ];
            if pin_visible {
                cells.push(Cell::preferred(pin_h));
            }
            cells.push(Cell::layout(30.0));
            cells.iter().map(|cell| cell.size).sum::<f32>() + 5.0 * (cells.len() - 1) as f32
        } else {
            0.0
        };
        let error_h = if error_visible {
            self.error.wrap(engine, width, 1000)
        } else {
            0.0
        };
        let header_h = self
            .title
            .implicit_height()
            .ceil()
            .max(self.power.implicit_height());
        let row_h = self
            .name
            .implicit_height()
            .ceil()
            .max(self.scan.implicit_height());
        let mut cells = vec![Cell::layout(header_h), Cell::layout(row_h), Cell::fill(0.0)];
        if prompt_visible {
            cells.push(Cell::layout(prompt_h));
        }
        if error_visible {
            cells.push(Cell::implicit(error_h));
        }
        let slots = linear(&cells, 12.0, height);
        let header = slots[0];
        let columns = linear(
            &[
                Cell::fill(self.title.implicit_width().ceil()),
                Cell::implicit(self.power.implicit_width()),
            ],
            5.0,
            width,
        );
        place_label(
            tree,
            &mut self.title,
            columns[0].pos,
            header,
            columns[0].size,
        );
        place_chip(tree, &mut self.power, columns[1].pos, header);
        let row = slots[1];
        let columns = linear(
            &[
                Cell::fill(self.name.implicit_width().ceil()),
                Cell::implicit(self.scan.implicit_width()),
            ],
            5.0,
            width,
        );
        place_label(tree, &mut self.name, columns[0].pos, row, columns[0].size);
        place_chip(tree, &mut self.scan, columns[1].pos, row);
        let flick = slots[2];
        let paired_count = self
            .cards
            .iter()
            .filter(|card| card.key.device.paired || card.key.device.connected)
            .count();
        let mut y = 0.0;
        let mut first_section = true;
        for (section, range) in [(0, 0..paired_count), (1, paired_count..self.cards.len())] {
            let visible = !range.is_empty();
            tree.set_visible(self.headers[section].node, visible);
            if !visible {
                continue;
            }
            if !first_section {
                y += 6.0;
            }
            first_section = false;
            y += 4.0;
            let lh = self.headers[section].implicit_height().ceil();
            let lw = self.headers[section].implicit_width().ceil();
            self.headers[section].place(tree, 0.0, y, lw, lh);
            y += lh;
            for index in range {
                y += 6.0;
                let card = &mut self.cards[index];
                let h = card.height();
                tree.set_pos(card.root, 0.0, y);
                tree.set_size(card.root, width, h);
                Self::layout_card(tree, engine, card, width, h);
                y += h;
            }
        }
        self.flick.place(tree, 0.0, flick.pos, width, flick.size, y);
        tree.set_visible(self.empty.node, self.data.bluetooth.devices.is_empty());
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, center(width, ew), center(flick.size, eh), ew, eh);
        for node in [
            self.prompt_title.node,
            self.prompt_mode.node,
            self.cancel.root,
            self.confirm.root,
        ] {
            tree.set_visible(node, prompt_visible);
        }
        tree.set_visible(self.pin_frame, pin_visible);
        let mut index = 3;
        if prompt_visible {
            let slot = slots[index];
            index += 1;
            let mut y = slot.pos;
            let th = self.prompt_title.implicit_height().ceil();
            self.prompt_title.place(tree, 0.0, y, width, th);
            y += th + 5.0;
            let mh = self.prompt_mode.implicit_height().ceil();
            self.prompt_mode.place(tree, 0.0, y, width, mh);
            y += mh + 5.0;
            if pin_visible {
                tree.set_pos(self.pin_frame, 0.0, y);
                tree.set_size(self.pin_frame, width, pin_h);
                self.pin
                    .place(tree, engine, 10.0, 6.0, width - 16.0, pin_h - 12.0);
                let ph = self.pin_placeholder.implicit_height().ceil();
                tree.set_visible(self.pin_placeholder.node, self.pin.text().is_empty());
                self.pin_placeholder
                    .place(tree, 10.0, 6.0, width - 16.0, ph);
                y += pin_h + 5.0;
            }
            let row = Slot { pos: y, size: 30.0 };
            let buttons = linear(
                &[
                    Cell::implicit(self.cancel.implicit_width()),
                    Cell::implicit(self.confirm.implicit_width()),
                ],
                5.0,
                width,
            );
            place_chip(tree, &mut self.cancel, buttons[0].pos, row);
            place_chip(tree, &mut self.confirm, buttons[1].pos, row);
        }
        tree.set_visible(self.error.node, error_visible);
        if error_visible {
            let slot = slots[index];
            self.error.place(tree, 0.0, slot.pos, width, error_h);
        }
    }

    fn layout_card(
        tree: &mut Tree,
        engine: &mut TextEngine,
        card: &mut Card,
        width: f32,
        height: f32,
    ) {
        let device = card.key.device.clone();
        let editing = card.key.editing;
        let show_profiles = card.show_profiles();
        let inner = width - 20.0;
        let name_h = if editing {
            20.0
        } else {
            card.name.implicit_height().ceil()
        };
        let text_h = name_h + 4.0 + card.status.implicit_height().ceil();
        let row_h = text_h.max(30.0).max(card.icon.implicit_height().ceil());
        let mut column = vec![Cell::layout(row_h)];
        if show_profiles {
            column.push(Cell::layout(22.0));
        }
        let rows = linear(&column, 6.0, height - 20.0);
        let row = Slot {
            pos: rows[0].pos + 10.0,
            size: rows[0].size,
        };
        let mut cells = vec![Cell::preferred(22.0), Cell::fill(0.0)];
        let mut order = Vec::new();
        if device.paired && !editing {
            cells.push(Cell::implicit(card.rename.implicit_width()));
            order.push(0);
        }
        if editing {
            cells.push(Cell::implicit(card.save.implicit_width()));
            order.push(1);
        }
        if device.paired {
            cells.push(Cell::implicit(card.trust.implicit_width()));
            order.push(2);
        }
        cells.push(Cell::implicit(card.action.implicit_width()));
        order.push(3);
        if device.paired {
            cells.push(Cell::implicit(card.forget.implicit_width()));
            order.push(4);
        }
        let columns = linear(&cells, 10.0, inner);
        place_label(
            tree,
            &mut card.icon,
            10.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        let (ty, _) = fit(row, text_h);
        let tx = 10.0 + columns[1].pos;
        let tw = columns[1].size;
        tree.set_visible(card.name.node, !editing);
        tree.set_visible(card.editor, editing);
        if editing {
            tree.set_pos(card.editor, tx, ty);
            tree.set_size(card.editor, tw, 20.0);
            let fh = card.alias.implicit_height();
            card.alias
                .place(tree, engine, 6.0, center(20.0, fh), tw - 12.0, fh);
        } else {
            card.name.place(tree, tx, ty, tw, name_h);
        }
        let sh = card.status.implicit_height().ceil();
        card.status.place(tree, tx, ty + name_h + 4.0, tw, sh);
        tree.set_visible(card.rename.root, order.contains(&0));
        tree.set_visible(card.save.root, order.contains(&1));
        tree.set_visible(card.trust.root, order.contains(&2));
        tree.set_visible(card.forget.root, order.contains(&4));
        for (slot, which) in columns[2..].iter().zip(&order) {
            let chip = match which {
                0 => &mut card.rename,
                1 => &mut card.save,
                2 => &mut card.trust,
                3 => &mut card.action,
                _ => &mut card.forget,
            };
            place_chip(tree, chip, 10.0 + slot.pos, row);
        }
        for (chip, _) in &card.profiles {
            tree.set_visible(chip.root, show_profiles);
        }
        if show_profiles {
            let slot = Slot {
                pos: rows[1].pos + 10.0,
                size: rows[1].size,
            };
            let mut cells: Vec<Cell> = card
                .profiles
                .iter()
                .map(|(chip, _)| Cell::implicit(chip.implicit_width()))
                .collect();
            cells.push(Cell::fill(0.0));
            let columns = linear(&cells, 4.0, inner - 32.0);
            for ((chip, _), column) in card.profiles.iter_mut().zip(&columns) {
                place_chip(tree, chip, 10.0 + 32.0 + column.pos, slot);
            }
        }
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for chip in [
            &mut self.power,
            &mut self.scan,
            &mut self.cancel,
            &mut self.confirm,
        ] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        for card in &mut self.cards {
            for chip in card.chips_mut() {
                chip.sync(now, input);
                running |= chip.tick(tree, now);
            }
            card.alias.tick(tree, now);
            for label in [&card.icon, &card.name, &card.status] {
                label.sync(tree);
            }
        }
        running |= self.flick.tick(tree, input, now);
        self.pin.tick(tree, now);
        for label in [
            &self.title,
            &self.name,
            &self.headers[0],
            &self.headers[1],
            &self.empty,
            &self.prompt_title,
            &self.prompt_mode,
            &self.pin_placeholder,
            &self.error,
        ] {
            label.sync(tree);
        }
        running
    }

    pub fn next_deadline(&self, now: Millis) -> Option<Millis> {
        let mut deadlines: Vec<Millis> = self
            .cards
            .iter()
            .filter_map(|card| card.alias.next_blink(now))
            .collect();
        deadlines.extend(self.pin.next_blink(now));
        deadlines.into_iter().reduce(f64::min)
    }

    fn submit_pin(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        actions: &mut Vec<PanelAction>,
    ) {
        let value = self.pin.text().to_string();
        actions.push(PanelAction::Net(NetAction::BtReply(true, value)));
        self.pin.set_text(tree, engine, "");
    }

    fn submit_alias(&mut self, actions: &mut Vec<PanelAction>) {
        if let Some(card) = self.cards.iter().find(|card| card.key.editing) {
            actions.push(PanelAction::Net(NetAction::BtRename(
                card.key.device.path.clone(),
                card.alias.text().to_string(),
            )));
        }
        self.renaming.clear();
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
        let bluetooth = self.data.bluetooth.clone();
        for event in events {
            match *event {
                InputEvent::Wheel(area, delta) if area == self.flick.area => {
                    self.flick.wheel(delta, now);
                }
                InputEvent::Press(area, MouseButton::Left) if area == self.pin.area => {
                    self.pin_focus = true;
                    self.pin.press(tree, engine, pointer.0, pointer.1, now);
                }
                InputEvent::Press(area, MouseButton::Left) => {
                    if let Some(card) = self.cards.iter_mut().find(|card| card.alias.area == area) {
                        card.alias.press(tree, engine, pointer.0, pointer.1, now);
                    }
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.power.area {
                        actions.push(PanelAction::Net(NetAction::BtPower(!bluetooth.powered)));
                    } else if area == self.scan.area {
                        actions.push(PanelAction::Net(NetAction::BtScan(!bluetooth.discovering)));
                    } else if area == self.cancel.area {
                        actions.push(PanelAction::Net(NetAction::BtReply(false, String::new())));
                    } else if area == self.confirm.area {
                        self.submit_pin(tree, engine, &mut actions);
                    } else if let Some(card) =
                        self.cards.iter().find(|card| card.areas().contains(&area))
                    {
                        let device = card.key.device.clone();
                        if area == card.rename.area {
                            self.renaming = device.path.clone();
                        } else if area == card.save.area {
                            self.submit_alias(&mut actions);
                        } else if area == card.trust.area {
                            actions.push(PanelAction::Net(NetAction::BtTrust(
                                device.path.clone(),
                                !device.trusted,
                            )));
                        } else if area == card.action.area {
                            let method = if device.pairing {
                                "CancelPairing"
                            } else if device.connected {
                                "Disconnect"
                            } else if device.paired {
                                "Connect"
                            } else {
                                ""
                            };
                            actions.push(PanelAction::Net(if method.is_empty() {
                                NetAction::BtPair(device.path.clone())
                            } else {
                                NetAction::BtDevice(device.path.clone(), method)
                            }));
                        } else if area == card.forget.area {
                            actions
                                .push(PanelAction::Net(NetAction::BtForget(device.path.clone())));
                        } else if let Some((_, profile)) =
                            card.profiles.iter().find(|(chip, _)| chip.area == area)
                            && let Some(audio) = &card.key.profiles
                        {
                            actions.push(PanelAction::Net(NetAction::BtProfile(
                                audio.id,
                                profile.index,
                            )));
                        }
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
        self.pin.drag(tree, engine, input, x, y);
        for card in &mut self.cards {
            card.alias.drag(tree, engine, input, x, y);
        }
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        event: &KeyEvent,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut actions = Vec::new();
        if self.alias_focus
            && let Some(card) = self.cards.iter_mut().find(|card| card.key.editing)
        {
            match card.alias.key(tree, engine, event, now) {
                KeyResult::Accepted => self.submit_alias(&mut actions),
                KeyResult::Escape => self.renaming.clear(),
                KeyResult::Handled => return (true, actions),
                KeyResult::Ignored => return (false, actions),
            }
            let data = self.data.clone();
            self.sync(tree, input, engine, &data, now);
            return (true, actions);
        }
        if self.pin_focus && self.pin_visible() {
            match self.pin.key(tree, engine, event, now) {
                KeyResult::Accepted => self.submit_pin(tree, engine, &mut actions),
                KeyResult::Handled => {}
                KeyResult::Escape | KeyResult::Ignored => return (false, actions),
            }
            return (true, actions);
        }
        (false, actions)
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        for card in &self.cards {
            let mut chips: Vec<&Chip> = vec![&card.rename, &card.trust, &card.forget];
            chips.extend(card.profiles.iter().map(|(chip, _)| chip));
            if let Some(chip) = chips
                .into_iter()
                .find(|chip| !chip.shortcut.is_empty() && chip.hovered(input))
            {
                return Some(TooltipSpot {
                    key: chip.area,
                    text: chip.shortcut.clone(),
                    node: chip.root,
                    width: chip.width(),
                    height: chip.height(),
                });
            }
        }
        None
    }
}
