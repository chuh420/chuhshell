use std::collections::HashSet;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static STOP: AtomicBool = AtomicBool::new(false);
static CHILDREN: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
const OUTPUT_LIMIT: u64 = 256 * 1024;

pub fn stopped() -> bool {
    STOP.load(Ordering::Relaxed)
}

pub fn shutdown() {
    STOP.store(true, Ordering::Relaxed);
    if let Some(children) = CHILDREN.get() {
        for pid in children.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            unsafe {
                libc::kill(-(*pid as i32), libc::SIGKILL);
            }
        }
    }
}

pub fn pause(duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while !stopped() && Instant::now() < deadline {
        std::thread::sleep(
            Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    !stopped()
}

pub struct ManagedChild(pub Child, bool);

impl ManagedChild {
    pub fn spawn(command: &mut Command) -> Result<Self, String> {
        let mut children = CHILDREN
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if stopped() {
            return Err("Shell is stopping".into());
        }
        let parent = std::process::id();
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() as u32 != parent {
                    return Err(std::io::Error::other("Parent exited"));
                }
                Ok(())
            });
        }
        let child = command
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| e.to_string())?;
        children.insert(child.id());
        Ok(Self(child, true))
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let mut children = CHILDREN
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.1 {
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
        }
        let _ = self.0.wait();
        children.remove(&self.0.id());
    }
}

pub struct IdleChild {
    child: Option<Child>,
}

impl IdleChild {
    pub fn spawn(command: &mut Command) -> Result<Self, String> {
        if stopped() {
            return Err("Shell is stopping".into());
        }
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .map_err(|e| e.to_string())?;
        let worker = Self { child: Some(child) };
        let fd = worker
            .child
            .as_ref()
            .unwrap()
            .stdin
            .as_ref()
            .unwrap()
            .as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(worker)
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        if bytes.len() > 4096 {
            return Err("Idle command exceeds size limit".into());
        }
        let input = self
            .child
            .as_mut()
            .and_then(|child| child.stdin.as_mut())
            .ok_or("Idle input closed")?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut remaining = bytes;
        while !remaining.is_empty() {
            if Instant::now() >= deadline {
                return Err("Idle command timed out".into());
            }
            match input.write(remaining) {
                Ok(0) => return Err("Idle input closed".into()),
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    let mut fd = libc::pollfd {
                        fd: input.as_raw_fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    if unsafe { libc::poll(&mut fd, 1, 50) } < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::Interrupted {
                            return Err(error.to_string());
                        }
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        self.child
            .as_mut()
            .ok_or("Idle worker already stopped")?
            .try_wait()
            .map_err(|e| e.to_string())
    }

    pub fn locked(&mut self) -> Result<bool, String> {
        let output = self
            .child
            .as_mut()
            .and_then(|child| child.stdout.as_mut())
            .ok_or("Idle output closed")?;
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut line = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err("Idle status timed out".into());
            }
            let mut fd = libc::pollfd {
                fd: output.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut fd, 1, 100) } < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error.to_string());
            }
            if fd.revents == 0 {
                continue;
            }
            let mut byte = [0];
            if output.read(&mut byte).map_err(|e| e.to_string())? == 0 {
                return Err("Idle output closed".into());
            }
            line.push(byte[0]);
            if line.len() > 4096 {
                return Err("Idle status exceeds size limit".into());
            }
            if byte[0] == b'\n' {
                if line.ends_with(b"idle-locked=true\n") {
                    return Ok(true);
                }
                if line.ends_with(b"idle-locked=false\n") {
                    return Ok(false);
                }
                line.clear();
            }
        }
    }

    pub fn finish(&mut self) -> Result<std::process::ExitStatus, String> {
        let child = self.child.as_mut().ok_or("Idle worker already stopped")?;
        child.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                self.child.take();
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let mut child = self.child.take().unwrap();
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(
                    "Idle worker did not stop; left alive to release its session lock".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(test)]
    pub fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }
}

impl Drop for IdleChild {
    fn drop(&mut self) {
        if self.child.is_some()
            && let Err(error) = self.finish()
        {
            eprintln!("chuhshell: {error}");
        }
    }
}

pub fn run(program: &str, args: &[&str]) -> Result<String, String> {
    run_with_timeout(program, args, Duration::from_secs(4))
}

pub fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    run_command(&mut command, timeout)
}

pub fn run_command(command: &mut Command, timeout: Duration) -> Result<String, String> {
    run_command_with(command, timeout, false)
}

pub fn run_background_command(command: &mut Command, timeout: Duration) -> Result<String, String> {
    run_command_with(command, timeout, true)
}

