use std::{collections::HashMap, sync::Arc};

use platform::{
    Damage, Edges, InputRegion, KeyboardMode, Layer, LayerConfig, OutputId, Platform, PointerKind,
    Rect, SurfaceId,
};
use services::notifications::Notification;
use ui::{
    Canvas, Node, NodeId, TextEngine, Tree,
    anim::{Easing, Millis, Tween},
    input::Input,
    tiny_skia::Pixmap,
};

use crate::{
    card::{Card, CardAction},
    theme::{NORMAL, SLOW},
};

const WIDTH: f32 = 380.0;
const SPACING: f32 = 8.0;

pub fn window_width() -> u32 {
    (WIDTH * crate::theme::scale()) as u32
}

pub fn layer_config(output: OutputId, height: u32) -> LayerConfig {
    LayerConfig {
        namespace: "material-notifications".into(),
        layer: Layer::Overlay,
        anchor: Edges {
            top: true,
            bottom: false,
            left: false,
            right: true,
        },
        margin_top: 50,
        margin_right: 14,
        margin_bottom: 0,
        margin_left: 0,
        width: window_width(),
        height,
        exclusive_zone: -1,
        keyboard: KeyboardMode::None,
        input: InputRegion::Full,
        output: Some(output),
    }
}

struct Entry {
    card: Card,
    y: Tween<f32>,
    x: Tween<f32>,
    removing: bool,
}

pub struct Popups {
    pub surface: Option<SurfaceId>,
    pub output: Option<OutputId>,
    size: Option<(u32, u32)>,
    requested: u32,
    canvas: Option<Canvas>,
    tree: Tree,
    input: Input,
    list: NodeId,
    entries: Vec<Entry>,
    content_height: f32,
    pointer: (f32, f32),
    input_rect: Option<Rect>,
    full: bool,
}

impl Popups {
    pub fn new() -> Self {
        let mut tree = Tree::new();
        let root = tree.root();
        let mut list = Node::new(0.0, 0.0, WIDTH, 1.0);
        list.scale = crate::theme::scale();
        list.origin = Some((0.0, 0.0));
        let list = tree.add(root, list);
        Self {
            surface: None,
            output: None,
            size: None,
            requested: 1,
            canvas: None,
            tree,
            input: Input::new(),
            list,
            entries: Vec::new(),
            content_height: 0.0,
            pointer: (0.0, 0.0),
            input_rect: None,
            full: true,
        }
    }

    pub fn owns(&self, surface: SurfaceId) -> bool {
        self.surface == Some(surface)
    }

    pub fn attach(&mut self, surface: SurfaceId, output: OutputId) {
        self.surface = Some(surface);
        self.output = Some(output);
        self.size = None;
        self.canvas = None;
        self.input_rect = None;
        self.full = true;
    }

    pub fn detach(&mut self) -> Option<SurfaceId> {
        self.size = None;
        self.canvas = None;
        self.output = None;
        let entries = std::mem::take(&mut self.entries);
        for entry in entries {
            entry.card.remove(&mut self.tree, &mut self.input);
        }
        self.content_height = 0.0;
        self.surface.take()
    }

    pub fn window_height(&self, screen_height: f32) -> u32 {
        let height = (self.content_height * crate::theme::scale() + 20.0).min(screen_height - 80.0);
        height.max(1.0) as u32
    }

    pub fn configure(&mut self, width: u32, height: u32) {
        self.size = Some((width, height));
        self.full = true;
    }

