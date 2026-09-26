use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub monitors: Vec<String>,
    pub disabled_modules: Vec<String>,
    pub battery: Option<String>,
    pub backlight: Option<String>,
    pub wifi: Option<String>,
    pub temperature_sensor: Option<PathBuf>,
    pub notification_history_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitors: Vec::new(),
            disabled_modules: Vec::new(),
            battery: None,
            backlight: None,
            wifi: None,
            temperature_sensor: None,
            notification_history_limit: 200,
        }
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("chuhshell/config.json")
}

pub fn read() -> Result<Config, String> {
    let contents = match std::fs::read_to_string(path()) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error.to_string()),
    };
    let mut config: Config = serde_json::from_str(&contents).map_err(|e| e.to_string())?;
    for name in [&config.battery, &config.backlight, &config.wifi]
        .into_iter()
        .flatten()
    {
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err("Device names must be simple directory names".into());
        }
    }
    config.notification_history_limit = config.notification_history_limit.clamp(10, 1000);
    Ok(config)
}

pub fn get() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        read().unwrap_or_else(|e| {
            eprintln!("chuhshell: invalid configuration: {e}");
            Config::default()
        })
    })
}
