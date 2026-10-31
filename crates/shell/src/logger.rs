use std::io::Write;

use log::{LevelFilter, Log, Metadata, Record};

struct Logger;

static LOGGER: Logger = Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "{} {}: {}",
                record.level().as_str().to_lowercase(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

pub fn init() {
    let variable = format!("{}_LOG", crate::APP_NAME.to_uppercase().replace('-', "_"));
    let level = std::env::var(variable)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(LevelFilter::Info);
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}