    pub fn refresh(&mut self, id: u32, now: Millis) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.card.id == id && !entry.removing)
        {
            entry.card.restart_timer(now);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        engine: &mut TextEngine,
        model: &[u32],
        notifications: &[Notification],
        ages: &HashMap<u32, String>,
        icons: &HashMap<u32, Option<Arc<Pixmap>>>,
        now: Millis,
    ) {
        for entry in &mut self.entries {
            if !entry.removing && !model.contains(&entry.card.id) {
                entry.removing = true;
                let current = entry.card.opacity.value();
                entry.card.opacity = Tween::new(current, 80.0, Easing::Linear);
                entry.card.opacity.start(now, current, 0.0);
            }
        }
        for (index, id) in model.iter().enumerate() {
            let Some(notification) = notifications.iter().find(|n| n.id == *id) else {
                continue;
            };
            let age = ages.get(id).map(String::as_str).unwrap_or("");
            let position = self
                .entries
                .iter()
                .position(|entry| entry.card.id == *id && !entry.removing);
            match position {
                Some(position) => {
                    let entry = &mut self.entries[position];
                    entry.card.set_notification(
                        &mut self.tree,
                        &mut self.input,
                        engine,
                        notification,
                        age,
                    );
                }
                None => {
                    let mut card = Card::new(
                        &mut self.tree,
                        &mut self.input,
                        engine,
                        self.list,
                        notification,
                        true,
                        age,
                        now,
                    );
                    card.opacity = Tween::new(0.0, NORMAL, Easing::Linear);
                    card.opacity.start(now, 0.0, 1.0);
                    let mut x = Tween::new(420.0, 460.0, Easing::OutBack(0.9));
                    x.start(now, 420.0, 0.0);
                    let insert_at = self
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| !entry.removing)
                        .nth(index)
                        .map_or(self.entries.len(), |(i, _)| i);
                    self.entries.insert(
                        insert_at,
                        Entry {
                            card,
                            y: Tween::new(f32::NAN, SLOW, Easing::OutCubic),
                            x,
                            removing: false,
                        },
                    );
                }
            }
        }
        for entry in &mut self.entries {
            let icon = icons.get(&entry.card.id).cloned().flatten();
            entry.card.set_icon(&mut self.tree, icon);
        }
        self.layout(engine, now);
    }

    fn layout(&mut self, engine: &mut TextEngine, now: Millis) {
        let mut y = 0.0;
        let mut count = 0;
        for entry in &mut self.entries {
            let height = entry.card.layout(&mut self.tree, engine, WIDTH);
            if entry.removing {
                continue;
            }
            if count > 0 {
                y += SPACING;
            }
            if entry.y.value().is_nan() {
                entry.y.jump(y);
            } else if entry.y.target() != y {
                entry.y.set_target(now, y);
                let x = entry.x.value();
                entry.x = Tween::new(x, SLOW, Easing::OutCubic);
                entry.x.start(now, x, 0.0);
            }
            y += height;
            count += 1;
        }
        self.content_height = y;
        self.tree
            .set_size(self.list, WIDTH, (self.content_height + 10.0).max(1.0));
    }

    pub fn wanted_height(&self, screen_height: f32) -> u32 {
        self.window_height(screen_height)
    }

    pub fn requested(&self) -> u32 {
        self.requested
    }

    pub fn set_requested(&mut self, height: u32) {
        self.requested = height;
    }

    pub fn pointer(
        &mut self,
        kind: PointerKind,
        x: f32,
        y: f32,
        engine: &mut TextEngine,
        now: Millis,
    ) -> Vec<CardAction> {
        self.pointer = (x, y);
        let tree = &self.tree;
        let events = match kind {
            PointerKind::Enter | PointerKind::Motion => self.input.motion(tree, x, y),
            PointerKind::Leave => self.input.leave(tree),
            PointerKind::Press(button) => self.input.press(tree, button, x, y),
            PointerKind::Release(button) => self.input.release(tree, button, x, y),
            PointerKind::Wheel { .. } => Vec::new(),
        };
        let mut actions = Vec::new();
        for entry in &mut self.entries {
            if matches!(kind, PointerKind::Motion) {
                entry.card.motion(&self.input, x);
            }
            actions.extend(entry.card.handle(&self.tree, engine, &events, (x, y), now));
        }
        actions
    }

    pub fn cursor(&self) -> platform::CursorShape {
        self.input.cursor()
    }

    pub fn schedule(&self, platform: &mut Platform) {
        if let Some(surface) = self.surface {
            platform.schedule_frame(surface);
        }
    }

    pub fn draw(
        &mut self,
        platform: &mut Platform,
        engine: &mut TextEngine,
        now: Millis,
    ) -> Vec<CardAction> {
        let mut actions = Vec::new();
        let (Some(surface), Some((width, height))) = (self.surface, self.size) else {
            return actions;
        };
        let mut running = false;
        for entry in &mut self.entries {
            running |= entry.y.tick(now);
            running |= entry.x.tick(now);
            running |= entry
                .card
                .tick(&mut self.tree, &self.input, now, &mut actions);
            let y = entry.y.value();
            self.tree.set_pos(
                entry.card.root,
                entry.x.value(),
                if y.is_nan() { 0.0 } else { y },
            );
        }
        let (gone, kept): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|entry| entry.removing && !entry.card.opacity.is_running());
        self.entries = kept;
        for entry in gone {
            entry.card.remove(&mut self.tree, &mut self.input);
        }
        let events = self.input.refresh(&self.tree);
        if !events.is_empty() {
            let pointer = self.pointer;
            for entry in &mut self.entries {
                actions.extend(entry.card.handle(&self.tree, engine, &events, pointer, now));
            }
        }
        let rect = Rect::new(
            0,
            0,
            (WIDTH * crate::theme::scale()).ceil() as i32,
            ((self.content_height + 10.0).max(1.0) * crate::theme::scale()).ceil() as i32,
        );
        if self.input_rect != Some(rect) {
            platform.set_input_rect(surface, Some(rect));
            self.input_rect = Some(rect);
            self.full = true;
        }
        if self
            .canvas
            .as_ref()
            .is_none_or(|canvas| canvas.width() != width || canvas.height() != height)
        {
            self.canvas = Canvas::new(width, height);
            self.full = true;
        }
        let Some(canvas) = self.canvas.as_mut() else {
            return actions;
        };
        let mut damage = self.tree.collect_damage();
        if self.full {
            damage = Damage::full(width as i32, height as i32);
        }
        if damage.is_empty() {
            if running {
                platform.schedule_frame(surface);
            }
            return actions;
        }
        canvas.render(&mut self.tree, engine, &damage);
        let presented = platform.present(surface, canvas.data(), &damage, running);
        self.full = !presented;
        actions
    }

    pub fn icon_requests(&self) -> Vec<(u32, u32)> {
        self.entries
            .iter()
            .map(|entry| (entry.card.id, entry.card.icon_size()))
            .collect()
    }
}
