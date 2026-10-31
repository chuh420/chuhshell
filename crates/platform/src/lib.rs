mod geometry;
mod surface;

use std::collections::HashMap;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
    shell::wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_surface},
};

pub use calloop_wayland_source::WaylandSource;
pub use geometry::{Damage, Rect};
pub use surface::{Edges, InputRegion, KeyboardMode, Layer, LayerConfig};

use surface::Surface;

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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    shm: Shm,
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
        let platform = Platform {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            seat_state: SeatState::new(&globals, &qh),
            compositor,
            layer_shell,
            shm,
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
            .find(|(_, surface)| surface.layer() == layer)
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
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
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
