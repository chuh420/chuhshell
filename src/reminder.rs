use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

const MAX_REMINDERS: usize = 100;
const MAX_TEXT: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Reminder {
    id: u64,
    text: String,
    due: i64,
}

enum Command {
    Load,
    Add { text: String, due: i64 },
    Delete(u64),
    Deliver,
}

struct Update {
    reminders: Vec<Reminder>,
    delivered: Vec<Reminder>,
}

fn path() -> PathBuf {
    crate::paths::data().join("chuhshell/reminders.json")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn load(path: &Path) -> Result<Vec<Reminder>, String> {
    let bytes = match crate::storage::read_limited(path, 256 * 1024) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Read reminders: {error}")),
    };
    let mut reminders: Vec<Reminder> =
        serde_json::from_slice(&bytes).map_err(|error| format!("Read reminders: {error}"))?;
    let mut ids = std::collections::HashSet::new();
    if reminders.len() > MAX_REMINDERS
        || reminders.iter().any(|reminder| {
            reminder.text.trim().is_empty()
                || reminder.text.chars().count() > MAX_TEXT
                || !(1..253_402_300_800).contains(&reminder.due)
                || reminder.id == 0
                || !ids.insert(reminder.id)
        })
    {
        return Err(
            "Invalid reminders or reminder limit exceeded (100 reminders, 300 characters each)"
                .into(),
        );
    }
    reminders.sort_by_key(|reminder| (reminder.due, reminder.id));
    Ok(reminders)
}

