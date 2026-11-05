use std::{
    collections::HashMap,
    io::Read,
    os::fd::AsFd,
    path::PathBuf,
    process::Child,
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use calloop::{Interest, Mode, PostAction, channel::Sender, generic::Generic};
use services::wallpapers::{self, Listing, StateWatcher, WallItem};
use ui::tiny_skia::Pixmap;

use crate::{
    app::{App, ServiceEvent},
    icons,
};

pub type PreviewKey = (String, String);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WallData {
    pub items: Vec<WallItem>,
    pub current: String,
    pub busy: bool,
    pub error: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub x: f32,
    pub y: f32,
    pub screen: String,
}

#[derive(Default)]
pub struct Walls {
    pub data: Arc<WallData>,
    pub image: String,
    pub live: bool,
    pub origin: Option<Origin>,
    pub previews: Arc<HashMap<PreviewKey, String>>,
    pub pixmaps: HashMap<String, Option<Arc<Pixmap>>>,
    pending: std::collections::HashSet<String>,
    scanning: bool,
    gallery: Option<mpsc::Sender<Vec<PreviewKey>>>,
    watcher: Option<StateWatcher>,
}

struct Job {
    item: PreviewKey,
    child: Child,
    temporary: PathBuf,
    target: PathBuf,
    deadline: Instant,
}

fn stop(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn gallery(requests: Receiver<Vec<PreviewKey>>, events: Sender<ServiceEvent>) {
    if let Err(err) = std::fs::create_dir_all(wallpapers::preview_cache()) {
        log::warn!("wallpaper previews: {err}");
    }
    let emit = |item: &PreviewKey, picture: String| {
        let _ = events.send(ServiceEvent::Preview(
            item.0.clone(),
            item.1.clone(),
            picture,
        ));
    };
    let mut requested: Vec<PreviewKey> = Vec::new();
    let mut completed: HashMap<PreviewKey, String> = HashMap::new();
    let mut job: Option<Job> = None;
    loop {
        let message = if job.is_some() {
            match requests.recv_timeout(Duration::from_millis(50)) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match requests.recv() {
                Ok(message) => Some(message),
                Err(_) => break,
            }
        };
        if let Some(mut message) = message {
            while let Ok(newer) = requests.try_recv() {
                message = newer;
            }
            if message.len() > 8 {
                log::warn!("wallpaper previews: Invalid preview request");
            } else {
                requested = message;
            }
            if let Some(current) = job.as_mut()
                && !requested.contains(&current.item)
            {
                stop(&mut current.child);
                let _ = std::fs::remove_file(&current.temporary);
                job = None;
            }
        }
        if let Some(current) = job.as_mut() {
            let code = current.child.try_wait().ok().flatten();
            if code.is_some() || Instant::now() >= current.deadline {
                stop(&mut current.child);
                let ok = code.is_some_and(|code| code.success()) && current.temporary.is_file();
                if ok && std::fs::rename(&current.temporary, &current.target).is_ok() {
                    completed.insert(
                        current.item.clone(),
                        current.target.to_string_lossy().into_owned(),
                    );
                    emit(&current.item, current.target.to_string_lossy().into_owned());
                } else {
                    emit(&current.item, String::new());
                }
                let _ = std::fs::remove_file(&current.temporary);
                job = None;
            }
        }
        while job.is_none() && !requested.is_empty() {
            let item = requested.remove(0);
            if let Some(picture) = completed.get(&item) {
                emit(&item, picture.clone());
                continue;
            }
            let target = match wallpapers::preview_target(&item.0) {
                Ok(target) => target,
                Err(_) => {
                    emit(&item, String::new());
                    continue;
                }
            };
            if target.is_file() {
                let picture = target.to_string_lossy().into_owned();
                completed.insert(item.clone(), picture.clone());
                emit(&item, picture);
                continue;
            }
            let stem = target
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let temporary = target.with_file_name(format!("{stem}.{}.png", std::process::id()));
            let source = PathBuf::from(&item.0);
            match wallpapers::preview_command(&source, &temporary).spawn() {
                Ok(child) => {
                    job = Some(Job {
                        item,
                        child,
                        temporary,
                        target,
                        deadline: Instant::now() + Duration::from_secs(20),
                    });
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&temporary);
                    emit(&item, String::new());
                }
            }
        }
    }
    if let Some(mut current) = job {
        stop(&mut current.child);
        let _ = std::fs::remove_file(&current.temporary);
    }
}

impl App {
    pub fn start_wallpapers(&mut self) {
        let (sender, receiver) = mpsc::channel();
        let events = self.events.clone();
        std::thread::spawn(move || gallery(receiver, events));
        self.walls.gallery = Some(sender);
        self.load_wallpaper_state(true);
        match StateWatcher::new() {
            Ok(watcher) => match watcher.as_fd().try_clone_to_owned() {
                Ok(fd) => {
                    self.walls.watcher = Some(watcher);
                    let source = Generic::new(fd, Interest::READ, Mode::Level);
                    let result = self.handle.insert_source(source, |_, _, app| {
                        let changed = app.walls.watcher.as_mut().is_some_and(StateWatcher::drain);
                        if changed {
                            app.load_wallpaper_state(false);
                        }
                        Ok(PostAction::Continue)
                    });
                    if let Err(err) = result {
                        log::error!("wallpaper state watch: {err}");
                    }
                }
                Err(err) => log::error!("wallpaper state watch: {err}"),
            },
            Err(err) => log::error!("wallpaper state watch: {err}"),
        }
        self.refresh_wallpapers();
    }

    fn update_wall_data(&mut self, change: impl FnOnce(&mut WallData)) {
        let mut data = (*self.walls.data).clone();
        change(&mut data);
        if data != *self.walls.data {
            self.walls.data = Arc::new(data);
        }
    }

    fn load_wallpaper_state(&mut self, startup: bool) {
        let previous = self.walls.image.clone();
        match wallpapers::read_state() {
            Some(state) => {
                self.update_wall_data(|data| data.current = state.path.clone());
                self.walls.image = state.image;
                self.walls.live = state.live;
            }
            None => {
                if self.walls.data.current.is_empty() {
                    let default = wallpapers::default_wallpaper()
                        .to_string_lossy()
                        .into_owned();
                    self.update_wall_data(|data| data.current = default.clone());
                    self.walls.image = default;
                }
            }
        }
        if !startup {
            self.sync();
        }
        self.sync_wallpapers();
        if !startup && self.walls.image != previous {
            self.theme_wallpaper_changed();
        }
    }

    pub fn refresh_wallpapers(&mut self) {
        if self.walls.scanning {
            return;
        }
        self.walls.scanning = true;
        self.update_wall_data(|data| data.error.clear());
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let _ = sender.send(ServiceEvent::WallList(wallpapers::list()));
        });
    }

    pub fn wallpapers_listed(&mut self, listing: Listing) {
        self.walls.scanning = false;
        self.update_wall_data(|data| {
            data.items = listing.items;
            data.current = listing.current;
            data.generation += 1;
        });
        self.walls.image = listing.image;
        self.walls.live = listing.live;
        self.sync();
        self.sync_wallpapers();
    }

    pub fn apply_wallpaper(&mut self, path: &str, origin: Origin) {
        if self.walls.data.busy {
            return;
        }
        self.update_wall_data(|data| {
            data.error.clear();
            data.busy = true;
        });
        self.walls.origin = Some(origin);
        let sender = self.events.clone();
        let path = path.to_string();
        std::thread::spawn(move || {
            let result = match wallpapers::apply_command(&path).spawn() {
                Ok(mut child) => {
                    let mut stderr = String::new();
                    if let Some(mut pipe) = child.stderr.take() {
                        let _ = pipe.read_to_string(&mut stderr);
                    }
                    match child.wait() {
                        Ok(status) if status.success() => Ok(()),
                        _ => Err(stderr.trim().to_string()),
                    }
                }
                Err(_) => Err(String::new()),
            };
            let result = result.and_then(|()| {
                wallpapers::adopt_legacy_state().map_err(|err| format!("wallpaper state: {err}"))
            });
            let _ = sender.send(ServiceEvent::WallApplied(result));
        });
        self.sync();
    }

    pub fn wallpaper_applied(&mut self, result: Result<(), String>) {
        self.update_wall_data(|data| {
            data.busy = false;
            if let Err(error) = &result {
                data.error = if error.is_empty() {
                    "Could not apply wallpaper".into()
                } else {
                    error.clone()
                };
            }
        });
        if result.is_ok() {
            self.walls.scanning = false;
            self.refresh_wallpapers();
        }
        self.sync();
    }

    pub fn request_previews(&mut self, items: Vec<PreviewKey>) {
        if let Some(gallery) = &self.walls.gallery {
            let _ = gallery.send(items);
        }
    }

    pub fn walls_panel_changed(&mut self, panel: &str) {
        if panel != "wallpapers" {
            self.request_previews(Vec::new());
            self.walls.pixmaps.clear();
            self.walls.pending.clear();
        } else {
            self.refresh_wallpapers();
            self.refresh_cursors();
        }
    }

    pub fn preview_ready(&mut self, path: String, revision: String, picture: String) {
        if picture.is_empty() {
            return;
        }
        let mut previews = (*self.walls.previews).clone();
        previews.insert((path, revision), picture);
        self.walls.previews = Arc::new(previews);
        self.sync();
        self.request_preview_pixmaps();
    }

    pub fn request_preview_pixmaps(&mut self) {
        for (picture, width, height) in self.overlay.preview_pixmap_requests() {
            if self.walls.pixmaps.contains_key(&picture)
                || !self.walls.pending.insert(picture.clone())
            {
                continue;
            }
            let sender = self.events.clone();
            std::thread::spawn(move || {
                let pixmap =
                    icons::load_cover(std::path::Path::new(&picture), width, height).map(Arc::new);
                let _ = sender.send(ServiceEvent::PreviewPixmap(picture, pixmap));
            });
        }
    }

    pub fn preview_pixmap_loaded(&mut self, picture: String, pixmap: Option<Arc<Pixmap>>) {
        if !self.walls.pending.remove(&picture) {
            return;
        }
        self.walls.pixmaps.insert(picture, pixmap);
        self.overlay.set_wall_pixmaps(&self.walls.pixmaps);
        self.overlay.schedule(&mut self.platform);
    }
}

