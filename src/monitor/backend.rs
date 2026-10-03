use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_CLIENTS: usize = 4096;
type Engines = HashMap<String, (u64, u64)>;
type Clients = HashMap<String, Engines>;

#[derive(Clone, Default)]
pub struct Snapshot {
    pub cpu: Option<f64>,
    pub memory: Option<(u64, u64)>,
    pub cpu_temperature: Option<i64>,
    pub gpu: Option<f64>,
    pub gpu_temperature: Option<i64>,
    pub gpu_partial: bool,
    pub gpu_sampled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cpu {
    total: u64,
    idle: u64,
}

fn text(path: &Path) -> Option<String> {
    crate::storage::read_text(path, 65536).ok()
}

fn number(path: &Path) -> Option<u64> {
    text(path)?.trim().parse().ok()
}

fn cpu(text: &str) -> Option<Cpu> {
    let mut fields = text.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values = fields
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    Some(Cpu {
        total: values.iter().try_fold(0u64, |sum, n| sum.checked_add(*n))?,
        idle: values[3].checked_add(values.get(4).copied().unwrap_or(0))?,
    })
}

fn cpu_usage(previous: Cpu, current: Cpu) -> Option<f64> {
    let total = current.total.checked_sub(previous.total)?;
    let idle = current.idle.checked_sub(previous.idle)?;
    (total > 0 && idle <= total).then(|| (total - idle) as f64 * 100.0 / total as f64)
}

fn memory(text: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next()? {
            "MemTotal:" => total = parts.next()?.parse::<u64>().ok(),
            "MemAvailable:" => available = parts.next()?.parse::<u64>().ok(),
            _ => {}
        }
    }
    let total = total?.checked_mul(1024)?;
    let available = available?.checked_mul(1024)?;
    (total > 0).then_some((total.checked_sub(available)?, total))
}

fn temperature(path: &Path) -> Option<i64> {
    text(path)?
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|value| (-20000..=150000).contains(value))
}

fn gpu_device() -> Option<PathBuf> {
    let mut cards = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .take(128)
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.strip_prefix("card").is_some_and(|index| {
                    !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit())
                })
            })
        })
        .map(|entry| entry.path().join("device"))
        .collect::<Vec<_>>();
    cards.sort();
    cards
        .iter()
        .find(|path| number(&path.join("boot_vga")) == Some(1))
        .cloned()
        .or_else(|| cards.into_iter().next())
}

fn gpu_temperature(device: &Path) -> Option<i64> {
    let mut fallback = None;
    for entry in std::fs::read_dir(device.join("hwmon"))
        .ok()?
        .take(32)
        .filter_map(Result::ok)
    {
        let path = entry.path();
        for index in 1..=16 {
            let Some(value) = temperature(&path.join(format!("temp{index}_input"))) else {
                continue;
            };
            let label = text(&path.join(format!("temp{index}_label")))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if matches!(label.as_str(), "edge" | "gpu" | "gpu temperature") {
                return Some(value);
            }
            fallback.get_or_insert(value);
        }
    }
    fallback
}

fn client(text: &str, device: &str) -> Option<(String, Engines)> {
    let mut id = None;
    let mut pdev = None;
    let mut engines = HashMap::new();
    let mut capacities = HashMap::new();
    for line in text.lines().take(256) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "drm-client-id" => id = value.parse::<u64>().ok(),
            "drm-pdev" => pdev = Some(value),
            _ => {
                if let Some(engine) = key.strip_prefix("drm-engine-capacity-") {
                    capacities.insert(engine.to_owned(), value.parse::<u64>().ok()?);
                } else if let Some(engine) = key.strip_prefix("drm-engine-") {
                    let mut parts = value.split_whitespace();
                    let counter = parts.next()?.parse::<u64>().ok()?;
                    if parts.next()? == "ns" {
                        engines.insert(engine.to_owned(), counter);
                    }
                }
            }
        }
    }
    if pdev? != device || engines.is_empty() {
        return None;
    }
    let engines = engines
        .into_iter()
        .map(|(name, counter)| {
            let capacity = capacities.get(&name).copied().unwrap_or(1).max(1);
            (name, (counter, capacity))
        })
        .collect();
    Some((format!("{device}:{}", id?), engines))
}

fn clients(device: &str) -> (Clients, bool) {
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut clients = HashMap::new();
    let mut partial = false;
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return (clients, true);
    };
    let mut checked = 0usize;
    for process in processes.take(8192).filter_map(Result::ok) {
        if !process
            .file_name()
            .to_str()
            .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        if Instant::now() >= deadline {
            partial = true;
            break;
        }
        let Ok(fds) = std::fs::read_dir(process.path().join("fd")) else {
            partial = true;
            continue;
        };
        for fd in fds.filter_map(Result::ok) {
            checked += 1;
            if checked > 32768 || clients.len() >= MAX_CLIENTS || Instant::now() >= deadline {
                return (clients, true);
            }
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            if !target.starts_with("/dev/dri") {
                continue;
            }
            let Some(info) = text(&process.path().join("fdinfo").join(fd.file_name())) else {
                partial = true;
                continue;
            };
            if let Some((id, engines)) = client(&info, device) {
                clients.entry(id).or_insert(engines);
            }
        }
    }
    (clients, partial)
}

