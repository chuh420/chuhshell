use glib::variant::ToVariant;
use serde::Serialize;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Serialize)]
struct Check {
    name: String,
    status: &'static str,
    detail: String,
}

fn executable(path: &Path) -> bool {
    let Ok(path_bytes) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    path.is_file() && unsafe { libc::access(path_bytes.as_ptr(), libc::X_OK) == 0 }
}

fn owner(bus: &gio::DBusConnection, name: &str) -> Result<String, String> {
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        Some(&(name,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    )
    .map(|reply| reply.child_get::<String>(0))
    .map_err(|error| error.to_string())
}

fn notification_owner(
    notification: Result<String, String>,
    shell: Result<String, String>,
) -> Result<String, String> {
    let notification = notification?;
    let shell = shell?;
    if notification == shell {
        Ok(notification)
    } else {
        Err(format!(
            "Notifications belongs to {notification}; chuhshell belongs to {shell}"
        ))
    }
}

fn enabled(config: &crate::config::Config, module: &str) -> bool {
    !config.disabled_modules.iter().any(|name| name == module)
}

fn recovery_journal(path: &Path) -> Result<String, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "Pending recovery: {}. Rerun the corresponding installer or autologin helper; preserve this journal.",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok("No pending transaction".into())
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn readable_number(path: &Path) -> Result<String, String> {
    let value = crate::storage::read_text(path, 4096)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    value
        .trim()
        .parse::<i64>()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!("{}: {}", path.display(), value.trim()))
}

struct ProtocolProbe;

impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for ProtocolProbe
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

fn wayland_protocols() -> Result<Vec<String>, String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (|| {
            let connection =
                wayland_client::Connection::connect_to_env().map_err(|error| error.to_string())?;
            let (globals, _) =
                wayland_client::globals::registry_queue_init::<ProtocolProbe>(&connection)
                    .map_err(|error| error.to_string())?;
            Ok(globals.contents().with_list(|globals| {
                globals
                    .iter()
                    .map(|global| global.interface.clone())
                    .collect()
            }))
        })();
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|error| format!("Wayland protocol probe: {error}"))?
}

fn protocol_available(
    protocols: &Result<Vec<String>, String>,
    name: &str,
) -> Result<String, String> {
    match protocols {
        Ok(protocols) if protocols.iter().any(|protocol| protocol == name) => {
            Ok(format!("{name} advertised"))
        }
        Ok(_) => Err(format!("{name} not advertised")),
        Err(error) => Err(error.clone()),
    }
}

