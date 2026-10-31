mod geometry;
mod keyboard;
mod pointer;
mod surface;

use std::collections::HashMap;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState, pointer::cursor_shape::CursorShapeManager},
    shell::{
        wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        xdg::{
            XdgPositioner, XdgShell,
            popup::{Popup, PopupConfigure, PopupHandler},
            window::{Window, WindowConfigure, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler},
    subcompositor::SubcompositorState,
};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_buffer, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::{
    wp::{
        cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
        idle_inhibit::zv1::client::{
            zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
            zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
        },
        single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1,
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    xdg::shell::client::xdg_positioner,
};

pub use calloop_wayland_source::WaylandSource;
pub use geometry::{Damage, Rect};
pub use keyboard::{KeyEvent, KeyRepeat, Modifiers, keys};
pub use pointer::{CursorShape, MouseButton, PointerEvent, PointerKind};
pub use surface::{Edges, InputRegion, KeyboardMode, Layer, LayerConfig, PopupPlacement};

use surface::{Role, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(u32);

#[derive(Clone, Debug, Default)]
pub struct OutputDesc {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub scale: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    OutputAdded(OutputId),
    OutputRemoved(OutputId),
    Configure {
        surface: SurfaceId,
        width: u32,
        height: u32,
    },
    Closed(SurfaceId),
    Frame(SurfaceId),
    Pointer(PointerEvent),
    Key(KeyEvent),
    KeyboardFocus(Option<SurfaceId>),
}

#[derive(Debug)]
pub enum Error {
    Connect(wayland_client::ConnectError),
    Globals(wayland_client::globals::GlobalError),
    Bind(wayland_client::globals::BindError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Connect(err) => write!(f, "wayland connect: {err}"),
            Error::Globals(err) => write!(f, "wayland globals: {err}"),
            Error::Bind(err) => write!(f, "wayland bind: {err}"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Platform {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    xdg_shell: XdgShell,
    shm: Shm,
    cursor_shapes: Option<CursorShapeManager>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_device: Option<WpCursorShapeDeviceV1>,
    enter_serial: Option<u32>,
    cursor: CursorShape,
    idle_manager: Option<ZwpIdleInhibitManagerV1>,
    inhibitors: HashMap<SurfaceId, ZwpIdleInhibitorV1>,
    subcompositor: Option<SubcompositorState>,
    viewporter: Option<WpViewporter>,
    single_pixel: Option<WpSinglePixelBufferManagerV1>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focus: Option<SurfaceId>,
    modifiers: Modifiers,
    key_repeat: Option<KeyRepeat>,
    qh: QueueHandle<Platform>,
    outputs: HashMap<OutputId, wl_output::WlOutput>,
    surfaces: HashMap<SurfaceId, Surface>,
    next_surface: u32,
    events: Vec<Event>,
}

impl Platform {
    pub fn connect() -> Result<(Platform, WaylandSource<Platform>), Error> {
        let conn = Connection::connect_to_env().map_err(Error::Connect)?;
        let (globals, queue): (_, EventQueue<Platform>) =
            registry_queue_init(&conn).map_err(Error::Globals)?;
        let qh = queue.handle();
        let compositor = CompositorState::bind(&globals, &qh).map_err(Error::Bind)?;
        let layer_shell = LayerShell::bind(&globals, &qh).map_err(Error::Bind)?;
        let shm = Shm::bind(&globals, &qh).map_err(Error::Bind)?;
        let xdg_shell = XdgShell::bind(&globals, &qh).map_err(Error::Bind)?;
        let cursor_shapes = match CursorShapeManager::bind(&globals, &qh) {
            Ok(manager) => Some(manager),
            Err(err) => {
                log::warn!("cursor shape: {err}");
                None
            }
        };
        let idle_manager = match globals.bind(&qh, 1..=1, IdleData) {
            Ok(manager) => Some(manager),
            Err(err) => {
                log::warn!("idle inhibit: {err}");
                None
            }
        };
        let subcompositor =
            match SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &qh) {
                Ok(state) => Some(state),
                Err(err) => {
                    log::warn!("subcompositor: {err}");
                    None
                }
            };
        let viewporter = match globals.bind(&qh, 1..=1, ProtocolData) {
            Ok(viewporter) => Some(viewporter),
            Err(err) => {
                log::warn!("viewporter: {err}");
                None
            }
        };
        let single_pixel = match globals.bind(&qh, 1..=1, ProtocolData) {
            Ok(manager) => Some(manager),
            Err(err) => {
                log::warn!("single pixel buffer: {err}");
                None
            }
        };
        let platform = Platform {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            seat_state: SeatState::new(&globals, &qh),
            compositor,
            layer_shell,
            xdg_shell,
            shm,
            cursor_shapes,
            pointer: None,
            cursor_device: None,
            enter_serial: None,
            cursor: CursorShape::Default,
            idle_manager,
            inhibitors: HashMap::new(),
            subcompositor,
            viewporter,
            single_pixel,
            keyboard: None,
            keyboard_focus: None,
            modifiers: Modifiers::default(),
            key_repeat: None,
            qh,
            outputs: HashMap::new(),
            surfaces: HashMap::new(),
            next_surface: 0,
            events: Vec::new(),
        };
        Ok((platform, WaylandSource::new(conn, queue)))
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn outputs(&self) -> Vec<OutputId> {
        let mut ids: Vec<OutputId> = self.outputs.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn output_desc(&self, id: OutputId) -> Option<OutputDesc> {
        let output = self.outputs.get(&id)?;
        let info = self.output_state.info(output)?;
        let (width, height) = info.logical_size.unwrap_or_default();
        Some(OutputDesc {
            name: info.name.unwrap_or_default(),
            width,
            height,
            scale: info.scale_factor,
        })
    }

    pub fn create_layer_surface(&mut self, config: &LayerConfig) -> SurfaceId {
        let id = SurfaceId(self.next_surface);
        self.next_surface += 1;
        let wl_surface = self.compositor.create_surface(&self.qh);
        let output = config.output.and_then(|output| self.outputs.get(&output));
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            wl_surface,
            config.layer.into(),
            Some(config.namespace.clone()),
            output,
        );
        let surface = Surface::new(layer, config, &self.compositor);
        self.surfaces.insert(id, surface);
        id
    }

    pub fn create_popup(
        &mut self,
        parent: SurfaceId,
        placement: &PopupPlacement,
    ) -> Option<SurfaceId> {
        let parent_layer = match self.surfaces.get(&parent)?.role() {
            Role::Layer(layer) => layer.clone(),
            Role::Popup(_) | Role::Subsurface(_) => return None,
        };
        let positioner = match XdgPositioner::new(&self.xdg_shell) {
            Ok(positioner) => positioner,
            Err(err) => {
                log::error!("positioner: {err}");
                return None;
            }
        };
        positioner.set_anchor_rect(
            placement.anchor_x,
            placement.anchor_y,
            placement.anchor_width.max(1),
            placement.anchor_height.max(1),
        );
        positioner.set_anchor(xdg_positioner::Anchor::BottomRight);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_size(
            placement.width.max(1) as i32,
            placement.height.max(1) as i32,
        );
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::from_bits_truncate(15),
        );
        positioner.set_offset(0, 0);
        let wl_surface = self.compositor.create_surface(&self.qh);
        let popup =
            match Popup::from_surface(None, &positioner, &self.qh, wl_surface, &self.xdg_shell) {
                Ok(popup) => popup,
                Err(err) => {
                    log::error!("popup: {err}");
                    return None;
                }
            };
        parent_layer.get_popup(popup.xdg_popup());
        popup.wl_surface().commit();
        let id = SurfaceId(self.next_surface);
        self.next_surface += 1;
        self.surfaces
            .insert(id, Surface::new_popup(popup, placement));
        Some(id)
    }

    pub fn create_subsurface(&mut self, parent: SurfaceId) -> Option<SurfaceId> {
        let parent_wl = self.surfaces.get(&parent)?.wl_surface().clone();
        let subcompositor = self.subcompositor.as_ref()?;
        let (subsurface, wl) = subcompositor.create_subsurface(parent_wl, &self.qh);
        match Region::new(&self.compositor) {
            Ok(region) => wl.set_input_region(Some(region.wl_region())),
            Err(err) => log::error!("input region: {err}"),
        }
        let id = SurfaceId(self.next_surface);
        self.next_surface += 1;
        self.surfaces
            .insert(id, Surface::new_subsurface(subsurface, wl));
        Some(id)
    }

    pub fn set_subsurface_geometry(
        &mut self,
        id: SurfaceId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        if let Role::Subsurface(subsurface) = surface.role() {
            subsurface.set_position(x, y);
        }
        surface.set_size(width, height);
    }

    pub fn set_layer_size(&mut self, id: SurfaceId, width: u32, height: u32) {
        let Some(surface) = self.surfaces.get(&id) else {
            return;
        };
        if let Role::Layer(layer) = surface.role() {
            layer.set_size(width, height);
            surface.wl_surface().commit();
        }
    }

    pub fn set_input_rect(&mut self, id: SurfaceId, rect: Option<Rect>) {
        let Some(surface) = self.surfaces.get(&id) else {
            return;
        };
        match rect {
            None => surface.wl_surface().set_input_region(None),
            Some(rect) => match Region::new(&self.compositor) {
                Ok(region) => {
                    region.add(rect.x, rect.y, rect.width, rect.height);
                    surface
                        .wl_surface()
                        .set_input_region(Some(region.wl_region()));
                }
                Err(err) => log::error!("input region: {err}"),
            },
        }
    }

    pub fn set_keyboard_mode(&mut self, id: SurfaceId, mode: KeyboardMode) {
        let Some(surface) = self.surfaces.get(&id) else {
            return;
        };
        if let Role::Layer(layer) = surface.role() {
            layer.set_keyboard_interactivity(mode.into());
        }
    }

    pub fn commit(&mut self, id: SurfaceId, request_frame: bool) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        if request_frame {
            surface.request_frame(&self.qh);
        }
        surface.wl_surface().commit();
    }

    pub fn frame_pending(&self, id: SurfaceId) -> bool {
        self.surfaces.get(&id).is_some_and(Surface::frame_pending)
    }

    pub fn set_solid(&mut self, id: SurfaceId, rgba: [f32; 4], request_frame: bool) -> bool {
        let Some((width, height)) = self.surfaces.get(&id).and_then(Surface::size) else {
            return false;
        };
        let alpha = rgba[3].clamp(0.0, 1.0);
        if let (Some(viewporter), Some(manager)) = (&self.viewporter, &self.single_pixel) {
            let Some(surface) = self.surfaces.get_mut(&id) else {
                return false;
            };
            let channel = |value: f32| {
                ((value.clamp(0.0, 1.0) * alpha) as f64 * u32::MAX as f64).round() as u32
            };
            let buffer = manager.create_u32_rgba_buffer(
                channel(rgba[0]),
                channel(rgba[1]),
                channel(rgba[2]),
                (alpha as f64 * u32::MAX as f64).round() as u32,
                &self.qh,
                PixelBuffer,
            );
            let wl = surface.wl_surface().clone();
            let viewport = surface
                .viewport
                .get_or_insert_with(|| viewporter.get_viewport(&wl, &self.qh, ProtocolData));
            viewport.set_destination(width as i32, height as i32);
            wl.attach(Some(&buffer), 0, 0);
            wl.damage_buffer(0, 0, 1, 1);
            if request_frame {
                surface.request_frame(&self.qh);
            }
            wl.commit();
            surface.mark_content();
            return true;
        }
        let pixel = [
            (rgba[0].clamp(0.0, 1.0) * alpha * 255.0).round() as u8,
            (rgba[1].clamp(0.0, 1.0) * alpha * 255.0).round() as u8,
            (rgba[2].clamp(0.0, 1.0) * alpha * 255.0).round() as u8,
            (alpha * 255.0).round() as u8,
        ];
        let pixels: Vec<u8> = pixel
            .iter()
            .copied()
            .cycle()
            .take(width as usize * height as usize * 4)
            .collect();
        let damage = Damage::full(width as i32, height as i32);
        self.present(id, &pixels, &damage, request_frame)
    }

    pub fn set_cursor(&mut self, shape: CursorShape) {
        if self.cursor == shape {
            return;
        }
        self.cursor = shape;
        self.apply_cursor();
    }

    fn apply_cursor(&mut self) {
        let (Some(device), Some(serial)) = (&self.cursor_device, self.enter_serial) else {
            return;
        };
        let shape = match self.cursor {
            CursorShape::Default => Shape::Default,
            CursorShape::Pointer => Shape::Pointer,
        };
        device.set_shape(serial, shape);
    }

    pub fn set_idle_inhibit(&mut self, id: SurfaceId, enabled: bool) {
        if !enabled {
            if let Some(inhibitor) = self.inhibitors.remove(&id) {
                inhibitor.destroy();
            }
            return;
        }
        if self.inhibitors.contains_key(&id) {
            return;
        }
        let (Some(manager), Some(surface)) = (&self.idle_manager, self.surfaces.get(&id)) else {
            return;
        };
        let inhibitor = manager.create_inhibitor(surface.wl_surface(), &self.qh, IdleData);
        self.inhibitors.insert(id, inhibitor);
    }

    pub fn destroy_surface(&mut self, id: SurfaceId) {
        self.set_idle_inhibit(id, false);
        self.surfaces.remove(&id);
    }

    pub fn surface_size(&self, id: SurfaceId) -> Option<(u32, u32)> {
        self.surfaces.get(&id).and_then(Surface::size)
    }

    pub fn present(
        &mut self,
        id: SurfaceId,
        pixels: &[u8],
        damage: &Damage,
        request_frame: bool,
    ) -> bool {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return false;
        };
        surface.present(&self.shm, &self.qh, pixels, damage, request_frame)
    }

    pub fn schedule_frame(&mut self, id: SurfaceId) {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.schedule_frame(&self.qh);
        }
    }

    fn surface_by_layer(&self, layer: &LayerSurface) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .find(|(_, surface)| matches!(surface.role(), Role::Layer(own) if own == layer))
            .map(|(id, _)| *id)
    }

    fn surface_by_popup(&self, popup: &Popup) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .find(|(_, surface)| matches!(surface.role(), Role::Popup(own) if own == popup))
            .map(|(id, _)| *id)
    }

    fn surface_by_wl(&self, wl: &wl_surface::WlSurface) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .find(|(_, surface)| surface.wl_surface() == wl)
            .map(|(id, _)| *id)
    }

    fn output_id(&self, output: &wl_output::WlOutput) -> Option<OutputId> {
        self.output_state.info(output).map(|info| OutputId(info.id))
    }
}