fn update(path: &Path, command: Command, timestamp: i64) -> Result<Update, String> {
    let mut reminders = load(path)?;
    let mut delivered = Vec::new();
    match command {
        Command::Load => {
            return Ok(Update {
                reminders,
                delivered,
            });
        }
        Command::Add { text, due } => {
            let text = text.trim();
            if text.is_empty() {
                return Err("Enter reminder text".into());
            }
            if text.chars().count() > MAX_TEXT {
                return Err("Reminder text may contain at most 300 characters".into());
            }
            if due <= timestamp || due >= 253_402_300_800 {
                return Err("Choose a future date and time".into());
            }
            if reminders.len() >= MAX_REMINDERS {
                return Err("Reminder limit reached (100)".into());
            }
            let id = reminders
                .iter()
                .map(|reminder| reminder.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("Reminder identifiers exhausted")?;
            reminders.push(Reminder {
                id,
                text: text.to_owned(),
                due,
            });
            reminders.sort_by_key(|reminder| (reminder.due, reminder.id));
        }
        Command::Delete(id) => {
            let index = reminders
                .iter()
                .position(|reminder| reminder.id == id)
                .ok_or("Reminder no longer exists")?;
            reminders.remove(index);
        }
        Command::Deliver => {
            let count = reminders
                .iter()
                .take_while(|reminder| reminder.due <= timestamp)
                .count()
                .min(4);
            if count == 0 {
                return Ok(Update {
                    reminders,
                    delivered,
                });
            }
            delivered.extend(reminders.drain(..count));
        }
    }
    let bytes = serde_json::to_vec(&reminders).map_err(|error| error.to_string())?;
    crate::storage::atomic_write(path, &bytes)
        .map_err(|error| format!("Save reminders: {error}"))?;
    Ok(Update {
        reminders,
        delivered,
    })
}

pub struct Service {
    path: PathBuf,
    center: Weak<crate::notification_center::NotificationCenter>,
    reminders: RefCell<Vec<Reminder>>,
    views: RefCell<Vec<Weak<ReminderView>>>,
    error: RefCell<Option<String>>,
    ready: Cell<bool>,
    busy: Cell<bool>,
    retry_after: Cell<i64>,
}

impl Service {
    fn new(path: PathBuf, center: &Rc<crate::notification_center::NotificationCenter>) -> Rc<Self> {
        let service = Rc::new(Self {
            path,
            center: Rc::downgrade(center),
            reminders: RefCell::new(Vec::new()),
            views: RefCell::new(Vec::new()),
            error: RefCell::new(None),
            ready: Cell::new(false),
            busy: Cell::new(false),
            retry_after: Cell::new(0),
        });
        service.dispatch(Command::Load, None);
        let weak = Rc::downgrade(&service);
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            let Some(service) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            service.tick();
            glib::ControlFlow::Continue
        });
        service
    }

    fn tick(self: &Rc<Self>) {
        let timestamp = now();
        if !self.ready.get() && !self.busy.get() && timestamp >= self.retry_after.get() {
            self.dispatch(Command::Load, None);
            return;
        }
        if self.ready.get()
            && !self.busy.get()
            && timestamp >= self.retry_after.get()
            && self
                .reminders
                .borrow()
                .first()
                .is_some_and(|reminder| reminder.due <= timestamp)
        {
            self.dispatch(Command::Deliver, None);
        }
    }

    fn dispatch(self: &Rc<Self>, command: Command, completed: Option<Box<dyn FnOnce()>>) {
        if self.busy.get() || (!self.ready.get() && !matches!(command, Command::Load)) {
            return;
        }
        let center = self.center.upgrade();
        if matches!(command, Command::Deliver) && center.is_none() {
            return;
        }
        self.busy.set(true);
        self.refresh();
        let service = self.clone();
        let path = self.path.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = crate::storage::run(move || update(&path, command, now())).await;
            service.busy.set(false);
            let completed = match result {
                Ok(update) => {
                    *service.reminders.borrow_mut() = update.reminders;
                    service.ready.set(true);
                    service.error.borrow_mut().take();
                    service.retry_after.set(0);
                    if let Some(center) = center {
                        for reminder in update.delivered {
                            center.reminder(&reminder.text);
                        }
                    }
                    completed
                }
                Err(error) => {
                    if service.error.borrow().as_ref() != Some(&error) {
                        eprintln!("chuhshell: reminder service: {error}");
                    }
                    *service.error.borrow_mut() = Some(error);
                    service.retry_after.set(now().saturating_add(5));
                    None
                }
            };
            service.refresh();
            if let Some(completed) = completed {
                completed();
            }
        });
    }

    fn refresh(self: &Rc<Self>) {
        self.views.borrow_mut().retain(|weak| {
            let Some(view) = weak.upgrade().filter(|view| view.root.upgrade().is_some()) else {
                return false;
            };
            view.render(self);
            true
        });
    }
}

pub fn start(
    state: &Rc<crate::app::AppState>,
    center: &Rc<crate::notification_center::NotificationCenter>,
) {
    if state.reminders.borrow().is_none() {
        *state.reminders.borrow_mut() = Some(Service::new(path(), center));
    }
}

struct ReminderView {
    root: glib::WeakRef<gtk::Box>,
    form: glib::WeakRef<gtk::Box>,
    list: glib::WeakRef<gtk::Box>,
    count: glib::WeakRef<gtk::Label>,
    message: glib::WeakRef<gtk::Label>,
    service: Weak<Service>,
}

