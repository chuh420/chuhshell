use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;

use crate::app::{AppState, LauncherMode};
use crate::apps::{self, AppEntry};
use crate::fuzzy;
use crate::modules;
use crate::ui::set_layer_window;

pub fn show(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode) {
    create(app, state, mode);
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

fn create(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode) {
    let old = state.launcher.borrow_mut().take();
    if let Some(old) = old {
        old.close();
    }
    state.launcher_mode.set(Some(mode));
    state
        .launcher_focus_window
        .set(Some(state.focused_window_id()));
    *state.launcher_apps.borrow_mut() = apps::load_apps();

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
        &[layer_shell::Edge::Top],
        0,
        layer_shell::KeyboardMode::OnDemand,
    );

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
    let counts = Rc::new(RefCell::new(apps::read_launch_counts()));
    let searching = Rc::new(std::cell::Cell::new(false));
    let names: HashMap<String, (String, String)> = state
        .launcher_apps
        .borrow()
        .iter()
        .filter(|entry| mode == LauncherMode::Manage || !entry.hidden)
        .map(|app| {
            (
                app.id.clone(),
                (app.name.to_lowercase(), app.id.to_lowercase()),
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
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
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
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    });

    let state_for_activate = Rc::downgrade(state);
    let window_for_activate = window.downgrade();
    let counts_for_activate = Rc::clone(&counts);
    list.connect_row_activated(move |_, row| {
        let Some(state_for_activate) = state_for_activate.upgrade() else {
            return;
        };
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
        } else if modules::spawn_detached("gtk-launch", &[&id]) {
            if let Err(error) = apps::record_launch(&mut counts_for_activate.borrow_mut(), &id) {
                eprintln!("chuhshell: failed to save launch counts: {error}");
            }
            if let Some(window) = window_for_activate.upgrade() {
                window.close();
            }
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
            gdk::Key::Down => {
                let index = list_keys.selected_row().map_or(0, |row| row.index() + 1);
                if let Some(row) = list_keys.row_at_index(index) {
                    list_keys.select_row(Some(&row));
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up => {
                let index = list_keys.selected_row().map_or(0, |row| row.index() - 1);
                if let Some(row) = list_keys.row_at_index(index.max(0)) {
                    list_keys.select_row(Some(&row));
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(row) = list_keys.selected_row() {
                    list_keys.emit_by_name::<()>("row-activated", &[&row]);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Page_Down | gdk::Key::Page_Up => {
                let direction = if key == gdk::Key::Page_Down { 1 } else { -1 };
                let current = list_keys.selected_row().map_or(0, |row| row.index());
                let max = list_keys.observe_children().n_items().saturating_sub(1) as i32;
                if let Some(row) = list_keys.row_at_index((current + direction * 5).clamp(0, max)) {
                    list_keys.select_row(Some(&row));
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

fn scroll_selected_row_into_view(list: &gtk::ListBox, scrolled: &gtk::ScrolledWindow) {
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
        let icon = if entry.icon.is_empty() {
            gtk::Image::from_icon_name("application-x-executable")
        } else {
            gtk::Image::from_icon_name(&entry.icon)
        };
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
mod tests {
    use super::*;

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
