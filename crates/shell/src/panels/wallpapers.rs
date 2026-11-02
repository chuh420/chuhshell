use std::{collections::HashMap, sync::Arc};

use platform::{KeyEvent, MouseButton, keys};
use services::wallpapers::WallItem;
use ui::{
    Color, Node, NodeId, Paint, TextEngine, TransformOp, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, linear},
    tiny_skia::Pixmap,
    widgets::{Chip, Label},
};

use super::{
    PanelAction, PanelData,
    kit::{Scroller, chip, icon_label, label, place_chip, place_label, rect, set_border},
};
use crate::{
    cursorctl::CursorData,
    theme::{self, FAST, NORMAL, SLOW, UI_SCALE},
    wallctl::{PreviewKey, WallData},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Wallpapers,
    Cursors,
}

const CACHE_BUFFER: f32 = 320.0;

struct Tile {
    root: NodeId,
    area: AreaId,
    frame: NodeId,
    placeholder: Label,
    badge: Option<(NodeId, Label)>,
    image: NodeId,
    name: Label,
    applied: Label,
    color: Tween<Color>,
    border: Tween<Color>,
    scale: Tween<f32>,
    opacity: Tween<f32>,
    placeholder_opacity: Tween<f32>,
    image_opacity: Tween<f32>,
    image_scale: Tween<f32>,
    applied_opacity: Tween<f32>,
    picture: String,
    pixmap_set: bool,
}

pub struct WallpapersPanel {
    pub root: NodeId,
    title: Label,
    subtitle: Label,
    mode_chip: Chip,
    refresh: Chip,
    close: Chip,
    animation_label: Label,
    animation: Chip,
    size_label: Label,
    size: Chip,
    scheme_label: Label,
    scheme_chip: Chip,
    scheme: &'static str,
    theme_auto: bool,
    grid: Scroller,
    tiles: Vec<Option<Tile>>,
    empty: Label,
    error: Label,
    footer: Label,
    data: Arc<WallData>,
    cursors: Arc<CursorData>,
    mode: Mode,
    previews: Arc<HashMap<PreviewKey, String>>,
    pixmaps: HashMap<String, Option<Arc<Pixmap>>>,
    current: usize,
    width: f32,
    cell_w: f32,
    cell_h: f32,
    grid_h: f32,
    sent: Option<Vec<PreviewKey>>,
    sent_pixmaps: Vec<String>,
    pending_follow: bool,
}

fn scheme_text(scheme: &str) -> String {
    scheme.replace('-', " ")
}

fn cursor_subtitle(count: usize) -> String {
    let noun = if count == 1 { "theme" } else { "themes" };
    format!("{count} {noun} · Pictures/Wallpapers/cursors")
}

fn subtitle(items: &[WallItem]) -> String {
    let live = items.iter().filter(|item| item.live).count();
    format!(
        "{} images · {} live · Pictures/Wallpapers",
        items.len() - live,
        live
    )
}

pub fn move_index(current: usize, count: usize, key: u32) -> usize {
    if count == 0 {
        return current;
    }
    match key {
        keys::RIGHT if current + 1 < count => current + 1,
        keys::LEFT if current > 0 => current - 1,
        keys::DOWN if current + 2 < count => current + 2,
        keys::UP if current >= 2 => current - 2,
        _ => current,
    }
}

