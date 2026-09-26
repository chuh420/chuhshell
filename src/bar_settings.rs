use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::BTreeSet;

pub const MODULES: &[(&str, &str)] = &[
    ("workspaces", "Workspaces"),
    ("background-apps", "Background apps"),
    ("clock", "Clock"),
    ("notifications", "Notifications"),
    ("audio", "Audio"),
    ("brightness", "Brightness"),
    ("language", "Keyboard layout"),
    ("temperature", "CPU temperature"),
    ("wifi", "Wi-Fi"),
    ("battery", "Battery"),
];

pub struct BarModules {
    disabled: RefCell<BTreeSet<String>>,
    widgets: RefCell<Vec<(String, glib::WeakRef<gtk::Box>)>>,
}

impl Default for BarModules {
    fn default() -> Self {
        Self {
            disabled: RefCell::new(
                crate::config::get()
                    .disabled_modules
                    .iter()
                    .cloned()
                    .collect(),
            ),
            widgets: RefCell::new(Vec::new()),
        }
    }
}

impl BarModules {
    pub fn enabled(&self, id: &str) -> bool {
        !self.disabled.borrow().contains(id)
    }

    pub fn wrap(&self, id: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        container.set_valign(gtk::Align::Center);
        container.append(widget);
        container.set_visible(self.enabled(id) && widget.get_visible());
        let weak = container.downgrade();
        let owned_id = id.to_owned();
        widget.connect_visible_notify(move |widget| {
            if let Some(container) = weak.upgrade() {
                let disabled = container.has_css_class("module-disabled");
                container.set_visible(!disabled && widget.get_visible());
            }
        });
        if !self.enabled(id) {
            container.add_css_class("module-disabled");
        }
        let mut widgets = self.widgets.borrow_mut();
        widgets.retain(|(_, widget)| widget.upgrade().is_some());
        widgets.push((owned_id, container.downgrade()));
        container
    }

    pub fn toggle(&self, id: &str) -> Result<bool, String> {
        let enabled = !self.enabled(id);
        let mut disabled = self.disabled.borrow().clone();
        if enabled {
            disabled.remove(id);
        } else {
            disabled.insert(id.to_owned());
        }
        let path = crate::config::path();
        let mut value: serde_json::Value = match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|e| format!("Could not read settings: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(e) => return Err(format!("Could not read settings: {e}")),
        };
        let object = value
            .as_object_mut()
            .ok_or("Settings must be a JSON object")?;
        object.insert("disabled_modules".into(), serde_json::json!(disabled));
        let write = || -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(path.parent().ok_or("Invalid settings path")?)?;
            let temporary = path.with_extension("json.tmp");
            std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
            std::fs::rename(temporary, &path)?;
            Ok(())
        };
        write().map_err(|e| format!("Could not save settings: {e}"))?;
        *self.disabled.borrow_mut() = disabled;
        self.widgets.borrow_mut().retain(|(name, weak)| {
            let Some(widget) = weak.upgrade() else {
                return false;
            };
            if name == id {
                if enabled {
                    widget.remove_css_class("module-disabled");
                } else {
                    widget.add_css_class("module-disabled");
                }
                widget.set_visible(
                    enabled
                        && widget
                            .first_child()
                            .is_some_and(|child| child.get_visible()),
                );
            }
            true
        });
        Ok(enabled)
    }
}
