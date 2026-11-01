use smithay_client_toolkit::seat::keyboard::{
    KeyEvent as SctkKeyEvent, KeyboardHandler, Keysym, Modifiers as SctkModifiers, RawModifiers,
    RepeatInfo,
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_keyboard::WlKeyboard, wl_surface::WlSurface},
};

use crate::{Event, Platform, SurfaceId};

pub mod keys {
    pub const ESCAPE: u32 = 0xff1b;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const TAB: u32 = 0xff09;
    pub const BACKSPACE: u32 = 0xff08;
    pub const DELETE: u32 = 0xffff;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const HOME: u32 = 0xff50;
    pub const END: u32 = 0xff57;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub surface: Option<SurfaceId>,
    pub keysym: u32,
    pub utf8: Option<String>,
    pub pressed: bool,
    pub modifiers: Modifiers,
    pub repeat: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyRepeat {
    pub rate: u32,
    pub delay: u32,
}

impl Platform {
    fn key(&mut self, event: SctkKeyEvent, pressed: bool) {
        let keysym: Keysym = event.keysym;
        self.events.push(Event::Key(KeyEvent {
            surface: self.keyboard_focus,
            keysym: keysym.raw(),
            utf8: event.utf8,
            pressed,
            modifiers: self.modifiers,
            repeat: false,
        }));
    }

    pub fn key_repeat(&self) -> Option<KeyRepeat> {
        self.key_repeat
    }
}

impl KeyboardHandler for Platform {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        surface: &WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.keyboard_focus = self.surface_by_wl(surface);
        self.events.push(Event::KeyboardFocus(self.keyboard_focus));
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
    ) {
        self.keyboard_focus = None;
        self.events.push(Event::KeyboardFocus(None));
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: SctkKeyEvent,
    ) {
        self.key(event, true);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: SctkKeyEvent,
    ) {
        self.key(event, true);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: SctkKeyEvent,
    ) {
        self.key(event, false);
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        modifiers: SctkModifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.modifiers = Modifiers {
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            logo: modifiers.logo,
        };
    }

    fn update_repeat_info(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        info: RepeatInfo,
    ) {
        self.key_repeat = match info {
            RepeatInfo::Repeat { rate, delay } => Some(KeyRepeat {
                rate: rate.get(),
                delay,
            }),
            RepeatInfo::Disable => None,
        };
    }
}
