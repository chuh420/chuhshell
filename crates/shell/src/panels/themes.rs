use std::{collections::HashMap, sync::Arc};

use platform::{KeyEvent, MouseButton, keys};
use ui::{
    Color, Node, NodeId, Paint, TextEngine, TransformOp, Tree,
    anim::{Easing, Millis, Tween},
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, Slot, linear},
    tiny_skia::Pixmap,
    widgets::{Chip, Label},
};

use super::{
    PanelAction, PanelData,
    kit::{Scroller, chip, icon_label, label, place_chip, place_label, rect, set_border},
};
use crate::{
    cursorctl::CursorData,
    theme::{self, BUILTIN, FAST, NORMAL, Palette, SLOW, display_name},
};

const PALETTE_COLUMNS: usize = 5;
const PALETTE_HEIGHT: f32 = 96.0;
const CURSOR_COLUMNS: usize = 4;
const GAP: f32 = 10.0;
const SECTION_GAP: f32 = 18.0;

#[derive(Clone, Debug)]
pub struct ThemesView {
    scroll: f32,
}

struct TileBase {
    root: NodeId,
    area: AreaId,
    frame: NodeId,
    name: Label,
    applied: Label,
    color: Tween<Color>,
    border: Tween<Color>,
    scale: Tween<f32>,
    opacity: Tween<f32>,
    applied_opacity: Tween<f32>,
}

impl TileBase {
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        name: &str,
        applied: &str,
        now: Millis,
    ) -> Self {
        let root = rect(tree, parent, theme::surface(), 14.0);
        set_border(tree, root, theme::line(), 1.0);
        let area = input.add(MouseArea::new(root));
        let frame = rect(tree, root, theme::elevated(), 9.0);
        if let Some(node) = tree.get_mut(frame) {
            node.clip = true;
        }
        let name = label(tree, engine, root, name, 12.0, theme::text());
        let applied = label(tree, engine, root, applied, 10.0, theme::green());
        let mut opacity = Tween::new(0.0, SLOW, Easing::Linear);
        opacity.start(now, 0.0, 1.0);
        Self {
            root,
            area,
            frame,
            name,
            applied,
            color: Tween::new(theme::surface(), FAST, Easing::Linear),
            border: Tween::new(theme::line(), NORMAL, Easing::Linear),
            scale: Tween::new(1.0, NORMAL, Easing::OutCubic),
            opacity,
            applied_opacity: Tween::new(0.0, NORMAL, Easing::Linear),
        }
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32, w: f32, h: f32, frame_h: f32) {
        tree.set_pos(self.root, x, y);
        tree.set_size(self.root, w, h);
        tree.set_pos(self.frame, 7.0, 7.0);
        tree.set_size(self.frame, (w - 14.0).max(0.0), frame_h);
        let nh = self.name.implicit_height().ceil();
        let ah = self.applied.implicit_height().ceil();
        let row_h = nh.max(ah);
        let row_y = 7.0 + frame_h + ((h - 7.0 - frame_h - row_h) / 2.0).max(0.0);
        let aw = self.applied.implicit_width();
        let ax = w - 13.0 - aw;
        self.applied
            .place(tree, ax, row_y + (row_h - ah) / 2.0, aw, ah);
        let name_w = (ax - 8.0 - 13.0).max(0.0);
        self.name
            .place(tree, 13.0, row_y + (row_h - nh) / 2.0, name_w, nh);
    }

    fn tick(&mut self, tree: &mut Tree, input: &Input, selected: bool, now: Millis) -> bool {
        let hovered = input.contains_mouse(self.area);
        let pressed = input.pressed(self.area);
        let color = if hovered {
            theme::elevated()
        } else {
            theme::surface()
        };
        if self.color.target() != color {
            self.color.set_target(now, color);
        }
        let border = if selected {
            theme::green()
        } else if hovered {
            theme::muted()
        } else {
            theme::line()
        };
        if self.border.target() != border {
            self.border.set_target(now, border);
        }
        let scale = if pressed {
            0.98
        } else if hovered {
            1.012
        } else {
            1.0
        };
        if self.scale.target() != scale {
            self.scale.set_target(now, scale);
        }
        let applied = if selected { 1.0 } else { 0.0 };
        if self.applied_opacity.target() != applied {
            self.applied_opacity.set_target(now, applied);
        }
        let mut running = false;
        for tween in [
            &mut self.scale,
            &mut self.opacity,
            &mut self.applied_opacity,
        ] {
            running |= tween.tick(now);
        }
        running |= self.color.tick(now);
        running |= self.border.tick(now);
        tree.set_fill(self.root, self.color.value());
        set_border(
            tree,
            self.root,
            self.border.value(),
            if selected { 2.0 } else { 1.0 },
        );
        let (w, h) = tree
            .get(self.root)
            .map_or((0.0, 0.0), |node| (node.width, node.height));
        let s = self.scale.value();
        tree.set_transforms(
            self.root,
            &[TransformOp::Scale {
                x: s,
                y: s,
                origin_x: w / 2.0,
                origin_y: h / 2.0,
            }],
        );
        tree.set_opacity(self.root, self.opacity.value());
        tree.set_opacity(self.applied.node, self.applied_opacity.value());
        running
    }
}