impl ReminderView {
    fn render(self: &Rc<Self>, service: &Rc<Service>) {
        let (Some(form), Some(list), Some(count), Some(message)) = (
            self.form.upgrade(),
            self.list.upgrade(),
            self.count.upgrade(),
            self.message.upgrade(),
        ) else {
            return;
        };
        let sensitive = service.ready.get() && !service.busy.get();
        form.set_sensitive(sensitive);
        list.set_sensitive(sensitive);
        if let Some(error) = service.error.borrow().as_ref() {
            message.set_text(error);
            message.set_visible(true);
        } else {
            message.set_visible(false);
        }
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let reminders = service.reminders.borrow();
        count.set_text(&if service.ready.get() {
            format!("Upcoming · {}", reminders.len())
        } else {
            "Upcoming".into()
        });
        if reminders.is_empty() {
            let empty = gtk::Label::new(Some(if service.ready.get() {
                "No reminders scheduled"
            } else if service.error.borrow().is_some() {
                "Reminders unavailable"
            } else {
                "Loading reminders…"
            }));
            empty.add_css_class("todo-empty");
            empty.add_css_class("menu-hint");
            list.append(&empty);
        }
        for reminder in reminders.iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("reminder-row");
            let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
            text.set_hexpand(true);
            let label = gtk::Label::new(Some(&reminder.text));
            label.add_css_class("menu-title");
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_max_width_chars(36);
            text.append(&label);
            let timestamp = glib::DateTime::from_unix_local(reminder.due)
                .and_then(|date| date.format("%Y-%m-%d · %H:%M"))
                .map(|text| text.to_string())
                .unwrap_or_else(|_| "Time unavailable".into());
            let when = gtk::Label::new(Some(&timestamp));
            when.add_css_class("menu-hint");
            when.set_xalign(0.0);
            text.append(&when);
            row.append(&text);
            let delete = gtk::Button::with_label("×");
            delete.add_css_class("todo-delete");
            delete.set_valign(gtk::Align::Center);
            delete.set_tooltip_text(Some("Delete reminder"));
            let view = self.clone();
            let id = reminder.id;
            delete.connect_clicked(move |_| {
                if let Some(service) = view.service.upgrade() {
                    service.dispatch(Command::Delete(id), None);
                }
            });
            row.append(&delete);
            list.append(&row);
        }
    }
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label
}

