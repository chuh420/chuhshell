use platform::{Damage, Platform, PopupPlacement, SurfaceId};
use ui::{
    Canvas, TextEngine,
    anim::Millis,
    input::AreaId,
    widgets::{TooltipStyle, TooltipView},
};

use crate::{
    bar::TooltipTarget,
    theme::{self, FONT},
};

const DELAY: Millis = 650.0;

fn style() -> TooltipStyle {
    TooltipStyle {
        family: FONT,
        text: theme::text(),
        surface: theme::surface(),
        line: theme::line(),
    }
}

enum Phase {
    Waiting(Millis),
    Open {
        surface: SurfaceId,
        view: Box<TooltipView>,
        canvas: Option<Canvas>,
    },
}

struct Tip {
    key: AreaId,
    parent: SurfaceId,
    text: String,
    anchor: PopupPlacement,
    phase: Phase,
}

#[derive(Default)]
pub struct Tooltips {
    tips: Vec<Tip>,
}

impl Tooltips {
    pub fn update(
        &mut self,
        platform: &mut Platform,
        parent: SurfaceId,
        desired: Option<TooltipTarget>,
        now: Millis,
    ) {
        let desired_key = desired.as_ref().map(|target| target.key);
        self.tips.retain_mut(|tip| {
            if tip.parent != parent {
                return true;
            }
            let wanted = Some(tip.key) == desired_key;
            match &mut tip.phase {
                Phase::Waiting(_) => wanted,
                Phase::Open { surface, view, .. } => {
                    if wanted && view.is_closing() {
                        view.open(now);
                        platform.schedule_frame(*surface);
                    } else if !wanted && !view.is_closing() {
                        view.close(now);
                        platform.schedule_frame(*surface);
                    }
                    true
                }
            }
        });
        if let Some(target) = desired
            && !self
                .tips
                .iter()
                .any(|tip| tip.parent == parent && tip.key == target.key)
        {
            self.tips.push(Tip {
                key: target.key,
                parent,
                text: target.text,
                anchor: target.anchor,
                phase: Phase::Waiting(now + DELAY),
            });
        }
    }

    pub fn next_deadline(&self) -> Option<Millis> {
        self.tips
            .iter()
            .filter_map(|tip| match tip.phase {
                Phase::Waiting(deadline) => Some(deadline),
                Phase::Open { .. } => None,
            })
            .reduce(f64::min)
    }

    pub fn poll(&mut self, platform: &mut Platform, engine: &mut TextEngine, now: Millis) {
        for tip in &mut self.tips {
            let Phase::Waiting(deadline) = tip.phase else {
                continue;
            };
            if deadline > now {
                continue;
            }
            let mut view = TooltipView::new(engine, &tip.text, &style());
            let placement = PopupPlacement {
                width: view.width.ceil() as u32,
                height: view.height.ceil() as u32,
                ..tip.anchor
            };
            let Some(surface) = platform.create_popup(tip.parent, &placement) else {
                continue;
            };
            view.open(now);
            tip.phase = Phase::Open {
                surface,
                view: Box::new(view),
                canvas: None,
            };
        }
    }

    pub fn owns(&self, surface: SurfaceId) -> bool {
        self.tips
            .iter()
            .any(|tip| matches!(tip.phase, Phase::Open { surface: own, .. } if own == surface))
    }

    pub fn configure(
        &mut self,
        platform: &mut Platform,
        engine: &mut TextEngine,
        surface: SurfaceId,
        width: u32,
        height: u32,
        now: Millis,
    ) {
        for tip in &mut self.tips {
            if let Phase::Open {
                surface: own,
                canvas,
                ..
            } = &mut tip.phase
                && *own == surface
            {
                *canvas = Canvas::new(width, height);
            }
        }
        self.draw(platform, engine, surface, now, true);
    }

    pub fn frame(
        &mut self,
        platform: &mut Platform,
        engine: &mut TextEngine,
        surface: SurfaceId,
        now: Millis,
    ) {
        self.draw(platform, engine, surface, now, false);
    }

    fn draw(
        &mut self,
        platform: &mut Platform,
        engine: &mut TextEngine,
        surface: SurfaceId,
        now: Millis,
        full: bool,
    ) {
        let mut finished = false;
        for tip in &mut self.tips {
            let Phase::Open {
                surface: own,
                view,
                canvas,
            } = &mut tip.phase
            else {
                continue;
            };
            if *own != surface {
                continue;
            }
            let Some(canvas) = canvas.as_mut() else {
                continue;
            };
            let running = view.tick(now);
            if view.finished_closing() {
                finished = true;
                continue;
            }
            let mut damage = view.tree.collect_damage();
            if full {
                damage = Damage::full(canvas.width() as i32, canvas.height() as i32);
            }
            if damage.is_empty() {
                if running {
                    platform.schedule_frame(surface);
                }
                continue;
            }
            canvas.render(&mut view.tree, engine, &damage);
            platform.present(surface, canvas.data(), &damage, running);
        }
        if finished {
            self.remove(platform, surface);
        }
    }

    pub fn drop_parent(&mut self, platform: &mut Platform, parent: SurfaceId) {
        self.tips.retain(|tip| {
            if tip.parent != parent {
                return true;
            }
            if let Phase::Open { surface, .. } = tip.phase {
                platform.destroy_surface(surface);
            }
            false
        });
    }

    pub fn closed(&mut self, surface: SurfaceId) {
        self.tips
            .retain(|tip| !matches!(tip.phase, Phase::Open { surface: own, .. } if own == surface));
    }

    fn remove(&mut self, platform: &mut Platform, surface: SurfaceId) {
        platform.destroy_surface(surface);
        self.closed(surface);
    }
}
