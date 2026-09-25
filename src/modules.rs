use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::SyncSender;
use std::thread;
use std::time::Duration;

const BACKLIGHT_ROOT: &str = "/sys/class/backlight";
const POWER_SUPPLY_ROOT: &str = "/sys/class/power_supply";
const THERMAL_ROOT: &str = "/sys/class/thermal";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkInfo {
    pub text: String,
    pub tooltip: String,
    pub status: String,
    pub ssid: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct BatteryStatus {
    pub text: String,
    pub level: String,
    pub tooltip: String,
    pub plugged: bool,
    pub charging: bool,
}

pub enum DeviceEvent {
    Peripheral(bool, String),
    Brightness,
    Power,
}

pub fn child_process(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn spawn_detached(program: &str, args: &[&str]) -> bool {
    let Ok(mut child) = Command::new(program).args(args).spawn() else {
        return false;
    };
    thread::spawn(move || {
        let _ = child.wait();
    });
    true
}

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn power_supply(prefixes: &[&str]) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(POWER_SUPPLY_ROOT)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    candidates.sort();
    candidates.into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| prefixes.iter().any(|prefix| name.starts_with(prefix)))
    })
}

pub fn thermal_sensor_path() -> Option<PathBuf> {
    let mut fallback = None;
    for entry in fs::read_dir(THERMAL_ROOT).ok()?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("thermal_zone") {
            continue;
        }
        let temp = path.join("temp");
        if read_trim(&path.join("type")).as_deref() == Some("x86_pkg_temp") {
            return Some(temp);
        }
        fallback.get_or_insert(temp);
    }
    fallback
}

pub fn backlight_device() -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(BACKLIGHT_ROOT)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort();
    names.into_iter().next()
}

pub fn wifi_interface() -> Option<String> {
    let mut names: Vec<String> = fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .filter(|entry| entry.path().join("wireless").exists())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort();
    names.into_iter().next()
}

fn remaining_charge(now: u64, full: Option<u64>, charging: bool) -> Option<u64> {
    if charging {
        full.map(|full| full.saturating_sub(now))
    } else {
        Some(now)
    }
}

fn format_estimate(remaining_charge: u64, power: u64) -> Option<String> {
    if power == 0 {
        return None;
    }
    let minutes = u128::from(remaining_charge) * 60 / u128::from(power);
    Some(format!("{}h {:02}m", minutes / 60, minutes % 60))
}

fn battery_estimate(battery: &Path, status: &str) -> String {
    let charging = status.eq_ignore_ascii_case("charging");
    for (now_key, full_key, power_key) in [
        ("energy_now", "energy_full", "power_now"),
        ("charge_now", "charge_full", "current_now"),
    ] {
        let read =
            |name: &str| read_trim(&battery.join(name)).and_then(|value| value.parse::<u64>().ok());
        let Some(now) = read(now_key) else {
            continue;
        };
        let Some(power) = read(power_key).filter(|value| *value > 0) else {
            continue;
        };
        let Some(remaining) = remaining_charge(now, read(full_key), charging) else {
            continue;
        };
        if let Some(estimate) = format_estimate(remaining, power) {
            return estimate;
        }
    }
    status.to_lowercase()
}

pub fn battery_status() -> BatteryStatus {
    let Some(battery) = power_supply(&["BAT"]) else {
        return BatteryStatus::default();
    };
    let Some(capacity) = read_trim(&battery.join("capacity")) else {
        return BatteryStatus::default();
    };
    let capacity_num: u32 = capacity.parse().unwrap_or(0);
    let status = read_trim(&battery.join("status")).unwrap_or_default();
    let online = power_supply(&["AC", "ADP", "AD"])
        .and_then(|adapter| read_trim(&adapter.join("online")))
        .is_some_and(|value| value == "1");
    let icon = if online && status.eq_ignore_ascii_case("charging") {
        "󰂄"
    } else if online {
        "󰚥"
    } else {
        match capacity_num {
            0..=10 => "󰂎",
            11..=30 => "󰁻",
            31..=50 => "󰁾",
            51..=75 => "󰂁",
            _ => "󰁹",
        }
    };
    let level = if capacity_num <= 10 {
        "critical"
    } else if capacity_num <= 20 {
        "warning"
    } else {
        "normal"
    };
    BatteryStatus {
        text: format!("{icon} {capacity}%"),
        level: level.to_owned(),
        tooltip: format!("{capacity}% • {}", battery_estimate(&battery, &status)),
        plugged: online,
        charging: status.eq_ignore_ascii_case("charging"),
    }
}

fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

fn signal_icon(signal: u8) -> &'static str {
    match signal {
        0..=24 => "󰤟",
        25..=49 => "󰤢",
        50..=74 => "󰤥",
        _ => "󰤨",
    }
}

