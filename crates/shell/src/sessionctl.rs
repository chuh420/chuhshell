use std::{process::Child, time::Duration};

use services::{
    live::{self, Player},
    session::{self, Power},
    wallpapers,
};

use crate::app::{App, reap};

#[derive(Default)]
pub struct Session {
    player: Option<Player>,
    lock: Option<Child>,
    live_timer: bool,
}

impl Session {
    fn lock_running(&mut self) -> bool {
        let running = self
            .lock
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !running {
            self.lock = None;
        }
        running
    }
}

impl App {
    fn spawn_line(&self, what: &str, line: &str) -> Option<Child> {
        match session::shell_command(line).spawn() {
            Ok(child) => Some(child),
            Err(err) => {
                log::warn!("{what}: {err}");
                None
            }
        }
    }

    fn close_panel_then(&mut self, action: impl FnOnce(&mut App) + 'static) {
        if self.state.panel.is_empty() {
            action(self);
            return;
        }
        self.state.panel.clear();
        self.sync();
        let mut action = Some(action);
        self.add_timer(Duration::from_millis(250), move |app| {
            if let Some(action) = action.take() {
                action(app);
            }
            None
        });
    }

    pub fn lock_screen(&mut self) {
        self.close_panel_then(|app| {
            if app.session.lock_running() || live::screen_locked() {
                return;
            }
            let line = session::lock_command();
            app.session.lock = app.spawn_line("lock", &line);
            app.update_live_pause();
        });
    }

    pub fn take_screenshot(&mut self) {
        let Some(line) = session::screenshot_command() else {
            services::programs::warn_missing("grim, slurp or wl-copy", "screenshots");
            return;
        };
        self.close_panel_then(move |app| {
            if let Some(child) = app.spawn_line("screenshot", &line) {
                reap(child);
            }
        });
    }

    pub fn open_mixer(&mut self) {
        let Some(line) = session::mixer_command() else {
            services::programs::warn_missing("pavucontrol", "the audio mixer");
            return;
        };
        if let Some(child) = self.spawn_line("mixer", &line) {
            reap(child);
        }
    }

    pub fn run_command(&mut self, name: &str) {
        match name {
            "lock" => self.lock_screen(),
            "screenshot" => self.take_screenshot(),
            "audio" | "mixer" => self.open_mixer(),
            "suspend" => self.power_action(Power::Suspend),
            other => log::warn!("unknown command: {other}"),
        }
    }

    pub fn power_action(&mut self, action: Power) {
        if action == Power::Suspend {
            self.state.panel.clear();
            self.sync();
        }
        let task = async move {
            if let Err(err) = session::power(action).await {
                log::warn!("logind {}: {err}", action.method());
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("logind: {err}");
        }
    }

    pub fn run_wallpaper_hook(&self, state: &wallpapers::State) {
        let Some(hook) = config::current().hooks.wallpaper_changed.clone() else {
            return;
        };
        let mut command = session::hook_command(&hook, &[&state.image, &state.path]);
        std::thread::spawn(move || match command.output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => log::warn!(
                "wallpaper_changed hook: {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(err) => log::warn!("wallpaper_changed hook: {err}"),
        });
    }

    pub fn sync_live(&mut self) {
        let wanted = self
            .walls
            .live
            .then(|| self.walls.data.current.clone())
            .filter(|path| !path.is_empty());
        let keep = match (&mut self.session.player, &wanted) {
            (Some(player), Some(video)) => player.video() == video && player.running(),
            _ => false,
        };
        if keep {
            return;
        }
        if let Some(player) = self.session.player.take() {
            player.stop();
        }
        if let Some(video) = wanted {
            self.session.player = Player::start(&video);
            self.update_live_pause();
            self.start_live_timer();
        }
    }

    fn start_live_timer(&mut self) {
        if self.session.live_timer || self.session.player.is_none() {
            return;
        }
        self.session.live_timer = true;
        self.add_timer(Duration::from_secs(5), |app| {
            if app.session.player.is_none() {
                app.session.live_timer = false;
                return None;
            }
            app.update_live_pause();
            Some(Duration::from_secs(5))
        });
    }

    pub fn update_live_pause(&mut self) {
        if self.session.player.is_none() {
            return;
        }
        let locked = self.session.lock_running() || live::screen_locked();
        let paused = self.state.workspace_occupied || locked;
        if let Some(player) = self.session.player.as_mut() {
            player.set_paused(paused);
        }
    }

    pub fn stop_live(&mut self) {
        if let Some(player) = self.session.player.take() {
            player.stop();
        }
    }
}
