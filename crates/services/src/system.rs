use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Resources {
    pub cpu: Option<u32>,
    pub ram_used: f64,
    pub ram_total: f64,
}

#[derive(Default)]
pub struct ResourceMonitor {
    previous: Option<HashMap<String, (u64, u64)>>,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

pub fn round_half_even(value: f64) -> f64 {
    let rounded = value.round();
    if (value - value.trunc()).abs() == 0.5 {
        2.0 * (value / 2.0).round()
    } else {
        rounded
    }
}

pub fn cpu_lines(stat: &str) -> HashMap<String, (u64, u64)> {
    let mut result = HashMap::new();
    for line in stat.lines() {
        if !line.starts_with("cpu") {
            break;
        }
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let values: Vec<u64> = fields.take(8).filter_map(|v| v.parse().ok()).collect();
        if values.len() < 5 {
            continue;
        }
        let total = values.iter().sum();
        result.insert(name.to_string(), (total, values[3] + values[4]));
    }
    result
}

pub fn usage(current: (u64, u64), previous: Option<(u64, u64)>) -> Option<u32> {
    let previous = previous?;
    let elapsed = current.0 as i64 - previous.0 as i64;
    if elapsed <= 0 {
        return Some(0);
    }
    let idle = current.1 as i64 - previous.1 as i64;
    let value = round_half_even(100.0 * (1.0 - idle as f64 / elapsed as f64));
    Some(value.clamp(0.0, 100.0) as u32)
}

pub fn meminfo(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let value = value.split_whitespace().next()?.parse().ok()?;
            Some((key.to_string(), value))
        })
        .collect()
}

impl ResourceMonitor {
    pub fn sample(&mut self) -> Resources {
        let lines = cpu_lines(&read("/proc/stat"));
        let memory = meminfo(&read("/proc/meminfo"));
        let cpu = match (lines.get("cpu"), &self.previous) {
            (Some(current), Some(previous)) => usage(*current, previous.get("cpu").copied()),
            _ => None,
        };
        let total = memory.get("MemTotal").copied().unwrap_or(0) as f64;
        let available = memory.get("MemAvailable").copied().unwrap_or(0) as f64;
        self.previous = Some(lines);
        Resources {
            cpu,
            ram_used: (total - available) / 1048576.0,
            ram_total: total / 1048576.0,
        }
    }
}

pub fn brightness() -> u32 {
    let Ok(entries) = std::fs::read_dir("/sys/class/backlight") else {
        return 50;
    };
    let mut devices: Vec<std::path::PathBuf> =
        entries.flatten().map(|entry| entry.path()).collect();
    devices.sort();
    let Some(device) = devices.first() else {
        return 50;
    };
    let number = |name: &str| -> f64 {
        std::fs::read_to_string(device.join(name))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0.0)
    };
    let maximum = number("max_brightness");
    if maximum <= 0.0 {
        return 50;
    }
    round_half_even(number("brightness") / maximum * 100.0).clamp(0.0, 100.0) as u32
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wired {
    pub network: String,
    pub connected: bool,
}

pub fn wired_from(route: &str, is_wireless: impl Fn(&str) -> bool) -> Wired {
    for line in route.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() > 3
            && fields[1] == "00000000"
            && u32::from_str_radix(fields[3], 16).is_ok_and(|flags| flags & 1 != 0)
            && !is_wireless(fields[0])
        {
            return Wired {
                network: fields[0].to_string(),
                connected: true,
            };
        }
    }
    Wired {
        network: "Offline".into(),
        connected: false,
    }
}

pub fn wired() -> Wired {
    wired_from(&read("/proc/net/route"), |device| {
        std::path::Path::new("/sys/class/net")
            .join(device)
            .join("wireless")
            .exists()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wired_needs_up_default_route_on_non_wireless() {
        let route = "Iface\tDestination\tGateway\tFlags\nwlan0\t00000000\t0101A8C0\t0003\nenp1\t00000000\t0101A8C0\t0003\n";
        let result = wired_from(route, |device| device == "wlan0");
        assert_eq!(result.network, "enp1");
        assert!(result.connected);
        let offline = wired_from(route, |_| true);
        assert_eq!(offline.network, "Offline");
    }

    #[test]
    fn python_rounding() {
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(3.5), 4.0);
        assert_eq!(round_half_even(3.4), 3.0);
    }

    #[test]
    fn usage_matches_resources_py() {
        let stat = "cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3 4 5 6 7 8\nintr 1\n";
        let first = cpu_lines(stat);
        assert_eq!(first.get("cpu"), Some(&(1000, 800)));
        let second = cpu_lines("cpu  150 0 150 750 150 0 0 0 0 0\n");
        assert_eq!(usage(second["cpu"], first.get("cpu").copied()), Some(50));
        assert_eq!(usage(second["cpu"], None), None);
    }

    #[test]
    fn meminfo_parses_kib() {
        let memory = meminfo("MemTotal:       16000000 kB\nMemAvailable:    8000000 kB\n");
        assert_eq!(memory["MemTotal"], 16000000);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatteryStats {
    pub health: i64,
    pub full: f64,
    pub design: f64,
    pub cycles: i64,
}

fn number(path: &std::path::Path) -> f64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn round_to(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    round_half_even(value * factor) / factor
}

pub fn battery_stats_from(dir: &std::path::Path) -> BatteryStats {
    let read = |name: &str| number(&dir.join(name));
    let voltage = read("voltage_now");
    let pick = |energy: &str, charge: &str| {
        let value = read(energy);
        if value != 0.0 {
            value
        } else {
            read(charge) * voltage / 1_000_000.0
        }
    };
    let full = pick("energy_full", "charge_full");
    let design = pick("energy_full_design", "charge_full_design");
    BatteryStats {
        health: if design != 0.0 {
            round_half_even(full / design * 100.0) as i64
        } else {
            0
        },
        full: round_to(full / 1_000_000.0, 1),
        design: round_to(design / 1_000_000.0, 1),
        cycles: read("cycle_count") as i64,
    }
}

pub fn battery_stats() -> Option<BatteryStats> {
    let mut batteries: Vec<std::path::PathBuf> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("BAT"))
        })
        .collect();
    batteries.sort();
    batteries.first().map(|dir| battery_stats_from(dir))
}