pub fn network_info() -> NetworkInfo {
    let active = child_process(
        "nmcli",
        &["-t", "-f", "ACTIVE,SSID,SIGNAL", "device", "wifi"],
    )
    .and_then(|data| {
        data.lines().find_map(|line| {
            let fields = split_terse(line);
            fields
                .first()
                .is_some_and(|field| field == "yes")
                .then_some(fields)
        })
    });
    let Some(fields) = active else {
        return NetworkInfo {
            text: "󰖪".to_owned(),
            tooltip: "wi-fi: disconnected".to_owned(),
            status: "disconnected".to_owned(),
            ssid: None,
        };
    };
    let ssid = fields.get(1).cloned().filter(|value| !value.is_empty());
    let name = ssid
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_else(|| "wi-fi".to_owned());
    let signal: u8 = fields
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let ip = wifi_interface()
        .and_then(|interface| {
            child_process(
                "nmcli",
                &[
                    "-t",
                    "-f",
                    "IP4.ADDRESS",
                    "device",
                    "show",
                    interface.as_str(),
                ],
            )
        })
        .and_then(|data| {
            data.lines()
                .find_map(|line| line.strip_prefix("IP4.ADDRESS[1]:").map(str::to_owned))
        })
        .unwrap_or_default();
    NetworkInfo {
        text: signal_icon(signal).to_owned(),
        tooltip: format!("{name}\n{signal}% • {ip}"),
        status: "connected".to_owned(),
        ssid,
    }
}

pub fn brightness_level(device: &str) -> Option<(u8, &'static str)> {
    let base = Path::new(BACKLIGHT_ROOT).join(device);
    let max =
        read_trim(&base.join("max_brightness")).and_then(|value| value.parse::<u64>().ok())?;
    let current =
        read_trim(&base.join("brightness")).and_then(|value| value.parse::<u64>().ok())?;
    let percent = (current.saturating_mul(100) / max.max(1)).min(100) as u8;
    let icon = if percent <= 15 {
        "󰃞"
    } else if percent <= 50 {
        "󰃝"
    } else {
        "󰃟"
    };
    Some((percent, icon))
}

