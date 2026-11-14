use tiny_skia::{IntRect, Mask, Pixmap, Transform};

use crate::{RectPaint, tree::Shadow};

const ENTRIES: usize = 6;

#[derive(Clone, Copy, PartialEq)]
struct Key {
    radius: u32,
    blur: u32,
    offset: u32,
    color: [u32; 4],
    scale: u32,
    opacity: u32,
}

impl Key {
    fn new(rect: &RectPaint, shadow: Shadow, scale: f32, opacity: f32) -> Self {
        let c = shadow.color;
        Self {
            radius: rect.radius.to_bits(),
            blur: shadow.blur.to_bits(),
            offset: shadow.offset.to_bits(),
            color: [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()],
            scale: scale.to_bits(),
            opacity: opacity.to_bits(),
        }
    }
}

struct Slices {
    key: Key,
    pad_x: i32,
    pad_y: i32,
    inner_x: i32,
    inner_y: i32,
    box_w: i32,
    box_h: i32,
    pieces: [Option<Pixmap>; 9],
}

#[derive(Default)]
pub(crate) struct ShadowCache {
    entries: Vec<Slices>,
}

fn build(key: Key, rect: &RectPaint, shadow: Shadow, scale: f32, opacity: f32) -> Option<Slices> {
    let radius = rect.radius.max(0.0) * scale;
    let offset = shadow.offset * scale;
    let blur = shadow.blur * scale;
    let inner_x = radius.ceil() as i32 + 2;
    let inner_y = radius.ceil() as i32 + offset.abs().ceil() as i32 + 2;
    let pad_x = blur.ceil() as i32 + 2;
    let pad_y = blur.ceil() as i32 + offset.abs().ceil() as i32 + 2;
    let box_w = inner_x * 2 + 1;
    let box_h = inner_y * 2 + 1;
    let width = (box_w + pad_x * 2) as u32;
    let height = (box_h + pad_y * 2) as u32;
    let mut model = Pixmap::new(width, height)?;
    super::render::draw_shadow(
        &mut model,
        rect,
        shadow,
        box_w as f32 / scale,
        box_h as f32 / scale,
        Transform::from_translate(pad_x as f32, pad_y as f32).pre_scale(scale, scale),
        opacity,
        None,
    );
    let cols = [0, pad_x + inner_x, pad_x + inner_x + 1, width as i32];
    let rows = [0, pad_y + inner_y, pad_y + inner_y + 1, height as i32];
    let mut pieces: [Option<Pixmap>; 9] = Default::default();
    for row in 0..3 {
        for col in 0..3 {
            if row == 1 && col == 1 {
                continue;
            }
            let area = IntRect::from_xywh(
                cols[col],
                rows[row],
                (cols[col + 1] - cols[col]) as u32,
                (rows[row + 1] - rows[row]) as u32,
            )?;
            pieces[row * 3 + col] = Some(model.clone_rect(area)?);
        }
    }
    Some(Slices {
        key,
        pad_x,
        pad_y,
        inner_x,
        inner_y,
        box_w,
        box_h,
        pieces,
    })
}

