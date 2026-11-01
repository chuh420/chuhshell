use platform::MouseButton;
use services::audio::{AudioNode, Direction};
use ui::{
    Node, NodeId, TextEngine, Tree,
    anim::Millis,
    input::{AreaId, Input, InputEvent},
    layout::{Cell, linear},
    widgets::{Chip, Label, Slider},
};

use super::{
    PanelAction, PanelData, TooltipSpot,
    control::SLIDER_STYLE,
    kit::{chip, fit, label, place_chip, place_label, rect},
};
use crate::theme::{GREEN, LINE, MUTED, RED, TEXT};

struct Device {
    id: u32,
    chip: Chip,
}

pub struct AudioPanel {
    pub root: NodeId,
    title: Label,
    mute: Chip,
    output: Label,
    percent: Label,
    volume: Slider,
    sinks: Vec<Device>,
    separator: NodeId,
    microphone: Label,
    mic: Chip,
    mic_volume: Slider,
    sources: Vec<Device>,
    mixer: Chip,
    data: PanelData,
    devices_key: Vec<(u32, String, bool)>,
}

fn device_label(node: &AudioNode, selected: bool) -> String {
    let name = if node.description.is_empty() {
        &node.name
    } else {
        &node.description
    };
    if selected {
        format!("✓  {name}")
    } else {
        name.clone()
    }
}

