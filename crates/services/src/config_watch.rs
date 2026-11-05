use std::{
    io,
    os::fd::{AsFd, BorrowedFd},
};

use inotify::{Inotify, WatchMask};

pub struct Watcher {
    inotify: Inotify,
}

impl Watcher {
    pub fn new() -> io::Result<Self> {
        let dir = &config::paths().config;
        std::fs::create_dir_all(dir)?;
        let inotify = Inotify::init()?;
        inotify.watches().add(
            dir,
            WatchMask::CLOSE_WRITE
                | WatchMask::MOVED_TO
                | WatchMask::MOVED_FROM
                | WatchMask::CREATE
                | WatchMask::DELETE,
        )?;
        Ok(Self { inotify })
    }

    pub fn drain(&mut self) -> bool {
        let mut buffer = [0u8; 4096];
        let mut changed = false;
        loop {
            match self.inotify.read_events(&mut buffer) {
                Ok(events) => {
                    for event in events {
                        changed |= event.name.is_some_and(|name| name == "config.toml");
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => {
                    log::warn!("config watch: {err}");
                    break;
                }
            }
        }
        changed
    }
}

impl AsFd for Watcher {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }
}
