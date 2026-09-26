use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub launcher_layout: Option<crate::layout::Geometry>,
    #[serde(rename = "bar_layout")]
    pub _legacy_bar_layout: Option<serde_json::Value>,
    pub bar_order: crate::bar_settings::ModuleOrder,
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
            launcher_layout: None,
            _legacy_bar_layout: None,
            bar_order: Default::default(),
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

pub fn save_value(key: &str, setting: serde_json::Value) -> Result<(), String> {
    let path = path();
    let mut value: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(contents) => {
            serde_json::from_str(&contents).map_err(|e| format!("Could not read settings: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(format!("Could not read settings: {e}")),
    };
    value
        .as_object_mut()
        .ok_or("Settings must be a JSON object")?
        .insert(key.into(), setting);
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(path.parent().ok_or("Invalid settings path")?)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
        std::fs::rename(temporary, &path)?;
        Ok(())
    };
    write().map_err(|e| format!("Could not save settings: {e}"))
}
