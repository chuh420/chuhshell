use platform::{
    Damage, Edges, InputRegion, KeyEvent, KeyboardMode, Layer, LayerConfig, OutputId, Platform,
    PointerKind, PopupPlacement, SurfaceId, keys,
};
use ui::{
    Canvas, Color, Node, NodeId, Paint, RectPaint, TextEngine, TransformOp, Tree,
    anim::{Easing, Millis, Spring, Tween},
    input::{AreaId, Buttons, Input, InputEvent, MouseArea},
};

use crate::{
    bar::TooltipTarget,
    panels::{
        PanelAction, PanelData, control::ControlPanel, notifications::NotificationsPanel,
        tray::TrayPanel,
    },
    theme::{BG, LINE, NORMAL, SLOW, UI_SCALE},
};

pub const ANCHORED: [&str; 8] = [
    "wifi",
    "bluetooth",
    "audio",
    "battery",
    "system",
    "tray",
    "calendar",
    "notifications",
];
pub const CENTERED: [&str; 4] = ["launcher", "wallpapers", "clipboard", "power"];
const MODULE: [&str; 7] = [
    "wifi",
    "bluetooth",
    "audio",
    "battery",
    "system",
    "calendar",
    "notifications",
];

const MARGIN: f32 = 48.0;
const DIM_ANCHORED: Color = Color {
    r: 0x1d as f32 / 255.0,
    g: 0x20 as f32 / 255.0,
    b: 0x21 as f32 / 255.0,
    a: 0x18 as f32 / 255.0,
};
const DIM_DEFAULT: Color = Color {
    a: 0x60 as f32 / 255.0,
    ..DIM_ANCHORED
};

pub fn panel_width(p: &str, available: f64) -> f64 {
    match p {
        "tray" => 293.0,
        "wallpapers" => 1000f64.min(available - 40.0),
        "launcher" | "clipboard" => 720.0,
        "control" => 520.0,
        "power" => 640.0,
        _ => 468.0,
    }
}

pub fn panel_height(p: &str, available: f64, tray_tall: bool) -> f64 {
    let value = match p {
        "battery" => 700.0,
        "system" => 800.0,
        "audio" => 492.0,
        "wifi" => 680.0,
        "bluetooth" => 600.0,
        "tray" => {
            if tray_tall {
                460.0
            } else {
                220.0
            }
        }
        "wallpapers" => 700.0,
        "launcher" | "clipboard" => 650.0,
        "calendar" => 500.0,
        "notifications" => 640.0,
        "power" => 290.0,
        _ => 704.0,
    };
    (available - 70.0).min(value)
}

pub fn panel_x(p: &str, w: f64, available: f64, anchor: f64) -> f64 {
    if CENTERED.contains(&p) {
        return (available - w) / 2.0;
    }
    if ANCHORED.contains(&p) && anchor != 0.0 {
        return (anchor - w / 2.0).max(14.0).min(available - w - 14.0);
    }
    available - w - 14.0
}

pub fn panel_y(p: &str, h: f64, available: f64) -> f64 {
    if p == "power" {
        return (available - h) / 2.0 - 20.0;
    }
    if CENTERED.contains(&p) {
        return 130f64.min((available - h) / 2.0);
    }
    58.0
}

