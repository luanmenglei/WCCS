//! Dynamic read-only inventory. No control decisions use these wall-clock samples.
use crate::model::Observation;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sample<T> {
    pub sampled_at: i64,
    pub observation: Observation<T>,
}

impl<T> Sample<T> {
    pub fn new(observation: Observation<T>) -> Self {
        Self {
            sampled_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            observation,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub uptime_seconds: Sample<u64>,
    pub cpu_percent: Sample<u32>,
    pub memory: Sample<Memory>,
    pub displays: Sample<Vec<Display>>,
    pub keyboards: Sample<u32>,
    pub mice: Sample<u32>,
}

/// GetSystemTimes includes idle time in kernel time. Reset samples cannot produce load.
pub fn cpu_percent(previous: (u64, u64), current: (u64, u64)) -> Option<u32> {
    let elapsed = current.0.checked_sub(previous.0)?;
    let idle = current.1.checked_sub(previous.1)?;
    if elapsed == 0 || idle > elapsed {
        return None;
    }
    Some(((elapsed - idle) as u128 * 100 / elapsed as u128) as u32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cpu_reset_zero_and_idle_are_distinct() {
        assert_eq!(super::cpu_percent((100, 50), (200, 70)), Some(80));
        assert_eq!(super::cpu_percent((100, 50), (100, 50)), None);
        assert_eq!(super::cpu_percent((100, 50), (90, 40)), None);
        assert_eq!(super::cpu_percent((100, 50), (110, 80)), None);
    }
}
