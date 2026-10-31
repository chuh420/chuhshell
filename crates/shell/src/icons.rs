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
    let mut out = Pixmap::new(size, size)?;
    let (w, h) = (source.width() as f32, source.height() as f32);
    let scale = (size as f32 / w).max(size as f32 / h);
    let dx = (size as f32 - w * scale) / 2.0;
    let dy = (size as f32 - h * scale) / 2.0;
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
    let pixmap = from_rgba(image.width(), image.height(), image.as_raw())?;
    cover(&pixmap, size)
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
