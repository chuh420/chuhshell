use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use serde::Deserialize;
use serde_json::Value;

use super::{Callback, Event, Keyboard, State, Window, Workspace, layout_label};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct RawWorkspace {
    id: u64,
    idx: u8,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    is_focused: bool,
    #[serde(default)]
    active_window_id: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct RawWindow {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    workspace_id: Option<u64>,
    #[serde(default)]
    is_focused: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct RawLayouts {
    names: Vec<String>,
    current_idx: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub state: bool,
    pub keyboard: bool,
    pub config: bool,
}

#[derive(Debug, Default)]
pub struct Niri {
    workspaces: Vec<RawWorkspace>,
    windows: HashMap<u64, RawWindow>,
    focused_window: Option<u64>,
    layouts: Vec<String>,
    layout: usize,
}

fn field<T: for<'de> Deserialize<'de>>(body: &Value, key: &str) -> Option<T> {
    serde_json::from_value(body.get(key)?.clone()).ok()
}

impl Niri {
    pub fn apply(&mut self, line: &str) -> Changes {
        let mut changes = Changes::default();
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(line) else {
            return changes;
        };
        let Some((name, body)) = map.into_iter().next() else {
            return changes;
        };
        match name.as_str() {
            "WorkspacesChanged" => {
                if let Some(workspaces) = field(&body, "workspaces") {
                    self.workspaces = workspaces;
                    changes.state = true;
                }
            }
            "WorkspaceActivated" => {
                let id: Option<u64> = field(&body, "id");
                let focused: bool = field(&body, "focused").unwrap_or(false);
                if let Some(id) = id {
                    let output = self
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.id == id)
                        .map(|workspace| workspace.output.clone());
                    if let Some(output) = output {
                        for workspace in &mut self.workspaces {
                            if workspace.output == output {
                                workspace.is_active = workspace.id == id;
                            }
                            if focused {
                                workspace.is_focused = workspace.id == id;
                            }
                        }
                        changes.state = true;
                    }
                }
            }
            "WorkspaceActiveWindowChanged" => {
                let id: Option<u64> = field(&body, "workspace_id");
                let window: Option<u64> = field(&body, "active_window_id");
                if let Some(workspace) = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| Some(workspace.id) == id)
                {
                    workspace.active_window_id = window;
                    changes.state = true;
                }
            }
            "WindowsChanged" => {
                if let Some(windows) = field::<Vec<RawWindow>>(&body, "windows") {
                    self.focused_window = windows
                        .iter()
                        .find(|window| window.is_focused)
                        .map(|window| window.id);
                    self.windows = windows
                        .into_iter()
                        .map(|window| (window.id, window))
                        .collect();
                    changes.state = true;
                }
            }
            "WindowOpenedOrChanged" => {
                if let Some(window) = field::<RawWindow>(&body, "window") {
                    if window.is_focused {
                        self.focused_window = Some(window.id);
                        for other in self.windows.values_mut() {
                            other.is_focused = false;
                        }
                    }
                    self.windows.insert(window.id, window);
                    changes.state = true;
                }
            }
            "WindowClosed" => {
                if let Some(id) = field::<u64>(&body, "id") {
                    self.windows.remove(&id);
                    if self.focused_window == Some(id) {
                        self.focused_window = None;
                    }
                    changes.state = true;
                }
            }
            "WindowFocusChanged" => {
                self.focused_window = field(&body, "id");
                for window in self.windows.values_mut() {
                    window.is_focused = Some(window.id) == self.focused_window;
                }
                changes.state = true;
            }
            "KeyboardLayoutsChanged" => {
                if let Some(layouts) = field::<RawLayouts>(&body, "keyboard_layouts") {
                    self.layouts = layouts.names;
                    self.layout = layouts.current_idx as usize;
                    changes.keyboard = true;
                }
            }
            "KeyboardLayoutSwitched" => {
                if let Some(idx) = field::<u8>(&body, "idx") {
                    self.layout = idx as usize;
                    changes.keyboard = true;
                }
            }
            "ConfigLoaded" => changes.config = true,
            _ => {}
        }
        changes
    }

    fn occupied(&self, workspace: &RawWorkspace) -> bool {
        workspace.active_window_id.is_some()
            || self
                .windows
                .values()
                .any(|window| window.workspace_id == Some(workspace.id))
    }

    pub fn state(&self) -> State {
        let focused = self
            .workspaces
            .iter()
            .find(|workspace| workspace.is_focused);
        let output = focused.and_then(|workspace| workspace.output.clone());
        let mut list: Vec<&RawWorkspace> = self
            .workspaces
            .iter()
            .filter(|workspace| output.is_none() || workspace.output == output)
            .collect();
        list.sort_by_key(|workspace| workspace.idx);
        let active_window = self
            .focused_window
            .and_then(|id| self.windows.get(&id))
            .map(|window| Window {
                address: window.id.to_string(),
                title: window.title.clone().unwrap_or_default(),
            });
        State {
            workspaces: list
                .into_iter()
                .map(|workspace| Workspace {
                    id: workspace.idx as i32,
                    name: workspace.name.clone().unwrap_or_default(),
                    output: workspace.output.clone().unwrap_or_default(),
                    active: workspace.is_active,
                    occupied: self.occupied(workspace),
                })
                .collect(),
            focused_output: output,
            active_workspace: focused.map(|workspace| workspace.idx as i32),
            active_window,
        }
    }

    pub fn keyboard(&self) -> Option<Keyboard> {
        let name = self.layouts.get(self.layout)?.clone();
        Some(Keyboard {
            device: String::new(),
            label: layout_label(&name),
            name,
        })
    }
}

