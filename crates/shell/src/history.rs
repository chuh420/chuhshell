use std::path::PathBuf;

use serde_json::Value;

pub const LIMIT: usize = 360;

#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub t: i64,
    pub level: i64,
    pub power: f64,
}

pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".local/state/quickshell/material/battery-history.json")
}

fn js_number(value: f64) -> String {
    if !value.is_finite() {
        "null".into()
    } else if value.fract() == 0.0 && value.abs() < 9.0e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

pub fn encode(points: &[Point]) -> String {
    let items: Vec<String> = points
        .iter()
        .map(|point| {
            format!(
                "{{\"t\":{},\"level\":{},\"power\":{}}}",
                point.t,
                point.level,
                js_number(point.power)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

pub fn decode(text: &str) -> Vec<Point> {
    let Ok(Value::Array(values)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|value| {
            Some(Point {
                t: value.get("t")?.as_f64()? as i64,
                level: value.get("level")?.as_f64()? as i64,
                power: value.get("power").and_then(Value::as_f64).unwrap_or(0.0),
            })
        })
        .collect()
}

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

pub struct History {
    pub points: Vec<Point>,
}

impl History {
    pub fn load() -> Self {
        let cutoff = now_ms() - LIMIT as i64 * 60000;
        let points = std::fs::read_to_string(path())
            .map(|text| decode(&text))
            .unwrap_or_default()
            .into_iter()
            .filter(|point| point.t >= cutoff)
            .collect();
        Self { points }
    }

    pub fn record(&mut self, level: u32, power: f64) {
        let now = now_ms();
        let cutoff = now - LIMIT as i64 * 60000;
        self.points.retain(|point| point.t >= cutoff);
        self.points.push(Point {
            t: now,
            level: level as i64,
            power: (power * 10.0 + 0.5).floor() / 10.0,
        });
        if self.points.len() > LIMIT {
            let excess = self.points.len() - LIMIT;
            self.points.drain(..excess);
        }
        self.save();
    }

    fn save(&self) {
        let target = path();
        if let Some(parent) = target.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            log::warn!("battery history: {err}");
            return;
        }
        let temporary = target.with_extension("json.tmp");
        let result = std::fs::write(&temporary, encode(&self.points))
            .and_then(|_| std::fs::rename(&temporary, &target));
        if let Err(err) = result {
            log::warn!("battery history: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_json_stringify() {
        let points = vec![
            Point {
                t: 1,
                level: 80,
                power: 12.3,
            },
            Point {
                t: 2,
                level: 79,
                power: 5.0,
            },
        ];
        assert_eq!(
            encode(&points),
            r#"[{"t":1,"level":80,"power":12.3},{"t":2,"level":79,"power":5}]"#
        );
        assert_eq!(decode(&encode(&points)), points);
    }
}