fn gpu_usage(previous: &Clients, current: &Clients, seconds: f64) -> Option<f64> {
    if seconds <= 0.0 || current.is_empty() {
        return None;
    }
    let mut totals = HashMap::<&str, f64>::new();
    let mut sampled = false;
    for (id, engines) in current {
        let Some(old) = previous.get(id) else {
            continue;
        };
        for (engine, &(counter, capacity)) in engines {
            let Some(&(before, old_capacity)) = old.get(engine) else {
                continue;
            };
            if capacity != old_capacity {
                continue;
            }
            if let Some(delta) = counter.checked_sub(before) {
                sampled = true;
                *totals.entry(engine).or_default() += delta as f64 / capacity as f64;
            }
        }
    }
    sampled.then(|| {
        (totals.values().copied().fold(0.0, f64::max) / (seconds * 1e9) * 100.0).clamp(0.0, 100.0)
    })
}

#[derive(Default)]
pub struct Sampler {
    previous_cpu: Option<Cpu>,
    previous_gpu: Option<(Instant, Clients)>,
}

impl Sampler {
    pub fn sample(&mut self, include_gpu: bool) -> Snapshot {
        let current_cpu = text(Path::new("/proc/stat")).and_then(|s| cpu(&s));
        let usage = self
            .previous_cpu
            .zip(current_cpu)
            .and_then(|(old, new)| cpu_usage(old, new));
        self.previous_cpu = current_cpu;
        let mut snapshot = Snapshot {
            cpu: usage,
            memory: text(Path::new("/proc/meminfo")).and_then(|s| memory(&s)),
            cpu_temperature: crate::modules::thermal_sensor_path()
                .as_deref()
                .and_then(temperature),
            gpu_sampled: include_gpu,
            ..Snapshot::default()
        };
        if include_gpu {
            if let Some(device) = gpu_device() {
                snapshot.gpu_temperature = gpu_temperature(&device);
                if let Some(busy) = number(&device.join("gpu_busy_percent")).filter(|n| *n <= 100) {
                    snapshot.gpu = Some(busy as f64);
                    self.previous_gpu = None;
                } else if let Some(pdev) = std::fs::canonicalize(&device).ok().and_then(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                }) {
                    let (current, partial) = clients(&pdev);
                    let now = Instant::now();
                    snapshot.gpu_partial = true;
                    if let Some((before, old)) = &self.previous_gpu {
                        snapshot.gpu =
                            gpu_usage(old, &current, now.duration_since(*before).as_secs_f64());
                    }
                    if partial && current.is_empty() {
                        snapshot.gpu = None
                    }
                    self.previous_gpu = Some((now, current));
                }
            }
        } else {
            self.previous_gpu = None;
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_excludes_guest_and_includes_iowait_in_idle() {
        let old = cpu("cpu 100 0 50 800 50 0 0 0 90 0\n").unwrap();
        let new = cpu("cpu 120 0 60 850 70 0 0 0 100 0\n").unwrap();
        assert_eq!(cpu_usage(old, new), Some(30.0));
        assert_eq!(cpu_usage(new, old), None);
        assert_eq!(cpu_usage(old, old), None);
        assert_eq!(cpu("cpu 1 bad 3 4"), None);
    }

    #[test]
    fn memory_uses_available_and_rejects_invalid_totals() {
        assert_eq!(
            memory("MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 750 kB\n"),
            Some((250 * 1024, 1000 * 1024))
        );
        assert_eq!(memory("MemTotal: 1000 kB"), None);
        assert_eq!(memory("MemTotal: 0 kB\nMemAvailable: 0 kB"), None);
        assert_eq!(memory("MemTotal: 10 kB\nMemAvailable: 20 kB"), None);
    }

    #[test]
    fn gpu_filters_device_and_measures_busiest_engine() {
        let info = "drm-client-id: 7\ndrm-pdev: 0000:00:02.0\ndrm-engine-render: 100000000 ns\ndrm-engine-video: 200000000 ns\ndrm-engine-capacity-video: 2\n";
        assert!(client(info, "0000:01:00.0").is_none());
        let (id, engines) = client(info, "0000:00:02.0").unwrap();
        let old = HashMap::from([(id.clone(), engines)]);
        let (_, engines) = client(
            &info
                .replace("100000000 ns", "600000000 ns")
                .replace("200000000 ns", "1800000000 ns"),
            "0000:00:02.0",
        )
        .unwrap();
        let current = HashMap::from([(id, engines)]);
        assert_eq!(gpu_usage(&old, &current, 1.0), Some(80.0));
        assert_eq!(gpu_usage(&Clients::new(), &current, 1.0), None);
        assert_eq!(gpu_usage(&current, &old, 1.0), None);
    }
}
