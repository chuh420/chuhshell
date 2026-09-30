mod local;
pub(crate) mod niri;

use gtk::prelude::*;
pub use local::{hint, is_default, remap};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
enum Target {
    Niri(niri::Binding),
    Local(&'static local::Shortcut),
}

struct Panel {
    root: std::path::PathBuf,
    catalog: RefCell<Option<niri::Catalog>>,
    target: RefCell<Option<Target>>,
    list: glib::WeakRef<gtk::ListBox>,
    status: glib::WeakRef<gtk::Label>,
    editor: glib::WeakRef<gtk::Box>,
    title: glib::WeakRef<gtk::Label>,
    entry: glib::WeakRef<gtk::Entry>,
    search: glib::WeakRef<gtk::SearchEntry>,
    refresh: glib::WeakRef<gtk::Button>,
    reset: glib::WeakRef<gtk::Button>,
    busy: Cell<bool>,
    recording: Cell<bool>,
}

fn button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("network-action");
    button
}

impl Panel {
    fn handle_key(
        &self,
        key: gtk::gdk::Key,
        keycode: u32,
        modifiers: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        if self.recording.get() {
            let key = if matches!(self.target.borrow().as_ref(), Some(Target::Niri(_)))
                && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
            {
                self.entry
                    .upgrade()
                    .and_then(|w| w.display().map_keycode(keycode))
                    .and_then(|keys| {
                        keys.into_iter()
                            .find(|(mapping, _)| mapping.group() == 0 && mapping.level() == 0)
                    })
                    .map_or(key, |(_, key)| key)
            } else {
                key
            };
            if key == gtk::gdk::Key::Escape {
                self.recording.set(false);
                self.status("Recording cancelled", false);
            } else if let Some(value) = local::capture(key, modifiers) {
                if let Some(entry) = self.entry.upgrade() {
                    entry.set_text(&value);
                }
                self.recording.set(false);
                self.status("Review the combination and Save", false);
            }
            return glib::Propagation::Stop;
        }
        if self.target.borrow().is_some() && key == gtk::gdk::Key::Escape {
            self.cancel();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    }

    fn status(&self, text: &str, error: bool) {
        if let Some(status) = self.status.upgrade() {
            status.set_text(text);
            if error {
                status.add_css_class("menu-error");
            } else {
                status.remove_css_class("menu-error");
            }
        }
    }

    fn busy(&self, busy: bool) {
        self.busy.set(busy);
        if let Some(list) = self.list.upgrade() {
            list.set_sensitive(!busy);
        }
        if let Some(editor) = self.editor.upgrade() {
            editor.set_sensitive(!busy);
        }
        if let Some(refresh) = self.refresh.upgrade() {
            refresh.set_sensitive(!busy);
        }
    }

    fn refresh(self: &Rc<Self>, saved: bool) {
        if self.busy.get() {
            return;
        }
        self.busy(true);
        self.status("Reading Niri and chuhshell keybindings…", false);
        let (tx, rx) = async_channel::bounded(1);
        let root = self.root.clone();
        std::thread::spawn(move || {
            let result = niri::Catalog::load(&root).map(|mut catalog| {
                catalog.enrich_descriptions();
                catalog
            });
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            let Some(panel) = weak.upgrade() else {
                return;
            };
            panel.busy(false);
            match result {
                Ok(catalog) => {
                    let count = catalog.bindings.len();
                    *panel.catalog.borrow_mut() = Some(catalog);
                    panel.status(
                        &format!(
                            "{}{} Niri · {} chuhshell",
                            if saved { "Saved · " } else { "" },
                            count,
                            local::SHORTCUTS.len()
                        ),
                        false,
                    );
                }
                Err(error) => {
                    panel.catalog.borrow_mut().take();
                    panel.status(
                        &format!("Niri: {error}. Internal shortcuts are available."),
                        true,
                    );
                }
            }
            panel.render();
        });
    }

    fn render(self: &Rc<Self>) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let query = self
            .search
            .upgrade()
            .map(|s| s.text().to_lowercase())
            .unwrap_or_default();
        let mut count = 0;
        if let Some(catalog) = self.catalog.borrow().as_ref() {
            for binding in &catalog.bindings {
                let source = format!(
                    "{} · {}:{}{}",
                    if binding.shell {
                        "chuhshell / Niri"
                    } else {
                        "Niri"
                    },
                    binding.path.display(),
                    catalog.files[&binding.path][..binding.start]
                        .bytes()
                        .filter(|b| *b == b'\n')
                        .count()
                        + 1,
                    if binding.overridden {
                        " · overridden"
                    } else {
                        ""
                    }
                );
                if format!(
                    "{} {} {} {}",
                    binding.key, binding.description, binding.action, source
                )
                .to_lowercase()
                .contains(&query)
                {
                    self.row(
                        &list,
                        &binding.description,
                        &binding.key,
                        &source,
                        &binding.action,
                        Target::Niri(binding.clone()),
                    );
                    count += 1;
                }
            }
        }
        for shortcut in local::SHORTCUTS {
            let value = local::value(shortcut);
            let source = format!("chuhshell · {}", shortcut.scope);
            if format!("{source} {} {value}", shortcut.description)
                .to_lowercase()
                .contains(&query)
            {
                self.row(
                    &list,
                    shortcut.description,
                    &value,
                    &source,
                    shortcut.id,
                    Target::Local(shortcut),
                );
                count += 1;
            }
        }
        if count == 0 {
            list.append(&crate::info::label("No matching keybindings", "menu-hint"));
        }
    }