impl App {
    pub fn add_wallpaper(&mut self, output: platform::OutputId) {
        if self.wall_views.contains_key(&output) {
            return;
        }
        let surface = self
            .platform
            .create_layer_surface(&crate::wallpaper::layer_config(output));
        let name = self
            .platform
            .output_desc(output)
            .map(|desc| desc.name)
            .unwrap_or_default();
        self.wall_views.insert(
            output,
            crate::wallpaper::WallSurface::new(output, name, surface),
        );
        self.sync_wallpapers();
    }

    pub fn remove_wallpaper(&mut self, output: platform::OutputId) {
        if let Some(view) = self.wall_views.remove(&output)
            && let Some(surface) = view.surface
        {
            self.platform.destroy_surface(surface);
        }
    }

    pub fn wallpaper_configure(
        &mut self,
        surface: platform::SurfaceId,
        width: u32,
        height: u32,
    ) -> bool {
        let Some(view) = self.wall_views.values_mut().find(|view| view.owns(surface)) else {
            return false;
        };
        if let Some(request) = view.configure(width, height) {
            self.spawn_wall_decode(request);
        }
        true
    }

    pub fn wallpaper_closed(&mut self, surface: platform::SurfaceId) {
        for view in self.wall_views.values_mut() {
            if view.owns(surface) {
                view.surface = None;
            }
        }
    }

