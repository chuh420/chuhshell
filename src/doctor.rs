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
    add(
        "Configuration",
        true,
        crate::config::read().map(|_| crate::config::path().display().to_string()),
    );
    add(
        "Niri IPC",
        true,
        crate::niri::window_processes()
            .map(|_| "Connected".into())
            .ok_or_else(|| "Unavailable".into()),
    );
    for (program, required) in [
        ("wpctl", true),
        ("pactl", true),
        ("brightnessctl", true),
        ("udevadm", true),
        ("wl-paste", true),
        ("wl-copy", true),
        ("curl", true),
    ] {
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
    for (name, destination, path, interface, method) in [
        (
            "Bluetooth service",
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
        ),
        (
            "NetworkManager",
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "GetDevices",
        ),
    ] {
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
        add(name, true, result);
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
