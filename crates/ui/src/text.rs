use std::{collections::HashSet, process::Command};

use cosmic_text::{
    Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping,
    SwashCache, SwashContent, Wrap, fontdb,
};
use tiny_skia::{IntSize, Pixmap};

use crate::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Weight {
    Light,
    #[default]
    Normal,
    Medium,
    DemiBold,
}

impl Weight {
    fn fontdb(self) -> fontdb::Weight {
        match self {
            Weight::Light => fontdb::Weight::LIGHT,
            Weight::Normal => fontdb::Weight::NORMAL,
            Weight::Medium => fontdb::Weight::MEDIUM,
            Weight::DemiBold => fontdb::Weight::SEMIBOLD,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub family: String,
    pub size: f32,
    pub weight: Weight,
}

impl Span {
    pub fn new(text: impl Into<String>, family: impl Into<String>, size: f32) -> Self {
        Self {
            text: text.into(),
            family: family.into(),
            size,
            weight: Weight::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub spans: Vec<Span>,
    pub color: Color,
    pub elide_width: Option<f32>,
}

impl TextSpec {
    pub fn plain(
        text: impl Into<String>,
        family: impl Into<String>,
        size: f32,
        color: Color,
    ) -> Self {
        Self {
            spans: vec![Span::new(text, family, size)],
            color,
            elide_width: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
}

pub struct Raster {
    pub pixmap: Pixmap,
    pub left: f32,
    pub top: f32,
    pub scale: f32,
}

pub struct TextEngine {
    fonts: FontSystem,
    swash: SwashCache,
}

fn font_files(family: &str) -> Vec<String> {
    match Command::new("fc-list")
        .arg("-f")
        .arg("%{file}\n")
        .arg(family)
        .output()
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
        Err(err) => {
            log::warn!("fc-list {family}: {err}");
            Vec::new()
        }
    }
}

impl TextEngine {
    pub fn new(families: &[&str]) -> Self {
        let mut db = fontdb::Database::new();
        let mut seen = HashSet::new();
        for family in families {
            for file in font_files(family) {
                if seen.insert(file.clone())
                    && let Err(err) = db.load_font_file(&file)
                {
                    log::warn!("font {file}: {err}");
                }
            }
        }
        if db.is_empty() {
            log::warn!("no fonts found through fc-list, loading every system font");
            db.load_system_fonts();
        }
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), db),
            swash: SwashCache::new(),
        }
    }

    fn shape(&mut self, spec: &TextSpec, scale: f32, elide: Option<f32>) -> Buffer {
        let max_size = spec.spans.iter().map(|span| span.size).fold(1.0, f32::max) * scale;
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(max_size, max_size));
        buffer.set_wrap(Wrap::None);
        if let Some(width) = elide {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
            buffer.set_size(Some(width * scale), None);
        }
        let default = Attrs::new();
        let spans = spec.spans.iter().map(|span| {
            let size = span.size * scale;
            (
                span.text.as_str(),
                Attrs::new()
                    .family(Family::Name(&span.family))
                    .weight(span.weight.fontdb())
                    .metrics(Metrics::new(size, size)),
            )
        });
        buffer.set_rich_text(spans, &default, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    fn line_metrics(&mut self, buffer: &mut Buffer) -> (f32, f32, f32) {
        let mut width: f32 = 0.0;
        let mut ascent: f32 = 0.0;
        let mut descent: f32 = 0.0;
        let lines = buffer.lines.len();
        for index in 0..lines {
            if let Some(layout) = buffer.line_layout(&mut self.fonts, index) {
                for line in layout {
                    width = width.max(line.w);
                    ascent = ascent.max(line.max_ascent);
                    descent = descent.max(line.max_descent);
                }
            }
        }
        (width, ascent, descent)
    }

    pub fn measure(&mut self, spec: &TextSpec, scale: f32) -> TextMetrics {
        let mut buffer = self.shape(spec, scale, None);
        let (width, ascent, descent) = self.line_metrics(&mut buffer);
        TextMetrics {
            width: width / scale,
            height: ((ascent + descent) / scale).ceil(),
            ascent: ascent / scale,
        }
    }

    pub fn raster(&mut self, spec: &TextSpec, scale: f32) -> Option<Raster> {
        let natural = self.measure(spec, scale);
        let elide = spec.elide_width.filter(|width| *width < natural.width);
        let mut buffer = self.shape(spec, scale, elide);
        let (width, ascent, descent) = self.line_metrics(&mut buffer);
        let max_size = spec.spans.iter().map(|span| span.size).fold(1.0, f32::max) * scale;
        let pad = (max_size * 0.4).ceil() as i32 + 2;
        let pw = width.ceil() as i32 + pad * 2;
        let ph = (ascent + descent).ceil() as i32 + pad * 2;
        let size = IntSize::from_wh(pw.max(1) as u32, ph.max(1) as u32)?;
        let mut pixmap = Pixmap::new(size.width(), size.height())?;
        let color = spec.color.to_rgba8();
        let stride = size.width() as i32;
        let data = pixmap.data_mut();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((pad as f32, pad as f32 + ascent), 1.0);
                let Some(image) = self
                    .swash
                    .get_image_uncached(&mut self.fonts, physical.cache_key)
                else {
                    continue;
                };
                let x0 = physical.x + image.placement.left;
                let y0 = physical.y - image.placement.top;
                let gw = image.placement.width as i32;
                let gh = image.placement.height as i32;
                for gy in 0..gh {
                    let y = y0 + gy;
                    if y < 0 || y >= ph {
                        continue;
                    }
                    for gx in 0..gw {
                        let x = x0 + gx;
                        if x < 0 || x >= pw {
                            continue;
                        }
                        let src = match image.content {
                            SwashContent::Mask => {
                                let coverage = image.data[(gy * gw + gx) as usize];
                                [color[0], color[1], color[2], mul(coverage, color[3])]
                            }
                            SwashContent::SubpixelMask => {
                                let i = ((gy * gw + gx) * 4) as usize;
                                let coverage = image.data[i + 1];
                                [color[0], color[1], color[2], mul(coverage, color[3])]
                            }
                            SwashContent::Color => {
                                let i = ((gy * gw + gx) * 4) as usize;
                                [
                                    image.data[i],
                                    image.data[i + 1],
                                    image.data[i + 2],
                                    mul(image.data[i + 3], color[3]),
                                ]
                            }
                        };
                        blend(&mut data[((y * stride + x) * 4) as usize..][..4], src);
                    }
                }
            }
        }
        Some(Raster {
            pixmap,
            left: -pad as f32,
            top: -pad as f32,
            scale,
        })
    }
}

fn mul(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}

fn blend(dst: &mut [u8], src: [u8; 4]) {
    let alpha = src[3];
    if alpha == 0 {
        return;
    }
    let inverse = 255 - alpha;
    for i in 0..3 {
        dst[i] = mul(src[i], alpha).saturating_add(mul(dst[i], inverse));
    }
    dst[3] = alpha.saturating_add(mul(dst[3], inverse));
}
