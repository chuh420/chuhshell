use platform::{KeyEvent, MouseButton, keys};
use ui::{
    Color, Node, NodeId, RectPaint, TextEngine, TransformOp, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, linear},
    widgets::{Chip, Label},
};

use super::{
    PanelAction,
    kit::{chip, icon_label, label, place_chip, rect},
    picker::eyebrow,
};
use crate::theme::{self, FAST, NORMAL, SLOW};

const HOLD: Millis = 1800.0;

struct Action {
    label: &'static str,
    icon: &'static str,
    ink: fn() -> Color,
    confirm: bool,
    session: &'static str,
}

const ACTIONS: [Action; 4] = [
    Action {
        label: "lock",
        icon: "󰌾",
        ink: theme::yellow,
        confirm: false,
        session: "lock",
    },
    Action {
        label: "sleep",
        icon: "󰒲",
        ink: theme::green,
        confirm: false,
        session: "suspend",
    },
    Action {
        label: "restart",
        icon: "󰜉",
        ink: theme::yellow,
        confirm: true,
        session: "reboot",
    },
    Action {
        label: "power off",
        icon: "󰐥",
        ink: theme::red,
        confirm: true,
        session: "poweroff",
    },
];

struct Tile {
    root: NodeId,
    area: AreaId,
    progress: NodeId,
    icon: Label,
    name: Label,
    number: Label,
    color: Tween<Color>,
    border: Tween<Color>,
    scale: Tween<f32>,
    icon_scale: Tween<f32>,
}

pub struct PowerPanel {
    pub root: NodeId,
    eyebrow: Label,
    title: Label,
    close: Chip,
    tiles: Vec<Tile>,
    footer: Label,
    current: usize,
    pending: Option<usize>,
    hold_start: Option<Millis>,
    countdown: f32,
}

pub fn countdown_title(pending: Option<usize>, countdown: f32) -> String {
    match pending {
        Some(index) => format!(
            "{} in {}s",
            ACTIONS[index].label,
            (1.8 * (1.0 - countdown as f64)).ceil() as i64
        ),
        None => "power".into(),
    }
}

