use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    thread,
    time::Duration,
};

use serde::Deserialize;

use super::{Callback, Event, Keyboard, State, Window, Workspace, layout_label};

#[derive(Deserialize)]
struct RawWorkspace {
    id: i32,
    name: String,
    #[serde(default)]
    monitor: String,
    #[serde(default)]
    windows: u32,
}

#[derive(Deserialize)]
struct RawWorkspaceRef {
    id: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMonitor {
    name: String,
    focused: bool,
    active_workspace: RawWorkspaceRef,
}

#[derive(Deserialize)]
pub struct RawWindow {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Deserialize)]
struct RawKeyboard {
    name: String,
    #[serde(default)]
    main: bool,
    #[serde(default)]
    active_keymap: String,
}

#[derive(Deserialize)]
struct RawDevices {
    keyboards: Vec<RawKeyboard>,
}

const TIMEOUT: Duration = Duration::from_secs(3);

fn socket_dir() -> io::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "HYPRLAND_INSTANCE_SIGNATURE is not set",
        )
    })?;
    Ok(PathBuf::from(runtime).join("hypr").join(signature))
}

pub fn request(command: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_dir()?.join(".socket.sock"))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

fn json<T: for<'de> Deserialize<'de>>(command: &str) -> io::Result<T> {
    let reply = request(command)?;
    serde_json::from_str(&reply).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn keep_window(previous: Option<&Window>, clients: &[RawWindow]) -> Option<Window> {
    let previous = previous?;
    clients
        .iter()
        .find(|client| client.address == previous.address)
        .map(|client| Window {
            address: client.address.clone(),
            title: client.title.clone(),
        })
}

pub fn snapshot(previous: Option<&Window>) -> io::Result<State> {
    let workspaces: Vec<RawWorkspace> = json("j/workspaces")?;
    let monitors: Vec<RawMonitor> = json("j/monitors")?;
    let window: RawWindow = json("j/activewindow")?;
    let focused = monitors.iter().find(|monitor| monitor.focused);
    let active_window = if window.address.is_empty() {
        let clients: Vec<RawWindow> = json("j/clients")?;
        keep_window(previous, &clients)
    } else {
        Some(Window {
            address: window.address,
            title: window.title,
        })
    };
    let active_workspace = focused.map(|monitor| monitor.active_workspace.id);
    Ok(State {
        workspaces: workspaces
            .into_iter()
            .map(|workspace| Workspace {
                active: Some(workspace.id) == active_workspace,
                occupied: workspace.windows > 0,
                id: workspace.id,
                name: workspace.name,
                output: workspace.monitor,
            })
            .collect(),
        focused_output: focused.map(|monitor| monitor.name.clone()),
        active_workspace,
        active_window,
    })
}

pub fn keyboard() -> io::Result<Keyboard> {
    let devices: RawDevices = json("j/devices")?;
    let device = devices
        .keyboards
        .iter()
        .find(|keyboard| keyboard.main)
        .or(devices.keyboards.first());
    let name = device.map_or_else(|| "Unknown".to_string(), |d| d.active_keymap.clone());
    let name = if name.is_empty() {
        "Unknown".to_string()
    } else {
        name
    };
    Ok(Keyboard {
        device: device.map(|d| d.name.clone()).unwrap_or_default(),
        label: layout_label(&name),
        name,
    })
}

pub fn focus_workspace(id: i32) -> io::Result<()> {
    request(&format!(
        "eval hl.dispatch(hl.dsp.focus({{workspace={id}}}))"
    ))
    .map(|_| ())
}

pub fn next_layout() -> io::Result<()> {
    let device = keyboard()?.device;
    if device.is_empty() {
        return Ok(());
    }
    request(&format!("switchxkblayout {device} next")).map(|_| ())
}

fn affects_state(name: &str) -> bool {
    matches!(
        name,
        "workspace"
            | "workspacev2"
            | "focusedmon"
            | "focusedmonv2"
            | "activewindow"
            | "activewindowv2"
            | "windowtitle"
            | "windowtitlev2"
            | "createworkspace"
            | "createworkspacev2"
            | "destroyworkspace"
            | "destroyworkspacev2"
            | "moveworkspace"
            | "moveworkspacev2"
            | "renameworkspace"
            | "activespecial"
            | "activespecialv2"
            | "openwindow"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
            | "monitoradded"
            | "monitoraddedv2"
            | "monitorremoved"
            | "monitorremovedv2"
            | "configreloaded"
    )
}

fn affects_keyboard(name: &str) -> bool {
    matches!(name, "activelayout" | "configreloaded")
}

pub fn parse_event(line: &str) -> Option<(&str, &str)> {
    line.split_once(">>")
}

pub(super) fn listen(callback: &Callback) -> io::Result<()> {
    let stream = UnixStream::connect(socket_dir()?.join(".socket2.sock"))?;
    let mut last = snapshot(None)?;
    callback(Event::State(last.clone()));
    callback(Event::Keyboard(keyboard()?));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut state_dirty = false;
    let mut keyboard_dirty = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "event socket closed",
            ));
        }
        if let Some((name, _)) = parse_event(line.trim_end_matches('\n')) {
            state_dirty |= affects_state(name);
            keyboard_dirty |= affects_keyboard(name);
            if name == "configreloaded" {
                callback(Event::ConfigReloaded);
            }
        }
        if !reader.buffer().is_empty() {
            continue;
        }
        if state_dirty {
            state_dirty = false;
            match snapshot(last.active_window.as_ref()) {
                Ok(state) => {
                    if state != last {
                        last = state.clone();
                        callback(Event::State(state));
                    }
                }
                Err(err) => log::warn!("hyprland state: {err}"),
            }
        }
        if keyboard_dirty {
            keyboard_dirty = false;
            thread::sleep(Duration::from_millis(60));
            match keyboard() {
                Ok(keyboard) => callback(Event::Keyboard(keyboard)),
                Err(err) => log::warn!("hyprland keyboard: {err}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_keyboard_py() {
        assert_eq!(layout_label("English (US)"), "EN");
        assert_eq!(layout_label("Russian"), "RU");
        assert_eq!(layout_label("Ukrainian"), "UK");
        assert_eq!(layout_label("Unknown"), "UN");
    }

    #[test]
    fn events_split_on_marker() {
        assert_eq!(
            parse_event("activelayout>>at-translated-set-2-keyboard,Russian"),
            Some(("activelayout", "at-translated-set-2-keyboard,Russian"))
        );
        assert_eq!(parse_event("garbage"), None);
    }

    #[test]
    fn empty_focus_keeps_existing_window_like_quickshell() {
        let previous = Window {
            address: "0x1".into(),
            title: "old".into(),
        };
        let clients = vec![RawWindow {
            address: "0x1".into(),
            title: "renamed".into(),
        }];
        assert_eq!(
            keep_window(Some(&previous), &clients),
            Some(Window {
                address: "0x1".into(),
                title: "renamed".into(),
            })
        );
        assert_eq!(keep_window(Some(&previous), &[]), None);
    }

    #[test]
    fn empty_active_window_parses() {
        let window: RawWindow = serde_json::from_str("{}").expect("valid json");
        assert!(window.address.is_empty());
    }
}
