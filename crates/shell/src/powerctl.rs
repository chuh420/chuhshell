use std::time::Duration;

use calloop::timer::TimeoutFuture;
use services::{audio, system, upower};

use crate::app::{App, ServiceEvent, reap};

impl App {
    pub fn start_power_panels(&mut self) {
        let sender = self.events.clone();
        let handle = self.handle.clone();
        let task = async move {
            loop {
                let callback_sender = sender.clone();
                if let Err(err) = upower::watch_profiles(move |profiles| {
                    let _ = callback_sender.send(ServiceEvent::Profiles(profiles));
                })
                .await
                {
                    log::warn!("power profiles: {err}");
                }
                TimeoutFuture::from_duration(&handle, Duration::from_secs(2)).await;
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::error!("power profiles: {err}");
        }
        self.add_timer(Duration::ZERO, |app| {
            app.read_battery_stats();
            None
        });
        self.add_timer(Duration::from_secs(30), |app| {
            app.read_battery_stats();
            Some(Duration::from_secs(if app.state.panel == "battery" {
                5
            } else {
                30
            }))
        });
    }

    pub fn read_battery_stats(&mut self) {
        let stats = system::battery_stats();
        if stats != self.battery_stats {
            self.battery_stats = stats;
            self.sync();
        }
    }

    pub fn profiles_changed(&mut self, profiles: upower::Profiles) {
        let performance_changed = profiles.has_performance != self.profiles.has_performance;
        self.profiles = profiles;
        if performance_changed {
            self.apply_profile();
        }
        self.sync();
    }

    pub fn power_panel_changed(&mut self, panel: &str) {
        if panel == "battery" {
            self.read_battery_stats();
        }
    }

    pub fn set_profile(&self, profile: &'static str) {
        let task = async move {
            if let Err(err) = upower::set_profile(profile.to_string()).await {
                log::warn!("power profile: {err}");
            }
        };
        if let Err(err) = self.scheduler.schedule(task) {
            log::warn!("power profile: {err}");
        }
    }

    pub fn set_mic_volume(&self, value: f64) {
        let source = self
            .state
            .audio
            .source()
            .filter(|source| source.has_audio)
            .map(|source| source.id);
        if let (Some(node), Some(handle)) = (source, &self.audio) {
            handle.send(audio::Command::SetVolume {
                node,
                volume: (value / 100.0) as f32,
            });
        }
    }

    pub fn set_default_node(&self, id: u32) {
        match std::process::Command::new("wpctl")
            .args(["set-default", &id.to_string()])
            .spawn()
        {
            Ok(child) => reap(child),
            Err(err) => log::warn!("wpctl: {err}"),
        }
    }

    pub fn session_action(&mut self, action: &'static str) {
        if action == "lock" {
            self.lock_screen();
        } else if let Some(power) = services::session::Power::parse(action) {
            self.power_action(power);
        }
    }
}
