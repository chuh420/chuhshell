use platform::{Damage, Rect};
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Mask, Paint as SkPaint, Path, PathBuilder, Pixmap,
    PixmapPaint, Transform,
};

use crate::{
    ArcPaint, RectPaint,
    shadow::ShadowCache,
    text::TextEngine,
    tree::{CASCADE_SHIFT, CASCADE_WINDOW, NodeId, Paint, Tree, cascade_step},
};

const MARGIN: i32 = 40;
const FLAT: f32 = 1.0 / 4096.0;

pub struct Canvas {
    pixmap: Pixmap,
    scratch: Vec<u8>,
    pool: Vec<Vec<u8>>,
    shadows: ShadowCache,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Some(Self {
            pixmap: Pixmap::new(width.max(1), height.max(1))?,
            scratch: Vec::new(),
            pool: Vec::new(),
            shadows: ShadowCache::default(),
        })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub fn data(&self) -> &[u8] {
        self.pixmap.data()
    }

    pub fn render(&mut self, tree: &mut Tree, engine: &mut TextEngine, damage: &Damage) {
        let full = Rect::new(0, 0, self.width() as i32, self.height() as i32);
        let clip = damage.bounds().intersect(&full);
        if clip.is_empty() {
            return;
        }
        if clip == full {
            self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
            let mut ctx = Context {
                pixmap: &mut self.pixmap,
                engine,
                full,
                masks: Vec::new(),
                pool: &mut self.pool,
                shadows: &mut self.shadows,
            };
            ctx.node(tree, tree.root(), Transform::identity(), 1.0, full);
            ctx.release();
            return;
        }
        let area = Rect::new(
            clip.x - MARGIN,
            clip.y - MARGIN,
            clip.width + MARGIN * 2,
            clip.height + MARGIN * 2,
        )
        .intersect(&full);
        let mut data = std::mem::take(&mut self.scratch);
        data.clear();
        data.resize(area.width as usize * area.height as usize * 4, 0);
        let Some(mut scratch) = tiny_skia::IntSize::from_wh(area.width as u32, area.height as u32)
            .and_then(|size| Pixmap::from_vec(data, size))
        else {
            return;
        };
        let local = Rect::new(0, 0, area.width, area.height);
        let mut ctx = Context {
            pixmap: &mut scratch,
            engine,
            full: local,
            masks: Vec::new(),
            pool: &mut self.pool,
            shadows: &mut self.shadows,
        };
        let origin = Transform::from_translate(-area.x as f32, -area.y as f32);
        ctx.node(tree, tree.root(), origin, 1.0, local);
        ctx.release();
        let stride = self.pixmap.width() as usize * 4;
        let source_stride = area.width as usize * 4;
        let row = clip.width as usize * 4;
        let (dx, dy) = ((clip.x - area.x) as usize, (clip.y - area.y) as usize);
        let source = scratch.data();
        let target = self.pixmap.data_mut();
        for y in 0..clip.height as usize {
            let at = (clip.y as usize + y) * stride + clip.x as usize * 4;
            let from = (dy + y) * source_stride + dx * 4;
            target[at..at + row].copy_from_slice(&source[from..from + row]);
        }
        self.scratch = scratch.take();
    }
}

struct Context<'a> {
    pixmap: &'a mut Pixmap,
    engine: &'a mut TextEngine,
    full: Rect,
    masks: Vec<(Rect, Mask)>,
    pool: &'a mut Vec<Vec<u8>>,
    shadows: &'a mut ShadowCache,
}

const POOLED_MASKS: usize = 4;

fn blank_mask(pool: &mut Vec<Vec<u8>>, width: u32, height: u32) -> Option<Mask> {
    let mut data = pool.pop().unwrap_or_default();
    data.clear();
    data.resize(width as usize * height as usize, 0);
    Mask::from_vec(data, tiny_skia::IntSize::from_wh(width, height)?)
}

