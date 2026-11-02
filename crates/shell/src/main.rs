mod app;
mod appsctl;
mod bar;
mod card;
mod clipctl;
mod cursorctl;
mod history;
mod icons;
mod ipc;
mod logger;
mod memory;
mod model;
mod netctl;
mod notify;
mod osd;
mod overlay;
mod panels;
mod popups;
mod powerctl;
mod state;
#[allow(dead_code)]
mod theme;
mod themectl;
mod tooltip;
mod trayctl;
mod wallctl;
mod wallpaper;

use calloop::{
    EventLoop,
    channel::{self, Channel},
    signals::{Signal, Signals},
};
use platform::Platform;

use crate::app::{App, ServiceEvent};

pub const APP_NAME: &str = "idk-yet";

fn run() -> Result<(), Box<dyn std::error::Error>> {
    memory::configure();
    if ipc::instance_running() {
        log::info!("another instance is running");
        return Ok(());
    }
    let listener = ipc::listen()?;
    let mut event_loop: EventLoop<'static, App> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM])?;
    handle.insert_source(signals, |_, _, app: &mut App| app.running = false)?;
    let (platform, source) = Platform::connect()?;
    handle.insert_source(source, |_, queue, app: &mut App| {
        queue.dispatch_pending(&mut app.platform)
    })?;
    let (executor, scheduler) = calloop::futures::executor::<()>()?;
    handle.insert_source(executor, |_, _, _| {})?;
    let (sender, events): (_, Channel<ServiceEvent>) = channel::channel();
    handle.insert_source(events, |event, _, app: &mut App| {
        if let channel::Event::Msg(event) = event {
            app.service(event);
        }
    })?;
    let mut app = App::new(platform, handle.clone(), scheduler, sender);
    app.listen_ipc(listener);
    app.start_services();
    while app.running {
        let timeout = app.timeout();
        event_loop.dispatch(timeout, &mut app)?;
        app.process();
    }
    let _ = std::fs::remove_file(ipc::socket_path());
    Ok(())
}

fn main() {
    logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("msg") {
        std::process::exit(ipc::client(&args[1..]));
    }
    if let Err(err) = run() {
        log::error!("{err}");
        std::process::exit(1);
    }
}