fn run_command_with(
    command: &mut Command,
    timeout: Duration,
    keep_background: bool,
) -> Result<String, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(command).map_err(|e| format!("{program}: {e}"))?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("Could not read command output")?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or("Could not read command errors")?;
    let finished = Arc::new(AtomicBool::new(false));
    let reader = read_output(stdout, finished.clone());
    let errors = read_output(stderr, finished.clone());
    let deadline = Instant::now() + timeout;
    let result = loop {
        if stopped() || Instant::now() >= deadline {
            break Err(format!("{program}: operation timed out or cancelled"));
        }
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let status = unsafe {
            libc::waitid(
                libc::P_PID,
                child.0.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if status < 0 {
            break Err(format!("{program}: {}", std::io::Error::last_os_error()));
        }
        if unsafe { info.si_pid() } != 0 {
            break if info.si_code == libc::CLD_EXITED && unsafe { info.si_status() } == 0 {
                Ok(())
            } else {
                Err(format!(
                    "{program}: command failed with status {}",
                    unsafe { info.si_status() }
                ))
            };
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if keep_background && result.is_ok() {
        child.1 = false;
    }
    drop(child);
    finished.store(true, Ordering::Release);
    let bytes = reader
        .join()
        .map_err(|_| "Output reader failed")?
        .map_err(|e| e.to_string())?;
    let errors = errors
        .join()
        .map_err(|_| "Error reader failed")?
        .map_err(|e| e.to_string())?;
    if let Err(error) = result {
        let detail = String::from_utf8_lossy(&errors);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            error
        } else {
            format!("{error}: {detail}")
        });
    }
    if bytes.len() > OUTPUT_LIMIT as usize {
        return Err(format!("{program}: excessive output"));
    }
    Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn read_output(
    mut stream: impl Read + AsRawFd + Send + 'static,
    finished: Arc<AtomicBool>,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let fd = stream.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    let keep = size.min((OUTPUT_LIMIT as usize + 1).saturating_sub(bytes.len()));
                    bytes.extend_from_slice(&buffer[..keep]);
                    if finished.load(Ordering::Acquire) && bytes.len() > OUTPUT_LIMIT as usize {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if finished.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_input_backpressure_times_out_without_killing_the_child() {
        let mut command = Command::new("sleep");
        command.arg("300");
        let mut child = IdleChild::spawn(&mut command).unwrap();
        let bytes = [b'x'; 4096];
        let error = loop {
            match child.send(&bytes) {
                Ok(()) => {}
                Err(error) => break error,
            }
        };
        assert_eq!(error, "Idle command timed out");
        assert!(child.try_wait().unwrap().is_none());
        child.child.as_mut().unwrap().kill().unwrap();
        child.finish().unwrap();
    }

    #[test]
    fn commands_report_failure_and_timeout() {
        assert_eq!(run("printf", &["hello"]).unwrap(), "hello");
        assert!(run("false", &[]).is_err());
        let start = Instant::now();
        assert!(run_with_timeout("sleep", &["5"], Duration::from_millis(50)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn failures_include_bounded_stderr_without_deadlock() {
        let error = run("sh", &["-c", "printf 'validation detail' >&2; exit 7"]).unwrap_err();
        assert!(error.contains("validation detail"));
        let error = run("sh", &["-c", "head -c 400000 /dev/zero >&2; exit 1"]).unwrap_err();
        assert!(error.len() < OUTPUT_LIMIT as usize + 200);
    }
    fn running(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, fields)| !fields.starts_with('Z'))
        })
    }

    #[test]
    fn arbitrary_commands_kill_descendants_on_timeout() {
        let path = std::env::temp_dir().join(format!("chuhshell-command-{}", std::process::id()));
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 300 & echo $! > \"$1\"; wait", "sh"])
            .arg(&path);
        assert!(
            run_command(&mut command, Duration::from_millis(100))
                .unwrap_err()
                .contains("timed out")
        );
        let pid = std::fs::read_to_string(&path)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        for _ in 0..50 {
            if !running(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!running(pid));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn background_commands_preserve_children_only_after_success() {
        for (status, survives) in [(0, true), (7, false)] {
            let path = std::env::temp_dir().join(format!(
                "chuhshell-background-{}-{status}",
                std::process::id()
            ));
            let mut command = Command::new("sh");
            command
                .args(["-c", "sleep 300 & echo $! > \"$1\"; exit \"$2\"", "sh"])
                .arg(&path)
                .arg(status.to_string());
            let result = run_background_command(&mut command, Duration::from_secs(2));
            let pid = std::fs::read_to_string(&path)
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap();
            assert_eq!(result.is_ok(), survives);
            for _ in 0..50 {
                if running(pid) == survives {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let alive = running(pid);
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            std::fs::remove_file(path).unwrap();
            assert_eq!(alive, survives);
        }
        let path = std::env::temp_dir().join(format!(
            "chuhshell-background-timeout-{}",
            std::process::id()
        ));
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 300 & echo $! > \"$1\"; wait", "sh"])
            .arg(&path);
        assert!(run_background_command(&mut command, Duration::from_millis(100)).is_err());
        let pid = std::fs::read_to_string(&path)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        for _ in 0..50 {
            if !running(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!running(pid));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn shutdown_cancels_managed_commands() {
        if std::env::var_os("CHUHSHELL_TEST_SHUTDOWN").is_some() {
            let worker = std::thread::spawn(|| run("sleep", &["300"]));
            for _ in 0..100 {
                if CHILDREN
                    .get()
                    .is_some_and(|children| !children.lock().unwrap().is_empty())
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                CHILDREN
                    .get()
                    .is_some_and(|children| !children.lock().unwrap().is_empty())
            );
            shutdown();
            assert!(worker.join().unwrap().is_err());
            assert!(CHILDREN.get().unwrap().lock().unwrap().is_empty());
            return;
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::shutdown_cancels_managed_commands",
                "--nocapture",
            ])
            .env("CHUHSHELL_TEST_SHUTDOWN", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn command_locale_is_predictable() {
        assert_eq!(run("sh", &["-c", "printf %s \"$LC_ALL\""]).unwrap(), "C");
    }
}
