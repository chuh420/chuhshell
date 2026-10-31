use std::{collections::HashMap, process::Command, sync::Arc};

use platform::{PointerEvent, SurfaceId};
use services::notifications::{self, CloseReason, Notification, Urgency};
use ui::anim::Millis;

use crate::{
    app::{App, ServiceEvent},
    card::{CardAction, age},
    history::now_ms,
    icons,
    popups::{self, Popups},
};

#[derive(Default)]
pub struct Notifications {
    pub handle: Option<notifications::Handle>,
    pub list: Vec<Notification>,
    pub seen: Vec<u32>,
    pub times: HashMap<u32, i64>,
    pub popups: Vec<u32>,
    pub icons: HashMap<u32, Option<Arc<ui::tiny_skia::Pixmap>>>,
    pub pending: Vec<u32>,
    pub view: Option<Popups>,
}

impl Notifications {
    pub fn history(&self) -> impl Iterator<Item = &Notification> {
        self.list.iter().filter(|n| !n.transient)
    }

    pub fn unread(&self) -> u32 {
        self.history()
            .filter(|n| !self.seen.contains(&n.id))
            .count() as u32
    }

    pub fn find(&self, id: u32) -> Option<&Notification> {
        self.list.iter().find(|n| n.id == id)
    }

    pub fn ages(&self) -> HashMap<u32, String> {
        let now = now_ms();
        let minute = now - now.rem_euclid(60_000);
        self.list
            .iter()
            .map(|n| (n.id, age(self.times.get(&n.id).copied(), minute)))
            .collect()
    }
}

