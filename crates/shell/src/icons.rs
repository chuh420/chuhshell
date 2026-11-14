use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use services::notifications::{Image, Notification};
use ui::{
    SvgTree,
    tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform},
};

#[derive(Clone, Debug, PartialEq)]
pub enum IconSource {
    Pixels {
        width: u32,
        height: u32,
        rgba: Arc<Vec<u8>>,
    },
    File(PathBuf),
}

fn file_url(value: &str) -> Option<PathBuf> {
    if let Some(path) = value.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }
    value.starts_with('/').then(|| PathBuf::from(value))
}

fn theme_icon(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    ["hicolor", "Adwaita", "AdwaitaLegacy"]
        .iter()
        .find_map(|theme| {
            freedesktop_icons::lookup(name)
                .with_theme(theme)
                .with_size(64)
                .with_cache()
                .find()
        })
        .or_else(|| {
            ["png", "svg", "xpm"]
                .iter()
                .map(|ext| PathBuf::from(format!("/usr/share/pixmaps/{name}.{ext}")))
                .find(|path| path.exists())
        })
}

fn search_dir(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    for ext in ["png", "svg", "xpm"] {
        let path = dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs.iter().find_map(|dir| search_dir(dir, name, depth - 1))
}

pub fn tray_source(item: &services::tray::TrayItem) -> Option<IconSource> {
    let (name, pixels) = if item.needs_attention()
        && (!item.attention_icon_name.is_empty() || item.attention_pixmap.is_some())
    {
        (&item.attention_icon_name, &item.attention_pixmap)
    } else {
        (&item.icon_name, &item.icon_pixmap)
    };
    if !name.is_empty() {
        if let Some(path) = file_url(name) {
            return Some(IconSource::File(path));
        }
        if !item.icon_theme_path.is_empty()
            && let Some(path) = search_dir(Path::new(&item.icon_theme_path), name, 3)
        {
            return Some(IconSource::File(path));
        }
        if let Some(path) = theme_icon(name) {
            return Some(IconSource::File(path));
        }
    }
    pixels.as_ref().map(|pixels| IconSource::Pixels {
        width: pixels.width,
        height: pixels.height,
        rgba: pixels.rgba.clone(),
    })
}

pub fn app_source(icon: &str) -> Option<IconSource> {
    if icon.starts_with('/') {
        return Some(IconSource::File(PathBuf::from(icon)));
    }
    theme_icon(icon).map(IconSource::File)
}

pub fn source(notification: &Notification) -> Option<IconSource> {
    match &notification.image {
        Some(Image::Pixels {
            width,
            height,
            rgba,
        }) => {
            return Some(IconSource::Pixels {
                width: *width,
                height: *height,
                rgba: rgba.clone(),
            });
        }
        Some(Image::Path(path)) => return file_url(path).map(IconSource::File),
        None => {}
    }
    let icon = notification.app_icon.as_str();
    if let Some(path) = file_url(icon) {
        return Some(IconSource::File(path));
    }
    if let Some(path) = theme_icon(icon) {
        return Some(IconSource::File(path));
    }
    theme_icon(&notification.desktop_entry.to_lowercase()).map(IconSource::File)
}

fn cover(source: &Pixmap, size: u32) -> Option<Pixmap> {
    cover_rect(source, size, size)
}

fn cover_rect(source: &Pixmap, width: u32, height: u32) -> Option<Pixmap> {
    let mut out = Pixmap::new(width, height)?;
    let (w, h) = (source.width() as f32, source.height() as f32);
    let scale = (width as f32 / w).max(height as f32 / h);
    let dx = (width as f32 - w * scale) / 2.0;
    let dy = (height as f32 - h * scale) / 2.0;
    out.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bicubic,
            ..PixmapPaint::default()
        },
        Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
        None,
    );
    Some(out)
}

pub fn rounded(source: &Pixmap, radius: f32) -> Option<Pixmap> {
    let (width, height) = (source.width(), source.height());
    let mut mask = ui::tiny_skia::Mask::new(width, height)?;
    let path = ui::rounded_rect(0.0, 0.0, width as f32, height as f32, radius)?;
    mask.fill_path(
        &path,
        ui::tiny_skia::FillRule::Winding,
        true,
        Transform::identity(),
    );
    let mut out = Pixmap::new(width, height)?;
    out.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &ui::tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
    Some(out)
}