struct PaletteTile {
    base: TileBase,
    key: Option<&'static str>,
    strip: NodeId,
    dots: Vec<NodeId>,
    mark: Option<Label>,
}

fn swatch(palette: &Palette) -> [Color; 5] {
    [
        palette.green,
        palette.yellow,
        palette.red,
        palette.blue,
        palette.text,
    ]
}

impl PaletteTile {
    #[allow(clippy::too_many_arguments)]
    fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        key: Option<&'static str>,
        name: &str,
        palette: &Palette,
        now: Millis,
    ) -> Self {
        let base = TileBase::new(tree, input, engine, parent, name, "✓", now);
        tree.set_fill(base.frame, palette.bg);
        set_border(tree, base.frame, palette.line, 1.0);
        let strip = rect(tree, base.frame, palette.surface, 0.0);
        let dots = swatch(palette)
            .iter()
            .map(|color| rect(tree, strip, *color, 0.0))
            .collect();
        let mark = key
            .is_none()
            .then(|| icon_label(tree, engine, base.frame, "󰸉", 17.0, palette.green));
        Self {
            base,
            key,
            strip,
            dots,
            mark,
        }
    }

    fn place(&mut self, tree: &mut Tree, x: f32, y: f32, w: f32, h: f32) {
        let frame_h = 48.0;
        self.base.place(tree, x, y, w, h, frame_h);
        let fw = (w - 14.0).max(0.0);
        let dot = 11.0;
        let spacing = 5.0;
        let pad = 9.0;
        let strip_w = pad * 2.0 + self.dots.len() as f32 * (dot + spacing) - spacing;
        let strip_h = 23.0;
        tree.set_pos(self.strip, 9.0, (frame_h - strip_h) / 2.0);
        tree.set_size(self.strip, strip_w, strip_h);
        set_radius(tree, self.strip, strip_h / 2.0);
        for (index, node) in self.dots.iter().enumerate() {
            tree.set_pos(
                *node,
                pad + index as f32 * (dot + spacing),
                (strip_h - dot) / 2.0,
            );
            tree.set_size(*node, dot, dot);
            set_radius(tree, *node, dot / 2.0);
        }
        if let Some(mark) = self.mark.as_mut() {
            let mw = mark.implicit_width();
            let mh = mark.implicit_height().ceil();
            let mx = fw - 12.0 - mw;
            tree.set_visible(mark.node, mx > 9.0 + strip_w + 4.0);
            mark.place(tree, mx, (frame_h - mh) / 2.0, mw, mh);
        }
    }
}

struct CursorTile {
    base: TileBase,
    key: String,
    placeholder: Label,
    image: NodeId,
    badge: Option<(NodeId, Label)>,
    picture: String,
    pixmap_set: bool,
    image_opacity: Tween<f32>,
    placeholder_opacity: Tween<f32>,
}

fn set_radius(tree: &mut Tree, node: NodeId, radius: f32) {
    if let Some(node) = tree.get_mut(node)
        && let Paint::Rect(paint) = &mut node.paint
        && paint.radius != radius
    {
        paint.radius = radius;
    }
}

fn scheme_text(scheme: &str) -> String {
    scheme.replace('-', " ")
}

