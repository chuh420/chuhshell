use gtk::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Task {
    pub(crate) text: String,
    pub(crate) done: bool,
}

enum Command {
    Load,
    Add(String),
    Toggle(usize),
    Delete(usize),
}

struct Request {
    command: Command,
    reply: async_channel::Sender<Result<Vec<Task>, String>>,
}

pub(crate) fn path() -> PathBuf {
    crate::paths::data().join("chuhshell/todo.json")
}

pub(crate) fn load(path: &Path) -> Result<Vec<Task>, String> {
    match crate::storage::read_limited(path, 512 * 1024) {
        Ok(bytes) => {
            let tasks: Vec<Task> =
                serde_json::from_slice(&bytes).map_err(|error| format!("Read tasks: {error}"))?;
            if tasks.len() > 500
                || tasks
                    .iter()
                    .any(|task| task.text.chars().count() > 200 || task.text.trim().is_empty())
            {
                return Err("Tasks exceed the limit (500 tasks, 200 characters per task)".into());
            }
            Ok(tasks)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("Read tasks: {error}")),
    }
}

fn update(path: &Path, command: Command) -> Result<Vec<Task>, String> {
    let mut tasks = load(path)?;
    match command {
        Command::Load => return Ok(tasks),
        Command::Add(text) => {
            let text = text.trim();
            if text.is_empty() {
                return Ok(tasks);
            }
            if text.chars().count() > 200 {
                return Err("Tasks may contain at most 200 characters".into());
            }
            if tasks.len() >= 500 {
                return Err("Task limit reached (500)".into());
            }
            tasks.push(Task {
                text: text.to_owned(),
                done: false,
            });
        }
        Command::Toggle(index) => {
            let task = tasks.get_mut(index).ok_or("Task no longer exists")?;
            task.done = !task.done;
        }
        Command::Delete(index) => {
            if index >= tasks.len() {
                return Err("Task no longer exists".into());
            }
            tasks.remove(index);
        }
    }
    let bytes = serde_json::to_vec(&tasks).map_err(|error| error.to_string())?;
    crate::storage::atomic_write(path, &bytes).map_err(|error| format!("Save tasks: {error}"))?;
    Ok(tasks)
}

fn sender() -> &'static async_channel::Sender<Request> {
    static SENDER: OnceLock<async_channel::Sender<Request>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = async_channel::bounded::<Request>(8);
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv_blocking() {
                let result = update(&path(), request.command);
                let _ = request.reply.send_blocking(result);
            }
        });
        sender
    })
}

struct TodoView {
    root: glib::WeakRef<gtk::Box>,
    entry: glib::WeakRef<gtk::Entry>,
    list: glib::WeakRef<gtk::Box>,
    count: glib::WeakRef<gtk::Label>,
    message: glib::WeakRef<gtk::Label>,
}

impl TodoView {
    fn dispatch(self: &Rc<Self>, command: Command) {
        let Some(root) = self.root.upgrade().filter(|root| root.is_sensitive()) else {
            return;
        };
        root.set_sensitive(false);
        let view = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let (reply, receiver) = async_channel::bounded(1);
            let result = match sender().send(Request { command, reply }).await {
                Ok(()) => receiver
                    .recv()
                    .await
                    .unwrap_or_else(|_| Err("Task service stopped".into())),
                Err(_) => Err("Task service stopped".into()),
            };
            let (Some(root), Some(message)) = (view.root.upgrade(), view.message.upgrade()) else {
                return;
            };
            root.set_sensitive(true);
            match result {
                Ok(tasks) => {
                    message.set_visible(false);
                    if let Some(entry) = view.entry.upgrade() {
                        entry.set_text("");
                        entry.grab_focus();
                    }
                    view.render(tasks);
                }
                Err(error) => {
                    message.set_text(&error);
                    message.set_visible(true);
                }
            }
        });
    }

    fn render(self: &Rc<Self>, tasks: Vec<Task>) {
        let (Some(list), Some(count)) = (self.list.upgrade(), self.count.upgrade()) else {
            return;
        };
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let remaining = tasks.iter().filter(|task| !task.done).count();
        count.set_text(&format!("{remaining} remaining"));
        if tasks.is_empty() {
            let empty = gtk::Label::new(Some("No tasks yet"));
            empty.add_css_class("menu-hint");
            empty.add_css_class("todo-empty");
            list.append(&empty);
        }
        for (index, task) in tasks.into_iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("todo-row");
            let check = gtk::CheckButton::new();
            check.add_css_class("todo-check");
            check.set_active(task.done);
            check.set_tooltip_text(Some(if task.done {
                "Mark as not done"
            } else {
                "Mark as done"
            }));
            let view = self.clone();
            check.connect_toggled(move |_| {
                view.dispatch(Command::Toggle(index));
            });
            row.append(&check);
            let text = gtk::Label::new(Some(&task.text));
            text.add_css_class("menu-title");
            if task.done {
                text.add_css_class("todo-done");
            }
            text.set_hexpand(true);
            text.set_xalign(0.0);
            text.set_wrap(true);
            row.append(&text);
            let delete = gtk::Button::with_label("×");
            delete.add_css_class("todo-delete");
            delete.set_tooltip_text(Some("Delete task"));
            let view = self.clone();
            delete.connect_clicked(move |_| {
                view.dispatch(Command::Delete(index));
            });
            row.append(&delete);
            list.append(&row);
        }
    }
}

