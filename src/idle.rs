use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

mod wayland;
mod worker;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Timer {
    pub enabled: bool,
    pub minutes: u32,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            enabled: true,
            minutes: 15,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub screensaver: Timer,
}

pub(super) enum Command {
    Settings(Settings),
    Show,
    Stop,
    #[cfg(test)]
    ReleaseScreensaver,
    Check(mpsc::Sender<bool>),
}

pub struct Service {
    settings: RefCell<Settings>,
    saving: std::cell::Cell<bool>,
    error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    sender: mpsc::SyncSender<Command>,
    worker: RefCell<Option<std::thread::JoinHandle<()>>>,
}

pub fn worker_main() -> glib::ExitCode {
    match worker::run() {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chuhshell: idle worker: {error}");
            glib::ExitCode::FAILURE
        }
    }
}

pub fn start(state: &Rc<crate::app::AppState>) {
    if state.idle.borrow().is_some() {
        return;
    }
    let settings = crate::config::get().idle;
    let (sender, receiver) = mpsc::sync_channel(16);
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let worker_error = error.clone();
    let worker = std::thread::spawn(move || {
        if let Err(error) = worker::supervise(settings, receiver) {
            eprintln!("chuhshell: idle service: {error}");
            *worker_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
        }
    });
    *state.idle.borrow_mut() = Some(Rc::new(Service {
        settings: RefCell::new(settings),
        saving: std::cell::Cell::new(false),
        error,
        sender,
        worker: RefCell::new(Some(worker)),
    }));
}

pub fn shutdown(state: &crate::app::AppState) {
    if let Some(service) = state.idle.borrow_mut().take() {
        let _ = service.sender.send(Command::Stop);
        if let Some(worker) = service.worker.borrow_mut().take()
            && worker.join().is_err()
        {
            eprintln!("chuhshell: idle worker interrupted during shutdown");
        }
    }
}

pub fn show(state: &Rc<crate::app::AppState>) -> Result<(), String> {
    start(state);
    let service = state.idle.borrow().as_ref().unwrap().clone();
    if let Some(error) = service
        .error
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        return Err(error.clone());
    }
    service
        .sender
        .try_send(Command::Show)
        .map_err(|e| e.to_string())
}

pub fn view(state: &Rc<crate::app::AppState>) -> gtk::Box {
    start(state);
    let service = state.idle.borrow().as_ref().unwrap().clone();
    let settings = *service.settings.borrow();
    let timer = settings.screensaver;
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("idle-settings");
    let enabled = gtk::ToggleButton::with_label(if timer.enabled { "On" } else { "Off" });
    enabled.add_css_class("network-action");
    enabled.add_css_class("idle-toggle");
    enabled.set_valign(gtk::Align::Center);
    enabled.set_active(timer.enabled);
    if timer.enabled {
        enabled.add_css_class("enabled");
    }
    enabled.connect_toggled(|button| {
        button.set_label(if button.is_active() { "On" } else { "Off" });
        if button.is_active() {
            button.add_css_class("enabled");
        } else {
            button.remove_css_class("enabled");
        }
    });
    let automatic = setting_row(
        "Automatic trigger",
        "Show the screensaver when inactive",
        &enabled,
    );
    let minutes = gtk::SpinButton::with_range(1.0, 1440.0, 1.0);
    minutes.set_numeric(true);
    minutes.set_width_chars(4);
    minutes.set_value(timer.minutes.clamp(1, 1440).into());
    minutes.set_valign(gtk::Align::Center);
    minutes.set_tooltip_text(Some("Minutes of inactivity"));
    let timing = setting_row("Trigger after", "Minutes of inactivity", &minutes);
    let save = gtk::Button::with_label("Apply");
    save.add_css_class("network-action");
    save.add_css_class("primary");
    save.set_halign(gtk::Align::End);
    let message = gtk::Label::new(None);
    message.add_css_class("menu-hint");
    message.set_xalign(0.0);
    message.set_wrap(true);
    message.set_visible(false);
    message.connect_notify_local(Some("label"), |label, _| {
        label.set_visible(!label.text().is_empty())
    });
    if let Some(error) = service
        .error
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        message.set_text(error);
    }
    outer.append(&automatic);
    outer.append(&timing);
    outer.append(&save);
    outer.append(&message);
    save.connect_clicked(move |button| {
        if service.saving.replace(true) {
            message.set_text("Settings update in progress");
            return;
        }
        let mut settings = *service.settings.borrow();
        let timer = Timer {
            enabled: enabled.is_active(),
            minutes: minutes.value_as_int() as u32,
        };
        settings.screensaver = timer;
        button.set_sensitive(false);
        let button = button.downgrade();
        let service = service.clone();
        let message = message.clone();
        glib::MainContext::default().spawn_local(async move {
            let result =
                crate::config::save_value_async("idle", serde_json::to_value(settings).unwrap())
                    .await;
            match result {
                Ok(()) => {
                    *service.settings.borrow_mut() = settings;
                    match service.sender.try_send(Command::Settings(settings)) {
                        Ok(()) => message.set_text("Saved"),
                        Err(e) => message.set_text(&format!("Saved, but idle service failed: {e}")),
                    }
                }
                Err(e) => message.set_text(&e),
            }
            service.saving.set(false);
            if let Some(button) = button.upgrade() {
                button.set_sensitive(true);
            }
        });
    });
    outer
}

fn setting_row(title: &str, hint: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("idle-setting-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    for (value, class) in [(title, "menu-title"), (hint, "menu-hint")] {
        let label = gtk::Label::new(Some(value));
        label.add_css_class(class);
        label.set_xalign(0.0);
        text.append(&label);
    }
    row.append(&text);
    row.append(control);
    row
}

pub fn timer(state: &Rc<crate::app::AppState>) -> Timer {
    let settings = state
        .idle
        .borrow()
        .as_ref()
        .map(|service| *service.settings.borrow())
        .unwrap_or(crate::config::get().idle);
    settings.screensaver
}

#[cfg(test)]
pub fn regression_checks() {
    wayland::regression_checks();
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "internal idle worker subprocess"]
    fn worker_process() {
        std::process::exit(super::worker_main().into());
    }

    #[test]
    #[ignore = "headless crash controller subprocess"]
    fn crash_controller() {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) }, 0);
        let mut child = super::worker::spawn().unwrap();
        super::worker::show_and_check(&mut child);
        let path = std::env::var_os("CHUHSHELL_IDLE_TEST_READY").unwrap();
        std::fs::write(path, child.id().to_string()).unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    fn default_timers_and_partial_settings() {
        let settings: super::Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.screensaver.enabled);
        assert_eq!(settings.screensaver.minutes, 15);
        assert!(serde_json::from_str::<super::Settings>(r#"{"unknown":true}"#).is_err());
    }
}
