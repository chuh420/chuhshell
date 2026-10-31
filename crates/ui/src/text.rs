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
    Bold,
}

impl Weight {
    fn fontdb(self) -> fontdb::Weight {
        match self {
            Weight::Light => fontdb::Weight::LIGHT,
            Weight::Normal => fontdb::Weight::NORMAL,
            Weight::Medium => fontdb::Weight::MEDIUM,
            Weight::DemiBold => fontdb::Weight::SEMIBOLD,
            Weight::Bold => fontdb::Weight::BOLD,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub family: String,
    pub size: f32,
    pub weight: Weight,
    pub letter_spacing: f32,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<Color>,
    pub link: Option<String>,
}

impl Span {
    pub fn new(text: impl Into<String>, family: impl Into<String>, size: f32) -> Self {
        Self {
            text: text.into(),
            family: family.into(),
            size,
            weight: Weight::Normal,
            letter_spacing: 0.0,
            italic: false,
            underline: false,
            color: None,
            link: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub spans: Vec<Span>,
    pub color: Color,
    pub elide_width: Option<f32>,
    pub wrap_width: Option<f32>,
    pub max_lines: Option<usize>,
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
            wrap_width: None,
            max_lines: None,
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
        if let Some(width) = spec.wrap_width {
            buffer.set_wrap(Wrap::WordOrGlyph);
            buffer.set_size(Some(width.max(1.0) * scale), None);
            if let Some(lines) = spec.max_lines {
                buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(lines.max(1))));
            }
        } else {
            buffer.set_wrap(Wrap::None);
            if let Some(width) = elide {
                buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
                buffer.set_size(Some(width * scale), None);
            }
        }
        let default_family = spec.spans.first().map_or("", |span| span.family.as_str());
        let default = Attrs::new().family(Family::Name(default_family));
        let spans = spec.spans.iter().enumerate().map(|(index, span)| {
            let size = span.size * scale;
            let mut attrs = Attrs::new()
                .family(Family::Name(&span.family))
                .weight(span.weight.fontdb())
                .letter_spacing(if span.size > 0.0 {
                    span.letter_spacing / span.size
                } else {
                    0.0
                })
                .metadata(index)
                .metrics(Metrics::new(size, size));
            if span.italic {
                attrs = attrs.style(fontdb::Style::Italic);
            }
            (span.text.as_str(), attrs)
        });
        buffer.set_rich_text(spans, &default, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    fn line_metrics(
        &mut self,
        buffer: &mut Buffer,
        max_lines: Option<usize>,
    ) -> (f32, f32, f32, usize) {
        let mut width: f32 = 0.0;
        let mut ascent: f32 = 0.0;
        let mut descent: f32 = 0.0;
        let mut count = 0;
        let lines = buffer.lines.len();
        for index in 0..lines {
            if let Some(layout) = buffer.line_layout(&mut self.fonts, index) {
                for line in layout {
                    if max_lines.is_some_and(|max| count >= max) {
                        break;
                    }
                    width = width.max(line.w);
                    ascent = ascent.max(line.max_ascent);
                    descent = descent.max(line.max_descent);
                    count += 1;
                }
            }
        }
        (width, ascent, descent, count.max(1))
    }

    pub fn measure(&mut self, spec: &TextSpec, scale: f32) -> TextMetrics {
        let mut buffer = self.shape(spec, scale, None);
        let (width, ascent, descent, lines) = self.line_metrics(&mut buffer, spec.max_lines);
        TextMetrics {
            width: width / scale,
            height: ((ascent + descent) / scale).ceil() * lines as f32,
            ascent: ascent / scale,
        }
    }

    pub fn carets(&mut self, spec: &TextSpec) -> Vec<(usize, f32)> {
        let buffer = self.shape(spec, 1.0, None);
        let mut carets = Vec::new();
        let mut end = 0.0f32;
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                carets.push((glyph.start, glyph.x));
                end = end.max(glyph.x + glyph.w);
            }
        }
        let total: usize = spec.spans.iter().map(|span| span.text.len()).sum();
        carets.push((total, end));
        carets.sort_by_key(|caret| caret.0);
        carets.dedup_by_key(|caret| caret.0);
        carets
    }

    pub fn link_at(&mut self, spec: &TextSpec, x: f32, y: f32) -> Option<String> {
        if spec.spans.iter().all(|span| span.link.is_none()) {
            return None;
        }
        let mut buffer = self.shape(spec, 1.0, spec.elide_width);
        let (_, ascent, descent, _) = self.line_metrics(&mut buffer, spec.max_lines);
        let line_height = (ascent + descent).ceil();
        let mut previous_line = None;
        for (index, run) in buffer.layout_runs().enumerate() {
            if spec.max_lines.is_some_and(|max| index >= max) {
                break;
            }
            let shift = hanging_space(&run, previous_line);
            previous_line = Some(run.line_i);
            let top = index as f32 * line_height;
            if y < top || y >= top + line_height {
                continue;
            }
            for glyph in run.glyphs {
                let gx = glyph.x - shift;
                if x >= gx && x < gx + glyph.w {
                    return spec.spans.get(glyph.metadata)?.link.clone();
                }
            }
        }
        None
    }

    pub fn raster(&mut self, spec: &TextSpec, scale: f32) -> Option<Raster> {
        let natural = self.measure(spec, scale);
        let elide = spec
            .elide_width
            .filter(|width| *width + 0.01 < natural.width);
        let mut buffer = self.shape(spec, scale, elide);
        let (width, ascent, descent, lines) = self.line_metrics(&mut buffer, spec.max_lines);
        let line_height = ((ascent + descent) / scale).ceil() * scale;
        let max_size = spec.spans.iter().map(|span| span.size).fold(1.0, f32::max) * scale;
        let pad = (max_size * 0.4).ceil() as i32 + 2;
        let pw = width.ceil() as i32 + pad * 2;
        let ph = (line_height * lines as f32).ceil() as i32 + pad * 2;
        let size = IntSize::from_wh(pw.max(1) as u32, ph.max(1) as u32)?;
        let mut pixmap = Pixmap::new(size.width(), size.height())?;
        let base = spec.color.to_rgba8();
        let stride = size.width() as i32;
        let data = pixmap.data_mut();
        let mut previous_line = None;
        for (line_index, run) in buffer.layout_runs().enumerate() {
            if line_index >= lines {
                break;
            }
            let shift = hanging_space(&run, previous_line);
            previous_line = Some(run.line_i);
            let baseline = pad as f32 + ascent + line_index as f32 * line_height;
            let origin = pad as f32 - shift;
            for glyph in run.glyphs {
                if shift > 0.0 && glyph.x < shift {
                    continue;
                }
                let span = spec.spans.get(glyph.metadata);
                let color = span.and_then(|span| span.color).map_or(base, |color| {
                    let mut rgba = color.to_rgba8();
                    rgba[3] = mul(rgba[3], base[3]);
                    rgba
                });
                if span.is_some_and(|span| span.underline) {
                    let size = span.map_or(max_size, |span| span.size * scale);
                    let thickness = (size / 14.0).max(1.0).round() as i32;
                    let y0 = (baseline + size * 0.1).round() as i32;
                    let x0 = (origin + glyph.x).floor() as i32;
                    let x1 = (origin + glyph.x + glyph.w).ceil() as i32;
                    for y in y0..y0 + thickness {
                        for x in x0..x1 {
                            if x >= 0 && x < pw && y >= 0 && y < ph {
                                blend(&mut data[((y * stride + x) * 4) as usize..][..4], color);
                            }
                        }
                    }
                }
                let physical = glyph.physical((origin, baseline), 1.0);
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

fn hanging_space(run: &cosmic_text::LayoutRun<'_>, previous_line: Option<usize>) -> f32 {
    if previous_line != Some(run.line_i) {
        return 0.0;
    }
    let mut shift = 0.0;
    for glyph in run.glyphs {
        let text = run.text.get(glyph.start..glyph.end).unwrap_or("");
        if text.is_empty() || !text.chars().all(char::is_whitespace) {
            break;
        }
        shift += glyph.w;
    }
    shift
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