impl CompositorHandler for Platform {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, wl: &wl_surface::WlSurface, _: u32) {
        if let Some(id) = self.surface_by_wl(wl) {
            if let Some(surface) = self.surfaces.get_mut(&id) {
                surface.frame_done();
            }
            self.events.push(Event::Frame(id));
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Platform {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Some(id) = self.output_id(&output) {
            self.outputs.insert(id, output);
            self.events.push(Event::OutputAdded(id));
        }
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let id = self
            .outputs
            .iter()
            .find(|(_, known)| **known == output)
            .map(|(id, _)| *id);
        if let Some(id) = id {
            self.outputs.remove(&id);
            self.events.push(Event::OutputRemoved(id));
        }
    }
}

impl LayerShellHandler for Platform {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.surface_by_layer(layer) {
            self.surfaces.remove(&id);
            self.events.push(Event::Closed(id));
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_by_layer(layer) else {
            return;
        };
        if let Some(surface) = self.surfaces.get_mut(&id) {
            let (width, height) = surface.configure(configure.new_size);
            self.events.push(Event::Configure {
                surface: id,
                width,
                height,
            });
        }
    }
}

impl SeatHandler for Platform {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(err) => log::error!("keyboard: {err}"),
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    self.cursor_device = self
                        .cursor_shapes
                        .as_ref()
                        .map(|manager| manager.get_shape_device(&pointer, qh));
                    self.pointer = Some(pointer);
                }
                Err(err) => log::error!("pointer: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
            self.keyboard_focus = None;
        }
        if capability == Capability::Pointer {
            if let Some(device) = self.cursor_device.take() {
                device.destroy();
            }
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
            self.enter_serial = None;
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

pub struct IdleData;

impl smithay_client_toolkit::dispatch2::Dispatch2<ZwpIdleInhibitManagerV1, Platform> for IdleData {
    fn event(
        &self,
        _: &mut Platform,
        _: &ZwpIdleInhibitManagerV1,
        _: <ZwpIdleInhibitManagerV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<Platform>,
    ) {
    }
}

impl smithay_client_toolkit::dispatch2::Dispatch2<ZwpIdleInhibitorV1, Platform> for IdleData {
    fn event(
        &self,
        _: &mut Platform,
        _: &ZwpIdleInhibitorV1,
        _: <ZwpIdleInhibitorV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<Platform>,
    ) {
    }
}

pub struct ProtocolData;

impl smithay_client_toolkit::dispatch2::Dispatch2<WpViewporter, Platform> for ProtocolData {
    fn event(
        &self,
        _: &mut Platform,
        _: &WpViewporter,
        _: <WpViewporter as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<Platform>,
    ) {
    }
}

impl smithay_client_toolkit::dispatch2::Dispatch2<WpViewport, Platform> for ProtocolData {
    fn event(
        &self,
        _: &mut Platform,
        _: &WpViewport,
        _: <WpViewport as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<Platform>,
    ) {
    }
}

impl smithay_client_toolkit::dispatch2::Dispatch2<WpSinglePixelBufferManagerV1, Platform>
    for ProtocolData
{
    fn event(
        &self,
        _: &mut Platform,
        _: &WpSinglePixelBufferManagerV1,
        _: <WpSinglePixelBufferManagerV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<Platform>,
    ) {
    }
}

pub struct PixelBuffer;

impl smithay_client_toolkit::dispatch2::Dispatch2<wl_buffer::WlBuffer, Platform> for PixelBuffer {
    fn event(
        &self,
        _: &mut Platform,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &Connection,
        _: &QueueHandle<Platform>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

impl WindowHandler for Platform {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        _: WindowConfigure,
        _: u32,
    ) {
    }
}

impl PopupHandler for Platform {
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        popup: &Popup,
        config: PopupConfigure,
    ) {
        let Some(id) = self.surface_by_popup(popup) else {
            return;
        };
        if let Some(surface) = self.surfaces.get_mut(&id) {
            let (width, height) =
                surface.configure((config.width.max(0) as u32, config.height.max(0) as u32));
            self.events.push(Event::Configure {
                surface: id,
                width,
                height,
            });
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        if let Some(id) = self.surface_by_popup(popup) {
            self.surfaces.remove(&id);
            self.events.push(Event::Closed(id));
        }
    }
}

impl ShmHandler for Platform {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Platform {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Platform);
delegate_dispatch2!(Platform);