pub fn view(state: &Rc<crate::app::AppState>) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("reminder");
    let Some(service) = state.reminders.borrow().as_ref().cloned() else {
        root.append(&label("Reminder service unavailable", "menu-error"));
        return root;
    };
    let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
    form.append(&label("New reminder", "menu-title"));
    let entry = gtk::Entry::new();
    entry.add_css_class("todo-entry");
    entry.add_css_class("reminder-text");
    entry.set_placeholder_text(Some("Reminder text…"));
    entry.set_max_length(MAX_TEXT as i32);
    form.append(&entry);
    let schedule = gtk::Box::new(gtk::Orientation::Vertical, 6);
    schedule.add_css_class("reminder-schedule");
    schedule.append(&label("Date and time", "menu-hint"));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let initial = glib::DateTime::from_unix_local(now().saturating_add(300)).expect("current date");
    let calendar = gtk::Calendar::new();
    calendar.add_css_class("reminder-calendar");
    calendar.select_day(&initial);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&calendar));
    let date = gtk::MenuButton::new();
    date.add_css_class("reminder-date");
    date.set_hexpand(true);
    date.set_label(&initial.format("%Y-%m-%d").unwrap());
    date.set_popover(Some(&popover));
    let weak_date = date.downgrade();
    let weak_popover = popover.downgrade();
    calendar.connect_day_selected(move |calendar| {
        if let Some(date) = weak_date.upgrade() {
            date.set_label(&calendar.date().format("%Y-%m-%d").unwrap());
        }
        if let Some(popover) = weak_popover.upgrade() {
            popover.popdown();
        }
    });
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    for (spin, value, title) in [
        (&hour, initial.hour(), "Hour"),
        (&minute, initial.minute(), "Minute"),
    ] {
        spin.set_numeric(true);
        spin.set_width_chars(2);
        spin.set_wrap(true);
        spin.set_value(value.into());
        spin.set_tooltip_text(Some(title));
        spin.set_text(&format!("{value:02}"));
        spin.connect_output(|spin| {
            let text = format!("{:02}", spin.value_as_int());
            if spin.text().as_str() != text {
                spin.set_text(&text);
            }
            glib::Propagation::Stop
        });
    }
    hour.add_css_class("reminder-hour");
    minute.add_css_class("reminder-minute");
    controls.append(&date);
    controls.append(&hour);
    controls.append(&gtk::Label::new(Some(":")));
    controls.append(&minute);
    schedule.append(&controls);
    form.append(&schedule);
    let add = gtk::Button::with_label("Add reminder");
    add.add_css_class("network-action");
    add.add_css_class("primary");
    add.add_css_class("reminder-add");
    add.set_halign(gtk::Align::End);
    form.append(&add);
    root.append(&form);
    let count = label("Upcoming · 0", "menu-hint");
    root.append(&count);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.add_css_class("reminder-list");
    let max_height = crate::ui::active_monitor().map_or(240, |monitor| {
        (monitor.geometry().height() - 400).clamp(80, 280)
    });
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(0)
        .max_content_height(max_height)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    root.append(&scroll);
    let message = label("", "menu-error");
    message.set_wrap(true);
    message.set_visible(false);
    root.append(&message);
    let view = Rc::new(ReminderView {
        root: root.downgrade(),
        form: form.downgrade(),
        list: list.downgrade(),
        count: count.downgrade(),
        message: message.downgrade(),
        service: Rc::downgrade(&service),
    });
    service
        .views
        .borrow_mut()
        .retain(|view| view.strong_count() > 0);
    service.views.borrow_mut().push(Rc::downgrade(&view));
    let activate: Rc<dyn Fn()> = Rc::new({
        let view = view.clone();
        let entry = entry.downgrade();
        move || {
            let (Some(service), Some(entry), Some(message)) = (
                view.service.upgrade(),
                entry.upgrade(),
                view.message.upgrade(),
            ) else {
                return;
            };
            let selected = calendar.date();
            let due = glib::DateTime::from_local(
                selected.year(),
                selected.month(),
                selected.day_of_month(),
                hour.value_as_int(),
                minute.value_as_int(),
                0.0,
            );
            match due {
                Ok(date) => {
                    let weak_entry = entry.downgrade();
                    service.dispatch(
                        Command::Add {
                            text: entry.text().to_string(),
                            due: date.to_unix(),
                        },
                        Some(Box::new(move || {
                            if let Some(entry) = weak_entry.upgrade() {
                                entry.set_text("");
                                entry.grab_focus();
                            }
                        })),
                    );
                }
                Err(_) => {
                    message.set_text("Selected local time is unavailable");
                    message.set_visible(true);
                }
            }
        }
    });
    let entry_activate = activate.clone();
    entry.connect_activate(move |_| entry_activate());
    add.connect_clicked(move |_| activate());
    view.render(&service);
    root
}

