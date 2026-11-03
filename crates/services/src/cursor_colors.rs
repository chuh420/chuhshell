use std::{collections::HashMap, io, path::Path};

use crate::cursors::{self, Image};

const OPAQUE: u32 = 192;
const DRAWN_BUCKETS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recolor {
    pub accent: [u8; 3],
    pub outline: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    outline: [f32; 3],
    body: [f32; 3],
    to_outline: [f32; 3],
    to_body: [f32; 3],
}

fn to_linear(channel: u8) -> f32 {
    let c = channel as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_gamma(channel: f32) -> u8 {
    let c = channel.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

pub fn oklab(rgb: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn srgb(lab: [f32; 3]) -> [u8; 3] {
    let [l, a, b] = lab;
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
    .map(to_gamma)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn chroma(lab: [f32; 3]) -> f32 {
    (lab[1] * lab[1] + lab[2] * lab[2]).sqrt()
}

fn unpremultiply(pixel: u32) -> (u32, [u8; 3]) {
    let a = pixel >> 24;
    if a == 0 {
        return (0, [0, 0, 0]);
    }
    let channel = |shift: u32| (((pixel >> shift) & 0xff) * 255 / a).min(255) as u8;
    (a, [channel(16), channel(8), channel(0)])
}

fn premultiply(a: u32, rgb: [u8; 3]) -> u32 {
    let channel = |c: u8| (c as u32 * a + 127) / 255;
    (a << 24) | (channel(rgb[0]) << 16) | (channel(rgb[1]) << 8) | channel(rgb[2])
}

struct Sample {
    lab: [f32; 3],
    edge: bool,
}

fn samples(image: &Image) -> Vec<Sample> {
    let w = image.width as usize;
    let h = image.height as usize;
    let alpha = |x: isize, y: isize| -> u32 {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return 0;
        }
        image.pixels[y as usize * w + x as usize] >> 24
    };
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (a, rgb) = unpremultiply(image.pixels[y * w + x]);
            if a < OPAQUE {
                continue;
            }
            let (xi, yi) = (x as isize, y as isize);
            let edge = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|(dx, dy)| alpha(xi + dx, yi + dy) < 128);
            out.push(Sample {
                lab: oklab(rgb),
                edge,
            });
        }
    }
    out
}

pub fn analyze(image: &Image, recolor: Recolor) -> Option<Mapping> {
    let samples = samples(image);
    let darkest = samples
        .iter()
        .min_by(|a, b| a.lab[0].total_cmp(&b.lab[0]))?
        .lab;
    let lightest = samples
        .iter()
        .max_by(|a, b| a.lab[0].total_cmp(&b.lab[0]))?
        .lab;
    let mut centers = [darkest, lightest];
    let mut groups = [Vec::new(), Vec::new()];
    for _ in 0..12 {
        groups = [Vec::new(), Vec::new()];
        for sample in &samples {
            let d0 = dot(sub(sample.lab, centers[0]), sub(sample.lab, centers[0]));
            let d1 = dot(sub(sample.lab, centers[1]), sub(sample.lab, centers[1]));
            groups[usize::from(d1 < d0)].push(sample);
        }
        if groups.iter().any(Vec::is_empty) {
            return None;
        }
        for (center, group) in centers.iter_mut().zip(&groups) {
            let n = group.len() as f32;
            *center = group.iter().fold([0.0; 3], |sum, sample| {
                [
                    sum[0] + sample.lab[0] / n,
                    sum[1] + sample.lab[1] / n,
                    sum[2] + sample.lab[2] / n,
                ]
            });
        }
    }
    let difference = sub(centers[1], centers[0]);
    if dot(difference, difference) < 0.0025 {
        return None;
    }
    let (c0, c1) = (chroma(centers[0]), chroma(centers[1]));
    let body = if (c0 - c1).abs() > 0.03 && c0.max(c1) > 0.04 {
        usize::from(c1 > c0)
    } else {
        let edges = |group: &Vec<&Sample>| {
            group.iter().filter(|sample| sample.edge).count() as f32 / group.len() as f32
        };
        usize::from(edges(&groups[1]) < edges(&groups[0]))
    };
    Some(Mapping {
        outline: centers[1 - body],
        body: centers[body],
        to_outline: oklab(recolor.outline),
        to_body: oklab(recolor.accent),
    })
}

