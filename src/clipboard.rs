use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const ITEM_LIMIT: usize = 2 * 1024 * 1024;
const TOTAL_LIMIT: usize = 20 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct Entry {
    mime: &'static str,
    data: Vec<u8>,
}

#[derive(Default)]
pub struct History {
    entries: RefCell<Vec<Entry>>,
    started: Cell<bool>,
    paused: Arc<AtomicBool>,
    revision: Cell<u64>,
    status: RefCell<String>,
}

pub fn capture() {
    let state = std::env::var("CLIPBOARD_STATE").unwrap_or_default();
    if matches!(state.as_str(), "sensitive" | "clear" | "nil") {
        return;
    }
    let mut data = Vec::new();
    if std::io::stdin()
        .take(ITEM_LIMIT as u64 + 1)
        .read_to_end(&mut data)
        .is_err()
        || data.is_empty()
        || data.len() > ITEM_LIMIT
    {
        return;
    }
    let mut output = std::io::stdout().lock();
    let _ = output
        .write_all(&(data.len() as u32).to_le_bytes())
        .and_then(|_| output.write_all(&data));
}

fn insert(entries: &mut Vec<Entry>, entry: Entry) {
    entries.retain(|old| old != &entry);
    entries.insert(0, entry);
    entries.truncate(100);
    let mut total = 0;
    entries.retain(|entry| {
        total += entry.data.len();
        total <= TOTAL_LIMIT
    });
}