impl WallpapersPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "wallpapers", 26.0, theme::text());
        let subtitle = label(tree, engine, root, "", 11.0, theme::muted());
        let mode_chip = chip(
            tree,
            input,
            engine,
            root,
            "cursors",
            theme::text(),
            false,
            None,
        );
        let refresh = chip(
            tree,
            input,
            engine,
            root,
            "refresh",
            theme::text(),
            false,
            None,
        );
        let close = chip(tree, input, engine, root, "×", theme::text(), false, None);
        let animation_label = label(tree, engine, root, "cursor animation", 11.0, theme::muted());
        let animation = chip(
            tree,
            input,
            engine,
            root,
            "off",
            theme::muted(),
            false,
            None,
        );
        let size_label = label(tree, engine, root, "size", 11.0, theme::muted());
        let size = chip(
            tree,
            input,
            engine,
            root,
            "24 px",
            theme::text(),
            false,
            None,
        );
        let scheme_label = label(tree, engine, root, "colors", 11.0, theme::muted());
        let scheme_chip = chip(
            tree,
            input,
            engine,
            root,
            &scheme_text(data.theme_scheme),
            theme::text(),
            false,
            None,
        );
        let grid = Scroller::new(tree, input, root, true);
        let empty = label(
            tree,
            engine,
            grid.view,
            "Add images to Pictures/Wallpapers",
            12.0,
            theme::muted(),
        );
        let error = label(tree, engine, root, "", 11.0, theme::red());
        let footer = label(tree, engine, root, "", 11.0, theme::muted());
        let mut panel = Self {
            root,
            title,
            subtitle,
            mode_chip,
            refresh,
            close,
            animation_label,
            animation,
            size_label,
            size,
            scheme_label,
            scheme_chip,
            scheme: data.theme_scheme,
            theme_auto: data.theme_auto,
            grid,
            tiles: Vec::new(),
            empty,
            error,
            footer,
            data: Arc::new(WallData::default()),
            cursors: data.cursors.clone(),
            mode: Mode::Wallpapers,
            previews: data.previews.clone(),
            pixmaps: HashMap::new(),
            current: 0,
            width: 0.0,
            cell_w: 0.0,
            cell_h: 0.0,
            grid_h: 0.0,
            sent: None,
            sent_pixmaps: Vec::new(),
            pending_follow: false,
        };
        panel.data = data.wall.clone();
        panel.reset(tree, input, engine);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.mode_chip.area,
            self.refresh.area,
            self.close.area,
            self.animation.area,
            self.size.area,
            self.scheme_chip.area,
        ];
        areas.extend(self.grid.areas());
        areas.extend(self.tiles.iter().flatten().map(|tile| tile.area));
        areas
    }

    fn reset(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine) {
        for tile in self.tiles.drain(..).flatten() {
            input.remove(tile.area);
            tree.remove(tile.root);
        }
        self.tiles = (0..self.count()).map(|_| None).collect();
        self.current = match self.mode {
            Mode::Wallpapers => 0,
            Mode::Cursors => self
                .cursors
                .items
                .iter()
                .position(|item| item.theme.name == self.cursors.current)
                .unwrap_or(0),
        };
        self.grid.jump(0.0);
        self.sent = None;
        self.sent_pixmaps.clear();
        let (title, text, other, empty) = match self.mode {
            Mode::Wallpapers => (
                "wallpapers",
                subtitle(&self.data.items),
                "cursors",
                "Add images to Pictures/Wallpapers",
            ),
            Mode::Cursors => (
                "cursors",
                cursor_subtitle(self.cursors.items.len()),
                "wallpapers",
                "Add cursor themes to Pictures/Wallpapers/cursors",
            ),
        };
        self.title.set_text(engine, title);
        self.subtitle.set_text(engine, &text);
        self.mode_chip.set_label(engine, other, 12.0);
        self.empty.set_text(engine, empty);
        self.update_texts(engine);
        self.pending_follow = true;
    }

    fn set_mode(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        mode: Mode,
    ) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.reset(tree, input, engine);
    }

    fn count(&self) -> usize {
        match self.mode {
            Mode::Wallpapers => self.data.items.len(),
            Mode::Cursors => self.cursors.items.len(),
        }
    }

    fn busy(&self) -> bool {
        match self.mode {
            Mode::Wallpapers => self.data.busy,
            Mode::Cursors => self.cursors.busy,
        }
    }

    fn update_texts(&mut self, engine: &mut TextEngine) {
        let footer = match (self.mode, self.busy()) {
            (_, true) => "Applying…",
            (Mode::Wallpapers, false) => {
                "Click a preview to apply  ·  ↑ ↓ ← → navigate  ·  Esc close"
            }
            (Mode::Cursors, false) => {
                "Click a theme to apply  ·  ↑ ↓ ← → navigate  ·  Tab wallpapers  ·  Esc close"
            }
        };
        self.footer.set_text(engine, footer);
        let error = match self.mode {
            Mode::Wallpapers => self.data.error.clone(),
            Mode::Cursors => self.cursors.error.clone(),
        };
        self.error.set_text(engine, &error);
        let on = self.cursors.animation;
        self.animation
            .set_label(engine, if on { "on" } else { "off" }, 12.0);
        self.animation
            .jump_ink(if on { theme::green() } else { theme::muted() });
        self.animation.selected = on;
        let size = format!("{} px", self.cursors.size);
        self.size.set_label(engine, &size, 12.0);
        self.scheme_chip
            .set_label(engine, &scheme_text(self.scheme), 12.0);
        self.scheme_chip.jump_ink(if self.theme_auto {
            theme::text()
        } else {
            theme::muted()
        });
    }

    fn error_text(&self) -> &str {
        match self.mode {
            Mode::Wallpapers => &self.data.error,
            Mode::Cursors => &self.cursors.error,
        }
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
    ) {
        self.previews = data.previews.clone();
        let theme_changed = self.scheme != data.theme_scheme || self.theme_auto != data.theme_auto;
        self.scheme = data.theme_scheme;
        self.theme_auto = data.theme_auto;
        if Arc::ptr_eq(&self.data, &data.wall) && Arc::ptr_eq(&self.cursors, &data.cursors) {
            if theme_changed {
                self.update_texts(engine);
            }
            return;
        }
        let reset = match self.mode {
            Mode::Wallpapers => self.data.generation != data.wall.generation,
            Mode::Cursors => self.cursors.generation != data.cursors.generation,
        };
        self.data = data.wall.clone();
        self.cursors = data.cursors.clone();
        if reset {
            self.reset(tree, input, engine);
        } else {
            self.update_texts(engine);
        }
    }

    pub fn set_pixmaps(
        &mut self,
        tree: &mut Tree,
        pixmaps: &HashMap<String, Option<Arc<Pixmap>>>,
        now: Millis,
    ) {
        self.pixmaps = pixmaps.clone();
        self.apply_pixmaps(tree, now);
    }

    fn apply_pixmaps(&mut self, tree: &mut Tree, now: Millis) {
        for tile in self.tiles.iter_mut().flatten() {
            if tile.pixmap_set || tile.picture.is_empty() {
                continue;
            }
            let Some(Some(pixmap)) = self.pixmaps.get(&tile.picture) else {
                continue;
            };
            tile.pixmap_set = true;
            if let Some(node) = tree.get_mut(tile.image) {
                node.paint = Paint::Image(pixmap.clone());
            }
            tile.image_opacity.start(now, 0.0, 1.0);
            tile.image_scale.start(now, 1.06, 1.0);
            tile.placeholder_opacity.set_target(now, 0.0);
        }
    }

    fn frame_size(&self) -> (f32, f32) {
        let w = (self.cell_w - 10.0 - 14.0).max(0.0);
        (w, w * 9.0 / 16.0)
    }

    pub fn pixmap_requests(&self) -> Vec<(String, u32, u32)> {
        let (fw, fh) = self.frame_size();
        let size = (
            (fw * UI_SCALE).ceil().max(1.0) as u32,
            (fh * UI_SCALE).ceil().max(1.0) as u32,
        );
        self.tiles
            .iter()
            .flatten()
            .filter(|tile| !tile.pixmap_set && !tile.picture.is_empty())
            .map(|tile| (tile.picture.clone(), size.0, size.1))
            .collect()
    }

    fn preview_window(&self) -> Vec<PreviewKey> {
        if self.cell_h <= 0.0 || self.mode == Mode::Cursors {
            return Vec::new();
        }
        let first = ((self.grid.value() / self.cell_h).floor().max(0.0) as usize) * 2;
        self.data
            .items
            .iter()
            .skip(first)
            .take(8)
            .map(|item| (item.path.clone(), item.revision.clone()))
            .collect()
    }

    fn make_tile(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        name: &str,
        badge: Option<&str>,
        now: Millis,
    ) -> Tile {
        let root = rect(tree, parent, theme::surface(), 14.0);
        set_border(tree, root, theme::line(), 1.0);
        let area = input.add(MouseArea::new(root));
        let frame = rect(tree, root, theme::elevated(), 9.0);
        if let Some(node) = tree.get_mut(frame) {
            node.clip = true;
        }
        let placeholder = icon_label(tree, engine, frame, "󰸉", 28.0, theme::line());
        let image = tree.add(frame, Node::new(0.0, 0.0, 0.0, 0.0));
        let badge = badge.map(|badge| {
            let pill = rect(tree, frame, theme::surface(), 0.0);
            tree.set_opacity(pill, 0.92);
            let text = label(tree, engine, pill, badge, 10.0, theme::green());
            (pill, text)
        });
        let name = label(tree, engine, root, name, 12.0, theme::text());
        let applied = label(tree, engine, root, "✓ applied", 10.0, theme::green());
        let mut opacity = Tween::new(0.0, SLOW, Easing::Linear);
        opacity.start(now, 0.0, 1.0);
        Tile {
            root,
            area,
            frame,
            placeholder,
            badge,
            image,
            name,
            applied,
            color: Tween::new(theme::surface(), FAST, Easing::Linear),
            border: Tween::new(theme::line(), NORMAL, Easing::Linear),
            scale: Tween::new(1.0, NORMAL, Easing::OutCubic),
            opacity,
            placeholder_opacity: Tween::new(1.0, NORMAL, Easing::Linear),
            image_opacity: Tween::new(0.0, SLOW, Easing::OutCubic),
            image_scale: Tween::new(1.06, 520.0, Easing::OutCubic),
            applied_opacity: Tween::new(0.0, NORMAL, Easing::Linear),
            picture: String::new(),
            pixmap_set: false,
        }
    }

    fn tile_pos(&self, index: usize) -> (f32, f32) {
        (
            (index % 2) as f32 * self.cell_w,
            (index / 2) as f32 * self.cell_h,
        )
    }

    fn ensure_tiles(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        now: Millis,
    ) {
        if self.cell_h <= 0.0 {
            return;
        }
        let top = self.grid.value() - CACHE_BUFFER;
        let bottom = self.grid.value() + self.grid_height() + CACHE_BUFFER;
        for index in 0..self.tiles.len() {
            let (_, y) = self.tile_pos(index);
            if self.tiles[index].is_some() || y + self.cell_h <= top || y >= bottom {
                continue;
            }
            let (name, badge) = match self.mode {
                Mode::Wallpapers => {
                    let item = &self.data.items[index];
                    (item.name.clone(), item.live.then_some("live"))
                }
                Mode::Cursors => {
                    let item = &self.cursors.items[index];
                    (
                        item.theme.name.clone(),
                        item.theme.animated.then_some("animated"),
                    )
                }
            };
            let tile = Self::make_tile(tree, input, engine, self.grid.content, &name, badge, now);
            self.tiles[index] = Some(tile);
        }
    }

    fn grid_height(&self) -> f32 {
        self.grid_h
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        self.width = width;
        let title_h = self.title.implicit_height().ceil();
        let subtitle_h = self.subtitle.implicit_height().ceil();
        let column_h = title_h + 6.0 + subtitle_h;
        let header_h = column_h
            .max(self.mode_chip.implicit_height().ceil())
            .max(self.refresh.implicit_height().ceil())
            .max(self.close.implicit_height().ceil());
        let error_visible = !self.error_text().is_empty();
        let error_h = if error_visible {
            self.error.wrap(engine, width, 1000)
        } else {
            0.0
        };
        let cursors = self.mode == Mode::Cursors;
        let footer_h = if cursors {
            self.footer
                .implicit_height()
                .ceil()
                .max(self.animation.implicit_height().ceil())
        } else {
            self.footer
                .implicit_height()
                .ceil()
                .max(self.scheme_chip.implicit_height().ceil())
        };
        let mut cells = vec![Cell::implicit(header_h), Cell::fill(0.0)];
        if error_visible {
            cells.push(Cell::implicit(error_h));
        }
        cells.push(Cell::implicit(footer_h));
        let slots = linear(&cells, 14.0, height);
        let header = slots[0];
        let columns = linear(
            &[
                Cell::implicit(
                    self.title
                        .implicit_width()
                        .max(self.subtitle.implicit_width())
                        .ceil(),
                ),
                Cell::fill(0.0),
                Cell::implicit(self.mode_chip.implicit_width()),
                Cell::implicit(self.refresh.implicit_width()),
                Cell::implicit(self.close.implicit_width()),
            ],
            5.0,
            width,
        );
        let top = header.pos + (header.size - column_h) / 2.0;
        let tw = self.title.implicit_width();
        self.title.place(tree, columns[0].pos, top, tw, title_h);
        let sw = self.subtitle.implicit_width();
        self.subtitle
            .place(tree, columns[0].pos, top + title_h + 6.0, sw, subtitle_h);
        place_chip(tree, &mut self.mode_chip, columns[2].pos, header);
        place_chip(tree, &mut self.refresh, columns[3].pos, header);
        place_chip(tree, &mut self.close, columns[4].pos, header);
        let grid = slots[1];
        self.cell_w = width / 2.0;
        self.cell_h = self.cell_w * 9.0 / 16.0 + 54.0;
        self.grid_h = grid.size;
        let rows = self.tiles.len().div_ceil(2);
        let content_h = rows as f32 * self.cell_h;
        self.grid
            .place(tree, 0.0, grid.pos, width, grid.size, content_h);
        if self.pending_follow && self.cell_h > 0.0 {
            self.pending_follow = false;
            self.follow();
        }
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, (width - ew) / 2.0, (grid.size - eh) / 2.0, ew, eh);
        let mut next = 2;
        tree.set_visible(self.error.node, error_visible);
        if error_visible {
            place_label(tree, &mut self.error, 0.0, slots[next], width);
            next += 1;
        }
        let fw = self.footer.implicit_width();
        place_label(tree, &mut self.footer, 0.0, slots[next], fw);
        for node in [
            self.animation_label.node,
            self.animation.root,
            self.size_label.node,
            self.size.root,
        ] {
            tree.set_visible(node, cursors);
        }
        for node in [self.scheme_label.node, self.scheme_chip.root] {
            tree.set_visible(node, !cursors);
        }
        if !cursors {
            let slot = slots[next];
            let x = width - self.scheme_chip.implicit_width().ceil();
            place_chip(tree, &mut self.scheme_chip, x, slot);
            let lw = self.scheme_label.implicit_width();
            place_label(tree, &mut self.scheme_label, x - 8.0 - lw, slot, lw);
        }
        if cursors {
            let slot = slots[next];
            let mut x = width - self.animation.implicit_width().ceil();
            place_chip(tree, &mut self.animation, x, slot);
            let lw = self.animation_label.implicit_width();
            x -= 8.0 + lw;
            place_label(tree, &mut self.animation_label, x, slot, lw);
            x -= 18.0 + self.size.implicit_width().ceil();
            place_chip(tree, &mut self.size, x, slot);
            let sw = self.size_label.implicit_width();
            x -= 8.0 + sw;
            place_label(tree, &mut self.size_label, x, slot, sw);
        }
        self.layout_tiles(tree);
    }

    fn layout_tiles(&mut self, tree: &mut Tree) {
        let tile_w = (self.cell_w - 10.0).max(0.0);
        let tile_h = (self.cell_h - 10.0).max(0.0);
        let (fw, fh) = self.frame_size();
        for index in 0..self.tiles.len() {
            let (x, y) = self.tile_pos(index);
            let Some(tile) = self.tiles[index].as_mut() else {
                continue;
            };
            tree.set_pos(tile.root, x, y);
            tree.set_size(tile.root, tile_w, tile_h);
            tree.set_pos(tile.frame, 7.0, 7.0);
            tree.set_size(tile.frame, fw, fh);
            let pw = tile.placeholder.implicit_width();
            let ph = tile.placeholder.implicit_height().ceil();
            tile.placeholder
                .place(tree, (fw - pw) / 2.0, (fh - ph) / 2.0, pw, ph);
            tree.set_size(tile.image, fw, fh);
            if let Some((pill, text)) = tile.badge.as_mut() {
                let bw = text.implicit_width();
                let bh = text.implicit_height().ceil();
                let (w, h) = (bw + 14.0, bh + 6.0);
                tree.set_pos(*pill, fw - 6.0 - w, 6.0);
                tree.set_size(*pill, w, h);
                if let Some(node) = tree.get_mut(*pill)
                    && let Paint::Rect(paint) = &mut node.paint
                {
                    paint.radius = h / 2.0;
                }
                text.place(tree, (w - bw) / 2.0, (h - bh) / 2.0, bw, bh);
            }
            let nh = tile.name.implicit_height().ceil();
            let ah = tile.applied.implicit_height().ceil();
            let row_h = nh.max(ah);
            let row_y = tile_h - 12.0 - row_h;
            let aw = tile.applied.implicit_width();
            let ax = tile_w - 13.0 - aw;
            tile.applied
                .place(tree, ax, row_y + (row_h - ah) / 2.0, aw, ah);
            let name_w = (ax - 8.0 - 13.0).max(0.0);
            tile.name
                .place(tree, 13.0, row_y + (row_h - nh) / 2.0, name_w, nh);
        }
    }

    fn follow(&mut self) {
        if self.cell_h <= 0.0 {
            return;
        }
        let pos = (self.current / 2) as f32 * self.cell_h;
        let end = pos + self.cell_h;
        let view = self.grid.value();
        if pos < view {
            self.grid.jump(pos);
        } else if end > view + self.grid_h {
            self.grid.jump(end - self.grid_h);
        }
    }

    pub fn tick(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut running = false;
        let mut actions = Vec::new();
        for chip in [
            &mut self.mode_chip,
            &mut self.refresh,
            &mut self.close,
            &mut self.animation,
            &mut self.size,
            &mut self.scheme_chip,
        ] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.grid.tick(tree, input, now);
        self.ensure_tiles(tree, input, engine, now);
        for index in 0..self.tiles.len() {
            let (picture, selected) = match self.mode {
                Mode::Wallpapers => {
                    let item = &self.data.items[index];
                    let picture = self
                        .previews
                        .get(&(item.path.clone(), item.revision.clone()))
                        .cloned()
                        .unwrap_or_default();
                    (picture, self.data.current == item.path)
                }
                Mode::Cursors => {
                    let item = &self.cursors.items[index];
                    (
                        item.picture.clone(),
                        self.cursors.current == item.theme.name,
                    )
                }
            };
            let current = index == self.current;
            let Some(tile) = self.tiles[index].as_mut() else {
                continue;
            };
            if tile.picture != picture {
                tile.picture = picture;
            }
            let hovered = input.contains_mouse(tile.area);
            let pressed = input.pressed(tile.area);
            let color = if hovered {
                theme::elevated()
            } else {
                theme::surface()
            };
            if tile.color.target() != color {
                tile.color.set_target(now, color);
            }
            let border = if selected {
                theme::green()
            } else if current {
                theme::muted()
            } else {
                theme::line()
            };
            if tile.border.target() != border {
                tile.border.set_target(now, border);
            }
            let scale = if pressed {
                0.98
            } else if hovered {
                1.012
            } else {
                1.0
            };
            if tile.scale.target() != scale {
                tile.scale.set_target(now, scale);
            }
            let applied = if selected { 1.0 } else { 0.0 };
            if tile.applied_opacity.target() != applied {
                tile.applied_opacity.set_target(now, applied);
            }
            for tween in [
                &mut tile.scale,
                &mut tile.opacity,
                &mut tile.placeholder_opacity,
                &mut tile.image_opacity,
                &mut tile.image_scale,
                &mut tile.applied_opacity,
            ] {
                running |= tween.tick(now);
            }
            running |= tile.color.tick(now);
            running |= tile.border.tick(now);
            tree.set_fill(tile.root, tile.color.value());
            set_border(
                tree,
                tile.root,
                tile.border.value(),
                if selected { 2.0 } else { 1.0 },
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
            tree.set_opacity(tile.root, tile.opacity.value());
            tree.set_opacity(tile.placeholder.node, tile.placeholder_opacity.value());
            tree.set_opacity(tile.image, tile.image_opacity.value());
            let (iw, ih) = tree
                .get(tile.image)
                .map_or((0.0, 0.0), |node| (node.width, node.height));
            let is = tile.image_scale.value();
            tree.set_transforms(
                tile.image,
                &[TransformOp::Scale {
                    x: is,
                    y: is,
                    origin_x: iw / 2.0,
                    origin_y: ih / 2.0,
                }],
            );
            tree.set_opacity(tile.applied.node, tile.applied_opacity.value());
        }
        self.apply_pixmaps(tree, now);
        tree.set_visible(self.empty.node, self.count() == 0 && !self.busy());
        let window = self.preview_window();
        if self.sent.as_ref() != Some(&window) && self.cell_h > 0.0 {
            self.sent = Some(window.clone());
            actions.push(PanelAction::WallPreviews(window));
        }
        if self.mode == Mode::Cursors {
            let pending: Vec<String> = self
                .pixmap_requests()
                .into_iter()
                .map(|(picture, _, _)| picture)
                .collect();
            if !pending.is_empty() && pending != self.sent_pixmaps {
                actions.push(PanelAction::CursorPixmaps);
            }
            self.sent_pixmaps = pending;
        }
        (running, actions)
    }

    fn apply_at(&self, index: usize, x: f32, y: f32) -> Option<PanelAction> {
        match self.mode {
            Mode::Wallpapers => {
                let item = self.data.items.get(index)?;
                Some(PanelAction::WallApply(item.path.clone(), x, y))
            }
            Mode::Cursors => {
                let item = self.cursors.items.get(index)?;
                Some(PanelAction::CursorApply(item.theme.name.clone()))
            }
        }
    }

    fn toggle_mode(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine) {
        let mode = match self.mode {
            Mode::Wallpapers => Mode::Cursors,
            Mode::Cursors => Mode::Wallpapers,
        };
        self.set_mode(tree, input, engine, mode);
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
        for event in events {
            match *event {
                InputEvent::Wheel(area, delta) if area == self.grid.area => {
                    self.grid.wheel(delta, now);
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.close.area {
                        actions.push(PanelAction::Close);
                    } else if area == self.mode_chip.area {
                        self.toggle_mode(tree, input, engine);
                        actions.push(PanelAction::CursorRefresh);
                    } else if area == self.animation.area && self.mode == Mode::Cursors {
                        actions.push(PanelAction::CursorAnimation);
                    } else if area == self.size.area && self.mode == Mode::Cursors {
                        actions.push(PanelAction::CursorSize);
                    } else if area == self.scheme_chip.area && self.mode == Mode::Wallpapers {
                        actions.push(PanelAction::ThemeScheme);
                    } else if area == self.refresh.area {
                        actions.push(match self.mode {
                            Mode::Wallpapers => PanelAction::WallRefresh,
                            Mode::Cursors => PanelAction::CursorRefresh,
                        });
                    } else if let Some(index) = self
                        .tiles
                        .iter()
                        .position(|tile| tile.as_ref().is_some_and(|tile| tile.area == area))
                    {
                        self.current = index;
                        actions.extend(self.apply_at(index, pointer.0, pointer.1));
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        event: &KeyEvent,
    ) -> (bool, Vec<PanelAction>) {
        let mut actions = Vec::new();
        match event.keysym {
            keys::ESCAPE => actions.push(PanelAction::Close),
            keys::TAB | keys::ISO_LEFT_TAB => {
                self.toggle_mode(tree, input, engine);
                actions.push(PanelAction::CursorRefresh);
            }
            keys::RETURN => {
                if let Some(tile) = self.tiles.get(self.current).and_then(Option::as_ref) {
                    let (w, h) = tree
                        .get(tile.root)
                        .map_or((0.0, 0.0), |node| (node.width, node.height));
                    let transform = tree.device_transform(tile.root);
                    let mut point = ui::tiny_skia::Point::from_xy(w / 2.0, h / 2.0);
                    transform.map_point(&mut point);
                    actions.extend(self.apply_at(self.current, point.x, point.y));
                }
            }
            key @ (keys::LEFT | keys::RIGHT | keys::UP | keys::DOWN) => {
                let next = move_index(self.current, self.count(), key);
                if next != self.current {
                    self.current = next;
                    self.follow();
                }
            }
            _ => return (false, actions),
        }
        (true, actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_keys_follow_qt() {
        assert_eq!(move_index(1, 5, keys::RIGHT), 2);
        assert_eq!(move_index(4, 5, keys::RIGHT), 4);
        assert_eq!(move_index(0, 5, keys::LEFT), 0);
        assert_eq!(move_index(2, 5, keys::DOWN), 4);
        assert_eq!(move_index(3, 5, keys::DOWN), 3);
        assert_eq!(move_index(1, 5, keys::UP), 1);
        assert_eq!(move_index(3, 5, keys::UP), 1);
    }

    #[test]
    fn subtitle_counts() {
        let item = |live| WallItem {
            path: String::new(),
            name: String::new(),
            live,
            revision: String::new(),
        };
        assert_eq!(
            subtitle(&[item(false), item(true), item(false)]),
            "2 images · 1 live · Pictures/Wallpapers"
        );
    }
}
