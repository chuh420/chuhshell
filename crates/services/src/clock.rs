use std::time::Duration;

use jiff::Zoned;

pub fn now() -> Zoned {
    Zoned::now()
}

pub fn until_next_minute(time: &Zoned) -> Duration {
    let seconds = time.second() as u64;
    let nanos = time.subsec_nanosecond().max(0) as u64;
    let elapsed = Duration::from_secs(seconds) + Duration::from_nanos(nanos);
    Duration::from_secs(60).saturating_sub(elapsed) + Duration::from_millis(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_until_boundary() {
        let time: Zoned = "2026-10-31T14:05:42.250+03:00[Europe/Moscow]"
            .parse()
            .expect("valid time");
        assert_eq!(until_next_minute(&time), Duration::from_millis(17_751));
    }
}