pub fn layer_config(output: OutputId) -> LayerConfig {
    LayerConfig {
        namespace: "material-overlay".into(),
        layer: Layer::Overlay,
        anchor: Edges::ALL,
        margin_top: 0,
        margin_right: 0,
        margin_bottom: 0,
        margin_left: 0,
        width: 0,
        height: 0,
        exclusive_zone: -1,
        keyboard: KeyboardMode::Exclusive,
        input: InputRegion::Full,
        output: Some(output),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reveal {
    Idle,
    Opening,
    Closing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Swap {
    Idle,
    Out,
    In,
}

enum View {
    None,
    Control(Box<ControlPanel>),
    Notifications(Box<NotificationsPanel>),
    Tray(Box<TrayPanel>),
}

pub struct Overlay {
    pub surface: Option<SurfaceId>,
    pub card_surface: Option<SurfaceId>,
    pub output: Option<OutputId>,
    pub keyboard: KeyboardMode,
    size: Option<(f32, f32)>,
    canvas: Option<Canvas>,
    origin: (i32, i32),
    full: bool,
    tree: Tree,
    input: Input,
    offset: NodeId,
    dim: NodeId,
    dim_area: AreaId,
    card: NodeId,
    content: NodeId,
    pub panel: String,
    pub anchor: f64,
    pub tray_tall: bool,
    queued: Vec<PanelAction>,
    pub displayed: String,
    content_panel: String,
    pending: String,
    reveal: f64,
    spring: Spring,
    close: Tween<f64>,
    phase: Reveal,
    morph: bool,
    morph_arm: Option<Millis>,
    x: Tween<f32>,
    y: Tween<f32>,
    width: Tween<f32>,
    height: Tween<f32>,
    radius: Tween<f32>,
    dim_color: Tween<Color>,
    content_opacity: Tween<f32>,
    content_shift: Tween<f32>,
    swap: Swap,
    view: View,
    data: PanelData,
    last_dim: Option<[f32; 4]>,
    pointer: (f32, f32),
}

impl Overlay {
    pub fn new() -> Self {
        let mut tree = Tree::new();
        let mut input = Input::new();
        let root = tree.root();
        let offset = tree.add(root, Node::new(0.0, 0.0, 0.0, 0.0));
        let dim = tree.add(offset, Node::new(0.0, 0.0, 0.0, 0.0));
        let mut dim_mouse = MouseArea::new(dim);
        dim_mouse.hover = false;
        dim_mouse.cursor = platform::CursorShape::Default;
        dim_mouse.buttons = Buttons::LEFT;
        let dim_area = input.add(dim_mouse);
        let mut card_node = Node::rect(
            0.0,
            0.0,
            0.0,
            0.0,
            RectPaint {
                color: BG,
                radius: 22.0,
                border_color: LINE,
                border_width: 1.0,
            },
        );
        card_node.scale = UI_SCALE;
        card_node.origin = Some((0.0, 0.0));
        let card = tree.add(offset, card_node);
        let mut card_mouse = MouseArea::new(card);
        card_mouse.hover = false;
        card_mouse.cursor = platform::CursorShape::Default;
        input.add(card_mouse);
        let content = tree.add(card, Node::new(0.0, 0.0, 0.0, 0.0));
        let mut spring = Spring::new(0.0, 4.2, 0.3);
        spring.epsilon = 0.002;
        Self {
            surface: None,
            card_surface: None,
            output: None,
            keyboard: KeyboardMode::Exclusive,
            size: None,
            canvas: None,
            origin: (0, 0),
            full: true,
            tree,
            input,
            offset,
            dim,
            dim_area,
            card,
            content,
            panel: String::new(),
            anchor: 0.0,
            tray_tall: false,
            queued: Vec::new(),
            displayed: String::new(),
            content_panel: String::new(),
            pending: String::new(),
            reveal: 0.0,
            spring,
            close: Tween::new(0.0, 170.0, Easing::InCubic),
            phase: Reveal::Idle,
            morph: false,
            morph_arm: None,
            x: Tween::new(0.0, SLOW, Easing::OutQuint),
            y: Tween::new(0.0, SLOW, Easing::OutQuint),
            width: Tween::new(0.0, SLOW, Easing::OutQuint),
            height: Tween::new(0.0, SLOW, Easing::OutQuint),
            radius: Tween::new(22.0, SLOW, Easing::Linear),
            dim_color: Tween::new(DIM_DEFAULT, NORMAL, Easing::Linear),
            content_opacity: Tween::new(1.0, 90.0, Easing::InQuad),
            content_shift: Tween::new(0.0, SLOW, Easing::OutCubic),
            swap: Swap::Idle,
            view: View::None,
            data: PanelData::default(),
            last_dim: None,
            pointer: (0.0, 0.0),
        }
    }

    pub fn visible(&self) -> bool {
        !self.displayed.is_empty()
    }

    pub fn owns(&self, surface: SurfaceId) -> bool {
        self.surface == Some(surface)
    }

    pub fn attach(&mut self, surface: SurfaceId, card: Option<SurfaceId>, output: OutputId) {
        self.surface = Some(surface);
        self.card_surface = card;
        self.output = Some(output);
        self.keyboard = KeyboardMode::Exclusive;
        self.size = None;
        self.canvas = None;
        self.origin = (i32::MIN, i32::MIN);
        self.last_dim = None;
        self.full = true;
    }

    pub fn detach(&mut self) -> (Option<SurfaceId>, Option<SurfaceId>) {
        self.size = None;
        self.canvas = None;
        self.output = None;
        let leave = self.input.leave(&self.tree);
        drop(leave);
        (self.card_surface.take(), self.surface.take())
    }

    pub fn panel_changed(&mut self, engine: &mut TextEngine, panel: &str, now: Millis) {
        if self.panel == "tray"
            && panel != "tray"
            && let View::Tray(view) = &mut self.view
        {
            let actions = view.closing();
            self.queued.extend(actions);
        }
        self.panel = panel.to_string();
        if !panel.is_empty() {
            if self.phase == Reveal::Closing {
                self.close.stop();
                self.phase = Reveal::Idle;
            }
            let switching = !self.displayed.is_empty();
            self.morph = switching;
            if !switching {
                self.morph_arm = Some(now + 40.0);
            }
            if switching && self.content_panel != panel {
                self.pending = panel.to_string();
                let current = self.content_opacity.value();
                self.content_opacity = Tween::new(current, 90.0, Easing::InQuad);
                self.content_opacity.start(now, current, 0.0);
                self.swap = Swap::Out;
            } else if !switching {
                self.swap = Swap::Idle;
                self.content_opacity.jump(1.0);
                self.content_shift.jump(0.0);
                self.set_content(engine, panel, now);
            }
            self.displayed = panel.to_string();
            let from = self.reveal;
            self.spring.start(now, from, 1.0);
            self.phase = Reveal::Opening;
        } else {
            self.spring.stop();
            let from = self.reveal;
            self.close.start(now, from, 0.0);
            self.phase = Reveal::Closing;
        }
    }

    fn set_content(&mut self, engine: &mut TextEngine, name: &str, now: Millis) {
        if self.content_panel == name && !matches!(self.view, View::None) {
            return;
        }
        let old = std::mem::replace(&mut self.view, View::None);
        let (areas, root) = match old {
            View::Control(panel) => (panel.areas(), Some(panel.root)),
            View::Notifications(panel) => (panel.areas(), Some(panel.root)),
            View::Tray(panel) => (panel.areas(), Some(panel.root)),
            View::None => (Vec::new(), None),
        };
        for area in areas {
            self.input.remove(area);
        }
        if let Some(root) = root {
            self.tree.remove(root);
        }
        self.content_panel = name.to_string();
        if name == "notifications" {
            let panel = NotificationsPanel::new(
                &mut self.tree,
                &mut self.input,
                engine,
                self.content,
                &self.data,
                now,
            );
            self.view = View::Notifications(Box::new(panel));
        } else if name == "tray" {
            let (panel, actions) = TrayPanel::new(
                &mut self.tree,
                &mut self.input,
                engine,
                self.content,
                &self.data,
            );
            self.queued.extend(actions);
            self.view = View::Tray(Box::new(panel));
        }
        if name == "control" {
            let mut panel = ControlPanel::new(
                &mut self.tree,
                &mut self.input,
                engine,
                self.content,
                &self.data,
            );
            panel.settle(&mut self.tree, &self.input);
            self.view = View::Control(Box::new(panel));
        }
    }

    pub fn set_data(&mut self, engine: &mut TextEngine, data: &PanelData, now: Millis) {
        if self.data == *data {
            return;
        }
        self.data = data.clone();
        match &mut self.view {
            View::Control(panel) => panel.sync(engine, data, now),
            View::Notifications(panel) => {
                panel.sync(&mut self.tree, &mut self.input, engine, data, now)
            }
            View::Tray(panel) => {
                let actions = panel.sync(&mut self.tree, &mut self.input, engine, data);
                self.queued.extend(actions);
            }
            View::None => {}
        }
    }

    pub fn set_icons(
        &mut self,
        icons: &std::collections::HashMap<u32, Option<std::sync::Arc<ui::tiny_skia::Pixmap>>>,
    ) {
        if let View::Notifications(panel) = &mut self.view {
            panel.set_icons(&mut self.tree, icons);
        }
    }

    pub fn icon_requests(&self) -> Vec<(u32, u32)> {
        match &self.view {
            View::Notifications(panel) => panel.icon_requests(),
            _ => Vec::new(),
        }
    }

    pub fn configure(&mut self, width: u32, height: u32) {
        self.size = Some((width as f32, height as f32));
        self.full = true;
    }

    pub fn next_deadline(&self) -> Option<Millis> {
        self.morph_arm
    }

    pub fn finished(&self) -> bool {
        self.displayed.is_empty() && self.phase == Reveal::Idle
    }

    fn tick(&mut self, engine: &mut TextEngine, now: Millis) -> bool {
        let mut running = false;
        if let Some(at) = self.morph_arm
            && at <= now
        {
            self.morph_arm = None;
            if !self.displayed.is_empty() {
                self.morph = true;
            }
        }
        match self.phase {
            Reveal::Opening => {
                running |= self.spring.tick(now);
                self.reveal = self.spring.value();
                if !self.spring.is_running() {
                    self.phase = Reveal::Idle;
                }
            }
            Reveal::Closing => {
                running |= self.close.tick(now);
                self.reveal = self.close.value();
                if !self.close.is_running() {
                    self.phase = Reveal::Idle;
                    self.morph = false;
                    self.displayed.clear();
                    self.set_content(engine, "", now);
                    self.content_panel.clear();
                }
            }
            Reveal::Idle => {}
        }
        match self.swap {
            Swap::Out => {
                running = true;
                self.content_opacity.tick(now);
                if !self.content_opacity.is_running() {
                    let pending = std::mem::take(&mut self.pending);
                    self.set_content(engine, &pending, now);
                    self.content_opacity = Tween::new(0.0, NORMAL, Easing::OutCubic);
                    self.content_opacity.start(now, 0.0, 1.0);
                    self.content_shift = Tween::new(8.0, SLOW, Easing::OutCubic);
                    self.content_shift.start(now, 8.0, 0.0);
                    self.swap = Swap::In;
                }
            }
            Swap::In => {
                running = true;
                self.content_opacity.tick(now);
                self.content_shift.tick(now);
                if !self.content_opacity.is_running() && !self.content_shift.is_running() {
                    self.swap = Swap::Idle;
                }
            }
            Swap::Idle => {}
        }
        running
    }

    fn targets(&self) -> Option<(f32, f32, f32, f32)> {
        let (sw, sh) = self.size?;
        let kind = self.displayed.as_str();
        let vw = panel_width(kind, sw as f64);
        let vh = panel_height(kind, sh as f64, self.tray_tall);
        let x = panel_x(kind, vw, sw as f64, self.anchor);
        let y = panel_y(kind, vh, sh as f64);
        Some((
            x as f32,
            y as f32,
            (vw / UI_SCALE as f64) as f32,
            (vh / UI_SCALE as f64) as f32,
        ))
    }

    fn update_card(&mut self, engine: &mut TextEngine, now: Millis) -> bool {
        let Some((tx, ty, tw, th)) = self.targets() else {
            return false;
        };
        let kind = self.displayed.clone();
        let radius = if ANCHORED.contains(&kind.as_str()) {
            14.0
        } else {
            22.0
        };
        let dim = if ANCHORED.contains(&kind.as_str()) {
            DIM_ANCHORED
        } else {
            DIM_DEFAULT
        };
        self.dim_color.set_target(now, dim);
        if self.morph {
            self.x.set_target(now, tx);
            self.y.set_target(now, ty);
            self.width.set_target(now, tw);
            self.height.set_target(now, th);
            self.radius.set_target(now, radius);
        } else {
            self.x.jump(tx);
            self.y.jump(ty);
            self.width.jump(tw);
            self.height.jump(th);
            self.radius.jump(radius);
        }
        let mut running = self.x.tick(now);
        running |= self.y.tick(now);
        running |= self.width.tick(now);
        running |= self.height.tick(now);
        running |= self.radius.tick(now);
        running |= self.dim_color.tick(now);
        let (x, y, w, h) = (
            self.x.value(),
            self.y.value(),
            self.width.value(),
            self.height.value(),
        );
        let r = self.reveal as f32;
        let anchored = ANCHORED.contains(&kind.as_str()) && self.anchor > 0.0;
        let centered = CENTERED.contains(&kind.as_str());
        let origin_x = if anchored {
            ((self.anchor as f32 - x) / UI_SCALE).clamp(0.0, w.max(0.0))
        } else if centered {
            w / 2.0
        } else {
            w
        };
        let origin_y = if kind == "power" { h / 2.0 } else { 0.0 };
        let (x_scale, y_scale) = if anchored {
            (0.86 + 0.14 * r, 0.8 + 0.2 * r)
        } else if centered {
            (0.95 + 0.05 * r, 0.95 + 0.05 * r)
        } else {
            (0.985 + 0.015 * r, 0.9 + 0.1 * r)
        };
        let transforms = [
            TransformOp::Translate {
                x: 0.0,
                y: if anchored { -10.0 } else { 24.0 } * (1.0 - r),
            },
            TransformOp::Scale {
                x: x_scale,
                y: y_scale,
                origin_x,
                origin_y,
            },
            TransformOp::Rotate {
                angle: if anchored { 0.0 } else { -1.6 * (1.0 - r) },
                origin_x: w - 30.0,
                origin_y: 0.0,
            },
        ];
        let tree = &mut self.tree;
        tree.set_pos(self.card, x, y);
        tree.set_size(self.card, w, h);
        tree.set_opacity(self.card, (r * 2.0).clamp(0.0, 1.0));
        tree.set_transforms(self.card, &transforms);
        let radius = self.radius.value();
        let stale = tree
            .get(self.card)
            .is_some_and(|node| matches!(&node.paint, Paint::Rect(rect) if rect.radius != radius));
        if stale
            && let Some(node) = tree.get_mut(self.card)
            && let Paint::Rect(rect) = &mut node.paint
        {
            rect.radius = radius;
        }
        let module = MODULE.contains(&self.content_panel.as_str());
        let margin = if module {
            24.0
        } else if self.content_panel == "launcher" || self.content_panel == "clipboard" {
            32.0
        } else {
            28.0
        } / UI_SCALE;
        let cw = w - margin * 2.0;
        let ch = h - margin * 2.0;
        tree.set_pos(self.content, margin, margin);
        tree.set_size(self.content, cw, ch);
        tree.set_opacity(self.content, self.content_opacity.value());
        tree.set_transforms(
            self.content,
            &[TransformOp::Translate {
                x: 0.0,
                y: self.content_shift.value(),
            }],
        );
        match &mut self.view {
            View::Control(panel) => panel.layout(tree, cw, ch),
            View::Notifications(panel) => panel.layout(tree, engine, cw, ch),
            View::Tray(panel) => panel.layout(tree, cw, ch),
            View::None => {}
        }
        running
    }

    fn frame_rect(&self) -> Option<(i32, i32, u32, u32)> {
        let (sw, sh) = self.size?;
        let (tx, ty, tw, th) = self.targets()?;
        let current = (
            self.x.value(),
            self.y.value(),
            self.width.value() * UI_SCALE,
            self.height.value() * UI_SCALE,
        );
        let from = if self.x.is_running() || self.width.is_running() {
            (
                current.0.min(tx),
                current.1.min(ty),
                (current.0 + current.2).max(tx + tw * UI_SCALE),
                (current.1 + current.3).max(ty + th * UI_SCALE),
            )
        } else {
            (
                current.0,
                current.1,
                current.0 + current.2,
                current.1 + current.3,
            )
        };
        let left = (from.0 - MARGIN).floor().max(0.0);
        let top = (from.1 - MARGIN).floor().max(0.0);
        let right = (from.2 + MARGIN).ceil().min(sw);
        let bottom = (from.3 + MARGIN).ceil().min(sh);
        if right <= left || bottom <= top {
            return None;
        }
        Some((
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        ))
    }

    pub fn draw(
        &mut self,
        platform: &mut Platform,
        engine: &mut TextEngine,
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = std::mem::take(&mut self.queued);
        let (Some(surface), Some(card_surface)) = (self.surface, self.card_surface) else {
            return actions;
        };
        let Some((sw, sh)) = self.size else {
            return actions;
        };
        let mut running = self.tick(engine, now);
        if self.displayed.is_empty() && self.phase == Reveal::Idle {
            return actions;
        }
        self.tree.set_size(self.dim, sw, sh);
        running |= self.update_card(engine, now);
        match &mut self.view {
            View::Control(panel) => running |= panel.tick(&mut self.tree, &self.input, now),
            View::Notifications(panel) => {
                let (busy, more) = panel.tick(&mut self.tree, &self.input, now);
                running |= busy;
                actions.extend(more);
            }
            View::Tray(panel) => running |= panel.tick(&mut self.tree, &self.input, now),
            View::None => {}
        }
        let events = self.input.refresh(&self.tree);
        if !events.is_empty() {
            actions.extend(self.handle_events(engine, &events, now));
        }
        running |= self.morph_arm.is_some();
        let mut presented = false;
        if let Some((ox, oy, cw, ch)) = self.frame_rect() {
            if self.origin != (ox, oy)
                || self
                    .canvas
                    .as_ref()
                    .is_none_or(|canvas| canvas.width() != cw || canvas.height() != ch)
            {
                platform.set_subsurface_geometry(card_surface, ox, oy, cw, ch);
                if self
                    .canvas
                    .as_ref()
                    .is_none_or(|canvas| canvas.width() != cw || canvas.height() != ch)
                {
                    self.canvas = Canvas::new(cw, ch);
                }
                self.origin = (ox, oy);
                self.tree.set_pos(self.offset, -ox as f32, -oy as f32);
                self.full = true;
            }
            if let Some(canvas) = self.canvas.as_mut() {
                let mut damage = self.tree.collect_damage();
                if self.full {
                    damage = Damage::full(canvas.width() as i32, canvas.height() as i32);
                }
                if !damage.is_empty() {
                    canvas.render(&mut self.tree, engine, &damage);
                    presented = platform.present(card_surface, canvas.data(), &damage, false);
                    self.full = !presented;
                }
            }
        }
        let color = self.dim_color.value();
        let alpha = color.a * (self.reveal as f32).clamp(0.0, 1.0);
        let rgba = [color.r, color.g, color.b, alpha];
        if self.last_dim != Some(rgba) {
            if platform.set_solid(surface, rgba, running) {
                self.last_dim = Some(rgba);
            }
        } else if presented || running {
            platform.commit(surface, running);
        }
        actions
    }

    pub fn schedule(&self, platform: &mut Platform) {
        if let Some(surface) = self.surface {
            platform.schedule_frame(surface);
        }
    }

    fn handle_events(
        &mut self,
        engine: &mut TextEngine,
        events: &[InputEvent],
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            if let InputEvent::Click(area, platform::MouseButton::Left) = event
                && *area == self.dim_area
            {
                actions.push(PanelAction::Close);
            }
        }
        let pointer = self.local(self.pointer);
        match &mut self.view {
            View::Control(panel) => actions.extend(panel.handle(&self.tree, events, pointer)),
            View::Notifications(panel) => {
                actions.extend(panel.handle(&self.tree, engine, events, pointer, now))
            }
            View::Tray(panel) => {
                actions.extend(panel.handle(&mut self.tree, &mut self.input, engine, events))
            }
            View::None => {}
        }
        actions
    }

    fn local(&self, point: (f32, f32)) -> (f32, f32) {
        (
            point.0 - self.origin.0 as f32,
            point.1 - self.origin.1 as f32,
        )
    }

    pub fn pointer(
        &mut self,
        kind: PointerKind,
        x: f32,
        y: f32,
        engine: &mut TextEngine,
        now: Millis,
    ) -> Vec<PanelAction> {
        self.pointer = (x, y);
        let (lx, ly) = self.local((x, y));
        let tree = &self.tree;
        let events = match kind {
            PointerKind::Enter | PointerKind::Motion => self.input.motion(tree, lx, ly),
            PointerKind::Leave => self.input.leave(tree),
            PointerKind::Press(button) => self.input.press(tree, button, lx, ly),
            PointerKind::Release(button) => self.input.release(tree, button, lx, ly),
            PointerKind::Wheel { y: delta, .. } => {
                if delta == 0 {
                    Vec::new()
                } else {
                    self.input.wheel(tree, lx, ly, delta)
                }
            }
        };
        let mut actions = Vec::new();
        if matches!(kind, PointerKind::Motion) {
            match &mut self.view {
                View::Control(panel) => {
                    actions.extend(panel.motion(&self.tree, &self.input, lx, ly))
                }
                View::Notifications(panel) => panel.motion(&self.input, lx),
                View::Tray(_) | View::None => {}
            }
        }
        actions.extend(self.handle_events(engine, &events, now));
        actions
    }

    pub fn key(&mut self, engine: &mut TextEngine, event: &KeyEvent) -> Vec<PanelAction> {
        if !event.pressed || self.panel.is_empty() {
            return Vec::new();
        }
        if event.keysym == keys::ESCAPE {
            return vec![PanelAction::Close];
        }
        match &mut self.view {
            View::Tray(panel) => panel.key(&mut self.tree, &mut self.input, engine, event.keysym),
            _ => Vec::new(),
        }
    }

    pub fn cursor(&self) -> platform::CursorShape {
        self.input.cursor()
    }

    pub fn tooltip(&self) -> Option<TooltipTarget> {
        let spot = match &self.view {
            View::Control(panel) => panel.tooltip(&self.input)?,
            View::Notifications(panel) => panel.tooltip(&self.input)?,
            View::Tray(_) | View::None => return None,
        };
        let transform = self.tree.device_transform(spot.node);
        let mut origin = ui::tiny_skia::Point::from_xy(0.0, 0.0);
        transform.map_point(&mut origin);
        Some(TooltipTarget {
            key: spot.key,
            text: spot.text,
            anchor: PopupPlacement {
                anchor_x: (origin.x + self.origin.0 as f32).floor() as i32,
                anchor_y: (origin.y + self.origin.1 as f32).floor() as i32,
                anchor_width: spot.width.ceil() as i32,
                anchor_height: spot.height.ceil() as i32,
                width: 0,
                height: 0,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_geometry_matches_qml() {
        let w = panel_width("control", 1920.0);
        let h = panel_height("control", 1080.0, false);
        assert_eq!((w, h), (520.0, 704.0));
        assert_eq!(panel_x("control", w, 1920.0, 0.0), 1386.0);
        assert_eq!(panel_y("control", h, 1080.0), 58.0);
    }

    #[test]
    fn anchored_panels_follow_anchor() {
        assert_eq!(panel_x("audio", 468.0, 1920.0, 1700.0), 1438.0);
        assert_eq!(panel_x("audio", 468.0, 1920.0, 100.0), 14.0);
        assert_eq!(panel_x("launcher", 720.0, 1920.0, 0.0), 600.0);
        assert_eq!(panel_y("launcher", 650.0, 1080.0), 130.0);
        assert_eq!(panel_y("power", 290.0, 1080.0), 375.0);
    }
}