fn recycle(pool: &mut Vec<Vec<u8>>, mask: Mask) {
    if pool.len() < POOLED_MASKS {
        pool.push(mask.take());
    }
}

fn to_skia_rect(rect: Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    )
}

impl Context<'_> {
    fn release(&mut self) {
        for (_, mask) in self.masks.drain(..) {
            recycle(self.pool, mask);
        }
    }

    fn prepare_mask(&mut self, clip: Rect) -> bool {
        if clip == self.full {
            return false;
        }
        if self.masks.iter().any(|(rect, _)| *rect == clip) {
            return true;
        }
        let (width, height) = (self.pixmap.width(), self.pixmap.height());
        let area = clip.intersect(&Rect::new(0, 0, width as i32, height as i32));
        let Some(mut mask) = blank_mask(self.pool, width, height) else {
            return false;
        };
        let stride = width as usize;
        let data = mask.data_mut();
        for y in area.y..area.y + area.height {
            let at = y as usize * stride + area.x as usize;
            data[at..at + area.width as usize].fill(255);
        }
        self.masks.push((clip, mask));
        true
    }

    fn node(&mut self, tree: &mut Tree, id: NodeId, parent: Transform, opacity: f32, clip: Rect) {
        let Some(node) = tree.get(id) else {
            return;
        };
        if !node.visible || node.opacity <= 0.0 {
            return;
        }
        let opacity = opacity * node.opacity;
        let transform = parent.pre_concat(node.local_transform());
        let outer = clip;
        let mut clip = clip;
        if node.clip {
            if let Some(bounds) =
                crate::tree::device_bounds(transform, node.width, node.height, -1.0)
            {
                clip = clip.intersect(&Rect::new(
                    bounds.x + 1,
                    bounds.y + 1,
                    bounds.width - 2,
                    bounds.height - 2,
                ));
            }
            if clip.is_empty() {
                return;
            }
        }
        let (width, height) = (node.width, node.height);
        let cascade = node.cascade;
        let count = node.children().len();
        let extent = if matches!(node.paint, Paint::None) {
            None
        } else {
            let pad = if matches!(node.paint, Paint::Path(_)) {
                0
            } else {
                1
            };
            crate::tree::device_bounds(transform, width, height, node.overflow()).map(|rect| {
                Rect::new(
                    rect.x + pad,
                    rect.y + pad,
                    rect.width - pad * 2,
                    rect.height - pad * 2,
                )
            })
        };
        let paints = extent.is_some_and(|extent| !extent.intersect(&outer).is_empty());
        let masked = paints
            && extent.is_some_and(|extent| extent.intersect(&outer) != extent)
            && self.prepare_mask(outer);
        let mask = if masked {
            self.masks
                .iter()
                .find(|(rect, _)| *rect == outer)
                .map(|(_, mask)| mask)
        } else {
            None
        };
        if paints && let Some(node) = tree.get_mut_clean(id) {
            match &mut node.paint {
                Paint::None => {}
                Paint::Rect(paint) => draw_rect(
                    self.pixmap,
                    self.shadows,
                    paint,
                    width,
                    height,
                    transform,
                    opacity,
                    mask,
                ),
                Paint::Text(text) => {
                    let scale = transform_scale(transform);
                    let stale = text
                        .raster
                        .as_ref()
                        .is_none_or(|raster| (raster.scale - scale).abs() > 0.001);
                    if stale {
                        text.raster = self.engine.raster(text.spec(), scale);
                    }
                    if let Some(raster) = &text.raster {
                        let (matrix, quality) = snap(
                            transform
                                .pre_scale(1.0 / raster.scale, 1.0 / raster.scale)
                                .pre_translate(raster.left, raster.top),
                        );
                        draw_image(self.pixmap, &raster.pixmap, matrix, quality, opacity, mask);
                    }
                }
                Paint::Svg(svg) => {
                    let natural = svg.tree.size().to_int_size();
                    let (pw, ph) = (natural.width(), natural.height());
                    if svg
                        .raster
                        .as_ref()
                        .is_none_or(|r| r.width() != pw || r.height() != ph)
                    {
                        svg.raster = crate::image::render_svg(&svg.tree, pw, ph);
                    }
                    if let Some(raster) = &svg.raster {
                        let (matrix, quality) =
                            snap(transform.pre_scale(width / pw as f32, height / ph as f32));
                        draw_image(self.pixmap, raster, matrix, quality, opacity, mask);
                    }
                }
                Paint::Path(path) => draw_path(self.pixmap, path, transform, opacity, mask),
                Paint::Arc(arc) => {
                    draw_arc(self.pixmap, arc, width, height, transform, opacity, mask)
                }
                Paint::Ripple(ripple) => draw_ripple(
                    self.pixmap,
                    self.pool,
                    ripple,
                    width,
                    height,
                    transform,
                    opacity,
                    (outer != self.full).then_some(outer),
                ),
                Paint::Image(image) => {
                    if image.width() > 0 && image.height() > 0 {
                        let (iw, ih) = (image.width() as f32, image.height() as f32);
                        let matrix = transform.pre_scale(width / iw, height / ih);
                        let fits = matrix.kx == 0.0
                            && matrix.ky == 0.0
                            && (matrix.sx * iw - iw).abs() < 1.0
                            && (matrix.sy * ih - ih).abs() < 1.0;
                        let (matrix, quality) = if fits {
                            (
                                Transform::from_translate(matrix.tx.round(), matrix.ty.round()),
                                FilterQuality::Nearest,
                            )
                        } else {
                            (matrix, FilterQuality::Bilinear)
                        };
                        draw_image(self.pixmap, image, matrix, quality, opacity, mask);
                    }
                }
            }
        }
        let child_at = |tree: &Tree, index: usize| {
            tree.get(id)
                .and_then(|node| node.children().get(index).copied())
        };
        if cascade >= 1.0 {
            for index in 0..count {
                if let Some(child) = child_at(tree, index) {
                    self.node(tree, child, transform, opacity, clip);
                }
            }
            return;
        }
        for index in 0..count {
            let Some(child) = child_at(tree, index) else {
                continue;
            };
            let position = tree
                .get(child)
                .map_or(0.0, |node| node.y / height.clamp(1.0, CASCADE_WINDOW));
            let step = cascade_step(cascade, position);
            if step <= 0.0 {
                continue;
            }
            self.node(
                tree,
                child,
                transform.pre_translate(0.0, CASCADE_SHIFT * (1.0 - step)),
                opacity * step,
                clip,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ripple(
    pixmap: &mut Pixmap,
    pool: &mut Vec<Vec<u8>>,
    ripple: &crate::RipplePaint,
    width: f32,
    height: f32,
    transform: Transform,
    opacity: f32,
    clip: Option<Rect>,
) {
    if ripple.radius <= 0.0 || ripple.color.is_transparent() {
        return;
    }
    let Some(shape) = rounded_rect(0.0, 0.0, width, height, ripple.corner) else {
        return;
    };
    let Some(mut mask) = blank_mask(pool, pixmap.width(), pixmap.height()) else {
        return;
    };
    mask.fill_path(&shape, FillRule::Winding, true, transform);
    if let Some(path) = clip.and_then(to_skia_rect).map(PathBuilder::from_rect) {
        mask.intersect_path(&path, FillRule::Winding, false, Transform::identity());
    }
    if let Some(circle) = PathBuilder::from_circle(ripple.x, ripple.y, ripple.radius) {
        fill(
            pixmap,
            &circle,
            ripple.color,
            opacity,
            FillRule::Winding,
            transform,
            Some(&mask),
        );
    }
    recycle(pool, mask);
}

fn arc_path(cx: f32, cy: f32, radius: f32, start: f32, sweep: f32) -> Option<Path> {
    if sweep.abs() < 0.01 || radius <= 0.0 {
        return None;
    }
    let pieces = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
    let step = (sweep / pieces as f32).to_radians();
    let handle = 4.0 / 3.0 * (step / 4.0).tan() * radius;
    let point = |angle: f32| (cx + radius * angle.sin(), cy - radius * angle.cos());
    let mut angle = start.to_radians();
    let mut pb = PathBuilder::new();
    let (x, y) = point(angle);
    pb.move_to(x, y);
    for _ in 0..pieces {
        let next = angle + step;
        let (x0, y0) = point(angle);
        let (x1, y1) = point(next);
        pb.cubic_to(
            x0 + handle * angle.cos(),
            y0 + handle * angle.sin(),
            x1 - handle * next.cos(),
            y1 - handle * next.sin(),
            x1,
            y1,
        );
        angle = next;
    }
    pb.finish()
}

fn draw_arc(
    pixmap: &mut Pixmap,
    arc: &ArcPaint,
    width: f32,
    height: f32,
    transform: Transform,
    opacity: f32,
    mask: Option<&Mask>,
) {
    let radius = (width.min(height) - arc.width) / 2.0;
    let (cx, cy) = (width / 2.0, height / 2.0);
    let stroke = tiny_skia::Stroke {
        width: arc.width,
        line_cap: tiny_skia::LineCap::Round,
        ..tiny_skia::Stroke::default()
    };
    let mut draw = |sweep: f32, color: crate::Color| {
        if color.is_transparent() {
            return;
        }
        if let Some(path) = arc_path(cx, cy, radius, arc.start, sweep) {
            let mut paint = SkPaint::default();
            paint.set_color(color.to_skia(opacity));
            paint.anti_alias = true;
            pixmap.stroke_path(&path, &paint, &stroke, transform, mask);
        }
    };
    draw(arc.span, arc.track);
    draw(arc.sweep.clamp(-arc.span.abs(), arc.span.abs()), arc.color);
}

fn draw_path(
    pixmap: &mut Pixmap,
    path: &crate::PathPaint,
    transform: Transform,
    opacity: f32,
    mask: Option<&Mask>,
) {
    if path.points.len() < 2 {
        return;
    }
    let mut line = PathBuilder::new();
    for (index, (x, y)) in path.points.iter().enumerate() {
        if index == 0 {
            line.move_to(*x, *y);
        } else {
            line.line_to(*x, *y);
        }
    }
    if let Some(line) = line.finish()
        && !path.stroke.is_transparent()
    {
        let mut paint = SkPaint::default();
        paint.set_color(path.stroke.to_skia(opacity));
        paint.anti_alias = true;
        let stroke = tiny_skia::Stroke {
            width: path.width,
            line_join: tiny_skia::LineJoin::Round,
            ..tiny_skia::Stroke::default()
        };
        pixmap.stroke_path(&line, &paint, &stroke, transform, mask);
    }
    if let Some(bottom) = path.fill_to
        && !path.fill.is_transparent()
    {
        let mut area = PathBuilder::new();
        for (index, (x, y)) in path.points.iter().enumerate() {
            if index == 0 {
                area.move_to(*x, *y);
            } else {
                area.line_to(*x, *y);
            }
        }
        if let (Some(first), Some(last)) = (path.points.first(), path.points.last()) {
            area.line_to(last.0, bottom);
            area.line_to(first.0, bottom);
        }
        area.close();
        if let Some(area) = area.finish() {
            fill(
                pixmap,
                &area,
                path.fill,
                opacity,
                FillRule::Winding,
                transform,
                mask,
            );
        }
    }
}

fn draw_image(
    pixmap: &mut Pixmap,
    source: &Pixmap,
    matrix: Transform,
    quality: FilterQuality,
    opacity: f32,
    mask: Option<&Mask>,
) {
    let exact = matches!(quality, FilterQuality::Nearest)
        && matrix.sx == 1.0
        && matrix.sy == 1.0
        && matrix.kx == 0.0
        && matrix.ky == 0.0
        && matrix.tx.fract() == 0.0
        && matrix.ty.fract() == 0.0;
    if exact {
        crate::blit::over(
            pixmap,
            source,
            tiny_skia::IntRect::from_xywh(
                matrix.tx as i32,
                matrix.ty as i32,
                source.width(),
                source.height(),
            ),
            mask,
            (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
        );
        return;
    }
    let paint = PixmapPaint {
        opacity,
        blend_mode: BlendMode::SourceOver,
        quality,
    };
    pixmap.draw_pixmap(0, 0, source.as_ref(), &paint, matrix, mask);
}

fn transform_scale(transform: Transform) -> f32 {
    (transform.sx * transform.sx + transform.ky * transform.ky).sqrt()
}

fn snap(matrix: Transform) -> (Transform, FilterQuality) {
    let axis_aligned = matrix.kx == 0.0
        && matrix.ky == 0.0
        && (matrix.sx - 1.0).abs() < 0.001
        && (matrix.sy - 1.0).abs() < 0.001;
    if axis_aligned {
        (
            Transform::from_translate(matrix.tx.round(), matrix.ty.round()),
            FilterQuality::Nearest,
        )
    } else {
        (matrix, FilterQuality::Bilinear)
    }
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(
            x, y, w, h,
        )?));
    }
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn fill(
    pixmap: &mut Pixmap,
    path: &Path,
    color: crate::Color,
    opacity: f32,
    rule: FillRule,
    transform: Transform,
    mask: Option<&Mask>,
) {
    let flat = |b: tiny_skia::Rect| b.width() <= FLAT || b.height() <= FLAT;
    if path.bounds().transform(transform).is_none_or(flat) {
        return;
    }
    let mut paint = SkPaint::default();
    paint.set_color(color.to_skia(opacity));
    paint.anti_alias = true;
    pixmap.fill_path(path, &paint, rule, transform, mask);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_shadow(
    pixmap: &mut Pixmap,
    rect: &RectPaint,
    shadow: crate::tree::Shadow,
    width: f32,
    height: f32,
    transform: Transform,
    opacity: f32,
    mask: Option<&Mask>,
) {
    if shadow.blur <= 0.0 || shadow.color.is_transparent() {
        return;
    }
    let scale = transform_scale(transform).max(0.01);
    let layers = ((shadow.blur * scale / 2.5).ceil() as usize).clamp(3, 14);
    let hole = rounded_rect(
        1.0,
        1.0,
        width - 2.0,
        height - 2.0,
        (rect.radius - 1.0).max(0.0),
    );
    let total: f32 = (1..=layers).map(|layer| layer as f32).sum();
    for layer in 0..layers {
        let grow = shadow.blur * (layers - layer) as f32 / layers as f32;
        let weight = (layer + 1) as f32 / total;
        let Some(outer) = rounded_rect(
            -grow,
            -grow + shadow.offset,
            width + grow * 2.0,
            height + grow * 2.0,
            rect.radius + grow,
        ) else {
            continue;
        };
        let mut pb = PathBuilder::new();
        pb.push_path(&outer);
        if let Some(hole) = &hole {
            pb.push_path(hole);
        }
        if let Some(ring) = pb.finish() {
            fill(
                pixmap,
                &ring,
                shadow.color.with_alpha(shadow.color.a * weight),
                opacity,
                FillRule::EvenOdd,
                transform,
                mask,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_rect(
    pixmap: &mut Pixmap,
    shadows: &mut ShadowCache,
    rect: &RectPaint,
    width: f32,
    height: f32,
    transform: Transform,
    opacity: f32,
    mask: Option<&Mask>,
) {
    if let Some(shadow) = rect.shadow
        && !shadows.draw(
            pixmap, rect, shadow, width, height, transform, opacity, mask,
        )
    {
        draw_shadow(
            pixmap, rect, shadow, width, height, transform, opacity, mask,
        );
    }
    let border = rect.border_width.min(width / 2.0).min(height / 2.0);
    let has_border = border > 0.0 && !rect.border_color.is_transparent();
    let inset = if has_border { border } else { 0.0 };
    if !rect.color.is_transparent()
        && let Some(path) = rounded_rect(
            inset,
            inset,
            width - inset * 2.0,
            height - inset * 2.0,
            (rect.radius - inset).max(0.0),
        )
    {
        fill(
            pixmap,
            &path,
            rect.color,
            opacity,
            FillRule::Winding,
            transform,
            mask,
        );
    }
    if has_border {
        let mut pb = PathBuilder::new();
        if let Some(outer) = rounded_rect(0.0, 0.0, width, height, rect.radius) {
            pb.push_path(&outer);
        }
        if let Some(inner) = rounded_rect(
            border,
            border,
            width - border * 2.0,
            height - border * 2.0,
            (rect.radius - border).max(0.0),
        ) {
            pb.push_path(&inner);
        }
        if let Some(ring) = pb.finish() {
            fill(
                pixmap,
                &ring,
                rect.border_color,
                opacity,
                FillRule::EvenOdd,
                transform,
                mask,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, Node, tree::NodeId};

    fn solid(r: f32, g: f32, b: f32, a: f32, radius: f32) -> RectPaint {
        RectPaint {
            color: Color { r, g, b, a },
            radius,
            border_color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.5,
            },
            border_width: 1.0,
            ..RectPaint::default()
        }
    }

    fn scene() -> (Tree, NodeId) {
        let mut tree = Tree::new();
        let root = tree.root();
        let card = tree.add(
            root,
            Node::rect(100.5, 80.5, 199.0, 159.0, solid(0.1, 0.1, 0.1, 1.0, 22.0)),
        );
        let mut list = Node::new(10.3, 20.7, 170.0, 100.0);
        list.clip = true;
        let list = tree.add(card, list);
        let content = tree.add(list, Node::new(0.0, 0.0, 170.0, 1000.0));
        for i in 0..40 {
            tree.add(
                content,
                Node::rect(
                    2.0,
                    i as f32 * 25.0,
                    180.0,
                    22.0,
                    solid(0.3, 0.2 + i as f32 / 100.0, 0.4, 0.8, 8.0),
                ),
            );
        }
        (tree, content)
    }

    #[test]
    fn partial_render_matches_full_render() {
        let mut engine = TextEngine::new::<&str>(&[]);
        let (mut tree, content) = scene();
        let mut partial = Canvas::new(500, 400).unwrap_or_else(|| unreachable!());
        partial.render(&mut tree, &mut engine, &Damage::full(500, 400));
        tree.collect_damage();
        for step in 1..8 {
            tree.set_pos(content, 0.0, -(step as f32) * 13.4);
            let damage = tree.collect_damage();
            assert!(!damage.is_empty());
            partial.render(&mut tree, &mut engine, &damage);
            let mut full = Canvas::new(500, 400).unwrap_or_else(|| unreachable!());
            full.render(&mut tree, &mut engine, &Damage::full(500, 400));
            let diffs: Vec<(usize, i32)> = partial
                .data()
                .iter()
                .zip(full.data())
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| (i, *a as i32 - *b as i32))
                .collect();
            let max = diffs.iter().map(|(_, d)| d.abs()).max().unwrap_or(0);
            let first = diffs.first().map(|(i, _)| ((i / 4) % 500, (i / 4) / 500));
            assert!(
                diffs.is_empty(),
                "step {step} count {} max {max} first {first:?} damage {:?}",
                diffs.len(),
                damage.bounds()
            );
        }
    }
}
