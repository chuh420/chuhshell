use std::{
    io::{self, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use serde_json::Value;

use super::{Callback, Event, Keyboard, State, Window, Workspace, layout_label};

const MAGIC: &[u8; 6] = b"i3-ipc";
const HEADER: usize = 14;
const TIMEOUT: Duration = Duration::from_secs(3);

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_TREE: u32 = 4;
pub const GET_INPUTS: u32 = 100;

pub const EVENT_WORKSPACE: u32 = 0x8000_0000;
pub const EVENT_WINDOW: u32 = 0x8000_0003;
pub const EVENT_SHUTDOWN: u32 = 0x8000_0006;
pub const EVENT_INPUT: u32 = 0x8000_0015;

pub fn encode(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    frame.extend_from_slice(&kind.to_ne_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub fn parse_header(header: &[u8; HEADER]) -> io::Result<(usize, u32)> {
    if &header[..6] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad i3-ipc magic",
        ));
    }
    let length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]) as usize;
    let kind = u32::from_ne_bytes([header[10], header[11], header[12], header[13]]);
    if length > 64 << 20 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "i3-ipc frame too large",
        ));
    }
    Ok((length, kind))
}

pub fn read_frame(reader: &mut impl Read) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; HEADER];
    reader.read_exact(&mut header)?;
    let (length, kind) = parse_header(&header)?;
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    Ok((kind, payload))
}

fn socket() -> io::Result<PathBuf> {
    std::env::var_os("SWAYSOCK")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SWAYSOCK is not set"))
}

fn request(kind: u32, payload: &str) -> io::Result<Value> {
    let mut stream = UnixStream::connect(socket()?)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(&encode(kind, payload.as_bytes()))?;
    let (_, body) = read_frame(&mut stream)?;
    serde_json::from_slice(&body).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn command_ok(reply: &Value) -> io::Result<()> {
    let results = reply.as_array().map(Vec::as_slice).unwrap_or_default();
    match results
        .iter()
        .find(|result| result.get("success") != Some(&Value::Bool(true)))
    {
        Some(failed) => Err(io::Error::other(
            failed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("sway command failed")
                .to_string(),
        )),
        None if results.is_empty() => Err(io::Error::other("empty sway reply")),
        None => Ok(()),
    }
}

fn run(command: &str) -> io::Result<()> {
    command_ok(&request(RUN_COMMAND, command)?)
}

pub fn focus_workspace(id: i32) -> io::Result<()> {
    run(&format!("workspace number {id}"))
}

pub fn next_layout() -> io::Result<()> {
    run("input type:keyboard xkb_switch_layout next")
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    ["nodes", "floating_nodes"]
        .into_iter()
        .filter_map(|key| node.get(key).and_then(Value::as_array))
        .flatten()
}

fn collect_tree<'a>(node: &'a Value, occupied: &mut Vec<String>, focused: &mut Option<&'a Value>) {
    if node.get("type").and_then(Value::as_str) == Some("workspace")
        && children(node).next().is_some()
        && let Some(name) = node.get("name").and_then(Value::as_str)
    {
        occupied.push(name.to_string());
    }
    let is_window = matches!(
        node.get("type").and_then(Value::as_str),
        Some("con" | "floating_con")
    );
    if is_window && node.get("focused") == Some(&Value::Bool(true)) {
        *focused = Some(node);
    }
    for child in children(node) {
        collect_tree(child, occupied, focused);
    }
}

pub fn build_state(workspaces: &Value, tree: &Value) -> State {
    let mut occupied = Vec::new();
    let mut focused = None;
    collect_tree(tree, &mut occupied, &mut focused);
    let mut list: Vec<Workspace> = workspaces
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|workspace| {
            let num = workspace.get("num")?.as_i64()?;
            if num < 0 {
                return None;
            }
            let name = workspace.get("name")?.as_str()?.to_string();
            Some(Workspace {
                id: num as i32,
                occupied: occupied.contains(&name),
                name,
                output: workspace
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                active: workspace.get("focused") == Some(&Value::Bool(true)),
            })
        })
        .collect();
    list.sort_by_key(|workspace| workspace.id);
    let active = list.iter().find(|workspace| workspace.active);
    let active_window = focused.map(|node| Window {
        address: node.get("id").map(Value::to_string).unwrap_or_default(),
        title: node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    });
    State {
        focused_output: active.map(|workspace| workspace.output.clone()),
        active_workspace: active.map(|workspace| workspace.id),
        active_window,
        workspaces: list,
    }
}

pub fn build_keyboard(inputs: &Value) -> Option<Keyboard> {
    let keyboard = inputs.as_array()?.iter().find(|input| {
        input.get("type").and_then(Value::as_str) == Some("keyboard")
            && input
                .get("xkb_active_layout_name")
                .is_some_and(Value::is_string)
    })?;
    let name = keyboard
        .get("xkb_active_layout_name")?
        .as_str()?
        .to_string();
    Some(Keyboard {
        device: keyboard
            .get("identifier")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        label: layout_label(&name),
        name,
    })
}

fn snapshot() -> io::Result<State> {
    Ok(build_state(
        &request(GET_WORKSPACES, "")?,
        &request(GET_TREE, "")?,
    ))
}

fn keyboard() -> io::Result<Option<Keyboard>> {
    Ok(build_keyboard(&request(GET_INPUTS, "")?))
}

