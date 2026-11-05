use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

const APP: &str = "chuhshell";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub runtime: Option<PathBuf>,
    pub wallpapers: PathBuf,
}

fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

pub fn pictures_dir(user_dirs: &str, home: &Path) -> Option<PathBuf> {
    user_dirs.lines().find_map(|line| {
        let value = line.trim().strip_prefix("XDG_PICTURES_DIR=")?;
        let value = value.trim().trim_matches('"');
        let path = if let Some(rest) = value.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else {
            PathBuf::from(value)
        };
        (path.is_absolute() && path != home).then_some(path)
    })
}

pub fn expand_home(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

impl Paths {
    pub fn from_env() -> Self {
        Self::resolve(
            |key| std::env::var_os(key),
            |path| std::fs::read_to_string(path).ok(),
        )
    }

    pub fn resolve(
        env: impl Fn(&str) -> Option<OsString>,
        read: impl Fn(&Path) -> Option<String>,
    ) -> Self {
        let home = absolute(env("HOME")).unwrap_or_else(|| PathBuf::from("/"));
        let base =
            |key: &str, fallback: &str| absolute(env(key)).unwrap_or_else(|| home.join(fallback));
        let config_home = base("XDG_CONFIG_HOME", ".config");
        let pictures = read(&config_home.join("user-dirs.dirs"))
            .and_then(|text| pictures_dir(&text, &home))
            .unwrap_or_else(|| home.join("Pictures"));
        Self {
            config: config_home.join(APP),
            state: base("XDG_STATE_HOME", ".local/state").join(APP),
            cache: base("XDG_CACHE_HOME", ".cache").join(APP),
            runtime: absolute(env("XDG_RUNTIME_DIR")),
            wallpapers: pictures.join("Wallpapers"),
            home,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn state_file(&self, name: &str) -> PathBuf {
        self.state.join(name)
    }

    pub fn cache_dir(&self, name: &str) -> PathBuf {
        self.cache.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn defaults_follow_home() {
        let paths = Paths::resolve(env(&[("HOME", "/home/a")]), |_| None);
        assert_eq!(paths.config, PathBuf::from("/home/a/.config/chuhshell"));
        assert_eq!(paths.state, PathBuf::from("/home/a/.local/state/chuhshell"));
        assert_eq!(paths.cache, PathBuf::from("/home/a/.cache/chuhshell"));
        assert_eq!(paths.runtime, None);
        assert_eq!(
            paths.wallpapers,
            PathBuf::from("/home/a/Pictures/Wallpapers")
        );
        assert_eq!(
            paths.config_file(),
            PathBuf::from("/home/a/.config/chuhshell/config.toml")
        );
    }

    #[test]
    fn xdg_variables_win() {
        let paths = Paths::resolve(
            env(&[
                ("HOME", "/home/a"),
                ("XDG_CONFIG_HOME", "/cfg"),
                ("XDG_STATE_HOME", "/st"),
                ("XDG_CACHE_HOME", "/ca"),
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ]),
            |path| {
                (path == Path::new("/cfg/user-dirs.dirs"))
                    .then(|| "XDG_PICTURES_DIR=\"$HOME/Bilder\"\n".to_string())
            },
        );
        assert_eq!(paths.config, PathBuf::from("/cfg/chuhshell"));
        assert_eq!(paths.state, PathBuf::from("/st/chuhshell"));
        assert_eq!(paths.cache, PathBuf::from("/ca/chuhshell"));
        assert_eq!(paths.runtime, Some(PathBuf::from("/run/user/1000")));
        assert_eq!(paths.wallpapers, PathBuf::from("/home/a/Bilder/Wallpapers"));
    }

    #[test]
    fn relative_xdg_values_are_ignored() {
        let paths = Paths::resolve(
            env(&[("HOME", "/home/a"), ("XDG_STATE_HOME", "relative")]),
            |_| None,
        );
        assert_eq!(paths.state, PathBuf::from("/home/a/.local/state/chuhshell"));
    }

    #[test]
    fn user_dirs_parse() {
        let home = Path::new("/home/a");
        let text =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"/data/pics\"\n";
        assert_eq!(pictures_dir(text, home), Some(PathBuf::from("/data/pics")));
        assert_eq!(pictures_dir("XDG_PICTURES_DIR=\"$HOME/\"\n", home), None);
        assert_eq!(pictures_dir("", home), None);
    }

    #[test]
    fn tilde_expands() {
        let home = Path::new("/home/a");
        assert_eq!(
            expand_home(Path::new("~/walls"), home),
            PathBuf::from("/home/a/walls")
        );
        assert_eq!(expand_home(Path::new("/abs"), home), PathBuf::from("/abs"));
    }
}