impl App {
    pub fn start_notifications(&mut self) {
        let sender = self.events.clone();
        let task = async move {
            let callback_sender = sender.clone();
            match notifications::serve(move |event| {
                let _ = callback_sender.send(ServiceEvent::Notification(event));
            })
            .await
            {
                Ok(handle) => {
                    let _ = sender.send(ServiceEvent::NotificationServer(handle));
                }
                Err(err) => log::error!("notification server: {err}"),
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("notification server: {err}");
        }
    }

    pub fn notification_event(&mut self, event: notifications::Event) {
        match event {
            notifications::Event::Added(notification) => {
                let id = notification.id;
                self.notes.times.insert(id, now_ms());
                if !notification.tag.is_empty() {
                    let others: Vec<u32> = self
                        .notes
                        .list
                        .iter()
                        .filter(|other| other.id != id && other.tag == notification.tag)
                        .map(|other| other.id)
                        .collect();
                    for other in others {
                        self.notes.popups.retain(|popup| *popup != other);
                        self.close_notification(other, CloseReason::Expired);
                    }
                }
                let transient = notification.transient;
                self.notes.list.push(notification.clone());
                if !transient {
                    self.notification_arrived();
                }
                self.show_popup(&notification);
            }
            notifications::Event::Updated {
                notification,
                summary_changed,
                body_changed,
            } => {
                let id = notification.id;
                if let Some(slot) = self.notes.list.iter_mut().find(|n| n.id == id) {
                    *slot = notification.clone();
                }
                self.notes.icons.remove(&id);
                for changed in [summary_changed, body_changed] {
                    if changed {
                        self.notes.times.insert(id, now_ms());
                        self.show_popup(&notification);
                    }
                }
            }
            notifications::Event::Closed(id) => self.forget(id),
        }
        self.sync();
    }

    fn notification_arrived(&mut self) {
        if self.state.dnd {
            return;
        }
        let now = self.now();
        self.ring_bells(now);
    }

    fn forget(&mut self, id: u32) {
        self.notes.list.retain(|n| n.id != id);
        self.notes.popups.retain(|popup| *popup != id);
        self.notes.times.remove(&id);
        self.notes.icons.remove(&id);
    }

    fn close_notification(&mut self, id: u32, reason: CloseReason) {
        if self.notes.find(id).is_none() {
            return;
        }
        self.forget(id);
        if let Some(handle) = self.notes.handle.clone() {
            let task = async move { handle.close(id, reason).await };
            if let Err(err) = self.scheduler.schedule(task) {
                log::warn!("notification close: {err}");
            }
        }
    }

    fn invoke_action(&mut self, id: u32, action: String) {
        let resident = self.notes.find(id).is_some_and(|n| n.resident);
        if let Some(handle) = self.notes.handle.clone() {
            let task = async move { handle.invoke(id, &action).await };
            if let Err(err) = self.scheduler.schedule(task) {
                log::warn!("notification action: {err}");
            }
        }
        if !resident {
            self.close_notification(id, CloseReason::Dismissed);
        }
    }

    fn show_popup(&mut self, notification: &Notification) {
        let blocked = self.state.panel == "notifications"
            || (self.state.dnd && notification.urgency != Urgency::Critical);
        if blocked {
            if notification.transient {
                self.close_notification(notification.id, CloseReason::Expired);
            }
            return;
        }
        if self.notes.popups.contains(&notification.id) {
            let now = self.now();
            if let Some(view) = self.notes.view.as_mut() {
                view.refresh(notification.id, now);
            }
            return;
        }
        self.notes.popups.insert(0, notification.id);
        while self.notes.popups.len() > 4 {
            if let Some(last) = self.notes.popups.last().copied() {
                self.drop_popup(last);
            }
        }
    }

    pub fn drop_popup(&mut self, id: u32) {
        self.notes.popups.retain(|popup| *popup != id);
        if self.notes.find(id).is_some_and(|n| n.transient) {
            self.close_notification(id, CloseReason::Expired);
        }
    }

    pub fn panel_opened(&mut self, panel: &str) {
        if panel != "notifications" {
            return;
        }
        self.notes.seen = self.notes.history().map(|n| n.id).collect();
        let popups: Vec<u32> = self.notes.popups.iter().rev().copied().collect();
        for id in popups {
            self.drop_popup(id);
        }
    }

    pub fn clear_notifications(&mut self) {
        let ids: Vec<u32> = self.notes.history().map(|n| n.id).collect();
        for id in ids {
            self.close_notification(id, CloseReason::Dismissed);
        }
    }

    pub fn card_action(&mut self, action: CardAction, popup: bool) {
        match action {
            CardAction::Invoke(id, key) => self.invoke_action(id, key),
            CardAction::Dismiss(id) => self.close_notification(id, CloseReason::Dismissed),
            CardAction::Finished(id) => {
                if popup {
                    self.drop_popup(id);
                }
            }
            CardAction::OpenLink(url) => match Command::new("xdg-open").arg(&url).spawn() {
                Ok(child) => crate::app::reap(child),
                Err(err) => log::warn!("open {url}: {err}"),
            },
        }
        self.sync();
    }

    pub fn sync_popups(&mut self) {
        let now = self.now();
        let ages = self.notes.ages();
        if self.notes.popups.is_empty() {
            if let Some(view) = self.notes.view.as_mut()
                && let Some(surface) = view.detach()
            {
                self.platform.destroy_surface(surface);
            }
            return;
        }
        let output = self.focused_output();
        let screen_height = output
            .and_then(|output| self.platform.output_desc(output))
            .map_or(1080.0, |desc| desc.height as f32);
        let view = self.notes.view.get_or_insert_with(Popups::new);
        if view.surface.is_some()
            && view.output != output
            && let Some(surface) = view.detach()
        {
            self.platform.destroy_surface(surface);
        }
        view.sync(
            &mut self.engine,
            &self.notes.popups,
            &self.notes.list,
            &ages,
            &self.notes.icons,
            now,
        );
        let height = view.wanted_height(screen_height);
        match view.surface {
            None => {
                if let Some(output) = output {
                    let surface = self
                        .platform
                        .create_layer_surface(&popups::layer_config(output, height));
                    view.attach(surface, output);
                    view.set_requested(height);
                }
            }
            Some(surface) => {
                if view.requested() != height {
                    self.platform
                        .set_layer_size(surface, popups::window_width(), height);
                    view.set_requested(height);
                }
                self.platform.schedule_frame(surface);
            }
        }
        self.request_icons();
    }

    pub fn request_icons(&mut self) {
        let mut wanted: Vec<(u32, u32)> = self
            .notes
            .view
            .as_ref()
            .map(Popups::icon_requests)
            .unwrap_or_default();
        wanted.extend(self.overlay.icon_requests());
        for (id, size) in wanted {
            if self.notes.icons.contains_key(&id) || self.notes.pending.contains(&id) {
                continue;
            }
            let Some(notification) = self.notes.find(id).cloned() else {
                continue;
            };
            self.notes.pending.push(id);
            let sender = self.events.clone();
            std::thread::spawn(move || {
                let pixmap =
                    icons::source(&notification).and_then(|source| icons::load(&source, size));
                let _ = sender.send(ServiceEvent::Icon(id, pixmap));
            });
        }
    }

    pub fn icon_loaded(&mut self, id: u32, pixmap: Option<ui::tiny_skia::Pixmap>) {
        self.notes.pending.retain(|pending| *pending != id);
        if self.notes.find(id).is_none() {
            return;
        }
        self.notes.icons.insert(id, pixmap.map(Arc::new));
        self.overlay.set_icons(&self.notes.icons);
        self.overlay.schedule(&mut self.platform);
        self.sync_popups();
    }

    pub fn popups_owns(&self, surface: SurfaceId) -> bool {
        self.notes
            .view
            .as_ref()
            .is_some_and(|view| view.owns(surface))
    }

    pub fn popups_configure(&mut self, width: u32, height: u32) {
        let now = self.now();
        if let Some(view) = self.notes.view.as_mut() {
            view.configure(width, height);
        }
        self.popups_draw(now);
    }

    pub fn popups_draw(&mut self, now: Millis) {
        let Some(view) = self.notes.view.as_mut() else {
            return;
        };
        let actions = view.draw(&mut self.platform, &mut self.engine, now);
        for action in actions {
            self.card_action(action, true);
        }
    }

    pub fn popups_pointer(&mut self, event: PointerEvent) {
        let now = self.now();
        let Some(view) = self.notes.view.as_mut() else {
            return;
        };
        let actions = view.pointer(event.kind, event.x, event.y, &mut self.engine, now);
        let cursor = view.cursor();
        view.schedule(&mut self.platform);
        self.platform.set_cursor(cursor);
        for action in actions {
            self.card_action(action, true);
        }
    }

    pub fn popups_closed(&mut self, surface: SurfaceId) {
        if let Some(view) = self.notes.view.as_mut()
            && view.owns(surface)
        {
            view.detach();
        }
    }

    pub fn notifications_json(&self) -> serde_json::Value {
        self.notes
            .list
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "app": n.app_name,
                    "summary": n.summary,
                    "transient": n.transient,
                    "timeout": n.expire_timeout,
                    "urgency": n.urgency as u8,
                })
            })
            .collect()
    }
}
