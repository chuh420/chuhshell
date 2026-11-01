use platform::{Damage, Rect};
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Mask, Paint as SkPaint, Path, PathBuilder, Pixmap,
    PixmapPaint, Transform,
};

use crate::{
    RectPaint,
    text::TextEngine,
    tree::{NodeId, Paint, Tree},
};

const MARGIN: i32 = 40;

pub struct Canvas {
    pixmap: Pixmap,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Some(Self {
            pixmap: Pixmap::new(width.max(1), height.max(1))?,
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
            };
            ctx.node(tree, tree.root(), Transform::identity(), 1.0, full);
            return;
        }
        let area = Rect::new(
            clip.x - MARGIN,
            clip.y - MARGIN,
            clip.width + MARGIN * 2,
            clip.height + MARGIN * 2,
        )
        .intersect(&full);
        let Some(mut scratch) = Pixmap::new(area.width as u32, area.height as u32) else {
            return;
        };
        let local = Rect::new(0, 0, area.width, area.height);
        let mut ctx = Context {
            pixmap: &mut scratch,
            engine,
            full: local,
            masks: Vec::new(),
        };
        let origin = Transform::from_translate(-area.x as f32, -area.y as f32);
        ctx.node(tree, tree.root(), origin, 1.0, local);
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
    }
}

struct Context<'a> {
    pixmap: &'a mut Pixmap,
    engine: &'a mut TextEngine,
    full: Rect,
    masks: Vec<(Rect, Mask)>,
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
    fn prepare_mask(&mut self, clip: Rect) -> bool {
        if clip == self.full {
            return false;
        }
        if self.masks.iter().any(|(rect, _)| *rect == clip) {
            return true;
        }
        let Some(mut mask) = Mask::new(self.pixmap.width(), self.pixmap.height()) else {
            return false;
        };
        let Some(path) = to_skia_rect(clip).map(PathBuilder::from_rect) else {
            return false;
        };
        mask.fill_path(&path, FillRule::Winding, false, Transform::identity());
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
        let children = node.children().to_vec();
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
        let paints = extent.is_some_and(|extent| !extent.intersect(&clip).is_empty());
        let masked = paints
            && extent.is_some_and(|extent| extent.intersect(&clip) != extent)
            && self.prepare_mask(clip);
        let mask = if masked {
            self.masks
                .iter()
                .find(|(rect, _)| *rect == clip)
                .map(|(_, mask)| mask)
        } else {
            None
        };
        if paints && let Some(node) = tree.get_mut_clean(id) {
            match &mut node.paint {
                Paint::None => {}
                Paint::Rect(paint) => {
                    draw_rect(self.pixmap, paint, width, height, transform, opacity, mask)
                }
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
                        let paint = PixmapPaint {
                            opacity,
                            blend_mode: BlendMode::SourceOver,
                            quality,
                        };
                        self.pixmap
                            .draw_pixmap(0, 0, raster.pixmap.as_ref(), &paint, matrix, mask);
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
                        let paint = PixmapPaint {
                            opacity,
                            blend_mode: BlendMode::SourceOver,
                            quality,
                        };
                        self.pixmap
                            .draw_pixmap(0, 0, raster.as_ref(), &paint, matrix, mask);
                    }
                }
                Paint::Path(path) => draw_path(self.pixmap, path, transform, opacity, mask),
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
                        let paint = PixmapPaint {
                            opacity,
                            blend_mode: BlendMode::SourceOver,
                            quality,
                        };
                        self.pixmap.draw_pixmap(
                            0,
                            0,
                            image.as_ref().as_ref(),
                            &paint,
                            matrix,
                            mask,
                        );
                    }
                }
            }
        }
        for child in children {
            self.node(tree, child, transform, opacity, clip);
        }
    }
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

pub(crate) fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
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
    let mut paint = SkPaint::default();
    paint.set_color(color.to_skia(opacity));
    paint.anti_alias = true;
    pixmap.fill_path(path, &paint, rule, transform, mask);
}

fn draw_rect(
    pixmap: &mut Pixmap,
    rect: &RectPaint,
    width: f32,
    height: f32,
    transform: Transform,
    opacity: f32,
    mask: Option<&Mask>,
) {
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
        let mut engine = TextEngine::new(&[]);
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
