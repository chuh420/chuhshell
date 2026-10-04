use super::{Command, Settings};
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::sync::mpsc;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
    wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};

struct Screen {
    dirty: std::cell::Cell<bool>,
    output: wl_output::WlOutput,
    surface: wl_surface::WlSurface,
    role: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    size: (u32, u32),
}

type RetiredScreens = (
    Vec<Screen>,
    Vec<(wl_buffer::WlBuffer, wl_surface::WlSurface)>,
);

struct State {
    connection: Connection,
    qh: QueueHandle<Self>,
    retired: std::collections::HashMap<u64, RetiredScreens>,
    next_retired: u64,
    buffers: std::cell::RefCell<Vec<(wl_buffer::WlBuffer, wl_surface::WlSurface)>>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    outputs: Vec<wl_output::WlOutput>,
    notifications: Vec<ext_idle_notification_v1::ExtIdleNotificationV1>,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    locked: bool,
    stopping: bool,
    synced: bool,
    screens: Vec<Screen>,
}

impl State {
    fn timers(&mut self, settings: Settings, qh: &QueueHandle<Self>) {
        for notification in self.notifications.drain(..) {
            notification.destroy();
        }
        if let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) {
            let timer = settings.screensaver;
            if timer.enabled {
                self.notifications.push(notifier.get_idle_notification(
                    timer.minutes.clamp(1, 1440) * 60_000,
                    seat,
                    qh,
                    (),
                ));
            }
        }
    }

    fn show(&mut self, qh: &QueueHandle<Self>) {
        if self.stopping || self.lock.is_some() {
            return;
        }
        self.lock = Some(self.manager.as_ref().unwrap().lock(qh, ()));
        for output in self.outputs.clone() {
            self.add_screen(output, qh);
        }
    }

    fn add_screen(&mut self, output: wl_output::WlOutput, qh: &QueueHandle<Self>) {
        let surface = self.compositor.as_ref().unwrap().create_surface(qh, ());
        let role = self
            .lock
            .as_ref()
            .unwrap()
            .get_lock_surface(&surface, &output, qh, ());
        self.screens.push(Screen {
            dirty: std::cell::Cell::new(true),
            output,
            surface,
            role,
            size: (0, 0),
        });
    }

    fn unlock(&mut self) {
        if !self.locked {
            return;
        }
        if let Some(lock) = self.lock.take() {
            lock.unlock_and_destroy();
        }
        let serial = self.next_retired;
        self.next_retired += 1;
        self.retired.insert(
            serial,
            (
                std::mem::take(&mut self.screens),
                std::mem::take(&mut *self.buffers.borrow_mut()),
            ),
        );
        self.connection.display().sync(&self.qh, serial);
        self.locked = false;
    }

    fn activity(&mut self) {
        self.unlock();
    }

    fn redraw(&self, qh: &QueueHandle<Self>) {
        for screen in &self.screens {
            screen.dirty.set(true);
        }
        self.paint_pending(qh);
    }

    fn paint_pending(&self, qh: &QueueHandle<Self>) {
        for screen in &self.screens {
            if !screen.dirty.get()
                || self
                    .buffers
                    .borrow()
                    .iter()
                    .filter(|(_, surface)| *surface == screen.surface)
                    .count()
                    >= 2
            {
                continue;
            }
            if screen.size.0 == 0 || screen.size.1 == 0 {
                continue;
            }
            if let Err(error) = self.draw(screen, qh) {
                eprintln!("chuhshell: screensaver rendering: {error}");
            }
        }
    }

    fn draw(
        &self,
        screen: &Screen,
        qh: &QueueHandle<Self>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (width, height) = screen.size;
        if width > 16384 || height > 16384 || u64::from(width) * u64::from(height) > 33_554_432 {
            return Err("Screensaver surface exceeds rendering limit".into());
        }
        let mut image = gtk::cairo::ImageSurface::create(
            gtk::cairo::Format::ARgb32,
            width as i32,
            height as i32,
        )?;
        let context = gtk::cairo::Context::new(&image)?;
        context.set_source_rgb(25.0 / 255.0, 23.0 / 255.0, 36.0 / 255.0);
        context.paint()?;
        context.select_font_face(
            "monospace",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        context.set_source_rgb(196.0 / 255.0, 167.0 / 255.0, 231.0 / 255.0);
        let art: Vec<_> = include_str!("../../assets/idle-art.txt").lines().collect();
        context.set_font_size(10.0);
        let max_width = art
            .iter()
            .map(|line| context.text_extents(line).map(|e| e.x_advance()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .fold(1.0, f64::max);
        let font = (f64::from(width) * 0.88 / max_width * 10.0).min(f64::from(height) / 20.0);
        context.set_font_size(font);
        let art_x = (f64::from(width) - max_width * font / 10.0) / 2.0;
        for (index, line) in art.iter().enumerate() {
            context.move_to(
                art_x,
                f64::from(height) / 2.0 + (index as f64 - 4.0) * font * 1.15,
            );
            context.show_text(line)?;
        }
        drop(context);
        image.flush();
        let stride = image.stride();
        let bytes = image.data()?;
        let fd =
            unsafe { libc::memfd_create(c"chuhshell-screensaver".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(&bytes)?;
        let pool = self
            .shm
            .as_ref()
            .unwrap()
            .create_pool(file.as_fd(), bytes.len() as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        pool.destroy();
        self.buffers
            .borrow_mut()
            .push((buffer.clone(), screen.surface.clone()));
        screen.dirty.set(false);
        screen.surface.attach(Some(&buffer), 0, 0);
        screen.surface.damage(0, 0, width as i32, height as i32);
        screen.surface.commit();
        Ok(())
    }
}

fn connect()
-> Result<(Connection, wayland_client::EventQueue<State>, State), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State {
        connection: connection.clone(),
        qh: qh.clone(),
        retired: Default::default(),
        next_retired: 0,
        buffers: Default::default(),
        keyboard: None,
        pointer: None,
        touch: None,
        compositor: None,
        shm: None,
        manager: None,
        notifier: None,
        seat: None,
        outputs: Vec::new(),
        notifications: Vec::new(),
        lock: None,
        locked: false,
        stopping: false,
        synced: false,
        screens: Vec::new(),
    };
    queue.roundtrip(&mut state)?;
    if state.manager.is_none()
        || state.compositor.is_none()
        || state.shm.is_none()
        || state.notifier.is_none()
        || state.seat.is_none()
    {
        return Err("Compositor lacks session-lock or idle-notify support".into());
    }
    Ok((connection, queue, state))
}

fn completed_io<T>(
    result: Result<T, wayland_client::backend::WaylandError>,
) -> Result<bool, wayland_client::backend::WaylandError> {
    match result {
        Ok(_) => Ok(true),
        Err(wayland_client::backend::WaylandError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn run(
    settings: Settings,
    receiver: mpsc::Receiver<Command>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, mut queue, mut state) = connect()?;
    let qh = queue.handle();
    state.timers(settings, &qh);
    let mut deadline = None;
    let mut sync_sent = false;
    let mut warned = false;
    loop {
        queue.dispatch_pending(&mut state)?;
        state.stopping |= super::worker::stopping();
        for _ in 0..16 {
            match receiver.try_recv() {
                Ok(Command::Settings(settings)) => state.timers(settings, &qh),
                Ok(Command::Show) => state.show(&qh),
                Ok(Command::Stop) => state.stopping = true,
                #[cfg(test)]
                Ok(Command::ReleaseScreensaver) => state.activity(),
                Ok(Command::Check(sender)) => {
                    let _ = sender.send(state.locked);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.stopping = true;
                    break;
                }
            }
        }
        if state.stopping {
            let limit = deadline.get_or_insert_with(|| {
                std::time::Instant::now() + std::time::Duration::from_secs(5)
            });
            for notification in state.notifications.drain(..) {
                notification.destroy();
            }
            state.unlock();
            if state.lock.is_none() && state.retired.is_empty() && !sync_sent {
                connection.display().sync(&qh, ());
                sync_sent = true;
            }
            if state.synced {
                return Ok(());
            }
            if std::time::Instant::now() >= *limit && !warned {
                eprintln!(
                    "chuhshell: compositor is slow to release screensaver; lock client will keep waiting"
                );
                warned = true;
            }
        }
        let flushed = completed_io(connection.flush())?;
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd {
                fd: connection.as_fd().as_raw_fd(),
                events: libc::POLLIN | if flushed { 0 } else { libc::POLLOUT },
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut fd, 1, 100) };
            if result > 0 && fd.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
                completed_io(guard.read())?;
            } else if result < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, u64> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        serial: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some((screens, buffers)) = state.retired.remove(serial) {
            for screen in screens {
                screen.role.destroy();
                screen.surface.destroy();
            }
            for (buffer, _) in buffers {
                if buffer.is_alive() {
                    buffer.destroy();
                }
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.synced = true;
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(5), qh, ()))
                }
                "ext_session_lock_manager_v1" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_idle_notifier_v1" => state.notifier = Some(registry.bind(name, 1, qh, ())),
                "wl_output" => {
                    let output =
                        registry.bind::<wl_output::WlOutput, _, _>(name, version.min(3), qh, name);
                    if state.lock.is_some() {
                        state.add_screen(output.clone(), qh);
                    }
                    state.outputs.push(output);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|output| {
                    if output.data::<u32>() == Some(&name) {
                        if output.version() >= 3 {
                            output.release();
                        }
                        false
                    } else {
                        true
                    }
                });
                state.screens.retain(|screen| {
                    if screen.output.data::<u32>() == Some(&name) {
                        screen.role.destroy();
                        screen.surface.destroy();
                        state.buffers.borrow_mut().retain(|(buffer, surface)| {
                            if *surface == screen.surface {
                                buffer.destroy();
                                false
                            } else {
                                true
                            }
                        });
                        false
                    } else {
                        true
                    }
                });
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Touch) && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            }
            if !capabilities.contains(wl_seat::Capability::Keyboard)
                && let Some(keyboard) = state.keyboard.take()
            {
                keyboard.release();
            }
            if !capabilities.contains(wl_seat::Capability::Pointer)
                && let Some(pointer) = state.pointer.take()
            {
                pointer.release();
            }
            if !capabilities.contains(wl_seat::Capability::Touch)
                && let Some(touch) = state.touch.take()
            {
                touch.release();
            }
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if matches!(event, ext_idle_notification_v1::Event::Idled) {
            state.show(qh);
        } else if matches!(event, ext_idle_notification_v1::Event::Resumed) {
            state.activity();
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.locked = true,
            ext_session_lock_v1::Event::Finished => {
                if state.locked {
                    state.unlock();
                    return;
                }
                if let Some(lock) = state.lock.take() {
                    lock.destroy();
                }
                eprintln!("chuhshell: compositor refused session lock");
                for (buffer, _) in state.buffers.borrow_mut().drain(..) {
                    buffer.destroy();
                }
                for screen in state.screens.drain(..) {
                    screen.role.destroy();
                    screen.surface.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        role: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            role.ack_configure(serial);
            if let Some(screen) = state.screens.iter_mut().find(|screen| screen.role == *role) {
                screen.size = (width, height);
            }
            state.redraw(qh);
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            state: wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed),
            ..
        } = event
        {
            state.activity();
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(
            event,
            wl_pointer::Event::Motion { .. }
                | wl_pointer::Event::Button { .. }
                | wl_pointer::Event::Axis { .. }
        ) {
            state.activity();
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            state
                .buffers
                .borrow_mut()
                .retain(|(pending, _)| pending != buffer);
            buffer.destroy();
            state.paint_pending(qh);
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_surface::WlSurface);
impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(State: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
delegate_noop!(State: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(
            event,
            wl_touch::Event::Down { .. } | wl_touch::Event::Motion { .. }
        ) {
            state.activity();
        }
    }
}

#[cfg(test)]
pub(super) fn regression_checks() {
    let (_connection, mut queue, mut state) = connect().unwrap();
    let qh = queue.handle();
    queue.roundtrip(&mut state).unwrap();
    state.show(&qh);
    for _ in 0..3 {
        queue.roundtrip(&mut state).unwrap();
    }
    assert!(state.locked);
    assert_eq!(state.screens.len(), 2);
    assert!(state.screens.iter().all(|screen| screen.size.0 > 0));
    crate::ui_tests::capture("screensaver");
    state.timers(Settings::default(), &qh);
    assert_eq!(state.notifications.len(), 1);
    state.activity();
    assert!(!state.locked);
    queue.roundtrip(&mut state).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let settings = Settings {
        screensaver: super::Timer {
            enabled: false,
            minutes: 15,
        },
    };
    let worker =
        std::thread::spawn(move || run(settings, receiver).map_err(|error| error.to_string()));
    let wait_locked = |expected| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let (reply, response) = mpsc::channel();
            sender.send(Command::Check(reply)).unwrap();
            if response
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                == expected
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Idle worker did not reach expected state"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    for _ in 0..20 {
        sender.send(Command::Show).unwrap();
        wait_locked(true);
        sender.send(Command::ReleaseScreensaver).unwrap();
        wait_locked(false);
    }
    sender.send(Command::Show).unwrap();
    wait_locked(true);
    drop(sender);
    worker.join().unwrap().unwrap();
    state.show(&qh);
    for _ in 0..3 {
        queue.roundtrip(&mut state).unwrap();
    }
    assert!(state.locked, "Disconnected worker left compositor locked");
    state.unlock();
    queue.roundtrip(&mut state).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let worker = std::thread::spawn(move || run(settings, receiver).map_err(|e| e.to_string()));
    sender.send(Command::Show).unwrap();
    sender.send(Command::Stop).unwrap();
    worker.join().unwrap().unwrap();
    state.show(&qh);
    for _ in 0..3 {
        queue.roundtrip(&mut state).unwrap();
    }
    assert!(
        state.locked,
        "Stopping during lock acquisition left compositor locked"
    );
    let (_, mut refused_queue, mut refused) = connect().unwrap();
    refused.show(&refused_queue.handle());
    for _ in 0..3 {
        refused_queue.roundtrip(&mut refused).unwrap();
    }
    assert!(refused.lock.is_none());
    assert!(!refused.locked);
    let lock = state.lock.clone().unwrap();
    <State as Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()>>::event(
        &mut state,
        &lock,
        ext_session_lock_v1::Event::Finished,
        &(),
        &_connection,
        &qh,
    );
    queue.roundtrip(&mut state).unwrap();
    assert!(!state.locked);
    assert!(state.lock.is_none());
    state.show(&qh);
    for _ in 0..3 {
        queue.roundtrip(&mut state).unwrap();
    }
    assert!(
        state.locked,
        "Finished after Locked did not release session lock"
    );
    state.unlock();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();
    for signal in [libc::SIGKILL, libc::SIGABRT] {
        let ready = crate::paths::state().join(format!("idle-crash-ready-{signal}"));
        std::fs::create_dir_all(ready.parent().unwrap()).unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "idle::tests::crash_controller",
                "--ignored",
                "--nocapture",
            ])
            .env("CHUHSHELL_IDLE_TEST_READY", &ready)
            .stdout(std::process::Stdio::null());
        let mut controller = crate::process::ManagedChild::spawn(&mut command).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                controller.0.try_wait().unwrap().is_none(),
                "Crash controller failed"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "Crash controller did not lock"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let helper_pid: i32 = std::fs::read_to_string(&ready).unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(controller.0.id() as i32, signal) }, 0);
        controller.0.wait().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            state.show(&qh);
            for _ in 0..3 {
                queue.roundtrip(&mut state).unwrap();
            }
            if state.locked {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Parent crash left compositor locked"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        state.unlock();
        queue.roundtrip(&mut state).unwrap();
        queue.roundtrip(&mut state).unwrap();
        while let Ok(stat) = std::fs::read_to_string(format!("/proc/{helper_pid}/stat")) {
            if stat
                .rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('Z')
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Orphaned lock client did not exit"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::remove_file(ready).unwrap();
    }
    let mut child = super::worker::spawn().unwrap();
    super::worker::show_and_check(&mut child);
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    assert!(child.finish().unwrap().success());
    state.show(&qh);
    for _ in 0..3 {
        queue.roundtrip(&mut state).unwrap();
    }
    assert!(state.locked, "SIGTERM left compositor locked");
    state.unlock();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();
}

#[cfg(test)]
mod tests {
    use super::completed_io;
    use wayland_client::backend::WaylandError;

    #[test]
    fn temporary_socket_errors_keep_the_worker_alive() {
        assert!(
            !completed_io::<()>(Err(WaylandError::Io(std::io::Error::from_raw_os_error(
                libc::EAGAIN
            ))))
            .unwrap()
        );
        assert!(
            !completed_io::<()>(Err(WaylandError::Io(std::io::Error::from_raw_os_error(
                libc::EINTR
            ))))
            .unwrap()
        );
        assert!(
            completed_io::<()>(Err(WaylandError::Io(std::io::Error::from_raw_os_error(
                libc::EPIPE
            ))))
            .is_err()
        );
        assert!(completed_io(Ok(())).unwrap());
    }
}
