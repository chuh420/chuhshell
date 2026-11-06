use std::io::Cursor;

use proptest::prelude::*;
use serde_json::{Map, Value};

use super::{hyprland, niri::Niri, sway};

const KEYS: &[&str] = &[
    "id",
    "idx",
    "name",
    "output",
    "is_active",
    "is_focused",
    "is_urgent",
    "active_window_id",
    "workspace_id",
    "title",
    "app_id",
    "workspaces",
    "windows",
    "window",
    "nodes",
    "floating_nodes",
    "focused",
    "focus",
    "num",
    "type",
    "change",
    "current",
    "visible",
    "representation",
    "identifier",
    "xkb_active_layout_name",
    "keyboard_layouts",
    "names",
    "current_idx",
    "WorkspacesChanged",
    "WindowsChanged",
    "WindowOpenedOrChanged",
    "WindowClosed",
    "WorkspaceActivated",
    "WindowFocusChanged",
    "KeyboardLayoutsChanged",
    "KeyboardLayoutSwitched",
    "Ok",
    "Err",
];

fn json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        (-3i64..12).prop_map(Value::from),
        any::<f64>().prop_map(|value| serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null)),
        "\\PC{0,8}".prop_map(Value::from),
        prop::sample::select(KEYS).prop_map(Value::from),
    ];
    leaf.prop_recursive(5, 96, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec(
                (
                    prop_oneof![
                        prop::sample::select(KEYS).prop_map(String::from),
                        "[a-z_]{1,6}"
                    ],
                    inner
                ),
                0..6
            )
            .prop_map(|entries| Value::Object(entries.into_iter().collect::<Map<_, _>>())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn niri_survives_any_event_stream(events in prop::collection::vec(json(), 0..12)) {
        let mut niri = Niri::default();
        for event in &events {
            let _ = niri.apply(&event.to_string());
            let _ = niri.state();
            let _ = niri.keyboard();
        }
    }

    #[test]
    fn niri_survives_broken_lines(lines in prop::collection::vec("\\PC{0,64}", 0..8)) {
        let mut niri = Niri::default();
        for line in &lines {
            let changes = niri.apply(line);
            if serde_json::from_str::<Value>(line).is_err() {
                prop_assert!(!changes.state && !changes.keyboard);
            }
        }
    }

    #[test]
    fn sway_state_from_any_tree(workspaces in json(), tree in json(), inputs in json()) {
        let _ = sway::build_state(&workspaces, &tree);
        let _ = sway::build_keyboard(&inputs);
        let _ = sway::command_ok(&workspaces);
    }

    #[test]
    fn sway_frames_round_trip(kind in any::<u32>(), payload in prop::collection::vec(any::<u8>(), 0..2048)) {
        let frame = sway::encode(kind, &payload);
        let (read_kind, read_payload) = sway::read_frame(&mut Cursor::new(frame)).expect("frame");
        prop_assert_eq!(read_kind, kind);
        prop_assert_eq!(read_payload, payload);
    }

    #[test]
    fn sway_rejects_garbage_frames(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let _ = sway::read_frame(&mut Cursor::new(bytes));
    }

    #[test]
    fn sway_refuses_huge_lengths(length in (64u32 << 20) + 1..=u32::MAX, kind in any::<u32>()) {
        let mut frame = b"i3-ipc".to_vec();
        frame.extend_from_slice(&length.to_ne_bytes());
        frame.extend_from_slice(&kind.to_ne_bytes());
        prop_assert!(sway::read_frame(&mut Cursor::new(frame)).is_err());
    }

    #[test]
    fn hyprland_events_split(line in "\\PC{0,80}") {
        if let Some((name, data)) = hyprland::parse_event(&line) {
            prop_assert_eq!(format!("{name}>>{data}"), line);
        }
    }

    #[test]
    fn hyprland_window_choice(clients in prop::collection::vec(("[a-f0-9x]{0,6}", "\\PC{0,8}"), 0..6), previous in proptest::option::of(("[a-f0-9x]{0,6}", "\\PC{0,8}"))) {
        let raw: Vec<hyprland::RawWindow> = clients
            .iter()
            .map(|(address, title)| hyprland::RawWindow { address: address.clone(), title: title.clone() })
            .collect();
        let previous = previous.map(|(address, title)| super::Window { address, title });
        if let Some(window) = hyprland::keep_window(previous.as_ref(), &raw) {
            prop_assert!(raw.iter().any(|client| client.address == window.address));
            prop_assert_eq!(previous.map(|previous| previous.address), Some(window.address));
        }
    }
}