#[cfg(test)]
pub fn regression_checks(window: &gtk::Window, state: &Rc<crate::app::AppState>) {
    fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
        if widget.has_css_class(class) {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = find(&widget, class) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }
    fn settled(service: &Service) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while service.busy.get() {
            assert!(
                std::time::Instant::now() < deadline,
                "Reminder service did not finish"
            );
            crate::ui_tests::pump(10);
        }
    }
    let service = state.reminders.borrow().as_ref().unwrap().clone();
    settled(&service);
    assert!(service.ready.get());
    let root = window.upcast_ref();
    let entry = find(root, "reminder-text")
        .unwrap()
        .downcast::<gtk::Entry>()
        .unwrap();
    let add = find(root, "reminder-add")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let date = find(root, "reminder-date")
        .unwrap()
        .downcast::<gtk::MenuButton>()
        .unwrap();
    let calendar = date
        .popover()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::Calendar>()
        .unwrap();
    let tomorrow = glib::DateTime::now_local().unwrap().add_days(1).unwrap();
    calendar.select_day(&tomorrow);
    assert_eq!(date.label().unwrap(), tomorrow.format("%Y-%m-%d").unwrap());
    entry.set_text("Take a break and stretch");
    add.emit_clicked();
    settled(&service);
    assert_eq!(service.reminders.borrow().len(), 1);
    assert_eq!(load(&service.path).unwrap().len(), 1);
    assert!(entry.text().is_empty());
    assert!(
        gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| focus
            == *entry.upcast_ref::<gtk::Widget>()
            || focus.is_ancestor(&entry))
    );
    crate::ui_tests::capture("reminder");
    date.popup();
    crate::ui_tests::capture("reminder-date");
    date.popdown();
    find(root, "todo-delete")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    settled(&service);
    assert!(load(&service.path).unwrap().is_empty());
    let yesterday = glib::DateTime::now_local().unwrap().add_days(-1).unwrap();
    calendar.select_day(&yesterday);
    entry.set_text("Past reminder");
    add.emit_clicked();
    settled(&service);
    assert!(service.error.borrow().as_ref().unwrap().contains("future"));
    assert_eq!(entry.text().as_str(), "Past reminder");
    assert!(load(&service.path).unwrap().is_empty());
    let center = service.center.upgrade().unwrap();
    service.dispatch(
        Command::Add {
            text: "Due reminder".into(),
            due: now() + 2,
        },
        None,
    );
    settled(&service);
    assert_eq!(service.reminders.borrow().len(), 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !service.reminders.borrow().is_empty() || service.busy.get() {
        assert!(
            std::time::Instant::now() < deadline,
            "Reminder was not delivered"
        );
        crate::ui_tests::pump(20);
    }
    assert!(load(&service.path).unwrap().is_empty());
    assert_eq!(center.reminder_count("Due reminder"), 1);
    let list = find(root, "reminder-list").unwrap();
    assert_eq!(
        list.first_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap()
            .text()
            .as_str(),
        "No reminders scheduled"
    );
    let restarted = Service::new(service.path.clone(), &center);
    settled(&restarted);
    restarted.tick();
    assert!(restarted.reminders.borrow().is_empty());
    assert_eq!(center.reminder_count("Due reminder"), 1);
    service.dispatch(
        Command::Add {
            text: "Keep on failure".into(),
            due: now() + 60,
        },
        None,
    );
    settled(&service);
    let saved = std::fs::read(&service.path).unwrap();
    std::fs::write(&service.path, b"invalid").unwrap();
    service.dispatch(Command::Deliver, None);
    settled(&service);
    assert_eq!(service.reminders.borrow().len(), 1);
    assert!(service.error.borrow().is_some());
    assert_eq!(std::fs::read(&service.path).unwrap(), b"invalid");
    std::fs::write(&service.path, saved).unwrap();
    let id = service.reminders.borrow()[0].id;
    service.dispatch(Command::Delete(id), None);
    settled(&service);
    assert!(load(&service.path).unwrap().is_empty());
    assert_eq!(center.reminder_count("Keep on failure"), 0);
    center.remove_test_reminder("Due reminder");
    assert_eq!(center.reminder_count("Due reminder"), 0);
    crate::ui_tests::pump(250);
    let temporary = view(state);
    let weak = temporary.downgrade();
    drop(temporary);
    crate::ui_tests::pump(80);
    assert!(weak.upgrade().is_none());
}

#[cfg(test)]
mod tests {
    use super::*;

    struct File {
        directory: PathBuf,
        path: PathBuf,
    }

    impl File {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "chuhshell-reminders-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            Self {
                path: directory.join("reminders.json"),
                directory,
            }
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn delivery_removes_due_items_once_and_preserves_future_items() {
        let file = File::new();
        for (text, due) in [("later", 300), ("first", 200), ("same time", 200)] {
            update(
                &file.path,
                Command::Add {
                    text: text.into(),
                    due,
                },
                100,
            )
            .unwrap();
        }
        assert!(
            update(&file.path, Command::Deliver, 199)
                .unwrap()
                .delivered
                .is_empty()
        );
        let result = update(&file.path, Command::Deliver, 200).unwrap();
        assert_eq!(
            result
                .delivered
                .iter()
                .map(|reminder| reminder.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "same time"]
        );
        assert_eq!(load(&file.path).unwrap(), result.reminders);
        assert_eq!(result.reminders[0].text, "later");
        assert!(
            update(&file.path, Command::Deliver, 200)
                .unwrap()
                .delivered
                .is_empty()
        );
        assert_eq!(
            update(&file.path, Command::Deliver, 1000)
                .unwrap()
                .delivered[0]
                .text,
            "later"
        );
        assert!(load(&file.path).unwrap().is_empty());
    }

