use tiny_skia::{IntRect, Mask, Pixmap};

fn mul(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}

pub(crate) fn over(
    pixmap: &mut Pixmap,
    piece: &Pixmap,
    area: Option<IntRect>,
    mask: Option<&Mask>,
    opacity: u8,
) {
    if opacity == 0 {
        return;
    }
    let Some(area) = area else {
        return;
    };
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let (pw, ph) = (piece.width() as i32, piece.height() as i32);
    let left = area.x().max(0);
    let right = (area.x() + area.width() as i32).min(width);
    let top = area.y().max(0);
    let bottom = (area.y() + area.height() as i32).min(height);
    if left >= right || top >= bottom {
        return;
    }
    let stretch_x = pw != area.width() as i32;
    let stretch_y = ph != area.height() as i32;
    let source = piece.data();
    let coverage = mask.map(Mask::data);
    let target = pixmap.data_mut();
    let span = (right - left) as usize;
    for y in top..bottom {
        let sy = if stretch_y { 0 } else { y - area.y() };
        let row_at = (sy * pw) as usize * 4;
        let from = if stretch_x {
            row_at
        } else {
            row_at + (left - area.x()) as usize * 4
        };
        let at = (y * width + left) as usize * 4;
        let dst = &mut target[at..at + span * 4];
        let masks = coverage.map(|data| {
            let at = (y * width + left) as usize;
            &data[at..at + span]
        });
        let dst = dst.as_chunks_mut::<4>().0;
        if stretch_x {
            let pixel = fade(
                [
                    source[from],
                    source[from + 1],
                    source[from + 2],
                    source[from + 3],
                ],
                opacity,
            );
            if pixel[3] == 0 {
                continue;
            }
            match masks {
                None => {
                    for out in dst {
                        blend(out, pixel);
                    }
                }
                Some(masks) => {
                    for (out, m) in dst.iter_mut().zip(masks) {
                        if *m != 0 {
                            blend(out, fade(pixel, *m));
                        }
                    }
                }
            }
        } else {
            let src = source[from..from + span * 4].as_chunks::<4>().0;
            match masks {
                None => {
                    for (out, pixel) in dst.iter_mut().zip(src) {
                        if pixel[3] != 0 {
                            blend(out, fade(*pixel, opacity));
                        }
                    }
                }
                Some(masks) => {
                    for ((out, pixel), m) in dst.iter_mut().zip(src).zip(masks) {
                        if pixel[3] != 0 && *m != 0 {
                            blend(out, fade(*pixel, mul(opacity, *m)));
                        }
                    }
                }
            }
        }
    }
}

fn fade(pixel: [u8; 4], alpha: u8) -> [u8; 4] {
    if alpha == 255 {
        pixel
    } else {
        pixel.map(|c| mul(c, alpha))
    }
}

fn blend(out: &mut [u8; 4], src: [u8; 4]) {
    let inverse = 255 - src[3];
    for (channel, value) in out.iter_mut().zip(src) {
        *channel = value.saturating_add(mul(*channel, inverse));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{
        BlendMode, Color, FillRule, FilterQuality, PathBuilder, Pixmap, PixmapPaint, Transform,
    };

    fn sample(width: u32, height: u32, seed: u32) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).unwrap_or_else(|| unreachable!());
        for (i, pixel) in pixmap.pixels_mut().iter_mut().enumerate() {
            let v = (i as u32).wrapping_mul(2_654_435_761).wrapping_add(seed);
            let a = (v >> 24) as u8;
            let c = |shift: u32| ((((v >> shift) & 0xff) * a as u32) / 255) as u8;
            *pixel = tiny_skia::PremultipliedColorU8::from_rgba(c(0), c(8), c(16), a)
                .unwrap_or_else(|| unreachable!());
        }
        pixmap
    }

    #[test]
    fn matches_tiny_skia() {
        let source = sample(23, 17, 7);
        let mut mask = Mask::new(40, 30).unwrap_or_else(|| unreachable!());
        let circle = PathBuilder::from_circle(20.0, 15.0, 11.0).unwrap_or_else(|| unreachable!());
        mask.fill_path(&circle, FillRule::Winding, true, Transform::identity());
        for (opacity, mask) in [
            (1.0, None),
            (0.6, None),
            (1.0, Some(&mask)),
            (0.3, Some(&mask)),
        ] {
            let mut expected = sample(40, 30, 3);
            let mut actual = expected.clone();
            let paint = PixmapPaint {
                opacity,
                blend_mode: BlendMode::SourceOver,
                quality: FilterQuality::Nearest,
            };
            expected.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &paint,
                Transform::from_translate(-4.0, 20.0),
                mask,
            );
            over(
                &mut actual,
                &source,
                IntRect::from_xywh(-4, 20, 23, 17),
                mask,
                (opacity * 255.0).round() as u8,
            );
            let worst = expected
                .data()
                .iter()
                .zip(actual.data())
                .map(|(a, b)| (*a as i32 - *b as i32).abs())
                .max()
                .unwrap_or(0);
            assert!(worst <= 2, "opacity {opacity} differs by {worst}");
        }
    }

    #[test]
    fn stretches_single_pixel_slices() {
        let mut piece = Pixmap::new(1, 2).unwrap_or_else(|| unreachable!());
        piece.fill(Color::from_rgba8(0, 0, 0, 255));
        let mut target = Pixmap::new(10, 10).unwrap_or_else(|| unreachable!());
        over(
            &mut target,
            &piece,
            IntRect::from_xywh(2, 3, 5, 2),
            None,
            255,
        );
        let opaque = target.pixels().iter().filter(|p| p.alpha() == 255).count();
        assert_eq!(opaque, 10);
        assert_eq!(target.pixel(2, 3).map(|p| p.alpha()), Some(255));
        assert_eq!(target.pixel(7, 3).map(|p| p.alpha()), Some(0));
    }
}
