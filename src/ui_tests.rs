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
    crate::bar::regression_checks();
    crate::notification_center::regression_checks(&app);
    let state = Rc::new(crate::app::AppState::default());
    let center = crate::notification_center::NotificationCenter::new(&app);
    crate::bar::create(&app, &state, &center);
    pump(300);
    assert_eq!(state.bars.borrow().len(), 2);
    crate::bar::create(&app, &state, &center);
    assert_eq!(state.bars.borrow().len(), 2);
    let buttons: Vec<_> = state
        .bars
        .borrow()
        .iter()
        .map(|(_, window)| {
            fn find(widget: &gtk::Widget) -> Option<gtk::Button> {
                if widget.has_css_class("notification-toggle") {
                    return widget.clone().downcast().ok();
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    if let Some(button) = find(&widget) {
                        return Some(button);
                    }
                    child = widget.next_sibling();
                }
                None
            }
            find(window.upcast_ref()).unwrap()
        })
        .collect();
    crate::ui::set_active_output(state.bars.borrow()[1].0.connector().as_deref());
    assert_eq!(
        crate::ui::active_button(&buttons.iter().map(|b| b.downgrade()).collect::<Vec<_>>()),
        Some(buttons[1].clone())
    );
    crate::ui::set_active_output(None);
    crate::menu::regression_checks(&app, &state);
    crate::clipboard::regression_checks(&app);
    crate::keybindings::regression_checks(&app);
    crate::bluetooth::regression_checks(&app);
    crate::network::regression_checks();
    crate::weather::regression_checks(&app);
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
    crate::launcher::profile(&app);
    crate::process::shutdown();
    for (_, window) in state.bars.borrow().iter() {
        window.close();
    }
    pump(100);
}

pub fn capture(name: &str) {
    let Some(directory) = std::env::var_os("CHUHSHELL_TEST_SCREENSHOTS") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let status = std::process::Command::new("grim")
        .arg(directory.join(format!("{name}.png")))
        .status()
        .unwrap();
    assert!(status.success());
}
