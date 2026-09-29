use gtk::gdk;
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

pub struct Shortcut {
    pub id: &'static str,
    pub scope: &'static str,
    pub description: &'static str,
    pub defaults: &'static str,
}

pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        id: "menu.close",
        scope: "menu",
        description: "Close chuh menu",
        defaults: "Escape",
    },
    Shortcut {
        id: "menu.back",
        scope: "menu",
        description: "Focus the back button",
        defaults: "Home",
    },
    Shortcut {
        id: "menu.next",
        scope: "menu",
        description: "Select the next menu item",
        defaults: "Down",
    },
    Shortcut {
        id: "menu.previous",
        scope: "menu",
        description: "Select the previous menu item",
        defaults: "Up",
    },
    Shortcut {
        id: "menu.page-next",
        scope: "menu",
        description: "Move five menu items down",
        defaults: "Page_Down",
    },
    Shortcut {
        id: "menu.page-previous",
        scope: "menu",
        description: "Move five menu items up",
        defaults: "Page_Up",
    },
    Shortcut {
        id: "menu.activate",
        scope: "menu",
        description: "Open the selected menu item",
        defaults: "Return, KP_Enter",
    },
    Shortcut {
        id: "launcher.close",
        scope: "launcher",
        description: "Close the app launcher",
        defaults: "Escape",
    },
    Shortcut {
        id: "launcher.next",
        scope: "launcher",
        description: "Select the next app",
        defaults: "Down",
    },
    Shortcut {
        id: "launcher.previous",
        scope: "launcher",
        description: "Select the previous app",
        defaults: "Up",
    },
    Shortcut {
        id: "launcher.page-next",
        scope: "launcher",
        description: "Move five apps down",
        defaults: "Page_Down",
    },
    Shortcut {
        id: "launcher.page-previous",
        scope: "launcher",
        description: "Move five apps up",
        defaults: "Page_Up",
    },
    Shortcut {
        id: "launcher.activate",
        scope: "launcher",
        description: "Launch the selected app or toggle its visibility",
        defaults: "Return, KP_Enter",
    },
    Shortcut {
        id: "clipboard.next",
        scope: "clipboard",
        description: "Select the next clipboard item",
        defaults: "Down",
    },
    Shortcut {
        id: "clipboard.previous",
        scope: "clipboard",
        description: "Select the previous clipboard item",
        defaults: "Up",
    },
    Shortcut {
        id: "clipboard.page-next",
        scope: "clipboard",
        description: "Move five clipboard items down",
        defaults: "Page_Down",
    },
    Shortcut {
        id: "clipboard.page-previous",
        scope: "clipboard",
        description: "Move five clipboard items up",
        defaults: "Page_Up",
    },
    Shortcut {
        id: "clipboard.copy",
        scope: "clipboard",
        description: "Copy the selected clipboard item",
        defaults: "Right, Return, KP_Enter",
    },
    Shortcut {
        id: "clipboard.delete",
        scope: "clipboard",
        description: "Remove the selected clipboard item",
        defaults: "Delete",
    },
    Shortcut {
        id: "clipboard.focus",
        scope: "clipboard-search",
        description: "Move from search to clipboard history",
        defaults: "Down",
    },
    Shortcut {
        id: "bar-editor.cancel",
        scope: "bar-editor",
        description: "Close the bar editor without saving",
        defaults: "Escape",
    },
    Shortcut {
        id: "layout-editor.cancel",
        scope: "layout-editor",
        description: "Close the launcher editor without saving",
        defaults: "Escape",
    },
];

fn settings() -> &'static RwLock<BTreeMap<String, String>> {
    static SETTINGS: OnceLock<RwLock<BTreeMap<String, String>>> = OnceLock::new();
    SETTINGS.get_or_init(|| RwLock::new(crate::config::get().keybindings.clone()))
}

pub fn value(shortcut: &Shortcut) -> String {
    settings()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(shortcut.id)
        .cloned()
        .unwrap_or_else(|| shortcut.defaults.into())
}

pub fn hint(id: &str) -> String {
    SHORTCUTS
        .iter()
        .find(|s| s.id == id)
        .map(value)
        .and_then(|value| value.split(',').next().map(str::to_owned))
        .unwrap_or_default()
}