    #[test]
    fn overdue_reminders_are_delivered_in_bounded_batches_and_can_be_cancelled() {
        let file = File::new();
        for index in 0..10 {
            update(
                &file.path,
                Command::Add {
                    text: index.to_string(),
                    due: 200,
                },
                100,
            )
            .unwrap();
        }
        let pending = load(&file.path).unwrap();
        update(&file.path, Command::Delete(pending[0].id), 201).unwrap();
        let result = update(&file.path, Command::Deliver, 500).unwrap();
        assert_eq!(result.delivered.len(), 4);
        assert_eq!(result.reminders.len(), 5);
        assert!(result.delivered.iter().all(|reminder| reminder.text != "0"));
        assert!(update(&file.path, Command::Delete(pending[0].id), 500).is_err());
        assert_eq!(load(&file.path).unwrap().len(), 5);
    }

    #[test]
    fn invalid_user_data_and_oversized_files_are_preserved() {
        let file = File::new();
        let reminder = Reminder {
            id: 1,
            text: "one".into(),
            due: 200,
        };
        let invalid = [
            b"invalid".to_vec(),
            serde_json::to_vec(&vec![reminder.clone(), reminder.clone()]).unwrap(),
            serde_json::to_vec(&vec![Reminder {
                id: 0,
                ..reminder.clone()
            }])
            .unwrap(),
            serde_json::to_vec(&vec![Reminder {
                due: 0,
                ..reminder.clone()
            }])
            .unwrap(),
            serde_json::to_vec(&vec![Reminder {
                text: "x".repeat(MAX_TEXT + 1),
                ..reminder
            }])
            .unwrap(),
            vec![b' '; 256 * 1024 + 1],
        ];
        for bytes in invalid {
            std::fs::write(&file.path, &bytes).unwrap();
            assert!(
                update(
                    &file.path,
                    Command::Add {
                        text: "two".into(),
                        due: 300
                    },
                    100
                )
                .is_err()
            );
            assert!(update(&file.path, Command::Deliver, 500).is_err());
            assert_eq!(std::fs::read(&file.path).unwrap(), bytes);
        }
    }

    #[test]
    fn input_and_queue_limits_are_enforced_without_changing_saved_reminders() {
        let file = File::new();
        update(
            &file.path,
            Command::Add {
                text: "  reminder  ".into(),
                due: 200,
            },
            100,
        )
        .unwrap();
        let saved = std::fs::read(&file.path).unwrap();
        assert_eq!(load(&file.path).unwrap()[0].text, "reminder");
        for (text, due) in [
            (" ".to_owned(), 300),
            ("valid".into(), 100),
            ("x".repeat(MAX_TEXT + 1), 300),
        ] {
            assert!(update(&file.path, Command::Add { text, due }, 100).is_err());
            assert_eq!(std::fs::read(&file.path).unwrap(), saved);
        }
        let reminders: Vec<_> = (1..=MAX_REMINDERS)
            .map(|id| Reminder {
                id: id as u64,
                text: "valid".into(),
                due: 200,
            })
            .collect();
        std::fs::write(&file.path, serde_json::to_vec(&reminders).unwrap()).unwrap();
        assert!(
            update(
                &file.path,
                Command::Add {
                    text: "overflow".into(),
                    due: 300
                },
                100
            )
            .is_err()
        );
        assert_eq!(load(&file.path).unwrap().len(), MAX_REMINDERS);
    }
}
