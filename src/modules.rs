use crate::{config, process};
use async_channel::Sender;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatteryStatus {
    pub text: String,
    pub level: String,
    pub tooltip: String,
    pub plugged: bool,
    pub charging: bool,
    pub available: bool,
}

pub enum DeviceEvent {
    Peripheral(bool, String),
    Brightness,
    Power,
}

pub fn child_process(program: &str, args: &[&str]) -> Option<String> {
    process::run(program, args)
        .map_err(|error| eprintln!("chuhshell: {error}"))
        .ok()
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

fn supplies(kind: &str) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(POWER_SUPPLY_ROOT)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| read_trim(&path.join("type")).as_deref() == Some(kind))
        .collect();
    paths.sort();
    paths
}

fn battery_path() -> Option<PathBuf> {
    if let Some(name) = &config::get().battery {
        return Some(Path::new(POWER_SUPPLY_ROOT).join(name));
    }
    supplies("Battery")
        .into_iter()
        .find(|path| read_trim(&path.join("scope")).as_deref() != Some("Device"))
}

pub fn thermal_sensor_path() -> Option<PathBuf> {
    if let Some(path) = &config::get().temperature_sensor {
        return Some(path.clone());
    }
    let mut fallback = None;
    for entry in fs::read_dir(THERMAL_ROOT).into_iter().flatten().flatten() {
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
    for entry in fs::read_dir("/sys/class/hwmon")
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if read_trim(&path.join("name")).is_some_and(|name| {
            ["coretemp", "k10temp", "zenpower", "cpu_thermal"].contains(&name.as_str())
        }) && path.join("temp1_input").exists()
        {
            return Some(path.join("temp1_input"));
        }
    }
    fallback
}