    fn row(
        self: &Rc<Self>,
        list: &gtk::ListBox,
        description: &str,
        key: &str,
        source: &str,
        action: &str,
        target: Target,
    ) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 5);
        row.add_css_class("keybinding-row");
        let title = crate::info::label(description, "menu-label");
        title.set_wrap(true);
        title.set_max_width_chars(48);
        row.append(&title);
        let details = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let origin = crate::info::label(source, "menu-hint");
        origin.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        origin.set_max_width_chars(28);
        origin.set_hexpand(true);
        origin.set_tooltip_text(Some(&format!("{source}\n{action}")));
        details.append(&origin);
        let edit = button(key);
        edit.set_tooltip_text(Some("Change this keybinding"));
        let weak = Rc::downgrade(self);
        edit.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.edit(target.clone());
            }
        });
        details.append(&edit);
        row.append(&details);
        list.append(&row);
    }

    fn edit(&self, target: Target) {
        let (description, value) = match &target {
            Target::Niri(binding) => (binding.description.clone(), binding.key.clone()),
            Target::Local(shortcut) => (shortcut.description.into(), local::value(shortcut)),
        };
        self.recording.set(false);
        if let Some(reset) = self.reset.upgrade() {
            reset.set_visible(matches!(target, Target::Local(_)));
        }
        *self.target.borrow_mut() = Some(target);
        if let Some(title) = self.title.upgrade() {
            title.set_text(&description);
        }
        if let Some(editor) = self.editor.upgrade() {
            editor.set_visible(true);
        }
        if let Some(entry) = self.entry.upgrade() {
            entry.set_text(&value);
            entry.grab_focus();
            entry.select_region(0, -1);
        }
        self.status(
            "Type an XKB combination, or Record it. Separate internal alternatives with commas.",
            false,
        );
    }

    fn cancel(&self) {
        self.recording.set(false);
        self.target.borrow_mut().take();
        if let Some(editor) = self.editor.upgrade() {
            editor.set_visible(false);
        }
        if let Some(search) = self.search.upgrade() {
            search.grab_focus();
        }
    }

    fn save(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let Some(target) = self.target.borrow().clone() else {
            return;
        };
        let Some(entry) = self.entry.upgrade() else {
            return;
        };
        let value = entry.text().trim().to_owned();
        let catalog = self.catalog.borrow().clone();
        self.recording.set(false);
        self.busy(true);
        self.status("Validating and saving…", false);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = match target {
                Target::Niri(binding) => catalog
                    .ok_or("Refresh the configuration first".into())
                    .and_then(|catalog| catalog.save(&binding, &value)),
                Target::Local(shortcut) => local::save(shortcut.id, &value),
            };
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            let Some(panel) = weak.upgrade() else {
                return;
            };
            panel.busy(false);
            match result {
                Ok(()) => {
                    panel.cancel();
                    panel.refresh(true);
                }
                Err(error) => panel.status(&error, true),
            }
        });
    }
}

