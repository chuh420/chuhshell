use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;

use crate::app::{AppState, LauncherMode};
use crate::apps::{self, AppEntry};
use crate::fuzzy;

use crate::ui::set_layer_window;

pub fn show(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode) {
    crate::menu::close(state);
    crate::ui::close_popover();
    let generation = state.launcher_generation.get().wrapping_add(1);
    state.launcher_generation.set(generation);
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking((apps::load_apps(), apps::read_launch_counts()));
    });
    let state = Rc::downgrade(state);
    let app = app.downgrade();
    glib::MainContext::default().spawn_local(async move {
        if let Ok((apps, counts)) = rx.recv().await
            && let (Some(app), Some(state)) = (app.upgrade(), state.upgrade())
            && state.launcher_generation.get() == generation
        {
            create(&app, &state, mode, apps, counts);
        }
    });
}

fn compare_apps(
    a_id: &str,
    b_id: &str,
    scores: &HashMap<String, i64>,
    names: &HashMap<String, (String, String)>,
    counts: &HashMap<String, u64>,
    mode: LauncherMode,
    searching: bool,
) -> Ordering {
    let score_order = if searching {
        scores.get(b_id).cmp(&scores.get(a_id))
    } else {
        Ordering::Equal
    };
    let count_order = if mode == LauncherMode::Normal {
        counts
            .get(b_id)
            .copied()
            .unwrap_or(0)
            .cmp(&counts.get(a_id).copied().unwrap_or(0))
    } else {
        Ordering::Equal
    };
    score_order
        .then(count_order)
        .then_with(|| {
            names
                .get(a_id)
                .map(|(name, _)| name)
                .cmp(&names.get(b_id).map(|(name, _)| name))
        })
        .then_with(|| a_id.cmp(b_id))
}

