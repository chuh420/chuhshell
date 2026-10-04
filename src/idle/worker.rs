use super::{Command, Settings};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize, serde::Deserialize)]
enum Message {
    Settings(Settings),
    Show,
    Stop,
    Check,
}

extern "C" fn stop_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

pub(super) fn stopping() -> bool {
    STOP.load(Ordering::Relaxed)
}

pub(super) fn run() -> Result<(), String> {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        if unsafe { libc::signal(signal, stop_signal as *const () as libc::sighandler_t) }
            == libc::SIG_ERR
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    let (sender, receiver) = mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        loop {
            let mut line = Vec::new();
            let result = std::io::Read::take(&mut input, 4097).read_until(b'\n', &mut line);
            if !matches!(result, Ok(1..)) {
                break;
            }
            if line.len() > 4096 {
                eprintln!("chuhshell: idle command exceeds size limit");
                break;
            }
            let command = match serde_json::from_slice::<Message>(&line) {
                Ok(Message::Settings(settings)) => Command::Settings(settings),
                Ok(Message::Show) => Command::Show,
                Ok(Message::Stop) => Command::Stop,
                Ok(Message::Check) => {
                    let (reply, response) = mpsc::channel();
                    if sender.send(Command::Check(reply)).is_err() {
                        break;
                    }
                    let Ok(locked) = response.recv_timeout(Duration::from_secs(3)) else {
                        break;
                    };
                    let mut output = std::io::stdout().lock();
                    if writeln!(output, "idle-locked={locked}")
                        .and_then(|_| output.flush())
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                Err(_) => {
                    eprintln!("chuhshell: invalid idle command");
                    break;
                }
            };
            let stop = matches!(command, Command::Stop);
            if sender.send(command).is_err() || stop {
                break;
            }
        }
    });
    super::wayland::run(
        Settings {
            screensaver: super::Timer {
                enabled: false,
                minutes: 15,
            },
        },
        receiver,
    )
    .map_err(|error| error.to_string())
}

pub(super) fn spawn() -> Result<crate::process::IdleChild, String> {
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    #[cfg(not(test))]
    command.arg("--idle-worker");
    #[cfg(test)]
    command.args([
        "--exact",
        "idle::tests::worker_process",
        "--ignored",
        "--nocapture",
    ]);
    crate::process::IdleChild::spawn(&mut command)
}

fn send(child: &mut crate::process::IdleChild, message: Message) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    child.send(&bytes)
}

pub(super) fn supervise(
    settings: Settings,
    receiver: mpsc::Receiver<Command>,
) -> Result<(), String> {
    let mut child = spawn()?;
    send(&mut child, Message::Settings(settings))?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(format!("Idle worker exited unexpectedly: {status}"));
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Settings(settings)) => send(&mut child, Message::Settings(settings))?,
            Ok(Command::Show) => send(&mut child, Message::Show)?,
            Ok(Command::Check(reply)) => {
                send(&mut child, Message::Check)?;
                let locked = child.locked()?;
                let _ = reply.send(locked);
            }
            #[cfg(test)]
            Ok(Command::ReleaseScreensaver) => {
                return Err("Release is only supported by the in-process test worker".into());
            }
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                send(&mut child, Message::Stop)?;
                let status = child.finish()?;
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("Idle worker failed: {status}"))
                };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
pub(super) fn show_and_check(child: &mut crate::process::IdleChild) {
    send(child, Message::Show).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        send(child, Message::Check).unwrap();
        if child.locked().unwrap() {
            return;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}
