use std::collections::HashSet;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
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

pub struct ManagedChild(pub Child);

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
        Ok(Self(child))
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let mut children = CHILDREN
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
        children.remove(&self.0.id());
    }
}

pub fn run(program: &str, args: &[&str]) -> Result<String, String> {
    run_with_timeout(program, args, Duration::from_secs(4))
}

fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = ManagedChild::spawn(&mut command).map_err(|e| format!("{program}: {e}"))?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("Could not read command output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
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
    drop(child);
    let bytes = reader
        .join()
        .map_err(|_| "Output reader failed")?
        .map_err(|e| e.to_string())?;
    result?;
    if bytes.len() > OUTPUT_LIMIT as usize {
        return Err(format!("{program}: excessive output"));
    }
    Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_report_failure_and_timeout() {
        assert_eq!(run("printf", &["hello"]).unwrap(), "hello");
        assert!(run("false", &[]).is_err());
        let start = Instant::now();
        assert!(run_with_timeout("sleep", &["5"], Duration::from_millis(50)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn command_locale_is_predictable() {
        assert_eq!(run("sh", &["-c", "printf %s \"$LC_ALL\""]).unwrap(), "C");
    }
}
