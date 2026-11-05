use std::{
    collections::HashSet,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

pub fn find_in(name: &str, path_var: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return executable(&path).then_some(path);
    }
    path_var
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| executable(candidate))
}

pub fn find(name: &str) -> Option<PathBuf> {
    find_in(name, &std::env::var("PATH").unwrap_or_default())
}

pub fn exists(name: &str) -> bool {
    find(name).is_some()
}

pub fn warn_missing(name: &str, purpose: &str) {
    static WARNED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    if let Ok(mut warned) = WARNED.lock()
        && warned
            .get_or_insert_with(HashSet::new)
            .insert(name.to_string())
    {
        log::info!("{name} not found, {purpose} is unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_on_path() {
        assert!(find_in("sh", "/nonexistent:/usr/bin:/bin").is_some());
        assert_eq!(find_in("chuhshell-no-such-tool", "/usr/bin"), None);
        assert_eq!(find_in("sh", ""), None);
        assert!(find_in("/bin/sh", "").is_some());
    }
}