pub fn view() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("todo");
    let form = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.add_css_class("todo-entry");
    entry.set_placeholder_text(Some("Add a task…"));
    entry.set_max_length(200);
    entry.set_hexpand(true);
    let add = gtk::Button::with_label("Add");
    add.add_css_class("network-action");
    add.add_css_class("todo-add");
    form.append(&entry);
    form.append(&add);
    root.append(&form);
    let count = gtk::Label::new(None);
    count.add_css_class("menu-hint");
    count.set_xalign(0.0);
    root.append(&count);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(0)
        .max_content_height(360)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    scroll.set_child(Some(&list));
    root.append(&scroll);
    let message = gtk::Label::new(None);
    message.add_css_class("menu-error");
    message.set_wrap(true);
    message.set_xalign(0.0);
    message.set_visible(false);
    root.append(&message);
    let view = Rc::new(TodoView {
        root: root.downgrade(),
        entry: entry.downgrade(),
        list: list.downgrade(),
        count: count.downgrade(),
        message: message.downgrade(),
    });
    let activate_view = view.clone();
    entry.connect_activate(move |entry| {
        activate_view.dispatch(Command::Add(entry.text().to_string()));
    });
    let add_view = view.clone();
    add.connect_clicked(move |_| {
        if let Some(entry) = add_view.entry.upgrade() {
            add_view.dispatch(Command::Add(entry.text().to_string()));
        }
    });
    view.dispatch(Command::Load);
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_files_are_bounded_before_parsing() {
        let path =
            std::env::temp_dir().join(format!("chuhshell-todo-limits-{}", std::process::id()));
        let task = Task {
            text: "x".repeat(201),
            done: false,
        };
        std::fs::write(&path, serde_json::to_vec(&vec![task]).unwrap()).unwrap();
        assert!(load(&path).is_err());
        let tasks = vec![
            Task {
                text: "one".into(),
                done: false
            };
            501
        ];
        std::fs::write(&path, serde_json::to_vec(&tasks).unwrap()).unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, vec![b' '; 512 * 1024 + 1]).unwrap();
        assert!(load(&path).unwrap_err().contains("size limit"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn tasks_survive_updates_and_reject_bad_data() {
        let directory = std::env::temp_dir().join(format!("chuhshell-todo-{}", std::process::id()));
        let file = directory.join("todo.json");
        assert!(load(&file).unwrap().is_empty());
        let tasks = update(&file, Command::Add("  one  ".into())).unwrap();
        assert_eq!(tasks[0].text, "one");
        assert!(update(&file, Command::Toggle(0)).unwrap()[0].done);
        assert!(load(&file).unwrap()[0].done);
        assert!(update(&file, Command::Delete(0)).unwrap().is_empty());
        std::fs::write(&file, b"invalid").unwrap();
        assert!(update(&file, Command::Add("two".into())).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"invalid");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
