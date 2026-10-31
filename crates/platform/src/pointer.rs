use smithay_client_toolkit::seat::pointer::{
    PointerEvent as SctkEvent, PointerEventKind, PointerHandler,
};
use wayland_client::{Connection, QueueHandle, protocol::wl_pointer::WlPointer};

use crate::{Event, Platform, SurfaceId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Default,
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u32),
}

impl MouseButton {
    fn from_linux(code: u32) -> Self {
        match code {
            0x110 => MouseButton::Left,
            0x111 => MouseButton::Right,
            0x112 => MouseButton::Middle,
            other => MouseButton::Other(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerKind {
    Enter,
    Leave,
    Motion,
    Press(MouseButton),
    Release(MouseButton),
    Wheel { x: i32, y: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub surface: SurfaceId,
    pub x: f32,
    pub y: f32,
    pub kind: PointerKind,
}

fn angle_delta(value120: i32, absolute: f64) -> i32 {
    if value120 != 0 {
        -value120
    } else {
        (-absolute * 8.0).round() as i32
    }
}

impl PointerHandler for Platform {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[SctkEvent],
    ) {
        for event in events {
            let Some(surface) = self.surface_by_wl(&event.surface) else {
                continue;
            };
            let kind = match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.enter_serial = Some(serial);
                    self.apply_cursor();
                    PointerKind::Enter
                }
                PointerEventKind::Leave { .. } => PointerKind::Leave,
                PointerEventKind::Motion { .. } => PointerKind::Motion,
                PointerEventKind::Press { button, .. } => {
                    PointerKind::Press(MouseButton::from_linux(button))
                }
                PointerEventKind::Release { button, .. } => {
                    PointerKind::Release(MouseButton::from_linux(button))
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => PointerKind::Wheel {
                    x: angle_delta(horizontal.value120, horizontal.absolute),
                    y: angle_delta(vertical.value120, vertical.absolute),
                },
            };
            self.events.push(Event::Pointer(PointerEvent {
                surface,
                x: event.position.0 as f32,
                y: event.position.1 as f32,
                kind,
            }));
        }
    }
}
