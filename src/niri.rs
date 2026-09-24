use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(600);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const SESSION_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: u64,
    pub idx: u8,
    pub name: Option<String>,
    pub output: Option<String>,
    pub is_urgent: bool,
    pub is_active: bool,
    pub is_focused: bool,
    pub active_window_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct KeyboardLayouts {
    pub names: Vec<String>,
    pub current_idx: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub layouts: KeyboardLayouts,
}

fn request(request: &str) -> Option<serde_json::Value> {
    let socket_path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    response(&line)
}

fn response(response: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(response)
        .ok()?
        .get("Ok")
        .cloned()
}

pub fn command(request: &str) -> bool {
    self::request(request).is_some()
}

fn activate_workspace(workspaces: &mut [Workspace], id: u64, focused: bool) -> bool {
    let Some(target) = workspaces.iter().find(|workspace| workspace.id == id) else {
        return false;
    };
    let output = target.output.clone();
    let mut changed = false;
    for workspace in workspaces.iter_mut() {
        if workspace.id == id {
            if !workspace.is_active {
                workspace.is_active = true;
                changed = true;
            }
            if workspace.is_focused != focused {
                workspace.is_focused = focused;
                changed = true;
            }
        } else {
            if output.is_some() && workspace.output == output && workspace.is_active {
                workspace.is_active = false;
                changed = true;
            }
            if focused && workspace.is_focused {
                workspace.is_focused = false;
                changed = true;
            }
        }
    }
    changed
}

pub fn apply_event(event: &serde_json::Value, snapshot: &mut Snapshot) -> bool {
    if let Some(value) = event
        .get("WorkspacesChanged")
        .and_then(|value| value.get("workspaces"))
        .cloned()
        && let Ok(next) = serde_json::from_value::<Vec<Workspace>>(value)
    {
        snapshot.workspaces = next;
        return true;
    }
    if let Some(value) = event
        .get("KeyboardLayoutsChanged")
        .and_then(|value| value.get("keyboard_layouts"))
        .cloned()
        && let Ok(next) = serde_json::from_value::<KeyboardLayouts>(value)
    {
        snapshot.layouts = next;
        return true;
    }
    if let Some(idx) = event
        .get("KeyboardLayoutSwitched")
        .and_then(|value| value.get("idx"))
        .and_then(serde_json::Value::as_u64)
    {
        snapshot.layouts.current_idx = idx as usize;
        return true;
    }
    if let Some(value) = event.get("WorkspaceActivated") {
        let id = value.get("id").and_then(serde_json::Value::as_u64);
        let focused = value
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if let Some(id) = id {
            return activate_workspace(&mut snapshot.workspaces, id, focused);
        }
        return false;
    }
    if let Some(value) = event.get("WorkspaceActiveWindowChanged") {
        let workspace_id = value
            .get("workspace_id")
            .and_then(serde_json::Value::as_u64);
        if let Some(workspace_id) = workspace_id {
            let active_window_id = value
                .get("active_window_id")
                .and_then(serde_json::Value::as_u64);
            if let Some(workspace) = snapshot
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id)
                && workspace.active_window_id != active_window_id
            {
                workspace.active_window_id = active_window_id;
                return true;
            }
        }
        return false;
    }
    if let Some(value) = event.get("WorkspaceUrgencyChanged") {
        let id = value.get("id").and_then(serde_json::Value::as_u64);
        let urgent = value
            .get("urgent")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if let Some(id) = id
            && let Some(workspace) = snapshot
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == id)
            && workspace.is_urgent != urgent
        {
            workspace.is_urgent = urgent;
            return true;
        }
    }
    false
}

fn read_ok_line(reader: &mut impl BufRead, line: &mut String) -> bool {
    line.clear();
    reader.read_line(line).is_ok() && response(line).is_some()
}

