use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub keybindings: std::collections::BTreeMap<String, String>,
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
    pub weather_location: Option<crate::weather::Location>,
    pub weather_system: bool,
    pub weather_units: crate::weather::Units,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keybindings: Default::default(),
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
            weather_location: None,
            weather_system: false,
            weather_units: Default::default(),
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
    save_value_at(&path(), key, setting)
}

fn save_value_at(
    path: &std::path::Path,
    key: &str,
    setting: serde_json::Value,
) -> Result<(), String> {
    static SAVE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SAVE.lock().unwrap_or_else(|e| e.into_inner());
    let mut value: serde_json::Value = match std::fs::read_to_string(path) {
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
    crate::storage::atomic_write(
        path,
        &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Could not save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_settings_updates_preserve_other_fields_and_invalid_json() {
        let path =
            std::env::temp_dir().join(format!("chuhshell-settings-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"existing":true}"#).unwrap();
        std::thread::scope(|scope| {
            for key in ["one", "two", "three"] {
                let path = &path;
                scope.spawn(move || save_value_at(path, key, true.into()).unwrap());
            }
        });
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        std::fs::write(&path, b"invalid").unwrap();
        assert!(save_value_at(&path, "one", false.into()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        std::fs::remove_file(path).unwrap();
    }
}