fn backlight_access(device: &str) -> Result<String, String> {
    crate::process::run("brightnessctl", &["-d", device, "get"])?;
    let path = std::path::PathBuf::from("/sys/class/backlight")
        .join(device)
        .join("brightness");
    let bytes = CString::new(path.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    if unsafe { libc::access(bytes.as_ptr(), libc::W_OK) } == 0 {
        return Ok(format!(
            "{} writable; brightnessctl read succeeded",
            path.display()
        ));
    }
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
        .map_err(|error| error.to_string())?;
    let property = |name: &str| {
        bus.call_sync(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&("org.freedesktop.login1.Session", name).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| error.to_string())?
        .child_value(0)
        .as_variant()
        .ok_or_else(|| "Invalid logind property response".to_string())
    };
    let active = property("Active")?.get::<bool>().unwrap_or(false);
    let remote = property("Remote")?.get::<bool>().unwrap_or(true);
    let user = property("User")?
        .get::<(u32, glib::variant::ObjectPath)>()
        .map(|(uid, _)| uid);
    if !active || remote || user != Some(unsafe { libc::getuid() }) {
        return Err(format!(
            "{} is not writable and no active local logind session belongs to this account",
            path.display()
        ));
    }
    let reply = bus
        .call_sync(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.DBus.Introspectable",
            "Introspect",
            None,
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| error.to_string())?;
    if !reply
        .child_get::<String>(0)
        .contains("method name=\"SetBrightness\"")
    {
        return Err("Logind session does not expose SetBrightness".into());
    }
    Ok("brightnessctl read succeeded; active local logind session exposes SetBrightness. Write access was not exercised; brightness was not changed.".into())
}

pub fn run(json: bool) -> glib::ExitCode {
    let mut checks = Vec::new();
    let mut add = |name: &str, required: bool, result: Result<String, String>| {
        let (status, detail) = match result {
            Ok(detail) => ("ok", detail),
            Err(detail) => (if required { "error" } else { "warning" }, detail),
        };
        checks.push(Check {
            name: name.into(),
            status,
            detail,
        });
    };
    let configuration = crate::config::initialize();
    let config = crate::config::get();
    add(
        "Configuration",
        true,
        configuration.map(|_| crate::config::path().display().to_string()),
    );
    add(
        "Niri IPC",
        true,
        crate::niri::window_processes()
            .map(|_| "Connected".into())
            .ok_or_else(|| "Unavailable".into()),
    );
    for (program, required) in [
        ("wpctl", enabled(&config, "audio")),
        ("pactl", enabled(&config, "audio")),
        (
            "brightnessctl",
            enabled(&config, "brightness") && crate::modules::backlight_device().is_some(),
        ),
        (
            "udevadm",
            enabled(&config, "brightness") || enabled(&config, "battery"),
        ),
        ("wl-paste", true),
        ("wl-copy", true),
        (
            "curl",
            config.weather_location.is_some() || config.weather_system,
        ),
    ] {
        if !required {
            add(program, false, Ok("Disabled by configuration".into()));
            continue;
        }
        let path = std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join(program))
                .find(|path| executable(path))
        });
        add(
            program,
            required,
            path.map(|path| path.display().to_string())
                .ok_or_else(|| "Executable missing from PATH".into()),
        );
    }
    let system = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE);
    for (name, required, active, destination, path, interface, method) in [
        (
            "Bluetooth service",
            false,
            true,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
        ),
        (
            "NetworkManager",
            true,
            enabled(&config, "wifi"),
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "GetDevices",
        ),
    ] {
        if !active {
            add(name, false, Ok("Disabled by configuration".into()));
            continue;
        }
        let result = match &system {
            Ok(bus) => bus
                .call_sync(
                    Some(destination),
                    path,
                    interface,
                    method,
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    5000,
                    gio::Cancellable::NONE,
                )
                .map(|_| "Available".into())
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        add(name, required, result);
    }
    if enabled(&config, "battery") {
        let profiles = system
            .as_ref()
            .map_err(|error| error.to_string())
            .and_then(|bus| {
                bus.call_sync(
                    Some("org.freedesktop.UPower.PowerProfiles"),
                    "/org/freedesktop/UPower/PowerProfiles",
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&("org.freedesktop.UPower.PowerProfiles", "ActiveProfile").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    3000,
                    gio::Cancellable::NONE,
                )
                .map(|reply| format!("Active profile: {}", reply.child_value(0)))
                .map_err(|error| error.to_string())
            });
        add("Power profiles", false, profiles);
        let battery = crate::modules::battery_status();
        add(
            "Battery",
            false,
            if battery.available {
                Ok(battery.tooltip)
            } else {
                Err("No battery available (optional on desktop hardware)".into())
            },
        );
    } else {
        add(
            "Power profiles",
            false,
            Ok("Battery module disabled".into()),
        );
    }
    if enabled(&config, "temperature") {
        add(
            "CPU sensor",
            false,
            crate::modules::thermal_sensor_path()
                .ok_or_else(|| "No CPU sensor available".into())
                .and_then(|path| readable_number(&path)),
        );
    } else {
        add(
            "CPU sensor",
            false,
            Ok("Temperature module disabled".into()),
        );
    }
    if enabled(&config, "brightness") {
        match crate::modules::backlight_device() {
            Some(device) => {
                let path = std::path::PathBuf::from("/sys/class/backlight").join(&device);
                add(
                    "Backlight",
                    true,
                    readable_number(&path.join("max_brightness")),
                );
                add("Brightness permissions", true, backlight_access(&device));
            }
            None => add(
                "Backlight",
                false,
                Err("No backlight available (optional on external monitors)".into()),
            ),
        }
    } else {
        add("Backlight", false, Ok("Brightness module disabled".into()));
    }
    let protocols = wayland_protocols();
    add(
        "Idle protocol",
        config.idle.screensaver.enabled,
        protocol_available(&protocols, "ext_idle_notifier_v1"),
    );
    add(
        "Session lock protocol",
        true,
        protocol_available(&protocols, "ext_session_lock_manager_v1"),
    );
    for (name, suffix) in [
        ("Installation recovery", "installation"),
        ("Autologin recovery", "autologin-backup"),
    ] {
        add(
            name,
            true,
            recovery_journal(
                &crate::paths::state()
                    .join("chuhshell")
                    .join(suffix)
                    .join("transaction.json"),
            ),
        );
    }
    let notifications = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| error.to_string())
        .and_then(|bus| {
            notification_owner(
                owner(&bus, "org.freedesktop.Notifications"),
                owner(&bus, "dev.chuh.chuhshell"),
            )
        });
    add("Notification service", true, notifications);
    let healthy = checks.iter().all(|check| check.status != "error");
    if json {
        println!(
            "{}",
            serde_json::json!({"healthy": healthy, "checks": checks})
        );
    } else {
        for check in checks {
            println!("{}: {} ({})", check.name, check.status, check.detail);
        }
    }
    if healthy {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn recovery_checks_preserve_pending_and_invalid_journals() {
        let root =
            std::env::temp_dir().join(format!("chuhshell-doctor-journal-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("transaction.json");
        assert!(recovery_journal(&path).is_ok());
        std::fs::write(&path, b"invalid journal").unwrap();
        assert!(
            recovery_journal(&path)
                .unwrap_err()
                .contains("Pending recovery")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid journal");
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), &path).unwrap();
        assert!(recovery_journal(&path).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn diagnostics_respect_disabled_modules_and_protocol_advertisements() {
        let config = crate::config::Config {
            disabled_modules: vec!["brightness".into()],
            ..Default::default()
        };
        assert!(!enabled(&config, "brightness"));
        assert!(enabled(&config, "audio"));
        let protocols = Ok(vec!["ext_idle_notifier_v1".into()]);
        assert!(protocol_available(&protocols, "ext_idle_notifier_v1").is_ok());
        assert!(protocol_available(&protocols, "ext_session_lock_manager_v1").is_err());
        assert_eq!(
            protocol_available(&Err("disconnected".into()), "ext_idle_notifier_v1"),
            Err("disconnected".into())
        );
    }

    #[test]
    fn command_checks_require_executable_files() {
        let path = std::env::temp_dir().join(format!("chuhshell-doctor-{}", std::process::id()));
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!executable(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(executable(&path));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn notifications_must_belong_to_the_shell() {
        assert!(notification_owner(Ok(":1.1".into()), Ok(":1.2".into())).is_err());
        assert!(notification_owner(Ok(":1.1".into()), Ok(":1.1".into())).is_ok());
        assert!(notification_owner(Ok(":1.1".into()), Err("No shell".into())).is_err());
    }
}