pub fn backlight_device() -> Option<String> {
    if let Some(name) = &config::get().backlight {
        return Some(name.clone());
    }
    let mut names: Vec<String> = fs::read_dir(BACKLIGHT_ROOT)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort_by_key(|name| {
        let kind =
            read_trim(&Path::new(BACKLIGHT_ROOT).join(name).join("type")).unwrap_or_default();
        (
            match kind.as_str() {
                "raw" => 0,
                "platform" => 1,
                _ => 2,
            },
            name.clone(),
        )
    });
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
    let Some(battery) = battery_path() else {
        return BatteryStatus::default();
    };
    let unavailable = || BatteryStatus {
        text: "󰂎 --".into(),
        tooltip: "Battery data unavailable".into(),
        ..BatteryStatus::default()
    };
    let Some(capacity) = read_trim(&battery.join("capacity")) else {
        return unavailable();
    };
    let Some(capacity_num) = capacity.parse::<u32>().ok().filter(|value| *value <= 100) else {
        return unavailable();
    };
    let status = read_trim(&battery.join("status")).unwrap_or_default();
    let online = ["Mains", "USB", "USB_C", "USB_PD"]
        .iter()
        .flat_map(|kind| supplies(kind))
        .any(|adapter| read_trim(&adapter.join("online")).as_deref() == Some("1"))
        || status.eq_ignore_ascii_case("charging");
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
        available: true,
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
    let unavailable = |status: &str| NetworkInfo {
        text: "󰖪".into(),
        tooltip: format!("Wi-Fi: {status}"),
        status: status.into(),
        ssid: None,
    };
    let Some(data) = child_process(
        "nmcli",
        &["-t", "-f", "DEVICE,TYPE,STATE", "device", "status"],
    ) else {
        return unavailable("unavailable");
    };
    let candidates: Vec<_> = data
        .lines()
        .map(split_terse)
        .filter(|fields| fields.get(1).is_some_and(|kind| kind == "wifi"))
        .filter(|fields| {
            config::get()
                .wifi
                .as_ref()
                .is_none_or(|name| fields.first() == Some(name))
        })
        .collect();
    if candidates.is_empty() {
        return unavailable("unavailable");
    }
    let Some(interface) = candidates
        .iter()
        .find(|fields| fields.get(2).is_some_and(|state| state == "connected"))
        .and_then(|fields| fields.first())
    else {
        let disabled = child_process("nmcli", &["radio", "wifi"]).as_deref() == Some("disabled");
        return unavailable(if disabled { "disabled" } else { "disconnected" });
    };
    let Some(data) = child_process(
        "nmcli",
        &[
            "-t",
            "-f",
            "ACTIVE,SSID,SIGNAL",
            "device",
            "wifi",
            "list",
            "ifname",
            interface,
            "--rescan",
            "no",
        ],
    ) else {
        return unavailable("unavailable");
    };
    let Some(fields) = data
        .lines()
        .map(split_terse)
        .find(|fields| fields.first().is_some_and(|value| value == "yes"))
    else {
        return unavailable("unavailable");
    };
    let ssid = fields.get(1).cloned().filter(|value| !value.is_empty());
    let signal = fields
        .get(2)
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(0);
    let ip = child_process("nmcli", &["-g", "IP4.ADDRESS", "device", "show", interface])
        .unwrap_or_default();
    NetworkInfo {
        text: signal_icon(signal).into(),
        tooltip: format!("{}\n{signal}% • {ip}", ssid.as_deref().unwrap_or("Wi-Fi")),
        status: "connected".into(),
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

pub fn spawn_audio_poller(sender: Sender<Option<String>>) {
    thread::spawn(move || {
        while !process::stopped() && !sender.is_closed() {
            if sender
                .send_blocking(child_process(
                    "wpctl",
                    &["get-volume", "@DEFAULT_AUDIO_SINK@"],
                ))
                .is_err()
            {
                return;
            }
            let mut command = Command::new("pactl");
            command
                .arg("subscribe")
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            if let Ok(mut child) = process::ManagedChild::spawn(&mut command)
                && let Some(stdout) = child.0.stdout.take()
            {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    if process::stopped() || sender.is_closed() {
                        break;
                    }
                    if (line.contains("sink") || line.contains("server"))
                        && sender
                            .send_blocking(child_process(
                                "wpctl",
                                &["get-volume", "@DEFAULT_AUDIO_SINK@"],
                            ))
                            .is_err()
                    {
                        return;
                    }
                }
            }
            if sender.send_blocking(None).is_err() {
                return;
            }
            if !process::pause(Duration::from_secs(2)) {
                return;
            }
        }
    });
}

pub fn spawn_temperature_poller(sender: Sender<Option<i64>>) {
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
            if value.is_none() {
                sensor = None;
            }
            if sender.send_blocking(value).is_err() || !process::pause(Duration::from_secs(3)) {
                return;
            }
        }
    });
}

pub fn spawn_network_poller(sender: Sender<NetworkInfo>) {
    thread::spawn(move || {
        loop {
            if sender.send_blocking(network_info()).is_err()
                || !process::pause(Duration::from_secs(5))
            {
                return;
            }
        }
    });
}

pub fn spawn_device_monitor(sender: Sender<DeviceEvent>) {
    thread::spawn(move || {
        while !process::stopped() && !sender.is_closed() {
            let mut command = Command::new("udevadm");
            command
                .args([
                    "monitor",
                    "--udev",
                    "--property",
                    "--subsystem-match=usb",
                    "--subsystem-match=backlight",
                    "--subsystem-match=power_supply",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            if let Ok(mut child) = process::ManagedChild::spawn(&mut command)
                && let Some(stdout) = child.0.stdout.take()
            {
                let mut properties = Vec::new();
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    if process::stopped() || sender.is_closed() {
                        break;
                    }
                    if line.is_empty() {
                        if let Some(event) = parse_device_event(&properties)
                            && sender.send_blocking(event).is_err()
                        {
                            return;
                        }
                        properties.clear();
                    } else if let Some((key, value)) = line.split_once('=') {
                        properties.push((key.to_owned(), value.to_owned()));
                    }
                }
            }
            if !process::pause(Duration::from_secs(2)) {
                return;
            }
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
