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
    detail: bool,
    process_state: Option<(std::time::Instant, HashMap<i32, ProcInfo>)>,
    sensor: Option<Option<std::path::PathBuf>>,
    last_detail: Option<Detail>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub name: String,
    pub cpu: f64,
    pub mem: i64,
    pub own: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Processes {
    pub cpu: Vec<Proc>,
    pub mem: Vec<Proc>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub cores: Vec<Option<u32>>,
    pub frequency: f64,
    pub temperature: Option<i64>,
    pub swap_used: f64,
    pub swap_total: f64,
    pub disk_used: f64,
    pub disk_total: f64,
    pub uptime: f64,
    pub load: Vec<f64>,
    pub processes: Processes,
}

#[derive(Clone, Debug)]
pub struct ProcInfo {
    name: String,
    ticks: u64,
    rss: u64,
    own: bool,
}

const TICKS: f64 = 100.0;
const PAGE: u64 = 4096;

fn temperature_path() -> Option<std::path::PathBuf> {
    let mut hwmons: Vec<std::path::PathBuf> = std::fs::read_dir("/sys/class/hwmon")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    hwmons.sort();
    for hwmon in &hwmons {
        if std::fs::read_to_string(hwmon.join("name"))
            .unwrap_or_default()
            .trim()
            != "coretemp"
        {
            continue;
        }
        let mut labels: Vec<std::path::PathBuf> = std::fs::read_dir(hwmon)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with("temp") && name.ends_with("_label")
                })
            })
            .collect();
        labels.sort();
        for label in labels {
            if std::fs::read_to_string(&label)
                .unwrap_or_default()
                .starts_with("Package")
            {
                let name = label
                    .file_name()?
                    .to_string_lossy()
                    .replace("_label", "_input");
                return Some(label.with_file_name(name));
            }
        }
        return Some(hwmon.join("temp1_input"));
    }
    let mut zones: Vec<std::path::PathBuf> = std::fs::read_dir("/sys/class/thermal")
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("temp"))
        .filter(|path| {
            path.parent()
                .and_then(|parent| parent.file_name())
                .is_some_and(|name| name.to_string_lossy().starts_with("thermal_zone"))
                && path.exists()
        })
        .collect();
    zones.sort();
    zones.into_iter().next()
}

fn frequency() -> f64 {
    let Ok(entries) = std::fs::read_dir("/sys/devices/system/cpu") else {
        return 0.0;
    };
    let values: Vec<f64> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.len() > 3
                && name.starts_with("cpu")
                && name[3..].chars().all(|c| c.is_ascii_digit())
        })
        .filter_map(|entry| {
            std::fs::read_to_string(entry.path().join("cpufreq/scaling_cur_freq"))
                .ok()
                .map(|text| text.trim().parse::<f64>().unwrap_or(0.0))
        })
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    round_half_even(values.iter().sum::<f64>() / values.len() as f64 / 1_000_000.0 * 100.0) / 100.0
}

fn processes() -> HashMap<i32, ProcInfo> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self")
        .map(|meta| meta.uid())
        .unwrap_or(u32::MAX);
    let mut result = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return result;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let path = entry.path();
        let (Ok(stat), Ok(statm), Ok(meta)) = (
            std::fs::read_to_string(path.join("stat")),
            std::fs::read_to_string(path.join("statm")),
            entry.metadata(),
        ) else {
            continue;
        };
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            continue;
        };
        let name = stat[open + 1..close].to_string();
        let fields: Vec<&str> = stat
            .get(close + 2..)
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let ticks = fields
            .get(11)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            + fields
                .get(12)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
        let rss = statm
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * PAGE;
        result.insert(
            pid,
            ProcInfo {
                name,
                ticks,
                rss,
                own: meta.uid() == uid,
            },
        );
    }
    result
}

pub fn top(
    current: &HashMap<i32, ProcInfo>,
    previous: &HashMap<i32, ProcInfo>,
    elapsed: f64,
    cores: f64,
) -> Processes {
    let mut pids: Vec<&i32> = current.keys().collect();
    pids.sort();
    let rows: Vec<Proc> = pids
        .into_iter()
        .map(|pid| {
            let item = &current[pid];
            let delta = match previous.get(pid) {
                Some(before) if before.name == item.name => item.ticks as f64 - before.ticks as f64,
                _ => 0.0,
            };
            let cpu = if elapsed > 0.0 {
                100.0 * delta / TICKS / elapsed / cores
            } else {
                0.0
            };
            Proc {
                pid: *pid,
                name: item.name.clone(),
                cpu: round_half_even(cpu * 10.0) / 10.0,
                mem: round_half_even(item.rss as f64 / 1_048_576.0) as i64,
                own: item.own,
            }
        })
        .collect();
    let mut by_cpu = rows.clone();
    by_cpu.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.mem.cmp(&a.mem))
    });
    by_cpu.truncate(8);
    let mut by_mem = rows;
    by_mem.sort_by_key(|row| std::cmp::Reverse(row.mem));
    by_mem.truncate(8);
    Processes {
        cpu: by_cpu,
        mem: by_mem,
    }
}

