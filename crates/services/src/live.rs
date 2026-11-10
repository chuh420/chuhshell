use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

const OPTIONS: &[&str] = &[
    "no-audio",
    "loop-file=inf",
    "hwdec=auto-safe",
    "panscan=1.0",
    "profile=fast",
    "vd-lavc-threads=1",
    "framedrop=vo",
    "interpolation=no",
    "cache=no",
    "demuxer-max-bytes=16MiB",
    "demuxer-max-back-bytes=0",
    "scale=bilinear",
    "dither-depth=no",
];

const LOCKERS: &[&str] = &["hyprlock", "swaylock", "gtklock", "waylock"];

pub fn available() -> bool {
    crate::programs::exists("mpvpaper")
}

pub fn command(video: &Path) -> Command {
    let mut command = Command::new("mpvpaper");
    command
        .args(["-p", "-l", "background", "-o"])
        .arg(OPTIONS.join(" "))
        .arg("ALL")
        .arg(video)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::wallpapers::bind_to_parent(&mut command);
    command
}

pub struct Player {
    child: Child,
    video: String,
    stopped: bool,
}

use crate::programs::{Signal, signal};

impl Player {
    pub fn start(video: &str) -> Option<Player> {
        if !available() {
            crate::programs::warn_missing("mpvpaper", "live wallpaper playback");
            return None;
        }
        match command(Path::new(video)).spawn() {
            Ok(child) => Some(Player {
                child,
                video: video.to_string(),
                stopped: false,
            }),
            Err(err) => {
                log::warn!("mpvpaper: {err}");
                None
            }
        }
    }

    pub fn video(&self) -> &str {
        &self.video
    }

    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn paused(&self) -> bool {
        self.stopped
    }

    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.stopped || !self.running() {
            return;
        }
        let wanted = if paused {
            Signal::Stop
        } else {
            Signal::Continue
        };
        if signal(self.child.id() as i32, wanted).is_ok() {
            self.stopped = paused;
        }
    }

    pub fn stop(mut self) {
        if self.stopped {
            let _ = signal(self.child.id() as i32, Signal::Continue);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn is_locker(comm: &str) -> bool {
    LOCKERS.contains(&comm.trim())
}

pub fn screen_locked() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
            && std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| is_locker(&comm))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpvpaper_command_line() {
        let command = command(Path::new("/w/sea.mp4"));
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..4], ["-p", "-l", "background", "-o"]);
        assert!(args[4].contains("loop-file=inf") && args[4].contains("no-audio"));
        assert_eq!(&args[5..], ["ALL", "/w/sea.mp4"]);
    }

    #[test]
    fn lockers() {
        assert!(is_locker("hyprlock\n"));
        assert!(is_locker("swaylock"));
        assert!(!is_locker("kitty"));
    }
}
