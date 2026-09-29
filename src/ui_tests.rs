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

fn find_button(widget: &gtk::Widget, class: &str) -> Option<gtk::Button> {
    if widget.has_css_class(class) {
        return widget.clone().downcast().ok();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = find_button(&widget, class) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
#[ignore = "requires scripts/check-headless.sh"]
fn ui_regressions() {
    gtk::init().unwrap();
    let provider = gtk::CssProvider::new();
    let css_errors = Rc::new(std::cell::RefCell::new(Vec::new()));
    provider.connect_parsing_error({
        let errors = css_errors.clone();
        move |_, _, error| errors.borrow_mut().push(error.to_string())
    });
    provider.load_from_data(crate::css::CSS);
    assert!(css_errors.borrow().is_empty(), "{:?}", css_errors.borrow());
    crate::css::install();
    let app = gtk::Application::builder()
        .application_id("dev.chuh.chuhshell.tests")
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    let settings = gtk::Settings::default().unwrap();
    let animations = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(true);
    let closing_window = gtk::Window::builder().application(&app).build();
    let close_requests = Rc::new(std::cell::Cell::new(0));
    closing_window.connect_close_request({
        let count = close_requests.clone();
        move |_| {
            count.set(count.get() + 1);
            glib::Propagation::Proceed
        }
    });
    crate::ui::animate_close(&closing_window);
    closing_window.present();
    pump(100);
    closing_window.close();
    assert!(closing_window.has_css_class("shell-closing"));
    assert!(!closing_window.can_target());
    pump(200);
    assert!(!closing_window.is_visible());
    assert_eq!(close_requests.get(), 1);
    settings.set_gtk_enable_animations(false);
    let immediate_window = gtk::Window::builder().application(&app).build();
    crate::ui::animate_close(&immediate_window);
    immediate_window.present();
    pump(100);
    immediate_window.close();
    assert!(!immediate_window.is_visible());
    assert!(!immediate_window.has_css_class("shell-closing"));
    settings.set_gtk_enable_animations(animations);
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
        .map(|(_, window)| find_button(window.upcast_ref(), "notification-toggle").unwrap())
        .collect();
    crate::ui::set_active_output(state.bars.borrow()[1].0.connector().as_deref());
    assert_eq!(
        crate::ui::active_button(&buttons.iter().map(|b| b.downgrade()).collect::<Vec<_>>()),
        Some(buttons[1].clone())
    );
    crate::ui::set_active_output(None);
    let network_button = find_button(state.bars.borrow()[0].1.upcast_ref(), "network").unwrap();
    let network_menu = state.network_menu.borrow().as_ref().unwrap().clone();
    for _ in 0..2 {
        network_menu.toggle(&network_button);
        pump(300);
        assert!(network_button.has_css_class("popup-open"));
        let popover = network_button
            .last_child()
            .unwrap()
            .downcast::<gtk::Popover>()
            .unwrap();
        assert!(popover.has_css_class("bar-attached"));
        assert!(popover.is_mapped());
        capture("wifi-attached");
        crate::ui::close_popover();
        pump(250);
        assert!(!network_button.has_css_class("popup-open"));
        assert!(popover.parent().is_none());
    }
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
    capture("notification-drawer");
    state.background_manager.borrow().as_ref().unwrap().toggle();
    pump(200);
    capture("background-apps");
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