fn socket() -> io::Result<PathBuf> {
    std::env::var_os("NIRI_SOCKET")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "NIRI_SOCKET is not set"))
}

fn connect(request: &Value) -> io::Result<BufReader<UnixStream>> {
    let mut stream = UnixStream::connect(socket()?)?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(format!("{request}\n").as_bytes())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.get_ref().set_read_timeout(Some(TIMEOUT))?;
    reader.read_line(&mut line)?;
    reply_ok(&line)?;
    Ok(reader)
}

pub fn reply_ok(line: &str) -> io::Result<()> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if let Some(err) = value.get("Err") {
        return Err(io::Error::other(
            err.as_str().unwrap_or("niri error").to_string(),
        ));
    }
    if value.get("Ok").is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected niri reply",
        ));
    }
    Ok(())
}

fn action(body: Value) -> io::Result<()> {
    connect(&serde_json::json!({ "Action": body })).map(|_| ())
}

pub fn focus_workspace(idx: i32) -> io::Result<()> {
    action(serde_json::json!({ "FocusWorkspace": { "reference": { "Index": idx } } }))
}

pub fn next_layout() -> io::Result<()> {
    action(serde_json::json!({ "SwitchLayout": { "layout": "Next" } }))
}

pub(super) fn listen(callback: &Callback) -> io::Result<()> {
    let mut reader = connect(&Value::String("EventStream".into()))?;
    reader.get_ref().set_read_timeout(None)?;
    let mut niri = Niri::default();
    let mut last: Option<State> = None;
    let mut last_keyboard: Option<Keyboard> = None;
    let mut line = String::new();
    let mut pending = Changes::default();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "event stream closed",
            ));
        }
        let changes = niri.apply(line.trim_end());
        pending.state |= changes.state;
        pending.keyboard |= changes.keyboard;
        if changes.config {
            callback(Event::ConfigReloaded);
        }
        if !reader.buffer().is_empty() {
            continue;
        }
        if pending.state {
            let state = niri.state();
            if last.as_ref() != Some(&state) {
                last = Some(state.clone());
                callback(Event::State(state));
            }
        }
        if pending.keyboard
            && let Some(keyboard) = niri.keyboard()
            && last_keyboard.as_ref() != Some(&keyboard)
        {
            last_keyboard = Some(keyboard.clone());
            callback(Event::Keyboard(keyboard));
        }
        pending = Changes::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACES: &str = r#"{"WorkspacesChanged":{"workspaces":[{"id":5,"idx":2,"name":null,"output":"eDP-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":null},{"id":4,"idx":1,"name":"web","output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":9},{"id":7,"idx":1,"name":null,"output":"HDMI-A-1","is_urgent":false,"is_active":true,"is_focused":false,"active_window_id":null}]}}"#;
    const WINDOWS: &str = r#"{"WindowsChanged":{"windows":[{"id":9,"title":"kitty","app_id":"kitty","pid":10,"workspace_id":4,"is_focused":true,"is_floating":false,"is_urgent":false}]}}"#;

    #[test]
    fn workspaces_and_windows_build_state() {
        let mut niri = Niri::default();
        assert!(niri.apply(WORKSPACES).state);
        assert!(niri.apply(WINDOWS).state);
        let state = niri.state();
        assert_eq!(state.focused_output.as_deref(), Some("eDP-1"));
        assert_eq!(state.active_workspace, Some(1));
        let ids: Vec<i32> = state.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(state.workspaces[0].name, "web");
        assert!(state.workspaces[0].occupied);
        assert!(!state.workspaces[1].occupied);
        assert_eq!(
            state.active_window,
            Some(Window {
                address: "9".into(),
                title: "kitty".into(),
            })
        );
        assert!(state.active_occupied());
    }

    #[test]
    fn activation_and_focus_events_update_state() {
        let mut niri = Niri::default();
        niri.apply(WORKSPACES);
        niri.apply(WINDOWS);
        niri.apply(r#"{"WorkspaceActivated":{"id":5,"focused":true}}"#);
        niri.apply(r#"{"WindowFocusChanged":{"id":null}}"#);
        let state = niri.state();
        assert_eq!(state.active_workspace, Some(2));
        assert_eq!(state.active_window, None);
        assert!(!state.active_occupied());
        niri.apply(r#"{"WindowOpenedOrChanged":{"window":{"id":11,"title":"zen","workspace_id":5,"is_focused":true}}}"#);
        let state = niri.state();
        assert_eq!(
            state.active_window.as_ref().map(|w| w.title.as_str()),
            Some("zen")
        );
        assert!(state.active_occupied());
        niri.apply(r#"{"WindowClosed":{"id":11}}"#);
        assert_eq!(niri.state().active_window, None);
    }

    #[test]
    fn keyboard_layouts() {
        let mut niri = Niri::default();
        assert_eq!(niri.keyboard(), None);
        let changes = niri.apply(r#"{"KeyboardLayoutsChanged":{"keyboard_layouts":{"names":["English (US)","Russian"],"current_idx":0}}}"#);
        assert!(changes.keyboard);
        assert_eq!(niri.keyboard().map(|k| k.label), Some("EN".into()));
        niri.apply(r#"{"KeyboardLayoutSwitched":{"idx":1}}"#);
        assert_eq!(niri.keyboard().map(|k| k.name), Some("Russian".into()));
    }

    #[test]
    fn unknown_and_broken_events_are_ignored() {
        let mut niri = Niri::default();
        assert_eq!(
            niri.apply(r#"{"OverviewOpenedOrClosed":{"is_open":true}}"#),
            Changes::default()
        );
        assert_eq!(niri.apply("not json"), Changes::default());
        assert!(niri.apply(r#"{"ConfigLoaded":{"failed":false}}"#).config);
    }

    #[test]
    fn replies() {
        assert!(reply_ok(r#"{"Ok":"Handled"}"#).is_ok());
        assert!(reply_ok(r#"{"Err":"no such workspace"}"#).is_err());
        assert!(reply_ok("garbage").is_err());
    }
}
