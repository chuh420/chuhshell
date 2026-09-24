use std::process::Command;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;

use crate::app::{AppState, LauncherMode};
use crate::apps::{self, AppEntry};
use crate::fuzzy;
use crate::ui::set_layer_window;

pub fn show(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode) {
    create(app, state, mode);
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
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);
    window.set_child(Some(&outer));

    populate_launcher_list(&list, &state.launcher_apps.borrow()[..], mode, "");
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    update_selected_row_styles(&list);
    scroll_selected_row_into_view(&list, &scrolled);
    let scrolled_for_selection = scrolled.clone();
    list.connect_selected_rows_changed(move |list| {
        update_selected_row_styles(list);
        scroll_selected_row_into_view(list, &scrolled_for_selection);
    });

    search.connect_search_changed({
        let state = Rc::clone(state);
        let list = list.clone();
        move |entry| {
            let query = entry.text().to_string();
            let apps = state.launcher_apps.borrow();
            populate_launcher_list(&list, &apps[..], mode, &query);
        }
    });

    let state_for_activate = Rc::clone(state);
    let window_for_activate = window.clone();
    let search_for_activate = search.clone();
    list.connect_row_activated(move |list, row| {
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
            let selected = row.index();
            populate_launcher_list(list, &apps[..], mode, &search_for_activate.text());
            let last = list.observe_children().n_items().saturating_sub(1) as i32;
            if let Some(row) = list.row_at_index(selected.min(last)) {
                list.select_row(Some(&row));
            }
        } else if Command::new("gtk-launch").arg(&id).spawn().is_ok() {
            window_for_activate.close();
        }
    });

    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list_keys = list.clone();
    let window_keys = window.clone();
    key.connect_key_pressed(move |_, key, _, _| match key {
        gdk::Key::Escape => {
            window_keys.close();
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
        let state = Rc::clone(state);
        move |_| {
            closing.set(true);
            let _ = state.launcher.borrow_mut().take();
            state.launcher_focus_window.set(None);
            state.launcher_mode.set(None);
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
    let name = gtk::Label::new(Some(&entry.name.to_lowercase()));
    name.set_xalign(0.0);
    details.append(&name);
    if !entry.comment.is_empty() {
        let comment = gtk::Label::new(Some(&entry.comment.to_lowercase()));
        comment.set_xalign(0.0);
        comment.set_ellipsize(gtk::pango::EllipsizeMode::End);
        comment.add_css_class("app-meta");
        details.append(&comment);
    }
    content.append(&details);
    row.set_child(Some(&content));
    row.set_tooltip_text(Some(&entry.exec));
    list.append(&row);
}

fn populate_launcher_list(list: &gtk::ListBox, apps: &[AppEntry], mode: LauncherMode, query: &str) {
    let mut matches: Vec<(i64, &AppEntry)> = apps
        .iter()
        .filter(|app| mode == LauncherMode::Manage || !app.hidden)
        .filter_map(|app| {
            fuzzy::score(&app.name.to_lowercase(), query)
                .or_else(|| fuzzy::score(&app.id, query))
                .map(|score| (score, app))
        })
        .collect();
    matches.sort_by(|(score_a, app_a), (score_b, app_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| app_a.name.to_lowercase().cmp(&app_b.name.to_lowercase()))
    });
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for (_, app) in matches {
        append_app_row(list, app, mode);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    update_selected_row_styles(list);
}
