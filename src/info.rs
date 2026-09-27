use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

pub fn button(text: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.add_css_class("network-action");
    button
}

pub fn calendar() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("calendar");
    let today = glib::DateTime::now_local().expect("local date");
    let selected = label(&today.format("%A, %e %B %Y").unwrap(), "menu-hint");
    let month = Rc::new(RefCell::new(
        glib::DateTime::from_local(today.year(), today.month(), 1, 12, 0, 0.0).unwrap(),
    ));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let previous = button("‹");
    let heading = label("", "menu-title");
    heading.set_hexpand(true);
    heading.set_xalign(0.5);
    let next = button("›");
    let reset = button("Today");
    controls.append(&previous);
    controls.append(&heading);
    controls.append(&next);
    controls.append(&reset);
    outer.append(&controls);
    let grid = gtk::Grid::builder()
        .column_homogeneous(true)
        .row_homogeneous(true)
        .column_spacing(5)
        .row_spacing(5)
        .build();
    outer.append(&grid);
    outer.append(&selected);
    let render: Rc<dyn Fn()> = Rc::new({
        let grid = grid.downgrade();
        let heading = heading.downgrade();
        let selected = selected.downgrade();
        let month = month.clone();
        move || {
            let (Some(grid), Some(heading), Some(selected)) =
                (grid.upgrade(), heading.upgrade(), selected.upgrade())
            else {
                return;
            };
            while let Some(child) = grid.first_child() {
                grid.remove(&child);
            }
            let date = month.borrow().clone();
            heading.set_text(&date.format("%B %Y").unwrap());
            for (column, day) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                .iter()
                .enumerate()
            {
                let day = label(day, "calendar-weekday");
                day.set_xalign(0.5);
                grid.attach(&day, column as i32, 0, 1, 1);
            }
            let today = glib::DateTime::now_local().unwrap();
            let start = date.add_days(1 - date.day_of_week()).unwrap();
            for index in 0..42 {
                let day = start.add_days(index).unwrap();
                let cell = button(&day.day_of_month().to_string());
                cell.add_css_class("calendar-day");
                if day.month() != date.month() {
                    cell.add_css_class("outside");
                }
                if (day.year(), day.day_of_year()) == (today.year(), today.day_of_year()) {
                    cell.add_css_class("today");
                }
                let selected = selected.downgrade();
                cell.connect_clicked(move |_| {
                    if let Some(selected) = selected.upgrade() {
                        selected.set_text(&day.format("%A, %e %B %Y").unwrap());
                    }
                });
                grid.attach(&cell, index % 7, index / 7 + 1, 1, 1);
            }
        }
    });
    for (control, offset) in [(previous, -1), (next, 1), (reset, 0)] {
        let month = month.clone();
        let render = render.clone();
        control.connect_clicked(move |_| {
            let next = if offset == 0 {
                glib::DateTime::now_local().and_then(|now| {
                    glib::DateTime::from_local(now.year(), now.month(), 1, 12, 0, 0.0)
                })
            } else {
                month.borrow().add_months(offset)
            };
            if let Ok(next) = next {
                *month.borrow_mut() = next;
                render();
            }
        });
    }
    render();
    outer
}