#[repr(C)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    spare: [i32; 6],
}

unsafe extern "C" {
    fn statvfs(path: *const std::ffi::c_char, buf: *mut StatVfs) -> std::ffi::c_int;
}

fn disk() -> (f64, f64) {
    let mut buf = std::mem::MaybeUninit::<StatVfs>::zeroed();
    let ok = unsafe { statvfs(c"/".as_ptr(), buf.as_mut_ptr()) } == 0;
    if !ok {
        return (0.0, 0.0);
    }
    let stat = unsafe { buf.assume_init() };
    let gib = 1_073_741_824.0;
    (
        (stat.f_blocks - stat.f_bfree) as f64 * stat.f_frsize as f64 / gib,
        stat.f_blocks as f64 * stat.f_frsize as f64 / gib,
    )
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
        self.last_detail = None;
        if self.detail {
            self.last_detail = Some(self.detail_sample(&lines, &memory));
        }
        self.previous = Some(lines);
        Resources {
            cpu,
            ram_used: (total - available) / 1048576.0,
            ram_total: total / 1048576.0,
        }
    }

    pub fn take_detail(&mut self) -> Option<Detail> {
        self.last_detail.take()
    }

    pub fn set_detail(&mut self, enabled: bool) -> bool {
        let started = enabled && !self.detail;
        if started {
            self.process_state = Some((std::time::Instant::now(), processes()));
        }
        if !enabled {
            self.process_state = None;
        }
        self.detail = enabled;
        started
    }

    fn detail_sample(
        &mut self,
        lines: &HashMap<String, (u64, u64)>,
        memory: &HashMap<String, u64>,
    ) -> Detail {
        let mut keys: Vec<&String> = lines.keys().filter(|key| *key != "cpu").collect();
        keys.sort_by_key(|key| key[3..].parse::<u32>().unwrap_or(0));
        let cores: Vec<Option<u32>> = keys
            .iter()
            .map(|key| {
                let previous = self.previous.as_ref()?.get(*key).copied();
                usage(lines[*key], previous)
            })
            .collect();
        let core_count = keys.len().max(1) as f64;
        let now = std::time::Instant::now();
        let current = processes();
        let processes = match &self.process_state {
            Some((at, previous)) => top(
                &current,
                previous,
                now.duration_since(*at).as_secs_f64(),
                core_count,
            ),
            None => Processes::default(),
        };
        self.process_state = Some((now, current));
        let sensor = self.sensor.get_or_insert_with(temperature_path).clone();
        let temperature = sensor.map(|path| {
            let value = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| text.trim().parse::<f64>().ok())
                .unwrap_or(0.0);
            round_half_even(value / 1000.0) as i64
        });
        let kib = |key: &str| memory.get(key).copied().unwrap_or(0) as f64;
        let (disk_used, disk_total) = disk();
        let load = read("/proc/loadavg")
            .split_whitespace()
            .take(3)
            .filter_map(|value| value.parse::<f64>().ok())
            .collect();
        let uptime = read("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        Detail {
            cores: if self.previous.is_some() {
                cores
            } else {
                vec![None; keys.len()]
            },
            frequency: frequency(),
            temperature,
            swap_used: (kib("SwapTotal") - kib("SwapFree")) / 1048576.0,
            swap_total: kib("SwapTotal") / 1048576.0,
            disk_used,
            disk_total,
            uptime,
            load,
            processes,
        }
    }
}

pub fn backlight_available() -> bool {
    std::fs::read_dir("/sys/class/backlight").is_ok_and(|mut entries| entries.next().is_some())
        && crate::programs::exists("brightnessctl")
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

#[cfg(test)]
mod detail_tests {
    use super::*;

    #[test]
    fn disk_reports_root() {
        let (used, total) = disk();
        assert!(total > 0.0 && used <= total);
    }

    #[test]
    fn top_orders_like_python() {
        let info = |name: &str, ticks, rss| ProcInfo {
            name: name.into(),
            ticks,
            rss,
            own: true,
        };
        let previous: HashMap<i32, ProcInfo> = [(1, info("a", 0, 0)), (2, info("b", 0, 0))]
            .into_iter()
            .collect();
        let current: HashMap<i32, ProcInfo> = [
            (1, info("a", 50, 1 << 20)),
            (2, info("b", 100, 3 << 20)),
            (3, info("c", 10, 2 << 20)),
        ]
        .into_iter()
        .collect();
        let top = top(&current, &previous, 1.0, 2.0);
        let cpu: Vec<i32> = top.cpu.iter().map(|p| p.pid).collect();
        let mem: Vec<i32> = top.mem.iter().map(|p| p.pid).collect();
        assert_eq!(cpu, vec![2, 1, 3]);
        assert_eq!(mem, vec![2, 3, 1]);
        assert_eq!(top.cpu[0].cpu, 50.0);
    }
}
