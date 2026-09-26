use gtk::prelude::*;
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModuleOrder {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

impl Default for ModuleOrder {
    fn default() -> Self {
        Self {
            left: vec!["workspaces".into()],
            center: ["background-apps", "clock", "notifications"]
                .map(str::to_owned)
                .into(),
            right: [
                "audio",
                "brightness",
                "language",
                "temperature",
                "wifi",
                "battery",
            ]
            .map(str::to_owned)
            .into(),
        }
    }
}

impl ModuleOrder {
    pub fn groups(&self) -> [&Vec<String>; 3] {
        [&self.left, &self.center, &self.right]
    }

    fn groups_mut(&mut self) -> [&mut Vec<String>; 3] {
        [&mut self.left, &mut self.center, &mut self.right]
    }

    pub fn normalized(mut self) -> Self {
        let mut seen = BTreeSet::new();
        for group in self.groups_mut() {
            group
                .retain(|id| MODULES.iter().any(|(name, _)| name == id) && seen.insert(id.clone()));
        }
        for (index, group) in Self::default().groups().iter().enumerate() {
            for id in group.iter() {
                if seen.insert(id.clone()) {
                    self.groups_mut()[index].push(id.clone());
                }
            }
        }
        self
    }

    pub fn place(&mut self, id: &str, zone: usize, index: usize) {
        if zone >= 3 || !MODULES.iter().any(|(name, _)| *name == id) {
            return;
        }
        let before = self.groups()[zone]
            .iter()
            .take(index)
            .filter(|name| name.as_str() != id)
            .count();
        for group in self.groups_mut() {
            group.retain(|name| name != id);
        }
        let group = &mut self.groups_mut()[zone];
        group.insert(before.min(group.len()), id.to_owned());
    }
}

struct Panel {
    window: glib::WeakRef<gtk::Window>,
    groups: [glib::WeakRef<gtk::Box>; 3],
    modules: Vec<(String, glib::WeakRef<gtk::Box>)>,
}

impl Panel {
    fn apply(&self, order: &ModuleOrder) {
        let modules: Vec<_> = self
            .modules
            .iter()
            .filter_map(|(id, widget)| widget.upgrade().map(|widget| (id, widget)))
            .collect();
        for (_, widget) in &modules {
            if let Some(parent) = widget.parent().and_downcast::<gtk::Box>() {
                parent.remove(widget);
            }
        }
        for (group, ids) in self.groups.iter().zip(order.groups()) {
            if let Some(group) = group.upgrade() {
                for id in ids {
                    if let Some((_, widget)) = modules.iter().find(|(name, _)| *name == id) {
                        widget.set_halign(gtk::Align::Center);
                        group.append(widget);
                    }
                }
            }
        }
    }
}

pub struct BarModules {
    order: RefCell<ModuleOrder>,
    panels: RefCell<Vec<Panel>>,
    disabled: RefCell<BTreeSet<String>>,
    widgets: RefCell<Vec<(String, glib::WeakRef<gtk::Box>)>>,
}

impl Default for BarModules {
    fn default() -> Self {
        Self {
            order: RefCell::new(crate::config::get().bar_order.clone().normalized()),
            panels: RefCell::new(Vec::new()),
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
    pub fn order(&self) -> ModuleOrder {
        self.order.borrow().clone()
    }

    pub fn preview(&self, order: ModuleOrder) {
        let order = order.normalized();
        *self.order.borrow_mut() = order.clone();
        self.panels.borrow_mut().retain(|panel| {
            if panel.window.upgrade().is_none() {
                return false;
            }
            panel.apply(&order);
            true
        });
    }

    pub fn save_order(&self) -> Result<(), String> {
        crate::config::save_value("bar_order", serde_json::json!(self.order()))
    }

    pub fn register(
        &self,
        window: &gtk::Window,
        groups: &[gtk::Box; 3],
        modules: Vec<(&str, gtk::Box)>,
    ) {
        let panel = Panel {
            window: window.downgrade(),
            groups: groups.each_ref().map(|group| group.downgrade()),
            modules: modules
                .iter()
                .map(|(id, widget)| ((*id).to_owned(), widget.downgrade()))
                .collect(),
        };
        panel.apply(&self.order());
        let mut panels = self.panels.borrow_mut();
        panels.retain(|panel| panel.window.upgrade().is_some());
        panels.push(panel);
    }

    pub fn module_at(&self, window: &gtk::Window, x: f64, y: f64) -> Option<(String, gtk::Box)> {
        let picked = window.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let panels = self.panels.borrow();
        let panel = panels
            .iter()
            .find(|panel| panel.window.upgrade().as_ref() == Some(window))?;
        panel.modules.iter().find_map(|(id, weak)| {
            let widget = weak.upgrade()?;
            if !widget.is_visible() || (picked != widget && !picked.is_ancestor(&widget)) {
                return None;
            }
            let bounds = widget.compute_bounds(window)?;
            bounds
                .contains_point(&gtk::graphene::Point::new(x as f32, y as f32))
                .then(|| (id.clone(), widget))
        })
    }

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
        crate::config::save_value("disabled_modules", serde_json::json!(disabled))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_recovers_missing_and_duplicate_modules() {
        let order = ModuleOrder {
            left: vec!["clock".into(), "clock".into(), "unknown".into()],
            center: vec![],
            right: vec![],
        }
        .normalized();
        assert_eq!(order.left, ["clock", "workspaces"]);
        let ids: Vec<_> = order.groups().into_iter().flatten().collect();
        assert_eq!(ids.len(), MODULES.len());
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), MODULES.len());
    }

    #[test]
    fn insertion_accounts_for_the_source_position() {
        let mut order = ModuleOrder::default();
        order.place("audio", 2, 3);
        assert_eq!(&order.right[..3], &["brightness", "language", "audio"]);
        order.place("clock", 0, 0);
        assert_eq!(order.left, ["clock", "workspaces"]);
        assert_eq!(order.center, ["background-apps", "notifications"]);
        let before = order.clone();
        order.place("unknown", 0, 0);
        order.place("clock", 4, 0);
        assert_eq!(order, before);
    }
}