fn parse(value: &str) -> Result<Vec<(gdk::Key, gdk::ModifierType)>, String> {
    value
        .split(',')
        .map(|chord| {
            let mut parts: Vec<_> = chord.trim().split('+').collect();
            let name = parts.pop().unwrap_or_default();
            let key =
                gdk::Key::from_name(name).ok_or_else(|| format!("Unknown XKB key: {name}"))?;
            let mut modifiers = gdk::ModifierType::empty();
            for part in parts {
                modifiers |= match part.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => gdk::ModifierType::CONTROL_MASK,
                    "shift" => gdk::ModifierType::SHIFT_MASK,
                    "alt" => gdk::ModifierType::ALT_MASK,
                    "super" | "win" | "mod" => gdk::ModifierType::SUPER_MASK,
                    _ => return Err(format!("Unknown modifier: {part}")),
                };
            }
            Ok((key.to_lower(), modifiers))
        })
        .collect()
}

fn overlaps(a: &Shortcut, b: &Shortcut) -> bool {
    a.scope == b.scope
        || (matches!(a.id, "menu.close" | "menu.back") && b.scope.starts_with("clipboard"))
        || (matches!(b.id, "menu.close" | "menu.back") && a.scope.starts_with("clipboard"))
}

fn validate(id: &str, value: &str, overrides: &BTreeMap<String, String>) -> Result<(), String> {
    let shortcut = SHORTCUTS
        .iter()
        .find(|s| s.id == id)
        .ok_or("Unknown shortcut")?;
    let keys = parse(value)?;
    for other in SHORTCUTS
        .iter()
        .filter(|other| other.id != id && overlaps(shortcut, other))
    {
        let other_value = overrides
            .get(other.id)
            .map(String::as_str)
            .unwrap_or(other.defaults);
        if parse(other_value)?.iter().any(|key| keys.contains(key)) {
            return Err(format!("Already assigned: {}", other.description));
        }
    }
    Ok(())
}

pub fn save(id: &str, value: &str) -> Result<(), String> {
    let mut overrides = crate::config::read()?.keybindings;
    validate(id, value, &overrides)?;
    overrides.insert(id.into(), value.into());
    crate::config::save_value(
        "keybindings",
        serde_json::to_value(&overrides).map_err(|e| e.to_string())?,
    )?;
    *settings().write().unwrap_or_else(|e| e.into_inner()) = overrides;
    Ok(())
}

pub fn remap(scope: &str, key: gdk::Key, modifiers: gdk::ModifierType) -> gdk::Key {
    let mask = gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::SUPER_MASK;
    let pressed = (key.to_lower(), modifiers & mask);
    let settings = settings().read().unwrap_or_else(|e| e.into_inner());
    for shortcut in SHORTCUTS.iter().filter(|s| s.scope == scope) {
        let binding = settings
            .get(shortcut.id)
            .map(String::as_str)
            .unwrap_or(shortcut.defaults);
        if parse(binding).unwrap_or_default().contains(&pressed) {
            return gdk::Key::from_name(shortcut.defaults.split(',').next().unwrap()).unwrap();
        }
    }
    gdk::Key::VoidSymbol
}

pub fn is_default(scope: &str, key: gdk::Key) -> bool {
    SHORTCUTS.iter().filter(|s| s.scope == scope).any(|s| {
        parse(s.defaults)
            .unwrap_or_default()
            .iter()
            .any(|(default, _)| *default == key.to_lower())
    })
}

pub fn capture(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<String> {
    if matches!(
        key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::ISO_Level3_Shift
    ) {
        return None;
    }
    let mut parts = Vec::new();
    for (flag, name) in [
        (gdk::ModifierType::SUPER_MASK, "Super"),
        (gdk::ModifierType::CONTROL_MASK, "Ctrl"),
        (gdk::ModifierType::ALT_MASK, "Alt"),
        (gdk::ModifierType::SHIFT_MASK, "Shift"),
    ] {
        if modifiers.contains(flag) {
            parts.push(name.to_owned());
        }
    }
    parts.push(key.to_lower().name()?.to_string());
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicts_are_scoped_and_include_parent_menu_actions() {
        let values = BTreeMap::new();
        assert!(validate("launcher.close", "Ctrl+q", &values).is_ok());
        assert!(validate("launcher.close", "Down", &values).is_err());
        assert!(validate("clipboard.delete", "Escape", &values).is_err());
        assert!(validate("menu.close", "Delete", &values).is_err());
        assert!(validate("launcher.close", "NotAKey", &values).is_err());
    }

    #[test]
    fn parses_alternatives_and_modifier_aliases() {
        assert_eq!(parse("Ctrl+q, Return").unwrap().len(), 2);
        assert_eq!(
            parse("Control+Shift+Q").unwrap(),
            parse("Shift+Ctrl+q").unwrap()
        );
        assert!(parse("").is_err());
    }
}