pub fn spawn_audio_poller(sender: SyncSender<Option<String>>) {
    thread::spawn(move || {
        loop {
            if let Some(value) = child_process("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
                let _ = sender.try_send(Some(value));
            }
            let Ok(mut child) = Command::new("pactl")
                .arg("subscribe")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if (line.contains("sink") || line.contains("server"))
                        && let Some(value) =
                            child_process("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
                    {
                        let _ = sender.try_send(Some(value));
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    });
}

pub fn spawn_temperature_poller(sender: SyncSender<Option<i64>>) {
    thread::spawn(move || {
        let mut sensor = thermal_sensor_path();
        loop {
            if sensor.is_none() {
                sensor = thermal_sensor_path();
            }
            let value = sensor
                .as_deref()
                .and_then(read_trim)
                .and_then(|text| text.parse::<i64>().ok());
            let _ = sender.try_send(value);
            thread::sleep(Duration::from_secs(3));
        }
    });
}

pub fn spawn_network_poller(sender: SyncSender<NetworkInfo>) {
    thread::spawn(move || {
        loop {
            let _ = sender.try_send(network_info());
            thread::sleep(Duration::from_secs(5));
        }
    });
}

pub fn spawn_device_monitor(sender: SyncSender<DeviceEvent>) {
    thread::spawn(move || {
        loop {
            let Ok(mut child) = Command::new("udevadm")
                .args([
                    "monitor",
                    "--udev",
                    "--property",
                    "--subsystem-match=usb",
                    "--subsystem-match=backlight",
                    "--subsystem-match=power_supply",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                thread::sleep(Duration::from_secs(2));
                continue;
            };
            if let Some(stdout) = child.stdout.take() {
                let mut properties = Vec::new();
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if line.is_empty() {
                        if let Some(event) = parse_device_event(&properties) {
                            let _ = sender.try_send(event);
                        }
                        properties.clear();
                    } else if let Some((key, value)) = line.split_once('=') {
                        properties.push((key.to_owned(), value.to_owned()));
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn parse_device_event(properties: &[(String, String)]) -> Option<DeviceEvent> {
    if properties
        .iter()
        .any(|(key, value)| key == "SUBSYSTEM" && value == "power_supply")
    {
        return Some(DeviceEvent::Power);
    }
    if properties
        .iter()
        .any(|(key, value)| key == "SUBSYSTEM" && value == "backlight")
        && properties
            .iter()
            .any(|(key, value)| key == "ACTION" && value == "change")
    {
        return Some(DeviceEvent::Brightness);
    }
    parse_udev_usb_event(properties)
        .map(|(connected, name)| DeviceEvent::Peripheral(connected, name))
}

fn parse_udev_usb_event(properties: &[(String, String)]) -> Option<(bool, String)> {
    let get = |key: &str| {
        properties
            .iter()
            .find(|(property, _)| property == key)
            .map(|(_, value)| value.as_str())
    };
    if get("SUBSYSTEM") != Some("usb") || get("DEVTYPE") != Some("usb_device") {
        return None;
    }
    let connected = match get("ACTION")? {
        "add" => true,
        "remove" => false,
        _ => return None,
    };
    let name = get("ID_MODEL_FROM_DATABASE")
        .or_else(|| get("ID_MODEL"))
        .or_else(|| get("PRODUCT"))
        .unwrap_or("USB device")
        .replace('_', " ");
    Some((connected, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_terse_output_on_unescaped_colons() {
        assert_eq!(
            split_terse("yes:My Network:74"),
            ["yes", "My Network", "74"]
        );
        assert_eq!(split_terse(r"yes:Net\:work:50"), ["yes", "Net:work", "50"]);
        assert_eq!(split_terse("no::0"), ["no", "", "0"]);
    }

    #[test]
    fn maps_signal_strength_to_icon() {
        assert_eq!(signal_icon(0), "󰤟");
        assert_eq!(signal_icon(100), "󰤨");
    }

    #[test]
    fn discharging_estimate_uses_current_charge() {
        assert_eq!(remaining_charge(5_000, Some(10_000), false), Some(5_000));
    }

    #[test]
    fn charging_estimate_uses_charge_until_full() {
        assert_eq!(remaining_charge(3_000, Some(10_000), true), Some(7_000));
    }

    #[test]
    fn charging_estimate_without_full_capacity_is_unknown() {
        assert_eq!(remaining_charge(3_000, None, true), None);
    }

    #[test]
    fn formats_estimate_with_zero_padded_minutes() {
        assert_eq!(format_estimate(7_200, 1_000).as_deref(), Some("7h 12m"));
        assert_eq!(format_estimate(3_600, 1_000).as_deref(), Some("3h 36m"));
        assert_eq!(format_estimate(90, 60).as_deref(), Some("1h 30m"));
        assert_eq!(format_estimate(59, 60).as_deref(), Some("0h 59m"));
        assert_eq!(format_estimate(1_000, 0), None);
    }

    #[test]
    fn estimate_does_not_overflow_on_large_values() {
        let minutes = u128::from(u64::MAX) * 60;
        let expected = format!("{}h {:02}m", minutes / 60, minutes % 60);
        assert_eq!(
            format_estimate(u64::MAX, 1).as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn parses_usb_add_and_remove_events_only_for_usb_devices() {
        let add = vec![
            ("ACTION".to_owned(), "add".to_owned()),
            ("SUBSYSTEM".to_owned(), "usb".to_owned()),
            ("DEVTYPE".to_owned(), "usb_device".to_owned()),
            ("ID_MODEL".to_owned(), "USB_Keyboard".to_owned()),
        ];
        assert_eq!(
            parse_udev_usb_event(&add),
            Some((true, "USB Keyboard".to_owned()))
        );

        let remove = vec![
            ("ACTION".to_owned(), "remove".to_owned()),
            ("SUBSYSTEM".to_owned(), "usb".to_owned()),
            ("DEVTYPE".to_owned(), "usb_device".to_owned()),
            (
                "ID_MODEL_FROM_DATABASE".to_owned(),
                "USB Keyboard".to_owned(),
            ),
        ];
        assert_eq!(
            parse_udev_usb_event(&remove),
            Some((false, "USB Keyboard".to_owned()))
        );
    }

    #[test]
    fn ignores_usb_interface_and_non_usb_events() {
        let interface = vec![
            ("ACTION".to_owned(), "add".to_owned()),
            ("SUBSYSTEM".to_owned(), "usb".to_owned()),
            ("DEVTYPE".to_owned(), "usb_interface".to_owned()),
        ];
        assert_eq!(parse_udev_usb_event(&interface), None);
        let input = vec![
            ("ACTION".to_owned(), "add".to_owned()),
            ("SUBSYSTEM".to_owned(), "input".to_owned()),
            ("DEVTYPE".to_owned(), "usb_device".to_owned()),
        ];
        assert_eq!(parse_udev_usb_event(&input), None);
    }

    #[test]
    fn device_monitor_distinguishes_brightness_and_power_events() {
        let brightness = vec![
            ("SUBSYSTEM".to_owned(), "backlight".to_owned()),
            ("ACTION".to_owned(), "change".to_owned()),
        ];
        assert!(matches!(
            parse_device_event(&brightness),
            Some(DeviceEvent::Brightness)
        ));
        let power = vec![("SUBSYSTEM".to_owned(), "power_supply".to_owned())];
        assert!(matches!(
            parse_device_event(&power),
            Some(DeviceEvent::Power)
        ));
    }
}