impl History {
    pub fn start(self: &Rc<Self>, state: &Rc<crate::app::AppState>) {
        if self.started.replace(true) {
            return;
        }
        let (tx, rx) = async_channel::bounded(8);
        for mime in ["text", "image/png"] {
            let tx = tx.clone();
            let paused = self.paused.clone();
            std::thread::spawn(move || {
                while !crate::process::stopped() {
                    let result = (|| -> Result<(), String> {
                        let binary = std::env::current_exe().map_err(|e| e.to_string())?;
                        let mut command = Command::new("wl-paste");
                        command
                            .args(["--no-newline", "--type", mime, "--watch"])
                            .arg(binary)
                            .arg("clipboard-capture")
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null());
                        let mut child = crate::process::ManagedChild::spawn(&mut command)?;
                        let mut output = child
                            .0
                            .stdout
                            .take()
                            .ok_or("Clipboard watcher has no output")?;
                        loop {
                            let mut length = [0; 4];
                            output
                                .read_exact(&mut length)
                                .map_err(|_| "Clipboard watcher disconnected".to_string())?;
                            let length = u32::from_le_bytes(length) as usize;
                            if length > ITEM_LIMIT {
                                return Err("Clipboard entry too large".into());
                            }
                            let mut data = vec![0; length];
                            output.read_exact(&mut data).map_err(|e| e.to_string())?;
                            if mime != "text"
                                || (!data.contains(&0) && std::str::from_utf8(&data).is_ok())
                            {
                                let record = !paused.load(Ordering::Relaxed);
                                let _ = tx.try_send(Ok((Entry { mime, data }, record)));
                            }
                        }
                    })();
                    if let Err(error) = result {
                        let _ = tx.try_send(Err(error));
                    }
                    if !crate::process::pause(std::time::Duration::from_secs(5)) {
                        break;
                    }
                }
            });
        }
        let weak = Rc::downgrade(self);
        let state = Rc::downgrade(state);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = rx.recv().await {
                let Some(history) = weak.upgrade() else {
                    break;
                };
                match result {
                    Ok((entry, record)) => {
                        if let Some(state) = state.upgrade() {
                            history.received(entry, record, &state);
                        }
                    }
                    Err(error) => *history.status.borrow_mut() = error,
                }
                history.revision.set(history.revision.get().wrapping_add(1));
            }
        });
    }

    fn received(&self, entry: Entry, record: bool, state: &Rc<crate::app::AppState>) {
        let title = if entry.mime == "image/png" {
            "Image copied"
        } else {
            "Copied to clipboard"
        };
        if record && !self.paused.load(Ordering::Relaxed) {
            insert(&mut self.entries.borrow_mut(), entry);
        }
        self.status.borrow_mut().clear();
        crate::notifications::show(
            state,
            crate::notifications::Notice::transient(
                crate::notifications::NoticeKind::Clipboard,
                title,
            ),
        );
    }

    pub fn view(self: &Rc<Self>, on_copy: impl Fn() + 'static) -> gtk::Box {
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search clipboard"));
        search.add_css_class("search");
        outer.append(&search);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let pause = crate::info::button(if self.paused.load(Ordering::Relaxed) {
            "Resume"
        } else {
            "Pause"
        });
        let clear = crate::info::button("Clear history");
        controls.append(&pause);
        controls.append(&clear);
        outer.append(&controls);
        let status = crate::info::label("", "menu-hint");
        outer.append(&status);
        let list = gtk::ListBox::new();
        list.add_css_class("menu-list");
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(260)
            .max_content_height(360)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        outer.append(&scroll);
        let visible = Rc::new(RefCell::new(Vec::<Entry>::new()));
        let render: Rc<dyn Fn()> = Rc::new({
            let history = Rc::downgrade(self);
            let list = list.downgrade();
            let search = search.downgrade();
            let status = status.downgrade();
            let visible = visible.clone();
            move || {
                let (Some(history), Some(list), Some(search), Some(status)) = (
                    history.upgrade(),
                    list.upgrade(),
                    search.upgrade(),
                    status.upgrade(),
                ) else {
                    return;
                };
                let query = search.text().to_lowercase();
                let entries: Vec<_> = history
                    .entries
                    .borrow()
                    .iter()
                    .filter(|entry| {
                        query.is_empty()
                            || (entry.mime == "text"
                                && String::from_utf8_lossy(&entry.data)
                                    .to_lowercase()
                                    .contains(&query))
                    })
                    .cloned()
                    .collect();
                let selected = list
                    .selected_row()
                    .and_then(|row| visible.borrow().get(row.index() as usize).cloned());
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                status.set_text(&if history.status.borrow().is_empty() {
                    format!(
                        "{} items · session only · {} to copy · {} to remove",
                        history.entries.borrow().len(),
                        crate::keybindings::hint("clipboard.copy"),
                        crate::keybindings::hint("clipboard.delete")
                    )
                } else {
                    history.status.borrow().clone()
                });
                for entry in &entries {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    if entry.mime == "image/png" {
                        row.append(&crate::info::label("▧", "clipboard-image"));
                    }
                    let text = if entry.mime == "text" {
                        String::from_utf8_lossy(&entry.data)
                            .chars()
                            .take(180)
                            .collect()
                    } else {
                        format!("Image · {} KB", entry.data.len() / 1024)
                    };
                    let label = crate::info::label(&text, "clipboard-preview");
                    label.set_lines(3);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_hexpand(true);
                    row.append(&label);
                    list.append(&row);
                }
                let index = selected
                    .as_ref()
                    .and_then(|selected| entries.iter().position(|entry| entry == selected))
                    .unwrap_or(0);
                *visible.borrow_mut() = entries;
                list.select_row(list.row_at_index(index as i32).as_ref());
            }
        });
        search.connect_search_changed({
            let render = render.clone();
            move |_| render()
        });
        pause.connect_clicked({
            let history = self.clone();
            move |button| {
                let paused = !history.paused.load(Ordering::Relaxed);
                history.paused.store(paused, Ordering::Relaxed);
                button.set_label(if paused { "Resume" } else { "Pause" });
            }
        });
        clear.connect_clicked({
            let history = self.clone();
            let render = render.clone();
            move |_| {
                history.entries.borrow_mut().clear();
                history.revision.set(history.revision.get().wrapping_add(1));
                render();
            }
        });
        list.connect_row_activated({
            let visible = visible.clone();
            let status = status.downgrade();
            move |list, row| {
                if let Some(entry) = visible.borrow().get(row.index() as usize) {
                    let result = if entry.mime == "text" {
                        list.clipboard()
                            .set_text(&String::from_utf8_lossy(&entry.data));
                        Ok(())
                    } else {
                        list.clipboard()
                            .set_content(Some(&gtk::gdk::ContentProvider::for_bytes(
                                entry.mime,
                                &glib::Bytes::from(&entry.data),
                            )))
                    };
                    if let Some(status) = status.upgrade() {
                        status.set_text(if result.is_ok() {
                            "Copied to clipboard"
                        } else {
                            "Could not copy item"
                        });
                    }
                    if result.is_ok() {
                        on_copy();
                    }
                }
            }
        });
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        key.connect_key_pressed({
            let history = self.clone();
            let visible = visible.clone();
            let list = list.downgrade();
            let render = render.clone();
            move |_, key, _, modifiers| {
                let default_key = crate::keybindings::is_default("clipboard", key);
                let key = crate::keybindings::remap("clipboard", key, modifiers);
                let Some(list) = list.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if matches!(
                    key,
                    gtk::gdk::Key::Down
                        | gtk::gdk::Key::Up
                        | gtk::gdk::Key::Page_Down
                        | gtk::gdk::Key::Page_Up
                ) {
                    let offset = match key {
                        gtk::gdk::Key::Up => -1,
                        gtk::gdk::Key::Page_Up => -5,
                        gtk::gdk::Key::Page_Down => 5,
                        _ => 1,
                    };
                    let rows = crate::launcher::visible_rows(&list);
                    let selected = rows
                        .iter()
                        .position(|row| Some(row) == list.selected_row().as_ref());
                    if let Some(index) =
                        crate::launcher::selection_index(rows.len(), selected, offset)
                    {
                        list.select_row(rows.get(index));
                    }
                    glib::Propagation::Stop
                } else if key == gtk::gdk::Key::Right {
                    if let Some(row) = list.selected_row() {
                        list.emit_by_name::<()>("row-activated", &[&row]);
                    }
                    glib::Propagation::Stop
                } else if key == gtk::gdk::Key::Delete {
                    let entry = list
                        .selected_row()
                        .and_then(|row| visible.borrow().get(row.index() as usize).cloned());
                    if let Some(entry) = entry {
                        history.entries.borrow_mut().retain(|old| old != &entry);
                        render();
                    }
                    glib::Propagation::Stop
                } else if default_key {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        list.add_controller(key);
        let search_keys = gtk::EventControllerKey::new();
        search_keys.connect_key_pressed({
            let list = list.downgrade();
            move |_, key, _, modifiers| {
                let key = crate::keybindings::remap("clipboard-search", key, modifiers);
                if key == gtk::gdk::Key::Down
                    && let Some(list) = list.upgrade()
                {
                    list.grab_focus();
                    if list.selected_row().is_none() {
                        list.select_row(list.row_at_index(0).as_ref());
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        search.add_controller(search_keys);
        search.connect_activate({
            let list = list.downgrade();
            move |_| {
                if let Some(list) = list.upgrade()
                    && let Some(row) = list.selected_row()
                {
                    list.emit_by_name::<()>("row-activated", &[&row]);
                }
            }
        });
        let weak = outer.downgrade();
        let history = Rc::downgrade(self);
        let revision = Cell::new(self.revision.get());
        glib::timeout_add_local(std::time::Duration::from_millis(300), {
            let render = render.clone();
            move || {
                let (Some(_outer), Some(history)) = (weak.upgrade(), history.upgrade()) else {
                    return glib::ControlFlow::Break;
                };
                if revision.replace(history.revision.get()) != history.revision.get() {
                    render();
                }
                glib::ControlFlow::Continue
            }
        });
        render();
        outer
    }
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application) {
    let history = Rc::new(History::default());
    for text in ["first item", "  второй\nitem\n"] {
        insert(
            &mut history.entries.borrow_mut(),
            Entry {
                mime: "text",
                data: text.as_bytes().to_vec(),
            },
        );
    }
    let view = history.view(|| {});
    view.add_css_class("menu-content");
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .child(&view)
        .build();
    window.present();
    crate::ui_tests::pump(100);
    let search = view
        .first_child()
        .unwrap()
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    let scroll = view
        .last_child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let list = scroll
        .child()
        .unwrap()
        .downcast::<gtk::Viewport>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::ListBox>()
        .unwrap();
    assert_eq!(crate::launcher::visible_rows(&list).len(), 2);
    crate::ui_tests::capture("clipboard");
    search.set_text("второй");
    crate::ui_tests::pump(250);
    assert_eq!(crate::launcher::visible_rows(&list).len(), 1);
    list.emit_by_name::<()>("row-activated", &[&list.row_at_index(0).unwrap()]);
    let copied = Rc::new(RefCell::new(None));
    glib::MainContext::default().spawn_local({
        let copied = copied.clone();
        let clipboard = list.clipboard();
        async move {
            *copied.borrow_mut() = Some(
                clipboard
                    .read_text_future()
                    .await
                    .unwrap()
                    .unwrap()
                    .to_string(),
            );
        }
    });
    crate::ui_tests::pump(100);
    assert_eq!(copied.borrow().as_deref(), Some("  второй\nitem\n"));
    let controllers = list.observe_controllers();
    for i in 0..controllers.n_items() {
        if let Some(key) = controllers
            .item(i)
            .and_downcast::<gtk::EventControllerKey>()
        {
            key.emit_by_name::<bool>(
                "key-pressed",
                &[
                    &gtk::gdk::Key::Delete,
                    &0u32,
                    &gtk::gdk::ModifierType::empty(),
                ],
            );
        }
    }
    assert_eq!(history.entries.borrow().len(), 1);
    assert!(list.row_at_index(0).is_none());
    window.close();
    let state = Rc::new(crate::app::AppState::default());
    history.received(
        Entry {
            mime: "text",
            data: b"private contents".to_vec(),
        },
        true,
        &state,
    );
    let osd = state.osd.borrow().as_ref().unwrap().clone();
    let title = osd
        .child()
        .unwrap()
        .last_child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(title.text(), "Copied to clipboard");
    let count = history.entries.borrow().len();
    history.paused.store(true, Ordering::Relaxed);
    history.received(
        Entry {
            mime: "image/png",
            data: vec![1, 2, 3],
        },
        false,
        &state,
    );
    assert_eq!(history.entries.borrow().len(), count);
    assert_eq!(state.osd.borrow().as_ref(), Some(&osd));
    assert_eq!(title.text(), "Image copied");
    crate::ui_tests::pump(1400);
    assert!(state.osd.borrow().is_none());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_deduplicates_and_bounds_memory() {
        let mut entries = Vec::new();
        for i in 0..150 {
            insert(
                &mut entries,
                Entry {
                    mime: "text",
                    data: vec![i],
                },
            );
        }
        assert_eq!(entries.len(), 100);
        insert(
            &mut entries,
            Entry {
                mime: "text",
                data: vec![100],
            },
        );
        assert_eq!(entries.len(), 100);
        assert_eq!(entries[0].data, vec![100]);
        for i in 0..15 {
            insert(
                &mut entries,
                Entry {
                    mime: "image/png",
                    data: vec![i; ITEM_LIMIT],
                },
            );
        }
        assert!(entries.iter().map(|entry| entry.data.len()).sum::<usize>() <= TOTAL_LIMIT);
    }
}
