use std::{sync::Arc, time::Duration};

use calloop::timer::TimeoutFuture;
use services::media::{self, Command, Track};
use ui::tiny_skia::Pixmap;

use crate::{
    app::{App, ServiceEvent},
    icons,
};

pub const ART: f32 = 46.0;

#[derive(Default)]
pub struct Media {
    pub track: Option<Track>,
    pub art: Option<Arc<Pixmap>>,
    art_key: String,
}

impl Media {
    pub fn bar_entry(&self) -> Option<(String, bool)> {
        let track = self.track.as_ref()?;
        let text = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} · {}", track.title, track.artist)
        };
        Some((text, track.playing))
    }
}

impl App {
    pub fn start_media(&mut self) {
        let sender = self.events.clone();
        let handle = self.handle.clone();
        let task = async move {
            loop {
                let callback_sender = sender.clone();
                if let Err(err) = media::watch(move |track| {
                    let _ = callback_sender.send(ServiceEvent::Media(track));
                })
                .await
                {
                    log::debug!("media: {err}");
                }
                let _ = sender.send(ServiceEvent::Media(None));
                TimeoutFuture::from_duration(&handle, Duration::from_secs(5)).await;
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("media: {err}");
        }
    }

    pub fn media_changed(&mut self, track: Option<Track>) {
        let key = track
            .as_ref()
            .map(|track| track.art.clone())
            .unwrap_or_default();
        if key != self.media.art_key {
            self.media.art_key = key.clone();
            self.media.art = None;
            if let Some(path) = track.as_ref().and_then(Track::art_path) {
                let sender = self.events.clone();
                let size = (ART * crate::theme::scale()).round() as u32;
                let radius = 11.0 * crate::theme::scale();
                std::thread::spawn(move || {
                    let art = icons::load_cover(&path, size, size)
                        .and_then(|cover| icons::rounded(&cover, radius));
                    let _ = sender.send(ServiceEvent::MediaArt(key, art));
                });
            }
        }
        self.media.track = track;
        self.sync();
    }

    pub fn media_art_loaded(&mut self, key: String, art: Option<Pixmap>) {
        if key == self.media.art_key {
            self.media.art = art.map(Arc::new);
            self.sync();
        }
    }

    pub fn media_command(&mut self, command: Command) {
        let Some(player) = self.media.track.as_ref().map(|track| track.player.clone()) else {
            return;
        };
        let task = async move {
            if let Err(err) = media::send(player, command).await {
                log::warn!("media: {err}");
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("media: {err}");
        }
    }
}
