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
    show_inner(app, state, mode, false);
}

pub fn configure(app: &gtk::Application, state: &Rc<AppState>) {
    show_inner(app, state, LauncherMode::Normal, true);
}

fn show_inner(app: &gtk::Application, state: &Rc<AppState>, mode: LauncherMode, editing: bool) {
    crate::menu::close(state);
    crate::ui::close_popover();
    let generation = state.launcher_generation.get().wrapping_add(1);
    state.launcher_generation.set(generation);
    let loaded = crate::storage::run(|| {
        Ok((
            apps::load_apps(),
            apps::read_launch_counts(),
            apps::read_launcher_preferences(),
        ))
    });
    let state = Rc::downgrade(state);
    let app = app.downgrade();
    glib::MainContext::default().spawn_local(async move {
        if let Ok((apps, counts, preferences)) = loaded.await
            && let (Some(app), Some(state)) = (app.upgrade(), state.upgrade())
            && state.launcher_generation.get() == generation
        {
            let window = create(&app, &state, mode, apps, (counts, preferences), editing);
            if editing {
                let app = app.downgrade();
                let state = Rc::downgrade(&state);
                window.add_tick_callback(move |window, _| {
                    if window.width() <= 0 || window.height() <= 0 {
                        return glib::ControlFlow::Continue;
                    }
                    let window = window.downgrade();
                    let app = app.clone();
                    let state = state.clone();
                    glib::idle_add_local_once(move || {
                        if let (Some(app), Some(state), Some(window)) =
                            (app.upgrade(), state.upgrade(), window.upgrade())
                            && state.launcher_generation.get() == generation
                            && window.is_visible()
                        {
                            crate::layout::show_ready(&app, &state);
                        }
                    });
                    glib::ControlFlow::Break
                });
            }
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
    settings: (HashMap<String, u64>, apps::LauncherPreferences),
    editing: bool,
) -> gtk::Window {
    let old = state.launcher.borrow_mut().take();
    if let Some(old) = old {
        old.close();
    }
    state.launcher_mode.set(Some(mode));
    state
        .launcher_focus_window
        .set((!editing).then(|| state.focused_window_id()));
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
        if editing {
            layer_shell::KeyboardMode::None
        } else {
            layer_shell::KeyboardMode::OnDemand
        },
    );

    window.set_monitor(crate::ui::active_monitor().as_ref());
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("launcher-box");
    outer.set_can_target(!editing);
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
    if mode == LauncherMode::Normal
        && let Some(geometry) = state.layouts.launcher.get()
    {
        crate::layout::apply_launcher(&window, geometry);
        scrolled.set_min_content_height(0);
        scrolled.set_propagate_natural_height(false);
        scrolled.set_vexpand(true);
    }
    window.set_child(Some(&outer));

    let preferences = Rc::new(RefCell::new(settings.1));
    let saving = Rc::new(std::cell::Cell::new(false));
    let save_error = gtk::Label::new(None);
    save_error.add_css_class("app-meta");
    save_error.set_wrap(true);
    save_error.set_visible(false);
    outer.append(&save_error);
    let sort = gtk::Button::new();
    sort.add_css_class("launcher-sort");
    sort.set_halign(gtk::Align::End);
    sort.set_label(if preferences.borrow().alphabetical {
        "sort: a–z"
    } else {
        "sort: most used"
    });
    sort.set_tooltip_text(Some("Switch application sorting"));
    if mode == LauncherMode::Normal {
        outer.append(&sort);
    }
    sort.connect_clicked({
        let preferences = preferences.clone();
        let saving = saving.clone();
        let error = save_error.downgrade();
        let list = list.downgrade();
        move |button| {
            if saving.replace(true) {
                return;
            }
            let saved = apps::change_launcher_preferences(apps::PreferenceChange::Sort);
            let focused = button.is_focus();
            button.set_sensitive(false);
            let button = button.downgrade();
            let preferences = preferences.clone();
            let saving = saving.clone();
            let list = list.clone();
            let error = error.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = saved.await;
                saving.set(false);
                if let Some(button) = button.upgrade() {
                    button.set_sensitive(true);
                    if focused {
                        button.grab_focus();
                    }
                    match result {
                        Ok(next) => {
                            button.set_label(if next.alphabetical {
                                "sort: a–z"
                            } else {
                                "sort: most used"
                            });
                            *preferences.borrow_mut() = next;
                            if let Some(list) = list.upgrade() {
                                list.invalidate_sort();
                            }
                            if let Some(error) = error.upgrade() {
                                error.set_visible(false);
                            }
                        }
                        Err(message) => {
                            if let Some(error) = error.upgrade() {
                                error.set_text(&message);
                                error.set_visible(true);
                            }
                        }
                    }
                }
            });
        }
    });
    let scores = Rc::new(RefCell::new(HashMap::new()));
    let counts = Rc::new(RefCell::new(settings.0));
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
    let search_names = Rc::new(names);
    let names = Rc::new(
        state
            .launcher_apps
            .borrow()
            .iter()
            .map(|app| {
                (
                    app.id.clone(),
                    (app.name.to_lowercase(), app.id.to_lowercase()),
                )
            })
            .collect::<HashMap<_, _>>(),
    );
    let mut entries: Vec<_> = state
        .launcher_apps
        .borrow()
        .iter()
        .filter(|entry| mode == LauncherMode::Manage || !entry.hidden)
        .cloned()
        .collect();
    scores
        .borrow_mut()
        .extend(entries.iter().map(|entry| (entry.id.clone(), 0_i64)));
    entries.sort_by(|left, right| {
        let preferences = preferences.borrow();
        preferences
            .pinned
            .contains(&right.id)
            .cmp(&preferences.pinned.contains(&left.id))
            .then_with(|| {
                compare_apps(
                    &left.id,
                    &right.id,
                    &scores.borrow(),
                    &names,
                    &counts.borrow(),
                    if preferences.alphabetical {
                        LauncherMode::Manage
                    } else {
                        mode
                    },
                    false,
                )
            })
    });
    let append_row = {
        let list = list.clone();
        let preferences = preferences.clone();
        let saving = saving.clone();
        let save_error = save_error.clone();
        move |entry: &AppEntry| {
            let row = append_app_row(&list, entry, mode);
            if mode == LauncherMode::Normal {
                let content = row.child().and_downcast::<gtk::Box>().unwrap();
                let pin = gtk::Button::new();
                pin.add_css_class("app-pin");
                pin.set_valign(gtk::Align::Center);
                update_pin(&pin, preferences.borrow().pinned.contains(&entry.id));
                content.append(&pin);
                pin.connect_has_focus_notify({
                    let list = list.downgrade();
                    let row = row.downgrade();
                    move |button| {
                        if button.has_focus()
                            && let Some(list) = list.upgrade()
                        {
                            list.select_row(row.upgrade().as_ref());
                        }
                    }
                });
                pin.connect_clicked({
                    let id = entry.id.clone();
                    let preferences = preferences.clone();
                    let saving = saving.clone();
                    let error = save_error.downgrade();
                    let list = list.downgrade();
                    let row = row.downgrade();
                    move |button| {
                        if saving.replace(true) {
                            return;
                        }
                        let saved = apps::change_launcher_preferences(apps::PreferenceChange::Pin(
                            id.clone(),
                        ));
                        let focused = button.is_focus();
                        button.set_sensitive(false);
                        let button = button.downgrade();
                        let id = id.clone();
                        let preferences = preferences.clone();
                        let saving = saving.clone();
                        let error = error.clone();
                        let list = list.clone();
                        let row = row.clone();
                        glib::MainContext::default().spawn_local(async move {
                            let result = saved.await;
                            saving.set(false);
                            if let Some(button) = button.upgrade() {
                                button.set_sensitive(true);
                                match result {
                                    Ok(next) => {
                                        update_pin(&button, next.pinned.contains(&id));
                                        *preferences.borrow_mut() = next;
                                        if let Some(list) = list.upgrade() {
                                            list.invalidate_sort();
                                            list.select_row(row.upgrade().as_ref());
                                            if focused {
                                                button.grab_focus();
                                            }
                                        }
                                        if let Some(error) = error.upgrade() {
                                            error.set_visible(false);
                                        }
                                    }
                                    Err(message) => {
                                        if let Some(error) = error.upgrade() {
                                            error.set_text(&message);
                                            error.set_visible(true);
                                        }
                                    }
                                }
                            }
                        });
                    }
                });
            }
        }
    };
    for entry in entries.iter().take(16) {
        append_row(entry);
    }
    let mut pending = entries.into_iter().skip(16);
    let populating = Rc::new(std::cell::Cell::new(pending.len() != 0));
    let population_state = populating.clone();
    let population_empty = empty.downgrade();
    if populating.get() {
        empty.set_text("loading applications…");
    }
    let population_window = window.downgrade();
    let population_list = list.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        if population_window
            .upgrade()
            .is_none_or(|window| !window.is_visible())
        {
            return glib::ControlFlow::Break;
        }
        let mut added = 0;
        for entry in pending.by_ref().take(16) {
            append_row(&entry);
            added += 1;
        }
        if let Some(list) = population_list.upgrade()
            && list.selected_row().is_none()
        {
            list.select_row(visible_rows(&list).first());
        }
        if added == 0 {
            population_state.set(false);
            if let Some(empty) = population_empty.upgrade() {
                empty.set_text("no applications found");
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
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
        let preferences = preferences.clone();
        move |a, b| {
            let a_id = a.widget_name();
            let b_id = b.widget_name();
            let a_id = a_id.as_str().strip_prefix("app-").unwrap_or("");
            let b_id = b_id.as_str().strip_prefix("app-").unwrap_or("");
            let preferences = preferences.borrow();
            let pin_order = if mode == LauncherMode::Normal {
                preferences
                    .pinned
                    .contains(b_id)
                    .cmp(&preferences.pinned.contains(a_id))
            } else {
                Ordering::Equal
            };
            pin_order
                .then_with(|| {
                    compare_apps(
                        a_id,
                        b_id,
                        &scores.borrow(),
                        &names,
                        &counts.borrow(),
                        if preferences.alphabetical {
                            LauncherMode::Manage
                        } else {
                            mode
                        },
                        searching.get(),
                    )
                })
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
            for (id, (name, lower_id)) in search_names.iter() {
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
    let saving = saving.clone();
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
            if saving.replace(true) {
                return;
            }
            let entries = state_for_activate.launcher_apps.borrow().clone();
            let target_id = id.clone();
            let saved = crate::storage::run(move || {
                apps::change_hidden(&entries, &target_id).map_err(|e| e.to_string())
            });
            row.set_sensitive(false);
            let row = row.downgrade();
            let state = Rc::downgrade(&state_for_activate);
            let saving = saving.clone();
            let error = save_error.downgrade();
            glib::MainContext::default().spawn_local(async move {
                let result = saved.await;
                saving.set(false);
                let Some(row) = row.upgrade() else {
                    return;
                };
                row.set_sensitive(true);
                match result {
                    Ok(hidden) => {
                        if let Some(state) = state.upgrade()
                            && let Some(entry) = state
                                .launcher_apps
                                .borrow_mut()
                                .iter_mut()
                                .find(|entry| entry.id == id)
                        {
                            entry.hidden = hidden;
                        }
                        if let Some(content) = row.child().and_downcast::<gtk::Box>() {
                            if let Some(icon) = content.first_child().and_downcast::<gtk::Label>() {
                                icon.set_text(if hidden { "󰈉" } else { "󰈈" });
                            }
                            if hidden {
                                content.add_css_class("hidden-app");
                            } else {
                                content.remove_css_class("hidden-app");
                            }
                        }
                        if let Some(error) = error.upgrade() {
                            error.set_visible(false);
                        }
                    }
                    Err(message) => {
                        if let Some(error) = error.upgrade() {
                            error.set_text(&message);
                            error.set_visible(true);
                        }
                    }
                }
            });
        } else {
            row.set_sensitive(false);
            let row = row.downgrade();
            let window = window_for_activate.clone();
            let counts = Rc::clone(&counts_for_activate);
            glib::MainContext::default().spawn_local(async move {
                match apps::launch(&id).await {
                    Ok(()) => {
                        if let Err(error) = crate::storage::run(move || {
                            let mut next = apps::read_launch_counts();
                            apps::record_launch(&mut next, &id).map_err(|e| e.to_string())?;
                            Ok(next)
                        })
                        .await
                        .map(|next| *counts.borrow_mut() = next)
                        {
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
    key.set_name(Some("launcher-navigation"));
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list_keys = list.downgrade();
    let window_keys = window.downgrade();
    let search_keys = search.downgrade();
    let sort_keys = sort.downgrade();
    key.connect_key_pressed(move |_, key, _, modifiers| {
        let default_key = crate::keybindings::is_default("launcher", key, modifiers);
        let key = crate::keybindings::remap("launcher", key, modifiers);
        let Some(list_keys) = list_keys.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let focused_button = window_keys
            .upgrade()
            .and_then(|window| gtk::prelude::GtkWindowExt::focus(&window))
            .and_downcast::<gtk::Button>();
        let pin_focused = focused_button
            .as_ref()
            .is_some_and(|button| button.has_css_class("app-pin"));
        let sort_focused = focused_button
            .as_ref()
            .is_some_and(|button| button.has_css_class("launcher-sort"));
        match key {
            gdk::Key::Escape => {
                if let Some(window) = window_keys.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Right if mode == LauncherMode::Normal => {
                if let Some(pin) = list_keys.selected_row().and_then(|row| pin_button(&row)) {
                    pin.grab_focus();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Left if pin_focused || sort_focused => {
                if let Some(search) = search_keys.upgrade() {
                    search.grab_focus();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Left | gdk::Key::Right => glib::Propagation::Proceed,
            gdk::Key::Down | gdk::Key::Up | gdk::Key::Page_Down | gdk::Key::Page_Up => {
                let rows = visible_rows(&list_keys);
                if sort_focused {
                    if matches!(key, gdk::Key::Up | gdk::Key::Page_Up) {
                        list_keys.select_row(rows.last());
                        if let Some(search) = search_keys.upgrade() {
                            search.grab_focus();
                        }
                    }
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Down
                    && mode == LauncherMode::Normal
                    && !populating.get()
                    && (rows.is_empty() || rows.last() == list_keys.selected_row().as_ref())
                {
                    if let Some(sort) = sort_keys.upgrade() {
                        sort.grab_focus();
                    }
                    return glib::Propagation::Stop;
                }
                let offset = match key {
                    gdk::Key::Up => -1,
                    gdk::Key::Page_Up => -5,
                    gdk::Key::Page_Down => 5,
                    _ => 1,
                };
                if let Some(index) = selection_index(
                    rows.len(),
                    rows.iter()
                        .position(|row| Some(row) == list_keys.selected_row().as_ref()),
                    offset,
                ) {
                    list_keys.select_row(rows.get(index));
                    if pin_focused && let Some(pin) = rows.get(index).and_then(pin_button) {
                        pin.grab_focus();
                    }
                } else {
                    list_keys.unselect_all();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(button) = focused_button.filter(|_| pin_focused || sort_focused) {
                    button.emit_clicked();
                    return glib::Propagation::Stop;
                }
                if let Some(row) = list_keys
                    .selected_row()
                    .filter(|row| row.is_visible() && row.is_child_visible() && row.is_sensitive())
                {
                    if let Some(pin) = pin_button(&row).filter(|pin| pin.is_focus()) {
                        pin.emit_clicked();
                    } else {
                        list_keys.emit_by_name::<()>("row-activated", &[&row]);
                    }
                }
                glib::Propagation::Stop
            }
            _ if default_key => glib::Propagation::Stop,
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
        } else if !editing && !closing_notify.get() && was_active.replace(false) {
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

    crate::ui::animate_close(&window);
    window.present();
    search.grab_focus();
    list.select_row(visible_rows(&list).first());
    let window: gtk::Window = window.upcast();
    *state.launcher.borrow_mut() = Some(window.clone());
    window
}

fn pin_button(row: &gtk::ListBoxRow) -> Option<gtk::Button> {
    row.child()?.last_child()?.downcast::<gtk::Button>().ok()
}

fn update_pin(button: &gtk::Button, pinned: bool) {
    button.set_label(if pinned { "󰐃" } else { "󰐄" });
    button.set_tooltip_text(Some(if pinned {
        "Unpin application"
    } else {
        "Pin application"
    }));
    if pinned {
        button.add_css_class("pinned");
    } else {
        button.remove_css_class("pinned");
    }
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

fn append_app_row(list: &gtk::ListBox, entry: &AppEntry, mode: LauncherMode) -> gtk::ListBoxRow {
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
        let icon = crate::ui::image(&entry.icon, 28);
        icon.set_pixel_size(28);
        icon.add_css_class("app-icon");
        content.append(&icon);
    }
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.set_valign(gtk::Align::Center);
    details.set_hexpand(true);
    let name = gtk::Label::new(Some(&entry.name.to_lowercase()));
    name.add_css_class("app-name");
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
    row
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
    create(
        app,
        &state,
        LauncherMode::Normal,
        entries,
        (
            HashMap::from([("Beta.desktop".to_owned(), 9)]),
            apps::LauncherPreferences::default(),
        ),
        false,
    );
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
    crate::ui_tests::pump(100);
    crate::ui_tests::capture("launcher");
    let frequent = list.selected_row().unwrap();
    let other = visible_rows(&list)[1].clone();
    let other_pin = pin_button(&other).unwrap();
    let unpinned_icon = other_pin.label().unwrap();
    other_pin.emit_clicked();
    crate::ui_tests::pump(100);
    assert_ne!(other_pin.label().unwrap(), unpinned_icon);
    assert!(other_pin.has_css_class("pinned"));
    assert_eq!(visible_rows(&list)[0], other);
    let controllers = window.observe_controllers();
    let key = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find(|key| key.name().as_deref() == Some("launcher-navigation"))
        .unwrap();
    let press = |value: gdk::Key| {
        key.emit_by_name::<bool>("key-pressed", &[&value, &0u32, &gdk::ModifierType::empty()]);
    };
    for key_value in [gdk::Key::Left, gdk::Key::Right] {
        for modifiers in [
            gdk::ModifierType::CONTROL_MASK,
            gdk::ModifierType::SHIFT_MASK,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
        ] {
            assert!(!key.emit_by_name::<bool>("key-pressed", &[&key_value, &0u32, &modifiers]));
        }
    }
    press(gdk::Key::Right);
    assert!(other_pin.is_focus());
    crate::ui_tests::pump(300);
    crate::ui_tests::capture("launcher-pin");
    press(gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert!(!other_pin.has_css_class("pinned"));
    assert_eq!(other_pin.label().unwrap(), unpinned_icon);
    assert!(other_pin.is_focus());
    press(gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert!(other_pin.has_css_class("pinned"));
    assert!(other_pin.is_focus());
    press(gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert_eq!(visible_rows(&list)[0], frequent);
    press(gdk::Key::Left);
    assert!(!other_pin.is_focus());
    let sort = outer.last_child().and_downcast::<gtk::Button>().unwrap();
    list.select_row(visible_rows(&list).last());
    press(gdk::Key::Right);
    press(gdk::Key::Down);
    assert!(sort.is_focus());
    crate::ui_tests::pump(300);
    crate::ui_tests::capture("launcher-sort");
    press(gdk::Key::Return);
    crate::ui_tests::pump(100);
    assert!(sort.is_focus());
    assert_eq!(sort.label().as_deref(), Some("sort: a–z"));
    assert_eq!(visible_rows(&list)[0], other);
    sort.emit_clicked();
    crate::ui_tests::pump(100);
    assert_eq!(sort.label().as_deref(), Some("sort: most used"));
    assert_eq!(visible_rows(&list)[0], frequent);
    press(gdk::Key::Up);
    assert!(!sort.is_focus());
    assert_eq!(list.selected_row().as_ref(), visible_rows(&list).last());
    press(gdk::Key::Right);
    assert!(
        pin_button(&list.selected_row().unwrap())
            .unwrap()
            .is_focus()
    );
    press(gdk::Key::Left);
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
    search.set_text("");
    search.emit_by_name::<()>("search-changed", &[]);
    let before = apps::read_launcher_preferences();
    let delayed = crate::storage::run(|| {
        std::thread::sleep(std::time::Duration::from_millis(200));
        Ok(())
    });
    other_pin.emit_clicked();
    window.close();
    let directory =
        std::path::PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()).join("applications");
    std::fs::create_dir_all(&directory).unwrap();
    for name in ["Alpha", "Beta"] {
        std::fs::write(
            directory.join(format!("{name}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
        )
        .unwrap();
    }
    apps::invalidate();
    show(app, &state, LauncherMode::Normal);
    crate::ui_tests::pump(400);
    glib::MainContext::default().block_on(delayed).unwrap();
    let reopened = state.launcher.borrow().clone().unwrap();
    let outer = reopened.child().unwrap();
    let sort = outer
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    sort.emit_clicked();
    crate::ui_tests::pump(100);
    let after = apps::read_launcher_preferences();
    assert_ne!(
        after.pinned.contains("Alpha.desktop"),
        before.pinned.contains("Alpha.desktop")
    );
    assert_ne!(after.alphabetical, before.alphabetical);
    let path = crate::config::path().with_file_name("launcher.json");
    let contents = std::fs::read(&path).unwrap();
    let disk: apps::LauncherPreferences = serde_json::from_slice(&contents).unwrap();
    assert_eq!(disk.pinned, after.pinned);
    assert_eq!(disk.alphabetical, after.alphabetical);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    sort.emit_clicked();
    crate::ui_tests::pump(100);
    assert!(sort.is_sensitive());
    assert_eq!(
        apps::read_launcher_preferences().alphabetical,
        after.alphabetical
    );
    assert!(sort.prev_sibling().unwrap().is_visible());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(path, contents).unwrap();
    reopened.close();
    for name in ["Alpha", "Beta"] {
        std::fs::remove_file(directory.join(format!("{name}.desktop"))).unwrap();
    }
    apps::invalidate();
}

#[cfg(test)]
pub fn profile(app: &gtk::Application) {
    use std::time::{Duration, Instant};
    if std::env::var_os("CHUHSHELL_PROFILE").is_none() {
        return;
    }
    let state = Rc::new(AppState::default());
    let cold = Instant::now();
    let catalog_size = apps::profile_catalog();
    println!(
        "CHUHSHELL_CATALOG {}",
        serde_json::json!({"apps": catalog_size, "cold_load_us": cold.elapsed().as_micros()})
    );
    let maximum_gap = Rc::new(std::cell::Cell::new(Duration::ZERO));
    let previous = Rc::new(std::cell::Cell::new(Instant::now()));
    let timer = glib::timeout_add_local(Duration::from_millis(5), {
        let maximum_gap = maximum_gap.clone();
        move || {
            let now = Instant::now();
            maximum_gap.set(
                maximum_gap
                    .get()
                    .max(now.duration_since(previous.replace(now))),
            );
            glib::ControlFlow::Continue
        }
    });
    let rss = || {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|line| line.starts_with("VmRSS:"))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .unwrap_or(0)
    };
    for count in [100, 500, 1000] {
        let entries: Vec<_> = (0..count)
            .map(|index| {
                apps::parse_entry(
                &format!("profile-{index}.desktop"),
                &format!(
                    "[Desktop Entry]\nType=Application\nName=Application {index}\nExec=/bin/true\n"
                ),
                false,
            )
            .unwrap()
            })
            .collect();
        let mut samples = Vec::new();
        let mut baseline = 0;
        let mut searches = Vec::new();
        let mut catalog_ready = Vec::new();
        maximum_gap.set(Duration::ZERO);
        for iteration in 0..25 {
            let start = Instant::now();
            let window = create(
                app,
                &state,
                LauncherMode::Normal,
                entries.clone(),
                (HashMap::new(), apps::LauncherPreferences::default()),
                false,
            );
            while window.width() <= 0 && start.elapsed() < Duration::from_secs(5) {
                crate::ui_tests::pump(1);
            }
            assert!(window.width() > 0);
            if iteration >= 5 {
                samples.push(start.elapsed().as_micros());
            }
            let list = window
                .child()
                .unwrap()
                .first_child()
                .unwrap()
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
            while list.row_at_index(count - 1).is_none()
                && start.elapsed() < Duration::from_secs(10)
            {
                crate::ui_tests::pump(5);
            }
            assert!(list.row_at_index(count - 1).is_some());
            if iteration >= 5 {
                catalog_ready.push(start.elapsed().as_micros());
            }
            let search = window
                .child()
                .unwrap()
                .first_child()
                .unwrap()
                .downcast::<gtk::SearchEntry>()
                .unwrap();
            for query in ["Application 9", "missing", ""] {
                let started = Instant::now();
                search.set_text(query);
                search.emit_by_name::<()>("search-changed", &[]);
                crate::ui_tests::pump(1);
                if iteration >= 5 {
                    searches.push(started.elapsed().as_micros());
                }
            }
            window.close();
            drop(window);
            crate::menu::show(app, &state);
            crate::ui_tests::pump(5);
            crate::menu::close(&state);
            crate::ui_tests::pump(5);
            if iteration == 4 {
                baseline = rss();
                maximum_gap.set(Duration::ZERO);
            }
        }
        samples.sort();
        searches.sort();
        catalog_ready.sort();
        println!(
            "CHUHSHELL_PROFILE {}",
            serde_json::json!({
                    "apps": count, "iterations": 20,
                    "launcher_median_us": samples[samples.len() / 2],
                    "launcher_p95_us": samples[(samples.len() * 95).div_ceil(100) - 1],
            "catalog_ready_median_us": catalog_ready[catalog_ready.len() / 2],
                    "search_median_us": searches[searches.len() / 2],
                    "search_p95_us": searches[(searches.len() * 95).div_ceil(100) - 1],
                    "main_loop_max_gap_us": maximum_gap.get().as_micros(),
                    "rss_warm_kib": baseline, "rss_final_kib": rss()
                })
        );
    }
    timer.remove();
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