impl ShadowCache {
    fn get(
        &mut self,
        rect: &RectPaint,
        shadow: Shadow,
        scale: f32,
        opacity: f32,
    ) -> Option<&Slices> {
        let key = Key::new(rect, shadow, scale, opacity);
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let entry = self.entries.remove(index);
            self.entries.push(entry);
        } else {
            let entry = build(key, rect, shadow, scale, opacity)?;
            if self.entries.len() >= ENTRIES {
                self.entries.remove(0);
            }
            self.entries.push(entry);
        }
        self.entries.last()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        rect: &RectPaint,
        shadow: Shadow,
        width: f32,
        height: f32,
        transform: Transform,
        opacity: f32,
        mask: Option<&Mask>,
    ) -> bool {
        let scale = transform.sx;
        let uniform = transform.kx == 0.0
            && transform.ky == 0.0
            && scale > 0.0
            && (transform.sy - scale).abs() < 1e-4;
        if !uniform {
            return false;
        }
        let x0 = transform.tx.round() as i32;
        let y0 = transform.ty.round() as i32;
        let x1 = (transform.tx + width * scale).round() as i32;
        let y1 = (transform.ty + height * scale).round() as i32;
        let Some(slices) = self.get(rect, shadow, scale, opacity) else {
            return false;
        };
        let (target_w, target_h) = (x1 - x0, y1 - y0);
        if target_w < slices.box_w || target_h < slices.box_h {
            return false;
        }
        let xs = [x0 - slices.pad_x, x0 + slices.inner_x, x1 - slices.inner_x];
        let ys = [y0 - slices.pad_y, y0 + slices.inner_y, y1 - slices.inner_y];
        let spans_x = [
            slices.pad_x + slices.inner_x,
            target_w - slices.box_w + 1,
            slices.pad_x + slices.inner_x,
        ];
        let spans_y = [
            slices.pad_y + slices.inner_y,
            target_h - slices.box_h + 1,
            slices.pad_y + slices.inner_y,
        ];
        for row in 0..3 {
            for col in 0..3 {
                let Some(piece) = &slices.pieces[row * 3 + col] else {
                    continue;
                };
                crate::blit::over(
                    pixmap,
                    piece,
                    IntRect::from_xywh(xs[col], ys[row], spans_x[col] as u32, spans_y[row] as u32),
                    mask,
                    255,
                );
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    #[test]
    fn slices_match_rings() {
        let rect = RectPaint {
            radius: 22.0,
            ..RectPaint::default()
        };
        let shadow = Shadow {
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.4,
            },
            blur: 30.0,
            offset: 10.0,
        };
        let mut cache = ShadowCache::default();
        for (width, height, x, y) in [(426.2, 557.4, 60.0, 58.0), (130.0, 110.6, 41.3, 70.7)] {
            let transform = Transform::from_row(1.22, 0.0, 0.0, 1.22, x, y);
            let mut rings = Pixmap::new(800, 900).unwrap_or_else(|| unreachable!());
            let mut sliced = rings.clone();
            crate::render::draw_shadow(
                &mut rings, &rect, shadow, width, height, transform, 0.8, None,
            );
            assert!(cache.draw(
                &mut sliced,
                &rect,
                shadow,
                width,
                height,
                transform,
                0.8,
                None
            ));
            let (left, top) = (x, y);
            let (right, bottom) = (x + width * 1.22, y + height * 1.22);
            let mut worst = 0;
            for (index, (a, b)) in rings.pixels().iter().zip(sliced.pixels()).enumerate() {
                let px = (index % 800) as f32 + 0.5;
                let py = (index / 800) as f32 + 0.5;
                let corner = 22.0 * 1.22;
                let cx = px.clamp(left + corner, right - corner);
                let cy = py.clamp(top + corner, bottom - corner);
                let inside = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() < corner + 1.0;
                if !inside {
                    let d = (a.alpha() as i32 - b.alpha() as i32).abs();
                    worst = worst.max(d);
                }
            }
            assert!(worst <= 6, "{width}x{height} differs by {worst}");
        }
    }

    #[test]
    fn small_or_skewed_boxes_fall_back() {
        let rect = RectPaint {
            radius: 22.0,
            ..RectPaint::default()
        };
        let shadow = Shadow {
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.4,
            },
            blur: 30.0,
            offset: 10.0,
        };
        let mut cache = ShadowCache::default();
        let mut pixmap = Pixmap::new(300, 300).unwrap_or_else(|| unreachable!());
        let scaled = Transform::from_row(1.22, 0.0, 0.0, 1.22, 50.0, 50.0);
        assert!(!cache.draw(&mut pixmap, &rect, shadow, 20.0, 20.0, scaled, 1.0, None));
        let skewed = Transform::from_row(1.22, 0.1, 0.0, 1.22, 50.0, 50.0);
        assert!(!cache.draw(&mut pixmap, &rect, shadow, 200.0, 200.0, skewed, 1.0, None));
    }
}
