use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn resolve(value: Option<&OsStr>, fallback: &Path) -> PathBuf {
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| fallback.to_owned())
}

pub fn home() -> PathBuf {
    let fallback = glib::home_dir();
    resolve(
        std::env::var_os("HOME").as_deref(),
        if fallback.is_absolute() {
            &fallback
        } else {
            Path::new("/")
        },
    )
}

fn directory(variable: &str, suffix: &str) -> PathBuf {
    resolve(std::env::var_os(variable).as_deref(), &home().join(suffix))
}

pub fn config() -> PathBuf {
    directory("XDG_CONFIG_HOME", ".config")
}
pub fn data() -> PathBuf {
    directory("XDG_DATA_HOME", ".local/share")
}
pub fn state() -> PathBuf {
    directory("XDG_STATE_HOME", ".local/state")
}
pub fn data_dirs() -> Vec<PathBuf> {
    let directories = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::env::split_paths(&directories)
        .filter(|path| path.is_absolute())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xdg_paths_ignore_empty_and_relative_values() {
        let fallback = Path::new("/home/test/.config");
        for value in [None, Some(OsStr::new("")), Some(OsStr::new("relative"))] {
            assert_eq!(resolve(value, fallback), fallback);
        }
        assert_eq!(
            resolve(Some(OsStr::new("/tmp/config")), fallback),
            Path::new("/tmp/config")
        );
    }
}