fn create(
    app: &gtk::Application,
    state: &Rc<AppState>,
    mode: LauncherMode,
    entries: Vec<AppEntry>,
    launch_counts: HashMap<String, u64>,
) {
    let old = state.launcher.borrow_mut().take();
    if let Some(old) = old {
        old.close();
    }
    state.launcher_mode.set(Some(mode));
    state
        .launcher_focus_window
        .set(Some(state.focused_window_id()));
    *state.launcher_apps.borrow_mut() = entries;

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("applications")
        .default_width(520)
        .build();
    window.set_widget_name("launcher");
    set_layer_window(
        &window,
        "chuhshell-launcher",
        layer_shell::Layer::Overlay,
        if mode == LauncherMode::Manage {
            &[]
        } else {
            &[layer_shell::Edge::Top]
        },
        0,
        layer_shell::KeyboardMode::OnDemand,
    );

    window.set_monitor(crate::ui::active_monitor().as_ref());
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.add_css_class("launcher-box");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("search applications…"));
    search.add_css_class("search");
    outer.append(&search);
    let scrolled = gtk::ScrolledWindow::builder()
        .min_content_height(120)
        .max_content_height(420)
        .propagate_natural_height(true)
        .build();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("app-list");
    let empty = gtk::Label::new(Some("no applications found"));
    empty.add_css_class("app-empty");
    list.set_placeholder(Some(&empty));
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);
    window.set_child(Some(&outer));

    let scores = Rc::new(RefCell::new(HashMap::new()));
    let counts = Rc::new(RefCell::new(launch_counts));
    let searching = Rc::new(std::cell::Cell::new(false));
    let names: HashMap<String, (String, String)> = state
        .launcher_apps
        .borrow()
        .iter()
        .filter(|entry| mode == LauncherMode::Manage || !entry.hidden)
        .map(|app| {
            (
                app.id.clone(),
                (
                    format!("{} {}", app.name, app.keywords).to_lowercase(),
                    app.id.to_lowercase(),
                ),
            )
        })
        .collect();
    let names = Rc::new(names);
    for entry in state.launcher_apps.borrow().iter() {
        if mode == LauncherMode::Manage || !entry.hidden {
            append_app_row(&list, entry, mode);
            scores.borrow_mut().insert(entry.id.clone(), 0_i64);
        }
    }
    list.set_filter_func({
        let scores = Rc::clone(&scores);
        move |row| {
            scores.borrow().contains_key(
                row.widget_name()
                    .as_str()
                    .strip_prefix("app-")
                    .unwrap_or(""),
            )
        }
    });
    list.set_sort_func({
        let scores = Rc::clone(&scores);
        let names = Rc::clone(&names);
        let counts = Rc::clone(&counts);
        let searching = Rc::clone(&searching);
        move |a, b| {
            let a_id = a.widget_name();
            let b_id = b.widget_name();
            let a_id = a_id.as_str().strip_prefix("app-").unwrap_or("");
            let b_id = b_id.as_str().strip_prefix("app-").unwrap_or("");
            compare_apps(
                a_id,
                b_id,
                &scores.borrow(),
                &names,
                &counts.borrow(),
                mode,
                searching.get(),
            )
            .into()
        }
    });
    list.select_row(visible_rows(&list).first());
    update_selected_row_styles(&list);
    scroll_selected_row_into_view(&list, &scrolled);
    let scrolled_for_selection = scrolled.downgrade();
    list.connect_selected_rows_changed(move |list| {
        update_selected_row_styles(list);
        if let Some(scrolled) = scrolled_for_selection.upgrade() {
            scroll_selected_row_into_view(list, &scrolled);
        }
    });

    search.connect_search_changed({
        let list = list.downgrade();
        let scores = Rc::clone(&scores);
        let searching = Rc::clone(&searching);
        move |entry| {
            let Some(list) = list.upgrade() else {
                return;
            };
            let query = entry.text().trim().to_lowercase();
            searching.set(!query.is_empty());
            let mut next = scores.borrow_mut();
            next.clear();
            for (id, (name, lower_id)) in names.iter() {
                if let Some(score) = fuzzy::score_lowercase(name, &query)
                    .or_else(|| fuzzy::score_lowercase(lower_id, &query))
                {
                    next.insert(id.clone(), score);
                }
            }
            drop(next);
            list.invalidate_filter();
            list.invalidate_sort();
            list.select_row(visible_rows(&list).first());
        }
    });

    let state_for_activate = Rc::downgrade(state);
    let window_for_activate = window.downgrade();
    let counts_for_activate = Rc::clone(&counts);
    list.connect_row_activated(move |_, row| {
        let Some(state_for_activate) = state_for_activate.upgrade() else {
            return;
        };
        if !row.is_child_visible() || !row.is_visible() {
            return;
        }
        let Some(id) = row.widget_name().strip_prefix("app-").map(str::to_owned) else {
            return;
        };
        if mode == LauncherMode::Manage {
            let apps = state_for_activate.launcher_apps.borrow();
            let Some(hidden) = apps::toggled_hidden(&apps, &id, apps::read_hidden()) else {
                return;
            };
            if let Err(error) = apps::write_hidden(&hidden) {
                eprintln!("chuhshell: failed to save hidden apps: {error}");
                return;
            }
            drop(apps);
            let mut apps = state_for_activate.launcher_apps.borrow_mut();
            if let Some(entry) = apps.iter_mut().find(|entry| entry.id == id) {
                entry.hidden = !entry.hidden;
            }
            if let Some(content) = row.child().and_downcast::<gtk::Box>() {
                if let Some(icon) = content.first_child().and_downcast::<gtk::Label>() {
                    icon.set_text(
                        if apps
                            .iter()
                            .find(|entry| entry.id == id)
                            .is_some_and(|entry| entry.hidden)
                        {
                            "󰈉"
                        } else {
                            "󰈈"
                        },
                    );
                }
                if apps
                    .iter()
                    .find(|entry| entry.id == id)
                    .is_some_and(|entry| entry.hidden)
                {
                    content.add_css_class("hidden-app");
                } else {
                    content.remove_css_class("hidden-app");
                }
            }
        } else {
            row.set_sensitive(false);
            let row = row.downgrade();
            let window = window_for_activate.clone();
            let counts = Rc::clone(&counts_for_activate);
            glib::MainContext::default().spawn_local(async move {
                match apps::launch(&id).await {
                    Ok(()) => {
                        if let Err(error) = apps::record_launch(&mut counts.borrow_mut(), &id) {
                            eprintln!("chuhshell: failed to save launch counts: {error}");
                        }
                        if let Some(window) = window.upgrade() {
                            window.close();
                        }
                    }
                    Err(error) => {
                        if let Some(row) = row.upgrade() {
                            row.set_sensitive(true);
                            row.set_tooltip_text(Some(&error));
                        }
                        if let Some(window) = window.upgrade() {
                            window.set_title(Some(&format!("Launch failed: {error}")));
                        }
                        eprintln!("chuhshell: {error}");
                    }
                }
            });
        }
    });

    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list_keys = list.downgrade();
    let window_keys = window.downgrade();
    key.connect_key_pressed(move |_, key, _, _| {
        let Some(list_keys) = list_keys.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gdk::Key::Escape => {
                if let Some(window) = window_keys.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::Up | gdk::Key::Page_Down | gdk::Key::Page_Up => {
                let offset = match key {
                    gdk::Key::Up => -1,
                    gdk::Key::Page_Up => -5,
                    gdk::Key::Page_Down => 5,
                    _ => 1,
                };
                let rows = visible_rows(&list_keys);
                if let Some(index) = selection_index(
                    rows.len(),
                    rows.iter()
                        .position(|row| Some(row) == list_keys.selected_row().as_ref()),
                    offset,
                ) {
                    list_keys.select_row(rows.get(index));
                } else {
                    list_keys.unselect_all();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(row) = list_keys
                    .selected_row()
                    .filter(|row| row.is_visible() && row.is_child_visible() && row.is_sensitive())
                {
                    list_keys.emit_by_name::<()>("row-activated", &[&row]);
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(key);

    let was_active = std::cell::Cell::new(false);
    let closing = Rc::new(std::cell::Cell::new(false));
    let closing_notify = Rc::clone(&closing);
    window.connect_is_active_notify(move |window| {
        if window.is_active() {
            was_active.set(true);
        } else if !closing_notify.get() && was_active.replace(false) {
            window.close();
        }
    });
    window.connect_close_request({
        let state = Rc::downgrade(state);
        move |_| {
            closing.set(true);
            if let Some(state) = state.upgrade() {
                let _ = state.launcher.borrow_mut().take();
                state.launcher_focus_window.set(None);
                state.launcher_mode.set(None);
                state.launcher_apps.borrow_mut().clear();
            }
            glib::Propagation::Proceed
        }
    });

    window.present();
    search.grab_focus();
    *state.launcher.borrow_mut() = Some(window.upcast());
}

pub(crate) fn visible_rows(list: &gtk::ListBox) -> Vec<gtk::ListBoxRow> {
    let mut rows = Vec::new();
    let mut child = list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
            && row.is_visible()
            && row.is_child_visible()
        {
            rows.push(row);
        }
    }
    rows
}

pub(crate) fn selection_index(len: usize, selected: Option<usize>, offset: i32) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(selected.map_or(0, |index| {
            (index as i64 + i64::from(offset)).clamp(0, len as i64 - 1) as usize
        }))
    }
}

fn update_selected_row_styles(list: &gtk::ListBox) {
    let selected = list.selected_row();
    let mut child = list.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let Some(row) = row.downcast_ref::<gtk::ListBoxRow>() else {
            continue;
        };
        if selected
            .as_ref()
            .is_some_and(|selected| selected.as_ptr() == row.as_ptr())
        {
            row.add_css_class("selected-row");
        } else {
            row.remove_css_class("selected-row");
        }
    }
}

pub(crate) fn scroll_selected_row_into_view(list: &gtk::ListBox, scrolled: &gtk::ScrolledWindow) {
    let Some(row) = list.selected_row() else {
        return;
    };
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    let adjustment = scrolled.vadjustment();
    let current = adjustment.value();
    let page_size = adjustment.page_size();
    let top = f64::from(bounds.y());
    let bottom = top + f64::from(bounds.height());
    let next = if top < current {
        top
    } else if bottom > current + page_size {
        bottom - page_size
    } else {
        return;
    };
    adjustment.set_value(next.clamp(
        adjustment.lower(),
        (adjustment.upper() - page_size).max(adjustment.lower()),
    ));
}

fn append_app_row(list: &gtk::ListBox, entry: &AppEntry, mode: LauncherMode) {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(&format!("app-{}", entry.id));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.add_css_class("app-row");
    if entry.hidden {
        content.add_css_class("hidden-app");
    }
    if mode == LauncherMode::Manage {
        let icon = gtk::Label::new(Some(if entry.hidden { "󰈉" } else { "󰈈" }));
        icon.add_css_class("app-icon");
        content.append(&icon);
    } else {
        let icon = crate::ui::image(&entry.icon, 22);
        icon.set_pixel_size(22);
        icon.add_css_class("app-icon");
        content.append(&icon);
    }
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.set_valign(gtk::Align::Center);
    details.set_hexpand(true);
    let name = gtk::Label::new(Some(&entry.name.to_lowercase()));
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(40);
    details.append(&name);
    if !entry.comment.is_empty() {
        let comment = gtk::Label::new(Some(&entry.comment.to_lowercase()));
        comment.set_xalign(0.0);
        comment.set_ellipsize(gtk::pango::EllipsizeMode::End);
        comment.set_max_width_chars(52);
        comment.add_css_class("app-meta");
        details.append(&comment);
    }
    content.append(&details);
    row.set_child(Some(&content));
    row.set_tooltip_text(Some(&entry.exec));
    list.append(&row);
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application) {
    let state = Rc::new(AppState::default());
    let entries = ["Alpha", "Beta"]
        .iter()
        .map(|name| {
            apps::parse_entry(
                &format!("{name}.desktop"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
                false,
            )
            .unwrap()
        })
        .collect();
    create(app, &state, LauncherMode::Normal, entries, HashMap::new());
    let window = state.launcher.borrow().clone().unwrap();
    let outer = window.child().unwrap().downcast::<gtk::Box>().unwrap();
    let search = outer
        .first_child()
        .unwrap()
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    let list = search
        .next_sibling()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::Viewport>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::ListBox>()
        .unwrap();
    search.set_text("no-such-application");
    search.emit_by_name::<()>("search-changed", &[]);
    assert!(list.selected_row().is_none());
    assert!(visible_rows(&list).is_empty());
    search.set_text("Beta");
    search.emit_by_name::<()>("search-changed", &[]);
    assert_eq!(visible_rows(&list).len(), 1);
    assert!(list.selected_row().unwrap().is_child_visible());
    let controllers = window.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(key) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            key.emit_by_name::<bool>(
                "key-pressed",
                &[&gdk::Key::Down, &0u32, &gdk::ModifierType::empty()],
            );
        }
    }
    assert!(list.selected_row().unwrap().is_child_visible());
    window.close();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_stays_inside_filtered_results() {
        assert_eq!(selection_index(0, None, 1), None);
        assert_eq!(selection_index(1, Some(0), 5), Some(0));
        assert_eq!(selection_index(3, Some(0), -1), Some(0));
        assert_eq!(selection_index(10, Some(2), 5), Some(7));
    }

    #[test]
    fn launch_frequency_orders_normal_mode_and_search_scores_stay_primary() {
        let names = HashMap::from([
            (
                "alpha.desktop".to_owned(),
                ("alpha".to_owned(), String::new()),
            ),
            (
                "beta.desktop".to_owned(),
                ("beta".to_owned(), String::new()),
            ),
        ]);
        let counts = HashMap::from([("beta.desktop".to_owned(), 9)]);
        let scores = HashMap::from([
            ("alpha.desktop".to_owned(), 20),
            ("beta.desktop".to_owned(), 10),
        ]);
        assert_eq!(
            compare_apps(
                "beta.desktop",
                "alpha.desktop",
                &scores,
                &names,
                &counts,
                LauncherMode::Normal,
                false
            ),
            Ordering::Less
        );
        assert_eq!(
            compare_apps(
                "alpha.desktop",
                "beta.desktop",
                &scores,
                &names,
                &counts,
                LauncherMode::Normal,
                true
            ),
            Ordering::Less
        );
        assert_eq!(
            compare_apps(
                "alpha.desktop",
                "beta.desktop",
                &scores,
                &names,
                &counts,
                LauncherMode::Manage,
                false
            ),
            Ordering::Less
        );
    }
}
