mod geometry;
mod pointer;
mod surface;

use std::collections::HashMap;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
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
};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::{
    wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    xdg::shell::client::xdg_positioner,
};

pub use calloop_wayland_source::WaylandSource;
pub use geometry::{Damage, Rect};
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
            Role::Popup(_) => return None,
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

    pub fn destroy_surface(&mut self, id: SurfaceId) {
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
