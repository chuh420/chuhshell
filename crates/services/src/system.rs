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

#[cfg(test)]
mod tests {
    use super::*;

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