const SHRINK_BELOW: f32 = 0.5;

pub fn load_cover(path: &Path, width: u32, height: u32) -> Option<Pixmap> {
    let image = match image::open(path) {
        Ok(image) => image,
        Err(err) => {
            log::warn!("image {}: {err}", path.display());
            return None;
        }
    };
    let image = match shrunk_size(image.width(), image.height(), width, height) {
        Some((w, h)) => image.thumbnail_exact(w, h),
        None => image,
    };
    let pixmap = premultiplied(image.into_rgba8())?;
    cover_rect(&pixmap, width, height)
}

fn shrunk_size(width: u32, height: u32, target_w: u32, target_h: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let scale = (target_w as f32 / width as f32).max(target_h as f32 / height as f32);
    if scale >= SHRINK_BELOW {
        return None;
    }
    let w = ((width as f32 * scale).ceil() as u32)
        .max(target_w)
        .min(width);
    let h = ((height as f32 * scale).ceil() as u32)
        .max(target_h)
        .min(height);
    Some((w, h))
}

fn premultiplied(image: image::RgbaImage) -> Option<Pixmap> {
    let size = IntSize::from_wh(image.width(), image.height())?;
    let mut data = image.into_raw();
    for pixel in data.as_chunks_mut::<4>().0 {
        let a = pixel[3] as u16;
        if a == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(data, size)
}

fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Pixmap> {
    let mut data = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        let a = pixel[3] as u16;
        let mul = |c: u8| ((c as u16 * a + 127) / 255) as u8;
        data.extend_from_slice(&[mul(pixel[0]), mul(pixel[1]), mul(pixel[2]), pixel[3]]);
    }
    Pixmap::from_vec(data, IntSize::from_wh(width, height)?)
}

fn decode_file(path: &Path, size: u32) -> Option<Pixmap> {
    let is_svg = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
    if is_svg {
        let data = std::fs::read(path).ok()?;
        let tree = SvgTree::from_data(&data, &Default::default()).ok()?;
        let natural = tree.size();
        let scale = (size as f32 / natural.width()).max(size as f32 / natural.height());
        let mut pixmap = Pixmap::new(size, size)?;
        let dx = (size as f32 - natural.width() * scale) / 2.0;
        let dy = (size as f32 - natural.height() * scale) / 2.0;
        ui::render_svg_into(
            &tree,
            Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
            &mut pixmap,
        );
        return Some(pixmap);
    }
    let image = match image::open(path) {
        Ok(image) => image.into_rgba8(),
        Err(err) => {
            log::warn!("icon {}: {err}", path.display());
            return None;
        }
    };
    cover(&premultiplied(image)?, size)
}

pub fn load(source: &IconSource, size: u32) -> Option<Pixmap> {
    match source {
        IconSource::Pixels {
            width,
            height,
            rgba,
        } => cover(&from_rgba(*width, *height, rgba)?, size),
        IconSource::File(path) => decode_file(path, size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinks_only_large_images() {
        assert_eq!(shrunk_size(2560, 1440, 1920, 1080), None);
        assert_eq!(shrunk_size(3840, 2160, 1920, 1080), None);
        assert_eq!(shrunk_size(0, 100, 1920, 1080), None);
        for (w, h) in [(7200, 4050), (6068, 2856), (4000, 9000)] {
            let (sw, sh) = shrunk_size(w, h, 1920, 1080).unwrap_or_else(|| unreachable!());
            assert!(sw >= 1920 && sh >= 1080, "{w}x{h} -> {sw}x{sh}");
            let ratio = (sw as f32 / sh as f32) / (w as f32 / h as f32);
            assert!((ratio - 1.0).abs() < 0.01, "{w}x{h} -> {sw}x{sh}");
        }
    }

    #[test]
    fn premultiplies_in_place() {
        let image = image::RgbaImage::from_raw(2, 1, vec![200, 100, 50, 255, 200, 100, 50, 128])
            .unwrap_or_else(|| unreachable!());
        let pixmap = premultiplied(image).unwrap_or_else(|| unreachable!());
        assert_eq!(pixmap.data(), &[200, 100, 50, 255, 100, 50, 25, 128]);
    }
}