pub fn drawn(image: &Image) -> bool {
    let mut buckets: HashMap<u32, usize> = HashMap::new();
    let mut total = 0;
    for &pixel in &image.pixels {
        let (a, [r, g, b]) = unpremultiply(pixel);
        if a < OPAQUE {
            continue;
        }
        total += 1;
        let key = (u32::from(r >> 5) << 6) | (u32::from(g >> 5) << 3) | u32::from(b >> 5);
        *buckets.entry(key).or_default() += 1;
    }
    let mut counts: Vec<usize> = buckets.into_values().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let mut covered = 0;
    let mut used = 0;
    for count in counts {
        if covered * 10 >= total * 9 {
            break;
        }
        covered += count;
        used += 1;
    }
    used > DRAWN_BUCKETS
}

fn sample_shape(theme: &Path) -> Option<Image> {
    cursors::load_shape(theme, &["left_ptr", "default", "arrow"], 48)
}

pub fn theme_drawn(theme: &Path) -> bool {
    sample_shape(theme).is_none_or(|image| drawn(&image))
}

pub fn theme_mapping(theme: &Path, recolor: Recolor) -> Option<Mapping> {
    analyze(&sample_shape(theme)?, recolor)
}

impl Mapping {
    pub fn color(&self, rgb: [u8; 3]) -> [u8; 3] {
        let lab = oklab(rgb);
        let axis = sub(self.body, self.outline);
        let length = dot(axis, axis);
        let t = if length > 1e-6 {
            (dot(sub(lab, self.outline), axis) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        srgb(lerp(self.to_outline, self.to_body, t))
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

pub fn recolor_bytes(
    bytes: &mut [u8],
    mapping: &Mapping,
    cache: &mut HashMap<[u8; 3], [u8; 3]>,
) -> bool {
    if bytes.get(0..4) != Some(b"Xcur") {
        return false;
    }
    let (Some(header), Some(count)) = (u32_at(bytes, 4), u32_at(bytes, 12)) else {
        return false;
    };
    for index in 0..(count as usize).min(4096) {
        let entry = header as usize + index * 12;
        let (Some(kind), Some(pos)) = (u32_at(bytes, entry), u32_at(bytes, entry + 8)) else {
            return false;
        };
        if kind != 0xfffd_0002 {
            continue;
        }
        let pos = pos as usize;
        let (Some(width), Some(height)) = (u32_at(bytes, pos + 16), u32_at(bytes, pos + 20)) else {
            continue;
        };
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            continue;
        }
        let start = pos + 36;
        let len = (width * height) as usize * 4;
        let Some(data) = bytes.get_mut(start..start + len) else {
            continue;
        };
        for chunk in data.as_chunks_mut::<4>().0 {
            let pixel = u32::from_le_bytes(*chunk);
            let (a, rgb) = unpremultiply(pixel);
            if a == 0 {
                continue;
            }
            let color = *cache.entry(rgb).or_insert_with(|| mapping.color(rgb));
            chunk.copy_from_slice(&premultiply(a, color).to_le_bytes());
        }
    }
    true
}

fn inherits(source: &Path) -> Option<String> {
    let text = std::fs::read_to_string(source.join("index.theme")).ok()?;
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("Inherits"))
        .map(str::to_string)
}

pub fn write_theme(source: &Path, target: &Path, name: &str, mapping: &Mapping) -> io::Result<()> {
    match std::fs::remove_dir_all(target) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        _ => {}
    }
    let cursors_out = target.join("cursors");
    std::fs::create_dir_all(&cursors_out)?;
    let mut cache = HashMap::new();
    for entry in std::fs::read_dir(source.join("cursors"))? {
        let entry = entry?;
        let path = entry.path();
        let out = cursors_out.join(entry.file_name());
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&path)?, &out)?;
        } else if meta.is_file() {
            let mut bytes = std::fs::read(&path)?;
            recolor_bytes(&mut bytes, mapping, &mut cache);
            std::fs::write(&out, &bytes)?;
        }
    }
    let mut index = format!("[Icon Theme]\nName={name}\n");
    if let Some(line) = inherits(source) {
        index.push_str(&line);
        index.push('\n');
    }
    std::fs::write(target.join("index.theme"), index)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECOLOR: Recolor = Recolor {
        accent: [0xb8, 0xc3, 0xff],
        outline: [0x19, 0x1b, 0x25],
    };

    fn arrow(body: [u8; 3], outline: [u8; 3]) -> Image {
        let size = 12u32;
        let mut pixels = vec![0u32; (size * size) as usize];
        for y in 1..11u32 {
            for x in 1..11u32 {
                let border = x == 1 || y == 1 || x == 10 || y == 10;
                let rgb = if border { outline } else { body };
                pixels[(y * size + x) as usize] = premultiply(255, rgb);
            }
        }
        Image {
            width: size,
            height: size,
            hotspot: (0, 0),
            delay: 0,
            pixels,
        }
    }

    fn close(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    #[test]
    fn oklab_round_trip() {
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [0xe0, 0x80, 0x00],
            [0x19, 0x1b, 0x25],
        ] {
            assert!(close(srgb(oklab(rgb)), rgb));
        }
    }

    #[test]
    fn gray_theme_fills_the_interior() {
        let mapping = analyze(&arrow([0, 0, 0], [230, 230, 230]), RECOLOR);
        let mapping = mapping.unwrap_or_else(|| panic!("no mapping"));
        assert!(close(mapping.color([0, 0, 0]), RECOLOR.accent));
        assert!(close(mapping.color([230, 230, 230]), RECOLOR.outline));
        let ice = analyze(&arrow([230, 230, 230], [0, 0, 0]), RECOLOR);
        let ice = ice.unwrap_or_else(|| panic!("no mapping"));
        assert!(close(ice.color([230, 230, 230]), RECOLOR.accent));
    }

    #[test]
    fn colored_part_becomes_the_accent() {
        let mapping = analyze(&arrow([0x20, 0x20, 0x20], [0xe0, 0xe0, 0xa0]), RECOLOR);
        let mapping = mapping.unwrap_or_else(|| panic!("no mapping"));
        assert!(close(mapping.color([0xe0, 0xe0, 0xa0]), RECOLOR.accent));
        assert!(close(mapping.color([0x20, 0x20, 0x20]), RECOLOR.outline));
    }

    #[test]
    fn flat_image_has_no_mapping() {
        assert!(analyze(&arrow([9, 9, 9], [9, 9, 9]), RECOLOR).is_none());
    }

    #[test]
    fn many_colors_count_as_drawn() {
        assert!(!drawn(&arrow([0, 0, 0], [230, 230, 230])));
        let mut image = arrow([0, 0, 0], [230, 230, 230]);
        for (index, pixel) in image.pixels.iter_mut().enumerate() {
            let v = (index * 37 % 256) as u8;
            *pixel = premultiply(255, [v, v.wrapping_mul(3), v.wrapping_mul(7)]);
        }
        assert!(drawn(&image));
    }

    #[test]
    fn recolors_xcursor_bytes_in_place() {
        let mut bytes = Vec::new();
        for value in [u32::from_le_bytes(*b"Xcur"), 16, 0x1_0000, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0xfffd_0002u32, 1, 28] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [36u32, 0xfffd_0002, 1, 1, 1, 1, 0, 0, 0, 0xff00_0000, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mapping = analyze(&arrow([0, 0, 0], [230, 230, 230]), RECOLOR)
            .unwrap_or_else(|| panic!("no mapping"));
        assert!(recolor_bytes(&mut bytes, &mapping, &mut HashMap::new()));
        let images = cursors::parse(&bytes).unwrap_or_default();
        assert_eq!(images.len(), 1);
        let (a, rgb) = unpremultiply(images[0].pixels[0]);
        assert_eq!(a, 255);
        assert!(close(rgb, RECOLOR.accent));
        assert!(!recolor_bytes(
            &mut b"nope".to_vec(),
            &mapping,
            &mut HashMap::new()
        ));
    }
}
