pub mod hyprland;
pub mod niri;
pub mod sway;

use std::{io, sync::OnceLock, thread, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hyprland,
    Niri,
    Sway,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Hyprland => "hyprland",
            Kind::Niri => "niri",
            Kind::Sway => "sway",
        }
    }

    pub fn fixed_workspaces(self) -> bool {
        self == Kind::Hyprland
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
    pub output: String,
    pub active: bool,
    pub occupied: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub address: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keyboard {
    pub device: String,
    pub name: String,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub workspaces: Vec<Workspace>,
    pub focused_output: Option<String>,
    pub active_workspace: Option<i32>,
    pub active_window: Option<Window>,
}

impl State {
    pub fn active_occupied(&self) -> bool {
        self.workspaces
            .iter()
            .any(|workspace| Some(workspace.id) == self.active_workspace && workspace.occupied)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    State(State),
    Keyboard(Keyboard),
    ConfigReloaded,
}

pub type Callback = dyn Fn(Event) + Send;

const RETRY: Duration = Duration::from_secs(2);

pub fn detect_from(env: impl Fn(&str) -> bool) -> Option<Kind> {
    if env("HYPRLAND_INSTANCE_SIGNATURE") {
        Some(Kind::Hyprland)
    } else if env("NIRI_SOCKET") {
        Some(Kind::Niri)
    } else if env("SWAYSOCK") {
        Some(Kind::Sway)
    } else {
        None
    }
}

pub fn kind() -> Option<Kind> {
    static KIND: OnceLock<Option<Kind>> = OnceLock::new();
    *KIND.get_or_init(|| {
        detect_from(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
    })
}

pub fn is_hyprland() -> bool {
    kind() == Some(Kind::Hyprland)
}

pub fn layout_label(name: &str) -> String {
    config::current().keyboard.label(name)
}

fn listen(kind: Kind, callback: &Callback) -> io::Result<()> {
    match kind {
        Kind::Hyprland => hyprland::listen(callback),
        Kind::Niri => niri::listen(callback),
        Kind::Sway => sway::listen(callback),
    }
}

pub fn spawn(
    callback: impl Fn(Event) + Send + 'static,
) -> io::Result<Option<thread::JoinHandle<()>>> {
    let Some(kind) = kind() else {
        log::info!("no supported compositor found, the bar runs without workspaces and titles");
        return Ok(None);
    };
    log::info!("compositor: {}", kind.name());
    thread::Builder::new()
        .name("compositor".into())
        .spawn(move || {
            loop {
                if let Err(err) = listen(kind, &callback) {
                    log::warn!("{}: {err}", kind.name());
                }
                thread::sleep(RETRY);
            }
        })
        .map(Some)
}

pub fn focus_workspace(id: i32) -> io::Result<()> {
    match kind() {
        Some(Kind::Hyprland) => hyprland::focus_workspace(id),
        Some(Kind::Niri) => niri::focus_workspace(id),
        Some(Kind::Sway) => sway::focus_workspace(id),
        None => Ok(()),
    }
}

pub fn next_layout() -> io::Result<()> {
    match kind() {
        Some(Kind::Hyprland) => hyprland::next_layout(),
        Some(Kind::Niri) => niri::next_layout(),
        Some(Kind::Sway) => sway::next_layout(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_order() {
        let only = |name: &'static str| move |key: &str| key == name;
        assert_eq!(
            detect_from(only("HYPRLAND_INSTANCE_SIGNATURE")),
            Some(Kind::Hyprland)
        );
        assert_eq!(detect_from(only("NIRI_SOCKET")), Some(Kind::Niri));
        assert_eq!(detect_from(only("SWAYSOCK")), Some(Kind::Sway));
        assert_eq!(detect_from(|_| false), None);
        assert_eq!(
            detect_from(|key| key == "SWAYSOCK" || key == "NIRI_SOCKET"),
            Some(Kind::Niri)
        );
    }

    #[test]
    fn active_occupied_follows_active_workspace() {
        let state = State {
            workspaces: vec![
                Workspace {
                    id: 1,
                    occupied: true,
                    ..Workspace::default()
                },
                Workspace {
                    id: 2,
                    ..Workspace::default()
                },
            ],
            active_workspace: Some(2),
            ..State::default()
        };
        assert!(!state.active_occupied());
        assert!(
            State {
                active_workspace: Some(1),
                ..state
            }
            .active_occupied()
        );
    }
}
