#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ServiceState<T> {
    #[default]
    Loading,
    Ready(T),
    Stale {
        value: T,
        error: String,
    },
    Error(String),
    Unavailable,
}

impl<T> ServiceState<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ready(value) | Self::Stale { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn status(&self) -> &str {
        match self {
            Self::Loading => "Loading…",
            Self::Ready(_) => "Ready",
            Self::Stale { error, .. } | Self::Error(error) => error,
            Self::Unavailable => "Unavailable",
        }
    }

    pub fn fail(&mut self, error: String) {
        *self = match std::mem::take(self) {
            Self::Ready(value) | Self::Stale { value, .. } => Self::Stale { value, error },
            _ => Self::Error(error),
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioState {
    pub percent: u8,
    pub muted: bool,
}

impl AudioState {
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut words = value.split_whitespace();
        if words.next() != Some("Volume:") {
            return Err("Invalid audio response".into());
        }
        let volume = words
            .next()
            .and_then(|word| word.parse::<f64>().ok())
            .filter(|volume| volume.is_finite() && *volume >= 0.0)
            .ok_or("Invalid audio response")?;
        let muted = match (words.next(), words.next()) {
            (None, None) => false,
            (Some("[MUTED]"), None) => true,
            _ => return Err("Invalid audio response".into()),
        };
        Ok(Self {
            percent: (volume * 100.0).round().clamp(0.0, 100.0) as u8,
            muted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_preserve_last_value_until_recovery() {
        let mut state = ServiceState::<u8>::default();
        assert_eq!(state, ServiceState::Loading);
        state.fail("offline".into());
        assert_eq!(state, ServiceState::Error("offline".into()));
        state = ServiceState::Ready(42);
        state.fail("offline".into());
        state.fail("retry failed".into());
        assert_eq!(state.value(), Some(&42));
        assert_eq!(state.ready(), None);
        assert_eq!(state.status(), "retry failed");
        state = ServiceState::Ready(43);
        assert_eq!(state.ready(), Some(&43));
        state = ServiceState::Unavailable;
        assert_eq!(state.value(), None);
    }

    #[test]
    fn audio_rejects_invalid_values_and_preserves_mute() {
        assert_eq!(
            AudioState::parse("Volume: 0.485 [MUTED]").unwrap(),
            AudioState {
                percent: 49,
                muted: true
            }
        );
        assert_eq!(AudioState::parse("Volume: 1.5").unwrap().percent, 100);
        for value in [
            "garbage 0.5",
            "Volume: NaN",
            "Volume: inf",
            "Volume: -1",
            "Volume: 0.5 garbage",
            "Volume:",
        ] {
            assert!(AudioState::parse(value).is_err(), "{value}");
        }
    }
}
