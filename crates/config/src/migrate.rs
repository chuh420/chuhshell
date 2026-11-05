use std::{
    io,
    path::{Path, PathBuf},
};

use crate::Paths;

pub fn legacy_files(home: &Path) -> Vec<(PathBuf, &'static str)> {
    let material = home.join(".local/state/quickshell/material");
    let rice = home.join(".local/state/rice");
    vec![
        (
            material.join("hidden-applications.json"),
            "hidden-applications.json",
        ),
        (
            material.join("battery-history.json"),
            "battery-history.json",
        ),
        (rice.join("wallpaper.json"), "wallpaper.json"),
        (rice.join("theme.json"), "theme.json"),
        (rice.join("cursor.json"), "cursor.json"),
    ]
}

pub fn run(paths: &Paths) -> io::Result<Vec<&'static str>> {
    if paths.state.exists() {
        return Ok(Vec::new());
    }
    copy_legacy(&legacy_files(&paths.home), &paths.state)
}

fn copy_legacy(files: &[(PathBuf, &'static str)], state: &Path) -> io::Result<Vec<&'static str>> {
    let staging = state.with_extension("migrating");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let mut copied = Vec::new();
    for (source, name) in files {
        if source.is_file() {
            std::fs::copy(source, staging.join(name))?;
            copied.push(*name);
        }
    }
    std::fs::rename(&staging, state)?;
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chuhshell-migrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn copies_present_files_once() {
        let root = scratch("copy");
        let home = root.join("home");
        let rice = home.join(".local/state/rice");
        std::fs::create_dir_all(&rice).ok();
        std::fs::write(rice.join("theme.json"), "{\"mode\":\"auto\"}").ok();
        let paths = Paths {
            home: home.clone(),
            config: root.join("config"),
            state: root.join("state/chuhshell"),
            cache: root.join("cache"),
            runtime: None,
            wallpapers: root.join("walls"),
        };
        let copied = run(&paths).ok();
        assert_eq!(copied, Some(vec!["theme.json"]));
        assert_eq!(
            std::fs::read_to_string(paths.state.join("theme.json")).ok(),
            Some("{\"mode\":\"auto\"}".to_string())
        );
        assert!(rice.join("theme.json").is_file());
        std::fs::write(rice.join("cursor.json"), "{}").ok();
        assert_eq!(run(&paths).ok(), Some(Vec::new()));
        assert!(!paths.state.join("cursor.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
