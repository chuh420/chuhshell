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
        if let Some(rect) = to_skia_rect(clip) {
            let clear = SkPaint {
                blend_mode: BlendMode::Clear,
                ..SkPaint::default()
            };
            self.pixmap
                .fill_rect(rect, &clear, Transform::identity(), None);
        }
        let mut ctx = Context {
            pixmap: &mut self.pixmap,
            engine,
            full,
        };
        ctx.node(tree, tree.root(), Transform::identity(), 1.0, clip);
    }
}

struct Context<'a> {
    pixmap: &'a mut Pixmap,
    engine: &'a mut TextEngine,
    full: Rect,
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
    fn mask(&self, clip: Rect) -> Option<Mask> {
        if clip == self.full {
            return None;
        }
        let mut mask = Mask::new(self.pixmap.width(), self.pixmap.height())?;
        let path = PathBuilder::from_rect(to_skia_rect(clip)?);
        mask.fill_path(&path, FillRule::Winding, false, Transform::identity());
        Some(mask)
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
        let mask = self.mask(clip);
        if let Some(node) = tree.get_mut_clean(id) {
            match &mut node.paint {
                Paint::None => {}
                Paint::Rect(paint) => draw_rect(
                    self.pixmap,
                    paint,
                    width,
                    height,
                    transform,
                    opacity,
                    mask.as_ref(),
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
                        let paint = PixmapPaint {
                            opacity,
                            blend_mode: BlendMode::SourceOver,
                            quality,
                        };
                        self.pixmap.draw_pixmap(
                            0,
                            0,
                            raster.pixmap.as_ref(),
                            &paint,
                            matrix,
                            mask.as_ref(),
                        );
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
                        self.pixmap.draw_pixmap(
                            0,
                            0,
                            raster.as_ref(),
                            &paint,
                            matrix,
                            mask.as_ref(),
                        );
                    }
                }
                Paint::Path(path) => {
                    draw_path(self.pixmap, path, transform, opacity, mask.as_ref())
                }
                Paint::Image(image) => {
                    if image.width() > 0 && image.height() > 0 {
                        let matrix = transform.pre_scale(
                            width / image.width() as f32,
                            height / image.height() as f32,
                        );
                        let paint = PixmapPaint {
                            opacity,
                            blend_mode: BlendMode::SourceOver,
                            quality: FilterQuality::Bilinear,
                        };
                        self.pixmap.draw_pixmap(
                            0,
                            0,
                            image.as_ref().as_ref(),
                            &paint,
                            matrix,
                            mask.as_ref(),
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