pub(super) fn listen(callback: &Callback) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket()?)?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(&encode(
        SUBSCRIBE,
        br#"["workspace","window","input","shutdown"]"#,
    ))?;
    let mut reader = BufReader::new(stream);
    let (_, reply) = read_frame(&mut reader)?;
    let reply: Value = serde_json::from_slice(&reply)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if reply.get("success") != Some(&Value::Bool(true)) {
        return Err(io::Error::other("sway refused the subscription"));
    }
    let mut last = snapshot()?;
    callback(Event::State(last.clone()));
    let mut last_keyboard = keyboard()?;
    if let Some(keyboard) = &last_keyboard {
        callback(Event::Keyboard(keyboard.clone()));
    }
    let mut state_dirty = false;
    let mut keyboard_dirty = false;
    loop {
        let (kind, _) = read_frame(&mut reader)?;
        match kind {
            EVENT_WORKSPACE | EVENT_WINDOW => state_dirty = true,
            EVENT_INPUT => keyboard_dirty = true,
            EVENT_SHUTDOWN => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "sway is shutting down",
                ));
            }
            _ => {}
        }
        if !reader.buffer().is_empty() {
            continue;
        }
        if state_dirty {
            state_dirty = false;
            match snapshot() {
                Ok(state) if state != last => {
                    last = state.clone();
                    callback(Event::State(state));
                }
                Ok(_) => {}
                Err(err) => log::warn!("sway state: {err}"),
            }
        }
        if keyboard_dirty {
            keyboard_dirty = false;
            match keyboard() {
                Ok(keyboard) if keyboard != last_keyboard => {
                    if let Some(next) = &keyboard {
                        callback(Event::Keyboard(next.clone()));
                    }
                    last_keyboard = keyboard;
                }
                Ok(_) => {}
                Err(err) => log::warn!("sway keyboard: {err}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frame = encode(GET_WORKSPACES, b"");
        assert_eq!(frame.len(), HEADER);
        let mut reply = encode(EVENT_WINDOW, br#"{"change":"focus"}"#);
        reply.extend_from_slice(&encode(EVENT_INPUT, b"{}"));
        let mut cursor = io::Cursor::new(reply);
        let (kind, body) = read_frame(&mut cursor).expect("first frame");
        assert_eq!(kind, EVENT_WINDOW);
        assert_eq!(body, br#"{"change":"focus"}"#);
        let (kind, _) = read_frame(&mut cursor).expect("second frame");
        assert_eq!(kind, EVENT_INPUT);
        assert!(read_frame(&mut cursor).is_err());
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut header = [0u8; HEADER];
        header[..6].copy_from_slice(b"i4-ipc");
        assert!(parse_header(&header).is_err());
    }

    #[test]
    fn state_from_workspaces_and_tree() {
        let workspaces: Value = serde_json::from_str(
            r#"[{"num":2,"name":"2","output":"eDP-1","focused":true,"visible":true},
                {"num":1,"name":"1: web","output":"eDP-1","focused":false,"visible":false},
                {"num":-1,"name":"scratch","output":"eDP-1","focused":false,"visible":false}]"#,
        )
        .expect("workspaces");
        let tree: Value = serde_json::from_str(
            r#"{"type":"root","nodes":[{"type":"output","name":"eDP-1","nodes":[
                {"type":"workspace","name":"1: web","nodes":[{"type":"con","id":31,"name":"zen","focused":false,"nodes":[]}],"floating_nodes":[]},
                {"type":"workspace","name":"2","nodes":[],"floating_nodes":[{"type":"floating_con","id":40,"name":"kitty","focused":true,"nodes":[]}]}
            ]}]}"#,
        )
        .expect("tree");
        let state = build_state(&workspaces, &tree);
        let ids: Vec<i32> = state.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert!(state.workspaces.iter().all(|w| w.occupied));
        assert_eq!(state.active_workspace, Some(2));
        assert_eq!(state.focused_output.as_deref(), Some("eDP-1"));
        assert_eq!(
            state.active_window,
            Some(Window {
                address: "40".into(),
                title: "kitty".into(),
            })
        );
        assert!(state.active_occupied());
    }

    #[test]
    fn empty_workspace_has_no_window() {
        let workspaces: Value =
            serde_json::from_str(r#"[{"num":3,"name":"3","output":"eDP-1","focused":true}]"#)
                .expect("workspaces");
        let tree: Value = serde_json::from_str(
            r#"{"type":"root","nodes":[{"type":"workspace","name":"3","nodes":[],"floating_nodes":[]}]}"#,
        )
        .expect("tree");
        let state = build_state(&workspaces, &tree);
        assert_eq!(state.active_window, None);
        assert!(!state.active_occupied());
    }

    #[test]
    fn keyboard_from_inputs() {
        let inputs: Value = serde_json::from_str(
            r#"[{"identifier":"1:1:mouse","type":"pointer"},
                {"identifier":"1:1:AT_Translated_Set_2_keyboard","type":"keyboard","xkb_active_layout_name":"Russian"}]"#,
        )
        .expect("inputs");
        let keyboard = build_keyboard(&inputs).expect("keyboard");
        assert_eq!(keyboard.label, "RU");
        assert_eq!(keyboard.device, "1:1:AT_Translated_Set_2_keyboard");
        assert_eq!(build_keyboard(&serde_json::json!([])), None);
    }

    #[test]
    fn command_replies() {
        assert!(command_ok(&serde_json::json!([{"success": true}])).is_ok());
        assert!(command_ok(&serde_json::json!([{"success": false, "error": "no"}])).is_err());
        assert!(command_ok(&serde_json::json!([])).is_err());
    }
}
