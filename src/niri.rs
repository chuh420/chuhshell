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
    pub idx: i64,
    pub name: Option<String>,
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

fn snapshot() -> Option<(Vec<Workspace>, KeyboardLayouts)> {
    let workspaces = request("\"Workspaces\"")
        .and_then(|value| value.get("Workspaces").cloned())
        .and_then(|value| serde_json::from_value::<Vec<Workspace>>(value).ok())?;
    let layouts = request("\"KeyboardLayouts\"")
        .and_then(|value| value.get("KeyboardLayouts").cloned())
        .and_then(|value| serde_json::from_value::<KeyboardLayouts>(value).ok())?;
    Some((workspaces, layouts))
}

fn apply_event(
    event: &serde_json::Value,
    workspaces: &mut Vec<Workspace>,
    layouts: &mut KeyboardLayouts,
) -> bool {
    let mut changed = false;
    if let Some(value) = event
        .get("WorkspacesChanged")
        .and_then(|value| value.get("workspaces"))
        .cloned()
        && let Ok(next) = serde_json::from_value::<Vec<Workspace>>(value)
    {
        *workspaces = next;
        changed = true;
    }
    if let Some(value) = event
        .get("KeyboardLayoutsChanged")
        .and_then(|value| value.get("keyboard_layouts"))
        .cloned()
        && let Ok(next) = serde_json::from_value::<KeyboardLayouts>(value)
    {
        *layouts = next;
        changed = true;
    }
    if let Some(idx) = event
        .get("KeyboardLayoutSwitched")
        .and_then(|value| value.get("idx"))
        .and_then(serde_json::Value::as_u64)
    {
        layouts.current_idx = idx as usize;
        changed = true;
    }
    changed
}

fn read_ok_line(reader: &mut impl BufRead, line: &mut String) -> bool {
    line.clear();
    reader.read_line(line).is_ok() && response(line).is_some()
}

pub fn spawn_poller(sender: mpsc::Sender<(Vec<Workspace>, KeyboardLayouts)>) {
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
            let Some((mut workspaces, mut layouts)) = snapshot() else {
                thread::sleep(RECONNECT_DELAY);
                continue;
            };
            if sender.send((workspaces.clone(), layouts.clone())).is_err() {
                return;
            }
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                            continue;
                        };
                        if apply_event(&event, &mut workspaces, &mut layouts)
                            && sender.send((workspaces.clone(), layouts.clone())).is_err()
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
    fn event_updates_keyboard_layout_immediately() {
        let mut workspaces = Vec::new();
        let mut layouts = KeyboardLayouts {
            names: vec!["English (US)".to_owned(), "Russian".to_owned()],
            current_idx: 0,
        };
        let event = serde_json::json!({
            "KeyboardLayoutsChanged": {
                "keyboard_layouts": {
                    "names": ["English (US)", "Russian"],
                    "current_idx": 1
                }
            }
        });

        assert!(apply_event(&event, &mut workspaces, &mut layouts));
        assert_eq!(layouts.current_idx, 1);
    }

    #[test]
    fn layout_switch_event_updates_current_index() {
        let mut workspaces = Vec::new();
        let mut layouts = KeyboardLayouts {
            names: vec!["English (US)".to_owned(), "Russian".to_owned()],
            current_idx: 0,
        };
        let event = serde_json::json!({ "KeyboardLayoutSwitched": { "idx": 1 } });

        assert!(apply_event(&event, &mut workspaces, &mut layouts));
        assert_eq!(layouts.current_idx, 1);
    }

    #[test]
    fn event_updates_workspaces() {
        let mut workspaces = Vec::new();
        let mut layouts = KeyboardLayouts::default();
        let event = serde_json::json!({
            "WorkspacesChanged": {
                "workspaces": [{
                    "idx": 2,
                    "name": null,
                    "is_urgent": false,
                    "is_active": true,
                    "is_focused": true,
                    "active_window_id": 42
                }]
            }
        });

        assert!(apply_event(&event, &mut workspaces, &mut layouts));
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].active_window_id, Some(42));
    }

    #[test]
    fn irrelevant_event_reports_no_change() {
        let mut workspaces = Vec::new();
        let mut layouts = KeyboardLayouts::default();
        let event = serde_json::json!({ "WindowOpenedOrChanged": {} });

        assert!(!apply_event(&event, &mut workspaces, &mut layouts));
    }
}