pub fn spawn_poller(sender: mpsc::Sender<Snapshot>) {
    thread::spawn(move || {
        loop {
            let Some(socket_path) = std::env::var_os("NIRI_SOCKET") else {
                thread::sleep(RECONNECT_DELAY);
                continue;
            };
            let Ok(mut stream) = UnixStream::connect(socket_path) else {
                thread::sleep(RECONNECT_DELAY);
                continue;
            };
            if stream.write_all(b"\"EventStream\"\n").is_err() {
                thread::sleep(RECONNECT_DELAY);
                continue;
            }
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if !read_ok_line(&mut reader, &mut line) {
                thread::sleep(RECONNECT_DELAY);
                continue;
            }
            let mut snapshot = Snapshot::default();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                            continue;
                        };
                        if apply_event(&event, &mut snapshot)
                            && sender.send(snapshot.clone()).is_err()
                        {
                            return;
                        }
                    }
                }
            }
            thread::sleep(SESSION_DELAY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: u64, idx: u8, output: &str) -> Workspace {
        Workspace {
            id,
            idx,
            name: None,
            output: Some(output.to_owned()),
            is_urgent: false,
            is_active: false,
            is_focused: false,
            active_window_id: None,
        }
    }

    #[test]
    fn response_unwraps_keyboard_layout_reply() {
        let raw =
            r#"{"Ok":{"KeyboardLayouts":{"names":["English (US)","Russian"],"current_idx":1}}}"#;
        let value = response(raw).expect("successful niri reply");
        let layouts: KeyboardLayouts = serde_json::from_value(
            value
                .get("KeyboardLayouts")
                .expect("keyboard-layout payload")
                .clone(),
        )
        .expect("valid keyboard-layout payload");

        assert_eq!(layouts.names, ["English (US)", "Russian"]);
        assert_eq!(layouts.current_idx, 1);
    }

    #[test]
    fn response_ignores_error_reply() {
        assert!(response(r#"{"Err":"unknown request"}"#).is_none());
        assert!(response("").is_none());
    }

    #[test]
    fn workspaces_changed_replaces_the_list() {
        let mut snapshot = Snapshot {
            workspaces: vec![workspace(9, 9, "DP-1")],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({
            "WorkspacesChanged": {
                "workspaces": [{
                    "id": 1,
                    "idx": 1,
                    "name": null,
                    "output": "eDP-1",
                    "is_urgent": false,
                    "is_active": true,
                    "is_focused": true,
                    "active_window_id": 42
                }]
            }
        });

        assert!(apply_event(&event, &mut snapshot));
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].id, 1);
        assert_eq!(snapshot.workspaces[0].active_window_id, Some(42));
    }

    #[test]
    fn layout_switch_event_updates_current_index() {
        let mut snapshot = Snapshot {
            workspaces: Vec::new(),
            layouts: KeyboardLayouts {
                names: vec!["English (US)".to_owned(), "Russian".to_owned()],
                current_idx: 0,
            },
        };
        let event = serde_json::json!({ "KeyboardLayoutSwitched": { "idx": 1 } });

        assert!(apply_event(&event, &mut snapshot));
        assert_eq!(snapshot.layouts.current_idx, 1);
    }

    #[test]
    fn workspace_activated_updates_activity_on_its_output_only() {
        let mut snapshot = Snapshot {
            workspaces: vec![
                {
                    let mut ws = workspace(1, 1, "eDP-1");
                    ws.is_active = true;
                    ws.is_focused = true;
                    ws
                },
                workspace(2, 2, "eDP-1"),
                {
                    let mut ws = workspace(3, 1, "DP-1");
                    ws.is_active = true;
                    ws
                },
            ],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({ "WorkspaceActivated": { "id": 2, "focused": false } });

        assert!(apply_event(&event, &mut snapshot));
        assert!(!snapshot.workspaces[0].is_active);
        assert!(snapshot.workspaces[0].is_focused);
        assert!(snapshot.workspaces[1].is_active);
        assert!(!snapshot.workspaces[1].is_focused);
        assert!(snapshot.workspaces[2].is_active);
        assert!(!snapshot.workspaces[2].is_focused);
    }

    #[test]
    fn workspace_activated_with_focus_moves_the_single_focus() {
        let mut snapshot = Snapshot {
            workspaces: vec![
                {
                    let mut ws = workspace(1, 1, "eDP-1");
                    ws.is_active = true;
                    ws.is_focused = true;
                    ws
                },
                workspace(2, 2, "eDP-1"),
            ],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({ "WorkspaceActivated": { "id": 2, "focused": true } });

        assert!(apply_event(&event, &mut snapshot));
        assert!(!snapshot.workspaces[0].is_active);
        assert!(!snapshot.workspaces[0].is_focused);
        assert!(snapshot.workspaces[1].is_active);
        assert!(snapshot.workspaces[1].is_focused);
    }

    #[test]
    fn workspace_activated_without_focus_clears_target_focus() {
        let mut snapshot = Snapshot {
            workspaces: vec![
                {
                    let mut ws = workspace(1, 1, "eDP-1");
                    ws.is_active = true;
                    ws.is_focused = true;
                    ws
                },
                workspace(2, 2, "eDP-1"),
            ],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({ "WorkspaceActivated": { "id": 1, "focused": false } });

        assert!(apply_event(&event, &mut snapshot));
        assert!(snapshot.workspaces[0].is_active);
        assert!(!snapshot.workspaces[0].is_focused);
    }

    #[test]
    fn workspace_without_output_does_not_deactivate_other_unknown_outputs() {
        let mut active = workspace(1, 1, "eDP-1");
        active.is_active = true;
        let mut target = workspace(2, 2, "eDP-1");
        target.output = None;
        let mut snapshot = Snapshot {
            workspaces: vec![active, target],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({ "WorkspaceActivated": { "id": 2, "focused": false } });

        assert!(apply_event(&event, &mut snapshot));
        assert!(snapshot.workspaces[0].is_active);
        assert!(snapshot.workspaces[1].is_active);
    }

    #[test]
    fn workspace_activated_with_unknown_id_is_ignored() {
        let mut snapshot = Snapshot {
            workspaces: vec![workspace(1, 1, "eDP-1")],
            layouts: KeyboardLayouts::default(),
        };
        let before = snapshot.clone();
        let event = serde_json::json!({ "WorkspaceActivated": { "id": 99, "focused": true } });

        assert!(!apply_event(&event, &mut snapshot));
        assert_eq!(snapshot, before);
    }

    #[test]
    fn workspace_active_window_changed_updates_the_window() {
        let mut snapshot = Snapshot {
            workspaces: vec![workspace(1, 1, "eDP-1")],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({
            "WorkspaceActiveWindowChanged": { "workspace_id": 1, "active_window_id": 7 }
        });

        assert!(apply_event(&event, &mut snapshot));
        assert_eq!(snapshot.workspaces[0].active_window_id, Some(7));
    }

    #[test]
    fn workspace_active_window_changed_with_unknown_id_is_ignored() {
        let mut snapshot = Snapshot {
            workspaces: vec![workspace(1, 1, "eDP-1")],
            layouts: KeyboardLayouts::default(),
        };
        let before = snapshot.clone();
        let event = serde_json::json!({
            "WorkspaceActiveWindowChanged": { "workspace_id": 99, "active_window_id": 7 }
        });

        assert!(!apply_event(&event, &mut snapshot));
        assert_eq!(snapshot, before);
    }

    #[test]
    fn workspace_urgency_changed_updates_the_flag() {
        let mut snapshot = Snapshot {
            workspaces: vec![workspace(1, 1, "eDP-1")],
            layouts: KeyboardLayouts::default(),
        };
        let event = serde_json::json!({ "WorkspaceUrgencyChanged": { "id": 1, "urgent": true } });

        assert!(apply_event(&event, &mut snapshot));
        assert!(snapshot.workspaces[0].is_urgent);
    }

    #[test]
    fn workspace_urgency_changed_with_unknown_id_is_ignored() {
        let mut snapshot = Snapshot {
            workspaces: vec![workspace(1, 1, "eDP-1")],
            layouts: KeyboardLayouts::default(),
        };
        let before = snapshot.clone();
        let event = serde_json::json!({ "WorkspaceUrgencyChanged": { "id": 99, "urgent": true } });

        assert!(!apply_event(&event, &mut snapshot));
        assert_eq!(snapshot, before);
    }

    #[test]
    fn irrelevant_event_reports_no_change() {
        let mut snapshot = Snapshot::default();
        let event = serde_json::json!({ "WindowOpenedOrChanged": {} });

        assert!(!apply_event(&event, &mut snapshot));
    }
}