pub struct View {
    pub widget: gtk::Box,
    panel: Rc<Panel>,
}

impl View {
    pub fn handle_key(
        &self,
        key: gtk::gdk::Key,
        keycode: u32,
        modifiers: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        self.panel.handle_key(key, keycode, modifiers)
    }
}

pub fn view() -> View {
    let (widget, panel) = build(niri::config_path());
    View { widget, panel }
}

fn build(root: std::path::PathBuf) -> (gtk::Box, Rc<Panel>) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search keys, actions or sources"));
    search.add_css_class("search");
    search.set_hexpand(true);
    let refresh = button("Refresh");
    toolbar.append(&search);
    toolbar.append(&refresh);
    outer.append(&toolbar);
    let status = crate::info::label("", "menu-hint");
    status.set_wrap(true);
    status.set_max_width_chars(54);
    outer.append(&status);
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 8);
    editor.add_css_class("keybinding-editor");
    let title = crate::info::label("", "menu-label");
    title.set_wrap(true);
    title.set_max_width_chars(48);
    editor.append(&title);
    let entry = gtk::Entry::new();
    entry.add_css_class("search");
    entry.set_placeholder_text(Some("Mod+Shift+T"));
    editor.append(&entry);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let record = button("Record");
    let reset = button("Default");
    let cancel = button("Cancel");
    let save = button("Save");
    save.add_css_class("primary");
    for action in [&record, &reset, &cancel, &save] {
        actions.append(action);
    }
    editor.append(&actions);
    editor.set_visible(false);
    outer.append(&editor);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("menu-list");
    let height =
        crate::ui::active_monitor().map_or(340, |m| (m.geometry().height() - 440).clamp(100, 340));
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(140.min(height))
        .max_content_height(height)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    outer.append(&scroll);
    let panel = Rc::new(Panel {
        root,
        catalog: RefCell::new(None),
        target: RefCell::new(None),
        list: list.downgrade(),
        status: status.downgrade(),
        editor: editor.downgrade(),
        title: title.downgrade(),
        entry: entry.downgrade(),
        search: search.downgrade(),
        refresh: refresh.downgrade(),
        reset: reset.downgrade(),
        busy: Cell::new(false),
        recording: Cell::new(false),
    });
    search.connect_search_changed({
        let panel = panel.clone();
        move |_| panel.render()
    });
    refresh.connect_clicked({
        let panel = panel.clone();
        move |_| {
            panel.cancel();
            panel.refresh(false);
        }
    });
    save.connect_clicked({
        let panel = panel.clone();
        move |_| panel.save()
    });
    entry.connect_activate({
        let panel = panel.clone();
        move |_| panel.save()
    });
    cancel.connect_clicked({
        let panel = panel.clone();
        move |_| panel.cancel()
    });
    reset.connect_clicked({
        let panel = panel.clone();
        move |_| {
            if let Some(Target::Local(shortcut)) = panel.target.borrow().as_ref()
                && let Some(entry) = panel.entry.upgrade()
            {
                entry.set_text(shortcut.defaults);
            }
        }
    });
    record.connect_clicked({
        let panel = panel.clone();
        move |_| {
            panel.recording.set(true);
            panel.status("Press a combination. Escape cancels recording. Global shortcuts may be intercepted by Niri; type those manually.", false);
            if let Some(entry) = panel.entry.upgrade() { entry.grab_focus(); }
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let panel = panel.clone();
        move |_, key, keycode, modifiers| panel.handle_key(key, keycode, modifiers)
    });
    outer.add_controller(keys);
    panel.refresh(false);
    (outer, panel)
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application) {
    fn wait(panel: &Panel) {
        for _ in 0..200 {
            crate::ui_tests::pump(20);
            if !panel.busy.get() {
                return;
            }
        }
        panic!("Keybindings worker did not finish");
    }
    let dir = crate::config::path()
        .parent()
        .unwrap()
        .join("keybindings-test");
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join("config.kdl");
    std::fs::write(&root, "binds { Mod+T hotkey-overlay-title=\"Terminal\" { spawn \"foot\"; }; Mod+Space { spawn \"chuhshell\" \"menu\"; }; }\n").unwrap();
    let (view, panel) = build(root.clone());
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 10);
    frame.add_css_class("menu-content");
    frame.append(&crate::info::label("Keybindings", "menu-heading"));
    frame.append(&view);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Keybindings")
        .default_width(460)
        .child(&frame)
        .build();
    window.set_widget_name("chuh-menu");
    crate::ui::set_layer_window(
        &window,
        "chuhshell-keybindings-test",
        gtk4_layer_shell::Layer::Overlay,
        &[],
        0,
        gtk4_layer_shell::KeyboardMode::OnDemand,
    );
    window.present();
    wait(&panel);
    assert_eq!(panel.catalog.borrow().as_ref().unwrap().bindings.len(), 2);
    assert!(panel.list.upgrade().unwrap().row_at_index(2).is_some());
    crate::ui_tests::pump(100);
    crate::ui_tests::capture("keybindings");
    let search = panel.search.upgrade().unwrap();
    search.set_text("Terminal");
    crate::ui_tests::pump(200);
    assert!(panel.list.upgrade().unwrap().row_at_index(0).is_some());
    assert!(panel.list.upgrade().unwrap().row_at_index(1).is_none());
    let binding = panel.catalog.borrow().as_ref().unwrap().bindings[0].clone();
    panel.edit(Target::Niri(binding));
    let entry = panel.entry.upgrade().unwrap();
    entry.set_text("Mod+Space");
    panel.save();
    wait(&panel);
    assert!(
        panel
            .status
            .upgrade()
            .unwrap()
            .text()
            .contains("already assigned")
    );
    assert!(panel.editor.upgrade().unwrap().is_visible());
    entry.set_text("Mod+Y");
    let validator_available = crate::process::run("niri", &["--version"]).is_ok();
    panel.save();
    wait(&panel);
    if validator_available {
        assert!(std::fs::read_to_string(&root).unwrap().contains("Mod+Y"));
        assert!(!panel.editor.upgrade().unwrap().is_visible());
    } else {
        assert!(std::fs::read_to_string(&root).unwrap().contains("Mod+T"));
        assert!(
            panel
                .status
                .upgrade()
                .unwrap()
                .text()
                .contains("Niri rejected")
        );
        panel.cancel();
    }
    let shortcut = &local::SHORTCUTS[7];
    assert_eq!(shortcut.id, "launcher.close");
    let original = local::value(shortcut);
    search.set_text("");
    panel.edit(Target::Local(shortcut));
    entry.set_text("Ctrl+q");
    crate::ui_tests::pump(200);
    crate::ui_tests::capture("keybindings-editor");
    panel.save();
    wait(&panel);
    assert_eq!(
        remap(
            "launcher",
            gtk::gdk::Key::q,
            gtk::gdk::ModifierType::CONTROL_MASK
        ),
        gtk::gdk::Key::Escape
    );
    assert_eq!(
        remap(
            "launcher",
            gtk::gdk::Key::Escape,
            gtk::gdk::ModifierType::empty()
        ),
        gtk::gdk::Key::VoidSymbol
    );
    assert_eq!(
        crate::config::read().unwrap().keybindings["launcher.close"],
        "Ctrl+q"
    );
    local::save(shortcut.id, &original).unwrap();
    window.close();
    std::fs::remove_dir_all(dir).unwrap();
}
