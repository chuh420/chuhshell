use gtk::prelude::*;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub fn pump(milliseconds: u64) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
#[ignore = "requires scripts/check-headless.sh"]
fn ui_regressions() {
    gtk::init().unwrap();
    crate::css::install();
    let app = gtk::Application::builder()
        .application_id("dev.chuh.chuhshell.tests")
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    crate::launcher::regression_checks(&app);
    crate::notification_center::regression_checks(&app);
    let state = Rc::new(crate::app::AppState::default());
    let center = crate::notification_center::NotificationCenter::new(&app);
    crate::bar::create(&app, &state, &center);
    pump(300);
    assert_eq!(state.bars.borrow().len(), 2);
    crate::bar::create(&app, &state, &center);
    assert_eq!(state.bars.borrow().len(), 2);
    crate::menu::regression_checks(&app, &state);
    crate::layout::regression_checks(&app, &state);
    crate::bar_editor::regression_checks(&app, &state);
    center.toggle_drawer();
    pump(100);
    state.background_manager.borrow().as_ref().unwrap().toggle();
    pump(200);
    center.toggle_drawer();
    pump(200);
    center.toggle_drawer();
    pump(100);
    crate::process::shutdown();
    for (_, window) in state.bars.borrow().iter() {
        window.close();
    }
    pump(100);
}
