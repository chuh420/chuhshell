use std::{path::Path, sync::Arc};

use services::cursors::{self, CursorTheme, PREVIEW_SHAPES, State};
use ui::tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};

use crate::app::{App, ServiceEvent};

const PREVIEW_W: u32 = 640;
const PREVIEW_H: u32 = 360;
const SHAPE_BOX: f32 = 92.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorItem {
    pub theme: CursorTheme,
    pub picture: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorData {
    pub items: Vec<CursorItem>,
    pub current: String,
    pub animation: bool,
    pub size: u32,
    pub busy: bool,
    pub error: String,
    pub generation: u64,
}

#[derive(Default)]
pub struct Cursors {
    pub data: Arc<CursorData>,
    pub state: State,
    scanning: bool,
}

fn shape_pixmap(image: &cursors::Image) -> Option<Pixmap> {
    let mut data = Vec::with_capacity(image.pixels.len() * 4);
    for &pixel in &image.pixels {
        let a = (pixel >> 24) as u8;
        let r = ((pixel >> 16) & 0xff) as u8;
        let g = ((pixel >> 8) & 0xff) as u8;
        let b = (pixel & 0xff) as u8;
        data.extend_from_slice(&[r.min(a), g.min(a), b.min(a), a]);
    }
    Pixmap::from_vec(data, IntSize::from_wh(image.width, image.height)?)
}

pub fn render_preview(theme: &Path) -> Option<Pixmap> {
    let mut canvas = Pixmap::new(PREVIEW_W, PREVIEW_H)?;
    let shapes: Vec<Pixmap> = PREVIEW_SHAPES
        .iter()
        .filter_map(|names| cursors::load_shape(theme, names, 96))
        .filter_map(|image| shape_pixmap(&image))
        .collect();
    if shapes.is_empty() {
        return None;
    }
    let count = shapes.len() as f32;
    let step = PREVIEW_W as f32 / count;
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..PixmapPaint::default()
    };
    for (index, shape) in shapes.iter().enumerate() {
        let largest = shape.width().max(shape.height()) as f32;
        let scale = (SHAPE_BOX / largest).min(step * 0.86 / largest);
        let w = shape.width() as f32 * scale;
        let h = shape.height() as f32 * scale;
        let x = step * index as f32 + (step - w) / 2.0;
        let y = (PREVIEW_H as f32 - h) / 2.0;
        canvas.draw_pixmap(
            0,
            0,
            shape.as_ref(),
            &paint,
            Transform::from_scale(scale, scale).post_translate(x, y),
            None,
        );
    }
    Some(canvas)
}

fn save_png(pixmap: &Pixmap, target: &Path) -> bool {
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = target.with_file_name(format!(".{name}.{}", std::process::id()));
    let saved = image::save_buffer_with_format(
        &temporary,
        &rgba,
        pixmap.width(),
        pixmap.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    );
    match saved.and_then(|()| std::fs::rename(&temporary, target).map_err(Into::into)) {
        Ok(()) => true,
        Err(err) => {
            log::warn!("cursor preview {}: {err}", target.display());
            let _ = std::fs::remove_file(&temporary);
            false
        }
    }
}

fn picture(theme: &CursorTheme) -> String {
    let target = cursors::preview_target(theme);
    if target.is_file() {
        return target.to_string_lossy().into_owned();
    }
    if let Err(err) = std::fs::create_dir_all(cursors::preview_cache()) {
        log::warn!("cursor previews: {err}");
        return String::new();
    }
    match render_preview(Path::new(&theme.path)) {
        Some(pixmap) if save_png(&pixmap, &target) => target.to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

fn listing() -> Vec<CursorItem> {
    cursors::list()
        .into_iter()
        .map(|theme| CursorItem {
            picture: picture(&theme),
            theme,
        })
        .collect()
}

impl App {
    pub fn start_cursors(&mut self) {
        let state = cursors::read_state();
        if !cursors::state_path().is_file()
            && let Err(err) = cursors::write_state(&state)
        {
            log::warn!("cursor state: {err}");
        }
        self.cursors.state = state;
        self.sync_cursor_state();
    }

    fn update_cursor_data(&mut self, change: impl FnOnce(&mut CursorData)) {
        let mut data = (*self.cursors.data).clone();
        change(&mut data);
        if data != *self.cursors.data {
            self.cursors.data = Arc::new(data);
        }
    }

    fn sync_cursor_state(&mut self) {
        let state = self.cursors.state.clone();
        self.update_cursor_data(|data| {
            data.current = state.theme;
            data.animation = state.animation;
            data.size = state.size;
        });
    }

    pub fn refresh_cursors(&mut self) {
        if self.cursors.scanning {
            return;
        }
        self.cursors.scanning = true;
        self.update_cursor_data(|data| data.error.clear());
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let _ = sender.send(ServiceEvent::CursorList(listing()));
        });
    }

    pub fn cursors_listed(&mut self, items: Vec<CursorItem>) {
        self.cursors.scanning = false;
        self.update_cursor_data(|data| {
            data.items = items;
            data.generation += 1;
        });
        self.sync();
        self.request_preview_pixmaps();
    }

    fn cursor_job(&mut self, job: impl FnOnce(State) -> Result<State, String> + Send + 'static) {
        if self.cursors.data.busy {
            return;
        }
        self.update_cursor_data(|data| {
            data.error.clear();
            data.busy = true;
        });
        let state = self.cursors.state.clone();
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let _ = sender.send(ServiceEvent::CursorApplied(job(state)));
        });
        self.sync();
    }

    pub fn apply_cursor(&mut self, name: &str) {
        let Some(theme) = self
            .cursors
            .data
            .items
            .iter()
            .find(|item| item.theme.name == name)
            .map(|item| item.theme.clone())
        else {
            return;
        };
        self.cursor_job(move |state| cursors::apply(&theme, &state));
    }

    pub fn toggle_cursor_animation(&mut self) {
        let enabled = !self.cursors.state.animation;
        self.cursor_job(move |state| cursors::set_animation(&state, enabled));
    }

    pub fn cycle_cursor_size(&mut self) {
        let size = cursors::next_size(self.cursors.state.size);
        self.cursor_job(move |state| cursors::set_size(&state, size));
    }

    pub fn cursor_applied(&mut self, result: Result<State, String>) {
        match result {
            Ok(state) => {
                self.cursors.state = state;
                self.update_cursor_data(|data| data.busy = false);
            }
            Err(error) => {
                self.update_cursor_data(|data| {
                    data.busy = false;
                    data.error = if error.is_empty() {
                        "Could not apply cursor".into()
                    } else {
                        error
                    };
                });
            }
        }
        self.sync_cursor_state();
        self.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_of_missing_theme_is_none() {
        assert!(render_preview(Path::new("/nonexistent/theme")).is_none());
    }
}
