use std::sync::Arc;

use platform::{
    Damage, Edges, InputRegion, KeyboardMode, Layer, LayerConfig, OutputId, Platform, Rect,
    SurfaceId,
};
use ui::{
    anim::{Easing, Millis, Tween},
    tiny_skia::{
        Color, FillRule, FilterQuality, Paint, PathBuilder, Pattern, Pixmap, PixmapPaint,
        SpreadMode, Transform,
    },
};

use crate::wallctl::Origin;

const REVEAL: Millis = 900.0;
const UNCOVER: Millis = 700.0;

pub fn layer_config(output: OutputId) -> LayerConfig {
    LayerConfig {
        namespace: "material-wallpaper".into(),
        layer: Layer::Bottom,
        anchor: Edges::ALL,
        margin_top: 0,
        margin_right: 0,
        margin_bottom: 0,
        margin_left: 0,
        width: 0,
        height: 0,
        exclusive_zone: -1,
        keyboard: KeyboardMode::None,
        input: InputRegion::Empty,
        output: Some(output),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeKind {
    Front,
    Reveal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodeRequest {
    pub output: OutputId,
    pub generation: u64,
    pub kind: DecodeKind,
    pub front: String,
    pub incoming: String,
    pub need_front: bool,
    pub width: u32,
    pub height: u32,
}

pub struct WallSurface {
    pub output: OutputId,
    pub name: String,
    pub surface: Option<SurfaceId>,
    size: Option<(u32, u32)>,
    target: String,
    front: String,
    front_ready: bool,
    front_pixmap: Option<Arc<Pixmap>>,
    incoming: String,
    incoming_pixmap: Option<Arc<Pixmap>>,
    work: Option<Pixmap>,
    painted: f32,
    reveal_start: Option<Millis>,
    origin: (f32, f32),
    reach: f32,
    cover: f32,
    uncover: Option<Tween<f32>>,
    generation: u64,
    waiting: Option<DecodeKind>,
    static_shown: bool,
}

pub fn reach(width: f32, height: f32, x: f32, y: f32) -> f32 {
    x.max(width - x).hypot(y.max(height - y)) + 80.0
}

impl WallSurface {
    pub fn new(output: OutputId, name: String, surface: SurfaceId) -> Self {
        Self {
            output,
            name,
            surface: Some(surface),
            size: None,
            target: String::new(),
            front: String::new(),
            front_ready: false,
            front_pixmap: None,
            incoming: String::new(),
            incoming_pixmap: None,
            work: None,
            painted: 0.0,
            reveal_start: None,
            origin: (0.0, 0.0),
            reach: 0.0,
            cover: 1.0,
            uncover: None,
            generation: 0,
            waiting: None,
            static_shown: false,
        }
    }

    pub fn owns(&self, surface: SurfaceId) -> bool {
        self.surface == Some(surface)
    }

    pub fn animating(&self) -> bool {
        self.reveal_start.is_some() || self.uncover.is_some()
    }

    fn request(&mut self, kind: DecodeKind) -> Option<DecodeRequest> {
        let (width, height) = self.size?;
        self.generation += 1;
        self.waiting = Some(kind);
        Some(DecodeRequest {
            output: self.output,
            generation: self.generation,
            kind,
            front: self.front.clone(),
            incoming: self.incoming.clone(),
            need_front: kind == DecodeKind::Front || !self.static_shown,
            width,
            height,
        })
    }

    pub fn configure(&mut self, width: u32, height: u32) -> Option<DecodeRequest> {
        if self.size == Some((width, height)) && self.front_ready {
            return None;
        }
        self.size = Some((width, height));
        if !self.incoming.is_empty() {
            return self.request(DecodeKind::Reveal);
        }
        if !self.front.is_empty() {
            return self.request(DecodeKind::Front);
        }
        None
    }

    pub fn set_target(&mut self, target: &str, origin: Option<&Origin>) -> Option<DecodeRequest> {
        if target == self.target {
            return None;
        }
        self.target = target.to_string();
        if target.is_empty() {
            return None;
        }
        if self.front.is_empty() {
            self.front = target.to_string();
            return self.request(DecodeKind::Front);
        }
        if target == self.front && self.incoming.is_empty() {
            return None;
        }
        let (width, height) = self.size.map_or((0.0, 0.0), |(w, h)| (w as f32, h as f32));
        let local = origin.filter(|origin| origin.screen.is_empty() || origin.screen == self.name);
        self.origin = match local {
            Some(origin) => (origin.x, origin.y),
            None => (width / 2.0, height / 2.0),
        };
        self.reveal_start = None;
        self.work = None;
        self.incoming_pixmap = None;
        self.incoming = target.to_string();
        self.request(DecodeKind::Reveal)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decoded(
        &mut self,
        platform: &mut Platform,
        generation: u64,
        kind: DecodeKind,
        front: Option<Arc<Pixmap>>,
        incoming: Option<Arc<Pixmap>>,
        live: bool,
        now: Millis,
    ) {
        if generation != self.generation || self.waiting != Some(kind) {
            return;
        }
        self.waiting = None;
        match kind {
            DecodeKind::Front => {
                let Some(front) = front else {
                    return;
                };
                if let Some(surface) = self.surface {
                    if self.cover >= 1.0 {
                        self.static_shown = platform.present_static(surface, front.data());
                    }
                    platform.schedule_frame(surface);
                }
                self.front_ready = true;
                self.front_pixmap = live.then_some(front);
            }
            DecodeKind::Reveal => {
                let front = front.or_else(|| {
                    let surface = self.surface?;
                    let (width, height) = self.size?;
                    let pixels = platform.read_back(surface)?;
                    let size = ui::tiny_skia::IntSize::from_wh(width, height)?;
                    Pixmap::from_vec(pixels, size).map(Arc::new)
                });
                let (Some(front), Some(incoming)) = (front, incoming) else {
                    self.incoming.clear();
                    return;
                };
                let (width, height) = (front.width() as f32, front.height() as f32);
                self.reach = reach(width, height, self.origin.0, self.origin.1);
                self.work = Some((*front).clone());
                self.incoming_pixmap = Some(incoming);
                self.painted = 0.0;
                self.reveal_start = Some(now);
                if let Some(surface) = self.surface {
                    platform.schedule_frame(surface);
                }
            }
        }
    }

    fn paint_ring(&mut self, radius: f32) -> Option<Rect> {
        let incoming = self.incoming_pixmap.clone()?;
        let work = self.work.as_mut()?;
        let (ox, oy) = self.origin;
        let mut builder = PathBuilder::new();
        builder.push_circle(ox, oy, radius.max(0.01));
        if self.painted > 1.5 {
            builder.push_circle(ox, oy, self.painted - 1.0);
        }
        let path = builder.finish()?;
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.shader = Pattern::new(
            incoming.as_ref().as_ref(),
            SpreadMode::Pad,
            FilterQuality::Nearest,
            1.0,
            Transform::identity(),
        );
        work.fill_path(
            &path,
            &paint,
            FillRule::EvenOdd,
            Transform::identity(),
            None,
        );
        self.painted = radius;
        let x0 = (ox - radius).floor().max(0.0) as i32;
        let y0 = (oy - radius).floor().max(0.0) as i32;
        let x1 = (ox + radius).ceil().min(work.width() as f32) as i32;
        let y1 = (oy + radius).ceil().min(work.height() as f32) as i32;
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    fn video_shown(&self, live: bool) -> bool {
        live && self.reveal_start.is_none() && self.incoming.is_empty() && self.front_ready
    }

    pub fn tick(
        &mut self,
        platform: &mut Platform,
        live: bool,
        now: Millis,
    ) -> (Option<DecodeRequest>, bool) {
        let mut request = None;
        if let Some(start) = self.reveal_start {
            let t = ((now - start) / REVEAL).clamp(0.0, 1.0);
            let radius = self.reach * Easing::InOutCubic.value(t) as f32;
            if t >= 1.0 {
                self.finish_reveal(platform, live);
            } else if let (Some(rect), Some(surface)) = (self.paint_ring(radius), self.surface)
                && let Some(work) = self.work.as_ref()
            {
                let mut damage = Damage::default();
                damage.add(rect);
                platform.present(surface, work.data(), &damage, true);
                self.static_shown = false;
            } else if let Some(surface) = self.surface {
                platform.schedule_frame(surface);
            }
        }
        let shown = self.video_shown(live);
        if shown {
            if self.cover > 0.0 && self.uncover.is_none() {
                if self.front_pixmap.is_some() {
                    let mut tween = Tween::new(self.cover, UNCOVER, Easing::InOutQuad);
                    tween.start(now, self.cover, 0.0);
                    self.uncover = Some(tween);
                } else if self.waiting.is_none() {
                    request = self.request(DecodeKind::Front);
                }
            }
            if let Some(tween) = self.uncover.as_mut() {
                tween.tick(now);
                self.cover = tween.value();
                if self.cover <= 0.0 {
                    self.uncover = None;
                    self.cover = 0.0;
                    self.work = None;
                    self.front_pixmap = None;
                    if let Some(surface) = self.surface.take() {
                        platform.destroy_surface(surface);
                    }
                    self.static_shown = false;
                    crate::memory::trim();
                } else {
                    self.draw_cover(platform);
                }
            }
        } else if self.uncover.is_some() || self.cover < 1.0 {
            self.uncover = None;
            self.cover = 1.0;
            self.work = None;
            if self.surface.is_none() {
                self.surface = Some(platform.create_layer_surface(&layer_config(self.output)));
                self.size = None;
                self.static_shown = false;
            } else if let (Some(surface), Some(front)) = (self.surface, self.front_pixmap.as_ref())
            {
                self.static_shown = platform.present_static(surface, front.data());
            }
        }
        (request, self.animating())
    }

    fn draw_cover(&mut self, platform: &mut Platform) {
        let (Some(surface), Some(front)) = (self.surface, self.front_pixmap.as_ref()) else {
            return;
        };
        if self
            .work
            .as_ref()
            .is_none_or(|work| work.width() != front.width() || work.height() != front.height())
        {
            self.work = Pixmap::new(front.width(), front.height());
        }
        let Some(work) = self.work.as_mut() else {
            return;
        };
        work.fill(Color::TRANSPARENT);
        work.draw_pixmap(
            0,
            0,
            front.as_ref().as_ref(),
            &PixmapPaint {
                opacity: self.cover,
                ..PixmapPaint::default()
            },
            Transform::identity(),
            None,
        );
        let damage = Damage::full(work.width() as i32, work.height() as i32);
        platform.present(surface, work.data(), &damage, true);
        self.static_shown = false;
    }

    fn finish_reveal(&mut self, platform: &mut Platform, live: bool) {
        self.reveal_start = None;
        self.work = None;
        let incoming = self.incoming_pixmap.take();
        if let (Some(surface), Some(pixmap)) = (self.surface, incoming.as_ref()) {
            self.static_shown = platform.present_static(surface, pixmap.data());
        }
        crate::memory::trim();
        self.front = std::mem::take(&mut self.incoming);
        self.front_ready = true;
        self.front_pixmap = if live { incoming } else { None };
        self.painted = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reach_covers_farthest_corner() {
        let r = reach(1920.0, 1080.0, 960.0, 540.0);
        assert!((r - (960f32.hypot(540.0) + 80.0)).abs() < 0.01);
        let corner = reach(1920.0, 1080.0, 0.0, 0.0);
        assert!((corner - (1920f32.hypot(1080.0) + 80.0)).abs() < 0.01);
    }
}
