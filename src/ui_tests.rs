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

fn confirmed_settings_regression() {
    let path = crate::config::path();
    let original = std::fs::read(&path).unwrap();
    let original_limit = crate::config::get().notification_history_limit;
    glib::MainContext::default()
        .block_on(crate::config::save_value_async(
            "notification_history_limit",
            25.into(),
        ))
        .unwrap();
    assert_eq!(crate::config::get().notification_history_limit, 25);
    std::fs::write(&path, b"invalid").unwrap();
    assert!(
        glib::MainContext::default()
            .block_on(crate::config::save_value_async(
                "notification_history_limit",
                50.into()
            ))
            .is_err()
    );
    assert_eq!(crate::config::get().notification_history_limit, 25);
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
    std::fs::write(&path, original).unwrap();
    glib::MainContext::default()
        .block_on(crate::config::save_value_async(
            "notification_history_limit",
            original_limit.into(),
        ))
        .unwrap();
}

fn command_forwarding_regression() {
    let binary = std::env::var_os("CHUHSHELL_TEST_BINARY").unwrap();
    let directory = crate::paths::config().join("command-forwarding");
    let config = directory.join("chuhshell/config.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, b"{}").unwrap();
    let log = std::fs::File::create(directory.join("shell.log")).unwrap();
    let mut primary = crate::process::ManagedChild::spawn(
        std::process::Command::new(&binary)
            .env("XDG_CONFIG_HOME", &directory)
            .stdout(log.try_clone().unwrap())
            .stderr(log),
    )
    .unwrap();
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let owned = connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&("dev.chuh.chuhshell",).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            )
            .unwrap()
            .get::<(bool,)>()
            .unwrap()
            .0;
        if owned {
            break;
        }
        assert!(Instant::now() < deadline, "primary shell did not register");
        assert!(
            primary.0.try_wait().unwrap().is_none(),
            "primary shell exited"
        );
        pump(10);
    }
    crate::process::run_command(
        std::process::Command::new(&binary)
            .arg("volume-up")
            .env("XDG_CONFIG_HOME", &directory),
        Duration::from_secs(10),
    )
    .unwrap();
    std::fs::write(&config, b"invalid").unwrap();
    crate::process::run_command(
        std::process::Command::new(&binary)
            .arg("volume-up")
            .env("XDG_CONFIG_HOME", &directory),
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(std::fs::read(&config).unwrap(), b"invalid");
    unsafe {
        libc::kill(primary.0.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while primary.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "primary shell did not shut down");
        pump(10);
    }
    assert!(
        crate::process::run_command(
            std::process::Command::new(&binary).env("XDG_CONFIG_HOME", &directory),
            Duration::from_secs(10)
        )
        .is_err()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires scripts/check-headless.sh"]
fn ui_regressions() {
    crate::config::initialize().unwrap();
    gtk::init().unwrap();
    command_forwarding_regression();
    confirmed_settings_regression();
    crate::apps::launch_regression();
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
    let heartbeats = Rc::new(std::cell::Cell::new(0));
    let heartbeat = glib::timeout_add_local(Duration::from_millis(5), {
        let heartbeats = heartbeats.clone();
        move || {
            heartbeats.set(heartbeats.get() + 1);
            glib::ControlFlow::Continue
        }
    });
    let delayed = crate::storage::run(|| {
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    });
    pump(80);
    assert!(heartbeats.get() >= 5);
    glib::MainContext::default().block_on(delayed).unwrap();
    heartbeat.remove();
    crate::idle::regression_checks();
    crate::launcher::regression_checks(&app);
    crate::bar::regression_checks();
    crate::notification_center::regression_checks(&app);
    let state = Rc::new(crate::app::AppState::default());
    let center = crate::notification_center::NotificationCenter::new(&app);
    crate::reminder::start(&state, &center);
    crate::bar::create(&app, &state, &center);
    pump(300);
    assert_eq!(state.bars.borrow().len(), 2);
    crate::bar::create(&app, &state, &center);
    assert_eq!(state.bars.borrow().len(), 2);
    let services = state.services.borrow().as_ref().unwrap().clone();
    let stable_bar = state.bars.borrow()[0].1.clone();
    let connector = state.bars.borrow()[1].0.connector().unwrap();
    let change_output = |enabled: bool| {
        let status = std::process::Command::new("wlr-randr")
            .args([
                "--output",
                connector.as_str(),
                if enabled { "--on" } else { "--off" },
            ])
            .status()
            .unwrap();
        assert!(status.success());
        pump(300);
    };
    change_output(false);
    assert_eq!(state.bars.borrow().len(), 1);
    assert_eq!(state.bars.borrow()[0].1, stable_bar);
    change_output(true);
    assert_eq!(state.bars.borrow().len(), 2);
    assert!(Rc::ptr_eq(
        state.services.borrow().as_ref().unwrap(),
        &services
    ));
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
    crate::network::regression_checks(&network_button);
    let battery_button = find_button(state.bars.borrow()[0].1.upcast_ref(), "battery").unwrap();
    crate::power::regression_checks(&battery_button);
    let temperature_button =
        find_button(state.bars.borrow()[0].1.upcast_ref(), "temperature").unwrap();
    crate::monitor::regression_checks(&temperature_button);
    crate::controls::regression_checks(&temperature_button);
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
    soak(&app, &state, &center);
    crate::idle::shutdown(&state);
    crate::storage::shutdown();
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
    pump(120);
    let status = std::process::Command::new("grim")
        .arg(directory.join(format!("{name}.png")))
        .status()
        .unwrap();
    assert!(status.success());
    if let Some(baselines) = std::env::var_os("CHUHSHELL_TEST_BASELINES") {
        let baseline = std::path::PathBuf::from(baselines).join(format!("{name}.png"));
        let expected = gtk::gdk::Texture::from_file(&gio::File::for_path(&baseline))
            .unwrap_or_else(|error| panic!("Missing baseline {}: {error}", baseline.display()));
        let actual = gtk::gdk::Texture::from_file(&gio::File::for_path(
            directory.join(format!("{name}.png")),
        ))
        .unwrap();
        assert_eq!(
            (actual.width(), actual.height()),
            (expected.width(), expected.height()),
            "{name}: screenshot dimensions changed"
        );
        let stride = actual.width() as usize * 4;
        let mut left = vec![0u8; stride * actual.height() as usize];
        let mut right = left.clone();
        actual.download(&mut left, stride);
        expected.download(&mut right, stride);
        let changed = left
            .as_chunks::<4>()
            .0
            .iter()
            .zip(right.as_chunks::<4>().0.iter())
            .filter(|(left, right)| {
                left.iter()
                    .zip(right.iter())
                    .any(|(left, right)| left.abs_diff(*right) > 24)
            })
            .count();
        let fraction = changed as f64 / (actual.width() * actual.height()) as f64;
        assert!(
            fraction <= 0.01,
            "{name}: {:.2}% changed pixels exceeds 1% tolerance",
            fraction * 100.0
        );
    }
}

fn soak(
    app: &gtk::Application,
    state: &Rc<crate::app::AppState>,
    center: &Rc<crate::notification_center::NotificationCenter>,
) {
    let seconds = std::env::var("CHUHSHELL_SOAK_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let started = Instant::now();
    let mut iteration = 0u64;
    while started.elapsed().as_secs() < seconds {
        center.stress();
        state.services.borrow().as_ref().unwrap().stress(iteration);
        if iteration.is_multiple_of(30) {
            crate::launcher::show(app, state, crate::app::LauncherMode::Normal);
            pump(50);
            let window = state.launcher.borrow().clone();
            if let Some(window) = window {
                window.close();
            }
            crate::menu::show(app, state);
            pump(50);
            crate::menu::close(state);
        }
        assert_eq!(state.bars.borrow().len(), 2);
        if iteration.is_multiple_of(60) {
            let memory = std::fs::read_to_string("/proc/self/status").unwrap();
            let rss = memory
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .unwrap();
            println!(
                "CHUHSHELL_SOAK {}",
                serde_json::json!({"elapsed_seconds": started.elapsed().as_secs(), "iteration": iteration, "rss": rss})
            );
        }
        iteration += 1;
        pump(1000);
    }
    if seconds > 0 {
        println!(
            "CHUHSHELL_SOAK {}",
            serde_json::json!({"elapsed_seconds": started.elapsed().as_secs(), "iterations": iteration, "complete": true})
        );
    }
}
