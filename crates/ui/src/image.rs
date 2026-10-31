use std::{path::Path, sync::Arc};

use resvg::usvg;
use tiny_skia::{Pixmap, Transform};

pub fn load_svg(path: &Path) -> Option<Arc<usvg::Tree>> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) => {
            log::warn!("svg {}: {err}", path.display());
            return None;
        }
    };
    let data = stretch_view_box(&data);
    match usvg::Tree::from_data(&data, &usvg::Options::default()) {
        Ok(tree) => Some(Arc::new(tree)),
        Err(err) => {
            log::warn!("svg {}: {err}", path.display());
            None
        }
    }
}

pub(crate) fn render_svg(tree: &usvg::Tree, width: u32, height: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let size = tree.size();
    let transform =
        Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Some(pixmap)
}

fn stretch_view_box(data: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(data);
    let Some(start) = text.find("<svg") else {
        return data.to_vec();
    };
    let Some(end) = text[start..].find('>').map(|end| start + end) else {
        return data.to_vec();
    };
    if text[start..end].contains("preserveAspectRatio") {
        let tag = &text[start..end];
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        let mut tag = tag.to_string();
        if let Some(attr) = tag.find("preserveAspectRatio=") {
            let quote_start = attr + "preserveAspectRatio=".len();
            let quote = tag[quote_start..].chars().next().unwrap_or('"');
            if let Some(close) = tag[quote_start + 1..].find(quote) {
                tag.replace_range(
                    attr..quote_start + 1 + close + 1,
                    "preserveAspectRatio=\"none\"",
                );
            }
        }
        out.push_str(&tag);
        out.push_str(&text[end..]);
        return out.into_bytes();
    }
    let mut out = String::with_capacity(text.len() + 32);
    out.push_str(&text[..start + 4]);
    out.push_str(" preserveAspectRatio=\"none\"");
    out.push_str(&text[start + 4..]);
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_none_aspect() {
        let out = stretch_view_box(b"<svg viewBox=\"0 0 1 1\"></svg>");
        assert_eq!(
            String::from_utf8_lossy(&out),
            "<svg preserveAspectRatio=\"none\" viewBox=\"0 0 1 1\"></svg>"
        );
    }

    #[test]
    fn replaces_existing_aspect() {
        let out = stretch_view_box(b"<svg preserveAspectRatio='xMidYMid'></svg>");
        assert_eq!(
            String::from_utf8_lossy(&out),
            "<svg preserveAspectRatio=\"none\"></svg>"
        );
    }
}