fn subtitle(cursors: usize) -> String {
    let noun = if cursors == 1 { "theme" } else { "themes" };
    format!(
        "{} palettes · {cursors} cursor {noun} · Pictures/Wallpapers/cursors",
        BUILTIN.len()
    )
}

pub struct ThemesPanel {
    pub root: NodeId,
    title: Label,
    subtitle: Label,
    wallpapers: Chip,
    refresh: Chip,
    close: Chip,
    colors_title: Label,
    colors_hint: Label,
    palettes: Vec<PaletteTile>,
    scheme_label: Label,
    schemes: Vec<(&'static str, Chip)>,
    cursor_title: Label,
    size_label: Label,
    size: Chip,
    match_label: Label,
    match_chip: Chip,
    animation_label: Label,
    animation: Chip,
    grid: Scroller,
    cursor_tiles: Vec<CursorTile>,
    empty: Label,
    error: Label,
    footer: Label,
    cursors: Arc<CursorData>,
    pixmaps: HashMap<String, Option<Arc<Pixmap>>>,
    palette: String,
    auto: bool,
    available: bool,
    scheme: &'static str,
    cell_w: f32,
    cell_h: f32,
    requested: bool,
    sent_pixmaps: Vec<String>,
    pending_scroll: Option<f32>,
}

impl ThemesPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "themes", 26.0, theme::text());
        let subtitle = label(
            tree,
            engine,
            root,
            &subtitle(data.cursors.items.len()),
            11.0,
            theme::muted(),
        );
        let text_chip =
            |tree: &mut Tree, input: &mut Input, engine: &mut TextEngine, text: &str| {
                chip(tree, input, engine, root, text, theme::text(), false, None)
            };
        let wallpapers = text_chip(tree, input, engine, "wallpapers");
        let refresh = text_chip(tree, input, engine, "refresh");
        let close = text_chip(tree, input, engine, "×");
        let colors_title = label(tree, engine, root, "colors", 13.0, theme::text());
        let colors_hint = label(tree, engine, root, "", 11.0, theme::muted());
        let mut palettes = Vec::new();
        if data.theme_available {
            let preview = data
                .theme_auto_preview
                .unwrap_or_else(crate::theme::palette);
            palettes.push(PaletteTile::new(
                tree,
                input,
                engine,
                root,
                None,
                "wallpaper",
                &preview,
                now,
            ));
        }
        for (name, palette) in BUILTIN {
            let shown = if *name == data.theme_palette && !data.theme_auto {
                crate::theme::palette()
            } else {
                *palette
            };
            palettes.push(PaletteTile::new(
                tree,
                input,
                engine,
                root,
                Some(name),
                &display_name(name),
                &shown,
                now,
            ));
        }
        let scheme_label = label(tree, engine, root, "wallpaper scheme", 11.0, theme::muted());
        let schemes = services::theme::SCHEMES
            .iter()
            .map(|scheme| {
                let chip = chip(
                    tree,
                    input,
                    engine,
                    root,
                    &scheme_text(scheme),
                    theme::muted(),
                    false,
                    None,
                );
                (*scheme, chip)
            })
            .collect();
        let cursor_title = label(tree, engine, root, "cursor", 13.0, theme::text());
        let size_label = label(tree, engine, root, "size", 11.0, theme::muted());
        let size = text_chip(tree, input, engine, "24 px");
        let match_label = label(tree, engine, root, "match colors", 11.0, theme::muted());
        let match_chip = text_chip(tree, input, engine, "off");
        let animation_label = label(tree, engine, root, "animation", 11.0, theme::muted());
        let animation = text_chip(tree, input, engine, "off");
        let grid = Scroller::new(tree, input, root, true);
        let empty = label(
            tree,
            engine,
            grid.view,
            "Add cursor themes to Pictures/Wallpapers/cursors",
            12.0,
            theme::muted(),
        );
        let error = label(tree, engine, root, "", 11.0, theme::red());
        let footer = label(tree, engine, root, "", 11.0, theme::muted());
        let mut panel = Self {
            root,
            title,
            subtitle,
            wallpapers,
            refresh,
            close,
            colors_title,
            colors_hint,
            palettes,
            scheme_label,
            schemes,
            cursor_title,
            size_label,
            size,
            match_label,
            match_chip,
            animation_label,
            animation,
            grid,
            cursor_tiles: Vec::new(),
            empty,
            error,
            footer,
            cursors: Arc::new(CursorData::default()),
            pixmaps: HashMap::new(),
            palette: data.theme_palette.clone(),
            auto: data.theme_auto,
            available: data.theme_available,
            scheme: data.theme_scheme,
            cell_w: 0.0,
            cell_h: 0.0,
            requested: false,
            sent_pixmaps: Vec::new(),
            pending_scroll: None,
        };
        panel.set_cursors(tree, input, engine, data.cursors.clone(), now);
        panel
    }

    pub fn view(&self) -> ThemesView {
        ThemesView {
            scroll: self.grid.value(),
        }
    }

    pub fn restore(&mut self, view: ThemesView) {
        self.requested = true;
        self.pending_scroll = Some(view.scroll);
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.wallpapers.area,
            self.refresh.area,
            self.close.area,
            self.size.area,
            self.match_chip.area,
            self.animation.area,
        ];
        areas.extend(self.schemes.iter().map(|(_, chip)| chip.area));
        areas.extend(self.palettes.iter().map(|tile| tile.base.area));
        areas.extend(self.grid.areas());
        areas.extend(self.cursor_tiles.iter().map(|tile| tile.base.area));
        areas
    }

    fn set_cursors(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        cursors: Arc<CursorData>,
        now: Millis,
    ) {
        let keys: Vec<&str> = cursors
            .items
            .iter()
            .map(|item| item.theme.name.as_str())
            .collect();
        let same = self.cursor_tiles.len() == keys.len()
            && self
                .cursor_tiles
                .iter()
                .zip(&keys)
                .all(|(tile, key)| tile.key == *key);
        if !same {
            for tile in self.cursor_tiles.drain(..) {
                input.remove(tile.base.area);
                tree.remove(tile.base.root);
            }
            for item in &cursors.items {
                let base = TileBase::new(
                    tree,
                    input,
                    engine,
                    self.grid.content,
                    &item.theme.name,
                    "✓ applied",
                    now,
                );
                let placeholder = icon_label(tree, engine, base.frame, "󰇀", 28.0, theme::line());
                let image = tree.add(base.frame, Node::new(0.0, 0.0, 0.0, 0.0));
                let badge = item.theme.animated.then(|| {
                    let pill = rect(tree, base.frame, theme::surface(), 0.0);
                    tree.set_opacity(pill, 0.92);
                    let text = label(tree, engine, pill, "animated", 10.0, theme::green());
                    (pill, text)
                });
                self.cursor_tiles.push(CursorTile {
                    base,
                    key: item.theme.name.clone(),
                    placeholder,
                    image,
                    badge,
                    picture: item.picture.clone(),
                    pixmap_set: false,
                    image_opacity: Tween::new(0.0, SLOW, Easing::OutCubic),
                    placeholder_opacity: Tween::new(1.0, NORMAL, Easing::Linear),
                });
            }
            self.sent_pixmaps.clear();
        } else {
            for (tile, item) in self.cursor_tiles.iter_mut().zip(&cursors.items) {
                if tile.picture != item.picture {
                    tile.picture = item.picture.clone();
                    tile.pixmap_set = false;
                }
            }
        }
        self.cursors = cursors;
        self.subtitle
            .set_text(engine, &subtitle(self.cursors.items.len()));
        self.update_texts(engine);
        self.apply_pixmaps(tree, now);
    }

    fn update_texts(&mut self, engine: &mut TextEngine) {
        let hint = if self.auto {
            "following the wallpaper".to_string()
        } else {
            display_name(&self.palette)
        };
        self.colors_hint.set_text(engine, &hint);
        for (scheme, chip) in &mut self.schemes {
            let active = *scheme == self.scheme;
            chip.selected = active;
            chip.jump_ink(match (active, self.auto) {
                (true, true) => theme::green(),
                (true, false) | (false, true) => theme::text(),
                (false, false) => theme::muted(),
            });
        }
        let size = format!("{} px", self.cursors.size);
        self.size.set_label(engine, &size, 12.0);
        let colored = self.cursors.colors;
        self.match_chip
            .set_label(engine, if colored { "on" } else { "off" }, 12.0);
        self.match_chip.jump_ink(if colored {
            theme::green()
        } else {
            theme::muted()
        });
        self.match_chip.selected = colored;
        let on = self.cursors.animation;
        self.animation
            .set_label(engine, if on { "on" } else { "off" }, 12.0);
        self.animation
            .jump_ink(if on { theme::green() } else { theme::muted() });
        self.animation.selected = on;
        let footer = if self.cursors.busy {
            "Applying…"
        } else {
            "Click a palette or a cursor to apply  ·  Tab wallpapers  ·  Esc close"
        };
        self.footer.set_text(engine, footer);
        self.error.set_text(engine, &self.cursors.error);
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
        now: Millis,
    ) {
        let changed = self.palette != data.theme_palette
            || self.auto != data.theme_auto
            || self.scheme != data.theme_scheme;
        self.palette = data.theme_palette.clone();
        self.auto = data.theme_auto;
        self.scheme = data.theme_scheme;
        if !Arc::ptr_eq(&self.cursors, &data.cursors) {
            self.set_cursors(tree, input, engine, data.cursors.clone(), now);
        } else if changed {
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
        for tile in &mut self.cursor_tiles {
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
            tile.placeholder_opacity.set_target(now, 0.0);
        }
    }

    fn frame_size(&self) -> (f32, f32) {
        let w = (self.cell_w - GAP - 14.0).max(0.0);
        (w, w * 9.0 / 16.0)
    }

    pub fn pixmap_requests(&self) -> Vec<(String, u32, u32)> {
        let (fw, fh) = self.frame_size();
        if fw <= 0.0 {
            return Vec::new();
        }
        let size = (
            (fw * theme::scale()).ceil().max(1.0) as u32,
            (fh * theme::scale()).ceil().max(1.0) as u32,
        );
        self.cursor_tiles
            .iter()
            .filter(|tile| !tile.pixmap_set && !tile.picture.is_empty())
            .map(|tile| (tile.picture.clone(), size.0, size.1))
            .collect()
    }

    fn chip_row(tree: &mut Tree, items: &mut [(&mut Label, &mut Chip)], right: f32, slot: Slot) {
        let mut x = right;
        for (label, chip) in items.iter_mut().rev() {
            x -= chip.implicit_width().ceil();
            place_chip(tree, chip, x, slot);
            let lw = label.implicit_width();
            x -= 8.0 + lw;
            place_label(tree, label, x, slot, lw);
            x -= 18.0;
        }
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        let title_h = self.title.implicit_height().ceil();
        let subtitle_h = self.subtitle.implicit_height().ceil();
        let column_h = title_h + 6.0 + subtitle_h;
        let header_h = column_h
            .max(self.wallpapers.implicit_height().ceil())
            .max(self.close.implicit_height().ceil());
        let row_h = self
            .size
            .implicit_height()
            .ceil()
            .max(self.colors_title.implicit_height().ceil());
        let palette_rows = self.palettes.len().div_ceil(PALETTE_COLUMNS);
        let palettes_h = palette_rows as f32 * (PALETTE_HEIGHT + GAP) - GAP;
        let schemes_shown = self.available;
        let error_visible = !self.cursors.error.is_empty();
        let error_h = if error_visible {
            self.error.wrap(engine, width, 1000)
        } else {
            0.0
        };
        let mut cells = vec![
            Cell::implicit(header_h),
            Cell::implicit(row_h),
            Cell::implicit(palettes_h.max(0.0)),
        ];
        if schemes_shown {
            cells.push(Cell::implicit(row_h));
        }
        cells.push(Cell::implicit(row_h));
        cells.push(Cell::fill(0.0));
        if error_visible {
            cells.push(Cell::implicit(error_h));
        }
        cells.push(Cell::implicit(self.footer.implicit_height().ceil()));
        let slots = linear(&cells, 12.0, height);
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
                Cell::implicit(self.wallpapers.implicit_width()),
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
        place_chip(tree, &mut self.wallpapers, columns[2].pos, header);
        place_chip(tree, &mut self.refresh, columns[3].pos, header);
        place_chip(tree, &mut self.close, columns[4].pos, header);

        let colors_row = Slot {
            pos: slots[1].pos + SECTION_GAP / 2.0,
            size: slots[1].size,
        };
        let cw = self.colors_title.implicit_width();
        place_label(tree, &mut self.colors_title, 0.0, colors_row, cw);
        let hw = self.colors_hint.implicit_width();
        place_label(tree, &mut self.colors_hint, cw + 10.0, colors_row, hw);

        let tile_w = (width - GAP * (PALETTE_COLUMNS as f32 - 1.0)) / PALETTE_COLUMNS as f32;
        let base_y = slots[2].pos + SECTION_GAP / 2.0;
        for (index, tile) in self.palettes.iter_mut().enumerate() {
            let column = (index % PALETTE_COLUMNS) as f32;
            let row = (index / PALETTE_COLUMNS) as f32;
            tile.place(
                tree,
                column * (tile_w + GAP),
                base_y + row * (PALETTE_HEIGHT + GAP),
                tile_w,
                PALETTE_HEIGHT,
            );
        }

        let mut next = 3;
        tree.set_visible(self.scheme_label.node, schemes_shown);
        for (_, chip) in &self.schemes {
            tree.set_visible(chip.root, schemes_shown);
        }
        if schemes_shown {
            let slot = Slot {
                pos: slots[next].pos + SECTION_GAP / 2.0,
                size: slots[next].size,
            };
            let lw = self.scheme_label.implicit_width();
            place_label(tree, &mut self.scheme_label, 0.0, slot, lw);
            let mut x = lw + 12.0;
            for (_, chip) in &mut self.schemes {
                place_chip(tree, chip, x, slot);
                x += chip.implicit_width().ceil() + 5.0;
            }
            next += 1;
        }

        let cursor_row = Slot {
            pos: slots[next].pos + SECTION_GAP,
            size: slots[next].size,
        };
        let ctw = self.cursor_title.implicit_width();
        place_label(tree, &mut self.cursor_title, 0.0, cursor_row, ctw);
        let animation_shown = services::compositor::is_hyprland();
        tree.set_visible(self.animation_label.node, animation_shown);
        tree.set_visible(self.animation.root, animation_shown);
        {
            let mut items: Vec<(&mut Label, &mut Chip)> = vec![
                (&mut self.size_label, &mut self.size),
                (&mut self.match_label, &mut self.match_chip),
            ];
            if animation_shown {
                items.push((&mut self.animation_label, &mut self.animation));
            }
            Self::chip_row(tree, &mut items, width, cursor_row);
        }
        next += 1;

        let grid_top = slots[next].pos + SECTION_GAP + 6.0;
        let grid_h = (slots[next].pos + slots[next].size - grid_top).max(0.0);
        self.cell_w = (width + GAP) / CURSOR_COLUMNS as f32;
        let (_, fh) = self.frame_size();
        self.cell_h = fh + 54.0;
        let rows = self.cursor_tiles.len().div_ceil(CURSOR_COLUMNS);
        let content_h = (rows as f32 * self.cell_h - GAP).max(0.0);
        self.grid
            .place(tree, 0.0, grid_top, width + GAP, grid_h, content_h);
        if let Some(scroll) = self.pending_scroll.take() {
            self.grid.jump(scroll);
        }
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, (width - ew) / 2.0, (grid_h - eh) / 2.0, ew, eh);
        self.layout_cursors(tree);
        next += 1;

        tree.set_visible(self.error.node, error_visible);
        if error_visible {
            place_label(tree, &mut self.error, 0.0, slots[next], width);
            next += 1;
        }
        let fw = self.footer.implicit_width();
        place_label(tree, &mut self.footer, 0.0, slots[next], fw);
    }

    fn layout_cursors(&mut self, tree: &mut Tree) {
        let tile_w = (self.cell_w - GAP).max(0.0);
        let tile_h = (self.cell_h - GAP).max(0.0);
        let (fw, fh) = self.frame_size();
        for (index, tile) in self.cursor_tiles.iter_mut().enumerate() {
            let x = (index % CURSOR_COLUMNS) as f32 * self.cell_w;
            let y = (index / CURSOR_COLUMNS) as f32 * self.cell_h;
            tile.base.place(tree, x, y, tile_w, tile_h, fh);
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
                set_radius(tree, *pill, h / 2.0);
                text.place(tree, (w - bw) / 2.0, (h - bh) / 2.0, bw, bh);
            }
        }
    }

    pub fn tick(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut running = false;
        let mut actions = Vec::new();
        if !self.requested {
            self.requested = true;
            actions.push(PanelAction::CursorRefresh);
        }
        for chip in [
            &mut self.wallpapers,
            &mut self.refresh,
            &mut self.close,
            &mut self.size,
            &mut self.match_chip,
            &mut self.animation,
        ] {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        for (_, chip) in &mut self.schemes {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        running |= self.grid.tick(tree, input, now);
        for tile in &mut self.palettes {
            let selected = match tile.key {
                None => self.auto,
                Some(key) => !self.auto && key == self.palette,
            };
            running |= tile.base.tick(tree, input, selected, now);
        }
        for tile in &mut self.cursor_tiles {
            let selected = self.cursors.current == tile.key;
            running |= tile.base.tick(tree, input, selected, now);
            running |= tile.image_opacity.tick(now);
            running |= tile.placeholder_opacity.tick(now);
            tree.set_opacity(tile.image, tile.image_opacity.value());
            tree.set_opacity(tile.placeholder.node, tile.placeholder_opacity.value());
        }
        self.apply_pixmaps(tree, now);
        tree.set_visible(
            self.empty.node,
            self.cursor_tiles.is_empty() && !self.cursors.busy,
        );
        let pending: Vec<String> = self
            .pixmap_requests()
            .into_iter()
            .map(|(picture, _, _)| picture)
            .collect();
        if !pending.is_empty() && pending != self.sent_pixmaps {
            actions.push(PanelAction::CursorPixmaps);
        }
        self.sent_pixmaps = pending;
        (running, actions)
    }

    pub fn handle(&mut self, events: &[InputEvent], now: Millis) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        for event in events {
            match *event {
                InputEvent::Wheel(area, delta) if area == self.grid.area => {
                    self.grid.wheel(delta, now);
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.close.area {
                        actions.push(PanelAction::Close);
                    } else if area == self.wallpapers.area {
                        actions.push(PanelAction::Toggle("wallpapers"));
                    } else if area == self.refresh.area {
                        actions.push(PanelAction::CursorRefresh);
                    } else if area == self.size.area {
                        actions.push(PanelAction::CursorSize);
                    } else if area == self.match_chip.area {
                        actions.push(PanelAction::CursorColors);
                    } else if area == self.animation.area && services::compositor::is_hyprland() {
                        actions.push(PanelAction::CursorAnimation);
                    } else if let Some((scheme, _)) =
                        self.schemes.iter().find(|(_, chip)| chip.area == area)
                    {
                        actions.push(PanelAction::ThemeScheme(scheme));
                    } else if let Some(tile) =
                        self.palettes.iter().find(|tile| tile.base.area == area)
                    {
                        actions.push(match tile.key {
                            None => PanelAction::ThemeAuto,
                            Some(key) => PanelAction::ThemePalette(key.to_string()),
                        });
                    } else if let Some(tile) =
                        self.cursor_tiles.iter().find(|tile| tile.base.area == area)
                    {
                        actions.push(PanelAction::CursorApply(tile.key.clone()));
                    }
                }
                _ => {}
            }
        }
        actions
    }

    pub fn key(&mut self, event: &KeyEvent) -> (bool, Vec<PanelAction>) {
        match event.keysym {
            keys::ESCAPE => (true, vec![PanelAction::Close]),
            keys::TAB | keys::ISO_LEFT_TAB => (true, vec![PanelAction::Toggle("wallpapers")]),
            _ => (false, Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_counts_palettes_and_cursors() {
        assert_eq!(
            subtitle(1),
            format!(
                "{} palettes · 1 cursor theme · Pictures/Wallpapers/cursors",
                BUILTIN.len()
            )
        );
        assert!(subtitle(3).contains("3 cursor themes"));
        assert_eq!(scheme_text("tonal-spot"), "tonal spot");
    }
}