impl AudioPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "sound", 18.0, TEXT);
        let mut mute = chip(tree, input, engine, root, "󰕾", GREEN, false, None);
        mute.shortcut = "XF86AudioMute".into();
        let output = label(tree, engine, root, "output", 12.0, MUTED);
        let percent = label(tree, engine, root, "", 12.0, TEXT);
        let mut volume = Slider::new(tree, input, root, &SLIDER_STYLE, GREEN, 0.0, 100.0);
        volume.shortcut = "XF86AudioRaiseVolume / XF86AudioLowerVolume".into();
        let separator = rect(tree, root, LINE, 0.0);
        let microphone = label(tree, engine, root, "microphone", 12.0, MUTED);
        let mut mic = chip(tree, input, engine, root, "on", GREEN, false, None);
        mic.shortcut = "XF86AudioMicMute".into();
        let mic_volume = Slider::new(tree, input, root, &SLIDER_STYLE, GREEN, 0.0, 100.0);
        let mixer = chip(tree, input, engine, root, "mixer ↗", TEXT, false, None);
        let mut panel = Self {
            root,
            title,
            mute,
            output,
            percent,
            volume,
            sinks: Vec::new(),
            separator,
            microphone,
            mic,
            mic_volume,
            sources: Vec::new(),
            mixer,
            data: PanelData::default(),
            devices_key: Vec::new(),
        };
        panel.sync(tree, input, engine, data, now);
        panel.volume.jump(tree, input);
        panel.mic_volume.jump(tree, input);
        for chip in [&mut panel.mute, &mut panel.mic] {
            chip.jump_color(input);
        }
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.mute.area,
            self.volume.area,
            self.mic.area,
            self.mic_volume.area,
            self.mixer.area,
        ];
        areas.extend(self.sinks.iter().chain(&self.sources).map(|d| d.chip.area));
        areas
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
        let audio = &data.audio;
        let sink = audio.sink().map(|node| node.id);
        let source = audio.source().map(|node| node.id);
        let muted = data.muted;
        self.mute
            .set_label(engine, if muted { "󰖁" } else { "󰕾" }, 12.0);
        self.mute.set_ink(now, if muted { RED } else { GREEN });
        self.percent.set_text(engine, &format!("{}%", data.volume));
        self.volume.set_value(data.volume as f64);
        let mic = audio.source().filter(|node| node.has_audio);
        let mic_muted = mic.is_some_and(|node| node.muted);
        let mic_on = mic.is_some_and(|node| !node.muted);
        self.mic
            .set_label(engine, if mic_on { "on" } else { "muted" }, 12.0);
        self.mic.set_ink(now, if mic_muted { RED } else { GREEN });
        self.mic_volume
            .set_value(mic.map_or(0.0, |node| node.volume as f64 * 100.0));
        let devices: Vec<&AudioNode> = audio.nodes.iter().filter(|node| node.has_audio).collect();
        let key: Vec<(u32, String, bool)> = devices
            .iter()
            .map(|node| {
                let selected = match node.direction {
                    Direction::Sink => sink == Some(node.id),
                    Direction::Source => source == Some(node.id),
                };
                (
                    node.id,
                    device_label(node, selected),
                    node.direction == Direction::Sink,
                )
            })
            .collect();
        if key != self.devices_key {
            for device in self.sinks.drain(..).chain(self.sources.drain(..)) {
                input.remove(device.chip.area);
                tree.remove(device.chip.root);
            }
            for ((id, text, is_sink), node) in key.iter().zip(&devices) {
                let selected = match node.direction {
                    Direction::Sink => sink == Some(node.id),
                    Direction::Source => source == Some(node.id),
                };
                let mut chip = chip(
                    tree,
                    input,
                    engine,
                    self.root,
                    text,
                    if selected { GREEN } else { TEXT },
                    false,
                    None,
                );
                chip.jump_color(input);
                let device = Device { id: *id, chip };
                if *is_sink {
                    self.sinks.push(device);
                } else {
                    self.sources.push(device);
                }
            }
            self.devices_key = key;
        }
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let h = |label: &Label| label.implicit_height().ceil();
        let chip_h = self.mute.implicit_height().ceil();
        let row1 = h(&self.title).max(chip_h);
        let row2 = h(&self.output).max(h(&self.percent));
        let mic_row = h(&self.microphone).max(self.mic.implicit_height().ceil());
        let device_h = |devices: &[Device]| {
            devices
                .iter()
                .map(|device| Cell::implicit(device.chip.implicit_height()))
                .collect::<Vec<_>>()
        };
        let mut cells = vec![
            Cell::implicit(row1),
            Cell::implicit(row2),
            Cell::layout(32.0),
        ];
        cells.extend(device_h(&self.sinks));
        cells.extend([
            Cell::layout(1.0),
            Cell::implicit(mic_row),
            Cell::layout(32.0),
        ]);
        cells.extend(device_h(&self.sources));
        cells.extend([
            Cell::fill(0.0),
            Cell::implicit(self.mixer.implicit_height()),
        ]);
        let slots = linear(&cells, 12.0, height);
        let mut next = slots.iter().copied();
        let slot = next.next().unwrap_or_default();
        let mw = self.mute.implicit_width().ceil();
        place_label(
            tree,
            &mut self.title,
            0.0,
            slot,
            (width - mw - 5.0).max(0.0),
        );
        place_chip(tree, &mut self.mute, width - mw, slot);
        let slot = next.next().unwrap_or_default();
        let pw = self.percent.implicit_width();
        place_label(tree, &mut self.percent, width - pw, slot, pw);
        place_label(
            tree,
            &mut self.output,
            0.0,
            slot,
            (width - pw - 5.0).max(0.0),
        );
        let slot = next.next().unwrap_or_default();
        self.volume.place(tree, 0.0, slot.pos, width, slot.size);
        for device in &mut self.sinks {
            let slot = next.next().unwrap_or_default();
            device.chip.set_fixed_width(Some(width));
            let (y, h) = fit(slot, device.chip.implicit_height().ceil());
            device.chip.place_sized(tree, 0.0, y, width, h);
        }
        let slot = next.next().unwrap_or_default();
        tree.set_pos(self.separator, 0.0, slot.pos);
        tree.set_size(self.separator, width, 1.0);
        let slot = next.next().unwrap_or_default();
        let cw = self.mic.implicit_width().ceil();
        place_label(
            tree,
            &mut self.microphone,
            0.0,
            slot,
            (width - cw - 5.0).max(0.0),
        );
        place_chip(tree, &mut self.mic, width - cw, slot);
        let slot = next.next().unwrap_or_default();
        self.mic_volume.place(tree, 0.0, slot.pos, width, slot.size);
        for device in &mut self.sources {
            let slot = next.next().unwrap_or_default();
            device.chip.set_fixed_width(Some(width));
            let (y, h) = fit(slot, device.chip.implicit_height().ceil());
            device.chip.place_sized(tree, 0.0, y, width, h);
        }
        let _ = next.next();
        let slot = next.next().unwrap_or_default();
        place_chip(tree, &mut self.mixer, 0.0, slot);
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        for chip in [&mut self.mute, &mut self.mic, &mut self.mixer]
            .into_iter()
            .chain(self.sinks.iter_mut().map(|d| &mut d.chip))
            .chain(self.sources.iter_mut().map(|d| &mut d.chip))
        {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.volume.tick(tree, input, now);
        running |= self.mic_volume.tick(tree, input, now);
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
                InputEvent::Press(area, MouseButton::Left) => {
                    if area == self.volume.area {
                        self.volume.press(tree, pointer.0, pointer.1);
                    } else if area == self.mic_volume.area {
                        self.mic_volume.press(tree, pointer.0, pointer.1);
                    }
                }
                InputEvent::Release(area, MouseButton::Left) => {
                    if area == self.volume.area
                        && let Some(value) = self.volume.release(tree, pointer.0, pointer.1)
                    {
                        actions.push(PanelAction::SetVolume(value));
                    } else if area == self.mic_volume.area
                        && let Some(value) = self.mic_volume.release(tree, pointer.0, pointer.1)
                    {
                        actions.push(PanelAction::SetMicVolume(value));
                    }
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.mute.area {
                        actions.push(PanelAction::ToggleMute);
                    } else if area == self.mic.area {
                        actions.push(PanelAction::ToggleMic);
                    } else if area == self.mixer.area {
                        actions.push(PanelAction::Close);
                        actions.push(PanelAction::Run("audio"));
                    } else if let Some(device) = self
                        .sinks
                        .iter()
                        .chain(&self.sources)
                        .find(|device| device.chip.area == area)
                    {
                        actions.push(PanelAction::SetDefaultNode(device.id));
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn motion(&mut self, tree: &Tree, input: &Input, x: f32, y: f32) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        if input.pressed(self.volume.area)
            && let Some(value) = self.volume.motion(tree, x, y)
        {
            actions.push(PanelAction::SetVolume(value));
        }
        if input.pressed(self.mic_volume.area)
            && let Some(value) = self.mic_volume.motion(tree, x, y)
        {
            actions.push(PanelAction::SetMicVolume(value));
        }
        actions
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        for chip in [&self.mute, &self.mic] {
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
        input.contains_mouse(self.volume.area).then(|| TooltipSpot {
            key: self.volume.area,
            text: self.volume.shortcut.clone(),
            node: self.volume.root,
            width: self.volume.width(),
            height: self.volume.height(),
        })
    }
}
