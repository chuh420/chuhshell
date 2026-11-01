use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use services::clipboard::{self, ClipEntry};
use ui::tiny_skia::Pixmap;

use crate::{
    app::{App, ServiceEvent},
    icons,
};

pub const THUMB_WIDTH: u32 = 224;
pub const THUMB_HEIGHT: u32 = 128;

#[derive(Default)]
pub struct Clip {
    pub entries: Option<Arc<Vec<ClipEntry>>>,
    pub thumbs: HashMap<String, Option<Arc<Pixmap>>>,
    pending: HashSet<String>,
    generation: u64,
}

fn background(
    name: &'static str,
    id: String,
    work: impl FnOnce(&str) -> std::io::Result<()> + Send + 'static,
) {
    std::thread::spawn(move || {
        if let Err(err) = work(&id) {
            log::warn!("clipboard {name} {id}: {err}");
        }
    });
}

impl App {
    pub fn clip_panel_changed(&mut self, panel: &str) {
        self.clip.generation += 1;
        self.clip.entries = None;
        self.clip.thumbs.clear();
        self.clip.pending.clear();
        if panel != "clipboard" {
            return;
        }
        let generation = self.clip.generation;
        let sender = self.events.clone();
        std::thread::spawn(move || {
            let entries = clipboard::list();
            let _ = sender.send(ServiceEvent::ClipList(generation, entries));
        });
    }

    pub fn clip_listed(&mut self, generation: u64, entries: Vec<ClipEntry>) {
        if generation != self.clip.generation {
            return;
        }
        self.clip.entries = Some(Arc::new(entries));
        self.sync();
        self.request_clip_thumbs();
    }

    pub fn clip_copy(&mut self, id: &str) {
        background("copy", id.to_string(), clipboard::copy);
    }

    pub fn clip_delete(&mut self, id: &str) {
        if let Some(entries) = &self.clip.entries {
            let kept: Vec<ClipEntry> = entries.iter().filter(|e| e.id != id).cloned().collect();
            self.clip.entries = Some(Arc::new(kept));
        }
        self.clip.thumbs.remove(id);
        background("delete", id.to_string(), clipboard::delete);
        self.sync();
    }

    pub fn clip_wipe(&mut self) {
        self.clip.entries = Some(Arc::new(Vec::new()));
        self.clip.thumbs.clear();
        background("wipe", String::new(), |_| clipboard::wipe());
        self.sync();
    }

    pub fn request_clip_thumbs(&mut self) {
        for (id, format) in self.overlay.clip_thumb_requests() {
            if self.clip.thumbs.contains_key(&id) || !self.clip.pending.insert(id.clone()) {
                continue;
            }
            let sender = self.events.clone();
            let generation = self.clip.generation;
            std::thread::spawn(move || {
                let pixmap = match clipboard::thumbnail(&id, &format) {
                    Ok(path) => icons::load_cover(&path, THUMB_WIDTH, THUMB_HEIGHT).map(Arc::new),
                    Err(err) => {
                        log::warn!("clipboard thumb {id}: {err}");
                        None
                    }
                };
                let _ = sender.send(ServiceEvent::ClipThumb(generation, id, pixmap));
            });
        }
    }

    pub fn clip_thumb_loaded(&mut self, generation: u64, id: String, pixmap: Option<Arc<Pixmap>>) {
        if generation != self.clip.generation || !self.clip.pending.remove(&id) {
            return;
        }
        self.clip.thumbs.insert(id, pixmap);
        self.overlay.set_clip_thumbs(&self.clip.thumbs);
        self.overlay.schedule(&mut self.platform);
    }
}