    pub fn wallpaper_frame(&mut self, surface: platform::SurfaceId) -> bool {
        let Some(output) = self
            .wall_views
            .values()
            .find(|view| view.owns(surface))
            .map(|view| view.output)
        else {
            return false;
        };
        self.tick_wallpaper(output);
        true
    }

    fn tick_wallpaper(&mut self, output: platform::OutputId) {
        let now = self.now();
        let live = self.walls.live;
        let Some(view) = self.wall_views.get_mut(&output) else {
            return;
        };
        let (request, _) = view.tick(&mut self.platform, live, now);
        if let Some(request) = request {
            self.spawn_wall_decode(request);
        }
    }

    pub fn sync_wallpapers(&mut self) {
        let image = self.walls.image.clone();
        let origin = self.walls.origin.clone();
        let outputs: Vec<platform::OutputId> = self.wall_views.keys().copied().collect();
        for output in outputs {
            let request = self
                .wall_views
                .get_mut(&output)
                .and_then(|view| view.set_target(&image, origin.as_ref()));
            if let Some(request) = request {
                self.spawn_wall_decode(request);
            }
            self.tick_wallpaper(output);
        }
    }

    fn spawn_wall_decode(&self, request: crate::wallpaper::DecodeRequest) {
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let load = |path: &str| {
                icons::load_cover(std::path::Path::new(path), request.width, request.height)
                    .map(Arc::new)
            };
            let front = if request.need_front {
                load(&request.front)
            } else {
                None
            };
            let incoming = match request.kind {
                crate::wallpaper::DecodeKind::Front => None,
                crate::wallpaper::DecodeKind::Reveal => load(&request.incoming),
            };
            let _ = sender.send(ServiceEvent::WallDecoded(
                request.output,
                request.generation,
                request.kind,
                front,
                incoming,
            ));
        });
    }

    pub fn wallpaper_decoded(
        &mut self,
        output: platform::OutputId,
        generation: u64,
        kind: crate::wallpaper::DecodeKind,
        front: Option<Arc<Pixmap>>,
        incoming: Option<Arc<Pixmap>>,
    ) {
        let now = self.now();
        let live = self.walls.live;
        if let Some(view) = self.wall_views.get_mut(&output) {
            view.decoded(
                &mut self.platform,
                generation,
                kind,
                front,
                incoming,
                live,
                now,
            );
        }
        self.tick_wallpaper(output);
    }
}
