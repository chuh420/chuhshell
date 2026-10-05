use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub idle: crate::idle::Settings,
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
            idle: Default::default(),
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
    crate::paths::config().join("chuhshell/config.json")
}

pub fn read() -> Result<Config, String> {
    let contents = match crate::storage::read_text(&path(), 1024 * 1024) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error.to_string()),
    };
    parse(&contents)
}

fn parse(contents: &str) -> Result<Config, String> {
    let mut config: Config = serde_json::from_str(contents).map_err(|e| e.to_string())?;
    for name in [&config.battery, &config.backlight, &config.wifi]
        .into_iter()
        .flatten()
    {
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err("Device names must be simple directory names".into());
        }
    }
    if !(1..=1440).contains(&config.idle.screensaver.minutes) {
        return Err("Idle timers must be between 1 and 1440 minutes".into());
    }
    if let Some(location) = &config.weather_location
        && (!location.latitude.is_finite()
            || !location.longitude.is_finite()
            || location.latitude.abs() > 90.0
            || location.longitude.abs() > 180.0
            || location.name.len() > 256)
    {
        return Err("Invalid weather location".into());
    }
    crate::keybindings::validate_overrides(&config.keybindings)?;
    config.notification_history_limit = config.notification_history_limit.clamp(10, 1000);
    Ok(config)
}

fn state() -> &'static RwLock<Arc<Config>> {
    static CONFIG: OnceLock<RwLock<Arc<Config>>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(Arc::new(Config::default())))
}

pub fn initialize() -> Result<(), String> {
    let config = std::thread::spawn(read)
        .join()
        .map_err(|_| "Configuration worker interrupted".to_string())??;
    *state().write().unwrap_or_else(|e| e.into_inner()) = Arc::new(config);
    Ok(())
}

pub fn get() -> Arc<Config> {
    state().read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn save_value(key: &str, setting: serde_json::Value) -> Result<(), String> {
    save_values_at(&path(), vec![(key.to_owned(), setting)])
}

pub fn save_values(
    settings: Vec<(String, serde_json::Value)>,
) -> impl std::future::Future<Output = Result<(), String>> {
    crate::storage::run(move || save_values_at(&path(), settings))
}

pub fn save_value_async(
    key: &str,
    setting: serde_json::Value,
) -> impl std::future::Future<Output = Result<(), String>> {
    save_values(vec![(key.to_owned(), setting)])
}

fn save_values_at(
    path: &std::path::Path,
    settings: Vec<(String, serde_json::Value)>,
) -> Result<(), String> {
    static SAVE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SAVE.lock().unwrap_or_else(|e| e.into_inner());
    let original = match crate::storage::read_text(path, 1024 * 1024) {
        Ok(contents) => Some(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Could not read settings: {e}")),
    };
    let mut value: serde_json::Value = match original.as_deref() {
        Some(contents) => {
            serde_json::from_str(contents).map_err(|e| format!("Could not read settings: {e}"))?
        }
        None => serde_json::json!({}),
    };
    parse(&serde_json::to_string(&value).map_err(|e| e.to_string())?)?;
    let object = value
        .as_object_mut()
        .ok_or("Settings must be a JSON object")?;
    for (key, setting) in settings {
        object.insert(key, setting);
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Settings exceed the size limit".into());
    }
    let confirmed = parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let current = match crate::storage::read_text(path, 1024 * 1024) {
        Ok(contents) => Some(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Could not recheck settings: {e}")),
    };
    if current != original {
        return Err("Settings changed while saving; reopen and try again".into());
    }
    crate::storage::atomic_write(path, &bytes)
        .map_err(|e| format!("Could not save settings: {e}"))?;
    if path == self::path() {
        *state().write().unwrap_or_else(|e| e.into_inner()) = Arc::new(confirmed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_configuration_uses_supported_fields() {
        let config: Config = serde_json::from_str(include_str!("../config.example.json")).unwrap();
        assert_eq!(config.notification_history_limit, 200);
    }

    #[test]
    fn invalid_settings_updates_preserve_the_original_file() {
        let path = std::env::temp_dir().join(format!(
            "chuhshell-invalid-settings-{}.json",
            std::process::id()
        ));
        let valid = br#"{"weather_system":true}"#;
        std::fs::write(&path, valid).unwrap();
        for setting in [serde_json::json!("../device"), serde_json::json!(42)] {
            assert!(save_values_at(&path, vec![("wifi".into(), setting)]).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), valid);
        }
        let invalid = br#"{"idle":{"screensaver":{"minutes":0}}}"#;
        std::fs::write(&path, invalid).unwrap();
        assert!(save_values_at(&path, vec![("weather_system".into(), false.into())]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), invalid);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn related_settings_are_saved_together() {
        let path = std::env::temp_dir().join(format!(
            "chuhshell-settings-transaction-{}.json",
            std::process::id()
        ));
        save_values_at(
            &path,
            vec![
                ("weather_units".into(), "celsius".into()),
                ("weather_system".into(), false.into()),
            ],
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["weather_units"], "celsius");
        assert_eq!(value["weather_system"], false);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn concurrent_settings_updates_preserve_other_fields_and_invalid_json() {
        let path =
            std::env::temp_dir().join(format!("chuhshell-settings-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"weather_system":true}"#).unwrap();
        std::thread::scope(|scope| {
            for key in ["battery", "backlight", "wifi"] {
                let path = &path;
                scope.spawn(move || {
                    save_values_at(path, vec![(key.to_owned(), "device".into())]).unwrap()
                });
            }
        });
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        std::fs::write(&path, b"invalid").unwrap();
        assert!(save_values_at(&path, vec![("weather_system".into(), false.into())]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        std::fs::remove_file(path).unwrap();
    }
}
