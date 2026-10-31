use smithay_client_toolkit::{
    compositor::{CompositorState, FrameCallbackData, Region},
    shell::{
        WaylandSurface,
        wlr_layer::{self, Anchor, KeyboardInteractivity, LayerSurface},
        xdg::popup::Popup,
    },
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_shm, wl_surface},
};

use crate::{Damage, OutputId, Platform, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

impl From<Layer> for wlr_layer::Layer {
    fn from(layer: Layer) -> Self {
        match layer {
            Layer::Background => wlr_layer::Layer::Background,
            Layer::Bottom => wlr_layer::Layer::Bottom,
            Layer::Top => wlr_layer::Layer::Top,
            Layer::Overlay => wlr_layer::Layer::Overlay,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    pub const ALL: Edges = Edges {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };

    fn anchor(self) -> Anchor {
        let mut anchor = Anchor::empty();
        if self.top {
            anchor |= Anchor::TOP;
        }
        if self.bottom {
            anchor |= Anchor::BOTTOM;
        }
        if self.left {
            anchor |= Anchor::LEFT;
        }
        if self.right {
            anchor |= Anchor::RIGHT;
        }
        anchor
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardMode {
    #[default]
    None,
    Exclusive,
    OnDemand,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputRegion {
    #[default]
    Full,
    Empty,
}

#[derive(Clone, Debug)]
pub struct LayerConfig {
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Edges,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    pub width: u32,
    pub height: u32,
    pub exclusive_zone: i32,
    pub keyboard: KeyboardMode,
    pub input: InputRegion,
    pub output: Option<OutputId>,
}

const MAX_BUFFERS: usize = 3;

struct Slot {
    buffer: Buffer,
    pending: Damage,
}

pub(crate) enum Role {
    Layer(LayerSurface),
    Popup(Popup),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupPlacement {
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_width: i32,
    pub anchor_height: i32,
    pub width: u32,
    pub height: u32,
}

pub(crate) struct Surface {
    wl: wl_surface::WlSurface,
    role: Role,
    requested: (u32, u32),
    size: Option<(u32, u32)>,
    pool: Option<SlotPool>,
    slots: Vec<Slot>,
    slot_size: (i32, i32),
    full_damage: bool,
    frame_pending: bool,
}

impl Surface {
    pub(crate) fn new(
        layer: LayerSurface,
        config: &LayerConfig,
        compositor: &CompositorState,
    ) -> Self {
        layer.set_anchor(config.anchor.anchor());
        layer.set_margin(
            config.margin_top,
            config.margin_right,
            config.margin_bottom,
            config.margin_left,
        );
        layer.set_size(config.width, config.height);
        layer.set_exclusive_zone(config.exclusive_zone);
        layer.set_keyboard_interactivity(match config.keyboard {
            KeyboardMode::None => KeyboardInteractivity::None,
            KeyboardMode::Exclusive => KeyboardInteractivity::Exclusive,
            KeyboardMode::OnDemand => KeyboardInteractivity::OnDemand,
        });
        if config.input == InputRegion::Empty {
            match Region::new(compositor) {
                Ok(region) => layer
                    .wl_surface()
                    .set_input_region(Some(region.wl_region())),
                Err(err) => log::error!("input region: {err}"),
            }
        }
        layer.commit();
        Self {
            wl: layer.wl_surface().clone(),
            role: Role::Layer(layer),
            requested: (config.width, config.height),
            size: None,
            pool: None,
            slots: Vec::new(),
            slot_size: (0, 0),
            full_damage: true,
            frame_pending: false,
        }
    }

    pub(crate) fn new_popup(popup: Popup, placement: &PopupPlacement) -> Self {
        Self {
            wl: popup.wl_surface().clone(),
            role: Role::Popup(popup),
            requested: (placement.width, placement.height),
            size: None,
            pool: None,
            slots: Vec::new(),
            slot_size: (0, 0),
            full_damage: true,
            frame_pending: false,
        }
    }

    pub(crate) fn role(&self) -> &Role {
        &self.role
    }

    pub(crate) fn wl_surface(&self) -> &wl_surface::WlSurface {
        &self.wl
    }

    pub(crate) fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    pub(crate) fn configure(&mut self, new_size: (u32, u32)) -> (u32, u32) {
        let width = if new_size.0 == 0 {
            self.requested.0
        } else {
            new_size.0
        };
        let height = if new_size.1 == 0 {
            self.requested.1
        } else {
            new_size.1
        };
        if self.size != Some((width, height)) {
            self.full_damage = true;
        }
        self.size = Some((width, height));
        (width, height)
    }

    pub(crate) fn frame_done(&mut self) {
        self.frame_pending = false;
    }

    pub(crate) fn schedule_frame(&mut self, qh: &QueueHandle<Platform>) {
        if self.frame_pending || self.slots.is_empty() {
            return;
        }
        let wl = &self.wl;
        wl.frame(qh, FrameCallbackData(wl.clone()));
        wl.commit();
        self.frame_pending = true;
    }

    pub(crate) fn present(
        &mut self,
        shm: &Shm,
        qh: &QueueHandle<Platform>,
        pixels: &[u8],
        damage: &Damage,
        request_frame: bool,
    ) -> bool {
        let Some((width, height)) = self.size else {
            return false;
        };
        let (width, height) = (width as i32, height as i32);
        let stride = width * 4;
        let len = (stride * height) as usize;
        if width <= 0 || height <= 0 || pixels.len() < len {
            return false;
        }
        if self.slot_size != (width, height) {
            self.slots.clear();
            self.slot_size = (width, height);
        }
        if self.pool.is_none() {
            match SlotPool::new(len * 2, shm) {
                Ok(pool) => self.pool = Some(pool),
                Err(err) => {
                    log::error!("shm pool: {err}");
                    return false;
                }
            }
        }
        let Some(pool) = self.pool.as_mut() else {
            return false;
        };
        let index = match self
            .slots
            .iter()
            .position(|slot| slot.buffer.canvas(pool).is_some())
        {
            Some(index) => index,
            None if self.slots.len() < MAX_BUFFERS => {
                match pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888) {
                    Ok((buffer, _)) => {
                        self.slots.push(Slot {
                            buffer,
                            pending: Damage::full(width, height),
                        });
                        self.slots.len() - 1
                    }
                    Err(err) => {
                        log::error!("shm buffer: {err}");
                        return false;
                    }
                }
            }
            None => return false,
        };
        let bounds = Rect::new(0, 0, width, height);
        let mut region = self.slots[index].pending.clone();
        region.extend(damage);
        region.clip(bounds);
        let Some(canvas) = self.slots[index].buffer.canvas(pool) else {
            return false;
        };
        for rect in region.rects() {
            copy_swizzled(pixels, canvas, stride as usize, rect);
        }
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if i == index {
                slot.pending.clear();
            } else {
                slot.pending.extend(damage);
            }
        }
        let wl = &self.wl;
        if self.full_damage {
            wl.damage_buffer(0, 0, width, height);
            self.full_damage = false;
        } else {
            let mut clipped = damage.clone();
            clipped.clip(bounds);
            for rect in clipped.rects() {
                wl.damage_buffer(rect.x, rect.y, rect.width, rect.height);
            }
        }
        if request_frame && !self.frame_pending {
            wl.frame(qh, FrameCallbackData(wl.clone()));
            self.frame_pending = true;
        }
        if let Err(err) = self.slots[index].buffer.attach_to(wl) {
            log::error!("attach: {err}");
            return false;
        }
        wl.commit();
        true
    }
}

fn copy_swizzled(src: &[u8], dst: &mut [u8], stride: usize, rect: &Rect) {
    let x0 = rect.x as usize * 4;
    let x1 = rect.right() as usize * 4;
    for y in rect.y as usize..rect.bottom() as usize {
        let row = y * stride;
        let from = &src[row + x0..row + x1];
        let to = &mut dst[row + x0..row + x1];
        for (s, d) in from
            .as_chunks::<4>()
            .0
            .iter()
            .zip(to.as_chunks_mut::<4>().0.iter_mut())
        {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
    }
}
