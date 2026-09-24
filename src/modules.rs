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
}

#[derive(Clone, Debug, Default)]
pub struct BatteryStatus {
    pub text: String,
    pub level: String,
    pub tooltip: String,
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

fn battery_estimate(battery: &Path, status: &str) -> String {
    for (now_key, power_key) in [("energy_now", "power_now"), ("charge_now", "current_now")] {
        let now = read_trim(&battery.join(now_key)).and_then(|value| value.parse::<u64>().ok());
        let power = read_trim(&battery.join(power_key))
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0);
        if let (Some(now), Some(power)) = (now, power) {
            let hours = now / power;
            let minutes = (now % power) * 60 / power;
            return format!("{hours}h {minutes:02}m");
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
        };
    };
    let name = fields
        .get(1)
        .map(|value| value.to_lowercase())
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
    }
}

fn brightness_level(device: &str) -> Option<(u8, &'static str)> {
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

pub fn spawn_brightness_poller(sender: SyncSender<Option<(u8, &'static str)>>) {
    thread::spawn(move || {
        let mut device = backlight_device();
        loop {
            if device.is_none() {
                device = backlight_device();
            }
            let level = device.as_deref().and_then(brightness_level);
            let _ = sender.try_send(level);
            thread::sleep(Duration::from_millis(50));
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

pub fn spawn_battery_poller(sender: SyncSender<BatteryStatus>) {
    thread::spawn(move || {
        loop {
            let _ = sender.try_send(battery_status());
            thread::sleep(Duration::from_secs(30));
        }
    });
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
}