impl PowerPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let eyebrow = eyebrow(tree, engine, root, "session / power");
        let title = label(tree, engine, root, "power", 22.0, theme::text());
        let close = chip(tree, input, engine, root, "×", theme::text(), false, None);
        let tiles = ACTIONS
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let root = tree.add(
                    root,
                    Node::rect(
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        RectPaint {
                            color: Color::TRANSPARENT,
                            radius: 16.0,
                            border_color: Color::TRANSPARENT,
                            border_width: 1.0,
                        },
                    ),
                );
                if let Some(node) = tree.get_mut(root) {
                    node.clip = true;
                }
                let area = input.add(MouseArea::new(root));
                let progress = rect(tree, root, (action.ink)(), 0.0);
                tree.set_opacity(progress, 0.16);
                let icon = icon_label(tree, engine, root, action.icon, 30.0, (action.ink)());
                let name = label(tree, engine, root, action.label, 12.0, theme::text());
                let number = icon_label(
                    tree,
                    engine,
                    root,
                    &(index + 1).to_string(),
                    9.0,
                    theme::muted(),
                );
                let active = index == 0;
                Tile {
                    root,
                    area,
                    progress,
                    icon,
                    name,
                    number,
                    color: Tween::new(
                        if active {
                            theme::elevated()
                        } else {
                            Color::TRANSPARENT
                        },
                        FAST,
                        Easing::Linear,
                    ),
                    border: Tween::new(
                        if active {
                            theme::line()
                        } else {
                            Color::TRANSPARENT
                        },
                        NORMAL,
                        Easing::Linear,
                    ),
                    scale: Tween::new(
                        if active { 1.0 } else { 0.97 },
                        NORMAL,
                        Easing::OutBack(1.70158),
                    ),
                    icon_scale: Tween::new(
                        if active { 1.1 } else { 1.0 },
                        SLOW,
                        Easing::OutBack(1.70158),
                    ),
                }
            })
            .collect();
        let footer = label(tree, engine, root, "", 10.0, theme::muted());
        let mut panel = Self {
            root,
            eyebrow,
            title,
            close,
            tiles,
            footer,
            current: 0,
            pending: None,
            hold_start: None,
            countdown: 0.0,
        };
        panel.update_texts(engine);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![self.close.area];
        areas.extend(self.tiles.iter().map(|tile| tile.area));
        areas
    }

    fn update_texts(&mut self, engine: &mut TextEngine) {
        self.title
            .set_text(engine, &countdown_title(self.pending, self.countdown));
        let footer = format!(
            "← →  select    ↵  confirm    1–4  quick pick    esc  {}",
            if self.pending.is_some() {
                "cancel"
            } else {
                "close"
            }
        );
        self.footer.set_text(engine, &footer);
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let text = if self.pending == Some(index) {
                "cancel"
            } else {
                ACTIONS[index].label
            };
            tile.name.set_text(engine, text);
        }
    }

    fn cancel(&mut self) {
        self.hold_start = None;
        self.pending = None;
        self.countdown = 0.0;
    }

    fn activate(&mut self, index: usize, now: Millis, actions: &mut Vec<PanelAction>) {
        self.current = index;
        if self.pending == Some(index) {
            self.cancel();
            return;
        }
        if !ACTIONS[index].confirm {
            self.cancel();
            actions.push(PanelAction::Session(ACTIONS[index].session));
            return;
        }
        self.pending = Some(index);
        self.countdown = 0.0;
        self.hold_start = Some(now);
    }

    pub fn layout(&mut self, tree: &mut Tree, width: f32, height: f32) {
        let eh = self.eyebrow.implicit_height().ceil();
        let th = self.title.implicit_height().ceil();
        let column_h = eh + 4.0 + th;
        let header_h = column_h.max(self.close.implicit_height().ceil());
        let fh = self.footer.implicit_height().ceil();
        let slots = linear(
            &[
                Cell::implicit(header_h),
                Cell::fill(0.0),
                Cell::implicit(fh),
            ],
            14.0,
            height,
        );
        let top = slots[0].pos + (header_h - column_h) / 2.0;
        let ew = self.eyebrow.implicit_width();
        self.eyebrow.place(tree, 0.0, top, ew, eh);
        let tw = self.title.implicit_width();
        self.title.place(tree, 0.0, top + eh + 4.0, tw, th);
        let cw = self.close.implicit_width().ceil();
        place_chip(tree, &mut self.close, width - cw, slots[0]);
        let row = slots[1];
        let columns = linear(&[Cell::fill(0.0); 4], 10.0, width);
        for (tile, column) in self.tiles.iter_mut().zip(columns) {
            tree.set_pos(tile.root, column.pos, row.pos);
            tree.set_size(tile.root, column.size, row.size);
            let ih = tile.icon.implicit_height().ceil();
            let nh = tile.name.implicit_height().ceil();
            let mh = tile.number.implicit_height().ceil();
            let block = ih + 8.0 + nh + 8.0 + mh;
            let y = (row.size - block) / 2.0;
            for (label, ly, lh) in [
                (&mut tile.icon, y, ih),
                (&mut tile.name, y + ih + 8.0, nh),
                (&mut tile.number, y + ih + 8.0 + nh + 8.0, mh),
            ] {
                let lw = label.implicit_width();
                label.place(tree, (column.size - lw) / 2.0, ly, lw, lh);
            }
        }
        let fw = self.footer.implicit_width();
        self.footer.place(tree, 0.0, slots[2].pos, fw, fh);
    }

    pub fn tick(
        &mut self,
        tree: &mut Tree,
        input: &Input,
        engine: &mut TextEngine,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut running = false;
        let mut actions = Vec::new();
        self.close.sync(now, input);
        running |= self.close.tick(tree, now);
        if let (Some(start), Some(index)) = (self.hold_start, self.pending) {
            self.countdown = ((now - start) / HOLD).min(1.0) as f32;
            running = true;
            if self.countdown >= 1.0 {
                self.cancel();
                actions.push(PanelAction::Session(ACTIONS[index].session));
            }
        }
        self.update_texts(engine);
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let active = index == self.current;
            let armed = self.pending == Some(index);
            let hovered = input.contains_mouse(tile.area);
            let pressed = input.pressed(tile.area);
            let color = if active {
                theme::elevated()
            } else if hovered {
                theme::surface()
            } else {
                Color::TRANSPARENT
            };
            let border = if armed {
                (ACTIONS[index].ink)()
            } else if active {
                theme::line()
            } else {
                Color::TRANSPARENT
            };
            let scale = if pressed {
                0.95
            } else if active {
                1.0
            } else {
                0.97
            };
            let icon_scale = if active { 1.1 } else { 1.0 };
            if tile.color.target() != color {
                tile.color.set_target(now, color);
            }
            if tile.border.target() != border {
                tile.border.set_target(now, border);
            }
            if tile.scale.target() != scale {
                tile.scale.set_target(now, scale);
            }
            if tile.icon_scale.target() != icon_scale {
                tile.icon_scale.set_target(now, icon_scale);
            }
            running |= tile.color.tick(now);
            running |= tile.border.tick(now);
            running |= tile.scale.tick(now);
            running |= tile.icon_scale.tick(now);
            tree.set_fill(tile.root, tile.color.value());
            super::kit::set_border(
                tree,
                tile.root,
                tile.border.value(),
                if armed { 1.5 } else { 1.0 },
            );
            let (w, h) = tree
                .get(tile.root)
                .map_or((0.0, 0.0), |node| (node.width, node.height));
            let s = tile.scale.value();
            tree.set_transforms(
                tile.root,
                &[TransformOp::Scale {
                    x: s,
                    y: s,
                    origin_x: w / 2.0,
                    origin_y: h / 2.0,
                }],
            );
            let progress = if armed { h * self.countdown } else { 0.0 };
            tree.set_pos(tile.progress, 0.0, h - progress);
            tree.set_size(tile.progress, w, progress);
            let (iw, ih) = tree
                .get(tile.icon.node)
                .map_or((0.0, 0.0), |node| (node.width, node.height));
            let is = tile.icon_scale.value();
            tree.set_transforms(
                tile.icon.node,
                &[TransformOp::Scale {
                    x: is,
                    y: is,
                    origin_x: iw / 2.0,
                    origin_y: ih / 2.0,
                }],
            );
        }
        (running, actions)
    }

    pub fn handle(&mut self, events: &[InputEvent], now: Millis) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Enter(area) => {
                    if let Some(index) = self.tiles.iter().position(|tile| tile.area == area) {
                        self.current = index;
                    }
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.close.area {
                        actions.push(PanelAction::Close);
                    } else if let Some(index) = self.tiles.iter().position(|tile| tile.area == area)
                    {
                        self.activate(index, now, &mut actions);
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn key(&mut self, event: &KeyEvent, now: Millis) -> (bool, Vec<PanelAction>) {
        let mut actions = Vec::new();
        match event.keysym {
            keys::ESCAPE => {
                if self.pending.is_some() {
                    self.cancel();
                } else {
                    actions.push(PanelAction::Close);
                }
            }
            keys::LEFT => self.current = (self.current + 3) % 4,
            keys::RIGHT | keys::TAB => self.current = (self.current + 1) % 4,
            keys::RETURN | keys::KP_ENTER | 0x20 => {
                let current = self.current;
                self.activate(current, now, &mut actions);
            }
            key @ 0x31..=0x34 => self.activate((key - 0x31) as usize, now, &mut actions),
            _ => return (false, actions),
        }
        (true, actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_text() {
        assert_eq!(countdown_title(None, 0.0), "power");
        assert_eq!(countdown_title(Some(2), 0.0), "restart in 2s");
        assert_eq!(countdown_title(Some(3), 0.5), "power off in 1s");
    }
}
