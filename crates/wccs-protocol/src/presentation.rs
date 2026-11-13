//! Display metadata carries no authority to enter or extend contest control.
use crate::control::{Command, WaitingIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Logo {
    pub school: String,
    pub source: String,
    pub sha256: String,
    pub bmp_hex: String,
}
impl Logo {
    pub fn pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        if self.school.is_empty()
            || self.school.len() > 200
            || self.source.len() > 300
            || self.source.chars().any(char::is_control)
            || self.bmp_hex.len() > 56_000
            || !self.bmp_hex.len().is_multiple_of(2)
        {
            return None;
        }
        let bytes: Option<Vec<u8>> = self
            .bmp_hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let text = std::str::from_utf8(pair).ok()?;
                u8::from_str_radix(text, 16).ok()
            })
            .collect();
        let bytes = bytes?;
        if bytes.len() < 54
            || &bytes[..2] != b"BM"
            || format!("{:x}", Sha256::digest(&bytes)) != self.sha256
        {
            return None;
        }
        let u32at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let width = u32at(18);
        let height = u32at(22);
        if u32at(2) as usize != bytes.len()
            || u32at(10) != 54
            || u32at(14) != 40
            || bytes[26..30] != [1, 0, 24, 0]
            || u32at(30) != 0
            || !(1..=96).contains(&width)
            || !(1..=96).contains(&height)
        {
            return None;
        }
        let size = (width * 3).div_ceil(4) * 4 * height;
        if bytes.len() != 54 + size as usize {
            return None;
        }
        Some((width, height, bytes[54..].to_vec()))
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub starts_at_ms: Option<i64>,
    pub domjudge_api: Option<String>,
    pub domjudge_contest: Option<String>,
    pub logos: Vec<Logo>,
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.starts_at_ms
            .is_none_or(|v| (0..=32_503_680_000_000).contains(&v))
            && self
                .domjudge_contest
                .as_ref()
                .is_none_or(|id| crate::valid_id(id))
            && self.domjudge_api.is_some() == self.domjudge_contest.is_some()
            && self.logos.len() <= 16
            && self.logos.iter().all(|logo| logo.pixels().is_some())
            && self
                .logos
                .iter()
                .map(|l| &l.school)
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.logos.len()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub schema_version: u32,
    pub command_id: String,
    pub device_id: String,
    pub generation: i64,
    pub identity: WaitingIdentity,
    pub configuration_version: i64,
    pub configuration_digest: String,
    pub server_time_ms: i64,
    pub starts_at_ms: Option<i64>,
    pub domjudge_api: Option<String>,
    pub domjudge_contest: Option<String>,
    pub logo: Option<Logo>,
}
impl Data {
    pub fn matches(&self, command: &Command) -> bool {
        self.schema_version == 1
            && command.action == "enter"
            && self.command_id == command.command_id
            && self.device_id == command.device_id
            && self.generation == command.generation
            && self.identity == command.identity
            && self
                .logo
                .as_ref()
                .is_none_or(|l| l.school == self.identity.school && l.pixels().is_some())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub state: String,
    pub source: String,
    pub checked_at_ms: i64,
    pub detail: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub data: Data,
    pub received_tick_ms: u64,
    pub round_trip_ms: u64,
    pub clock_offset_ms: i64,
    pub wccs: Connection,
    pub oj: Connection,
}
impl Display {
    pub fn server_now(&self, tick: u64) -> Option<i64> {
        let elapsed = tick.checked_sub(self.received_tick_ms)?;
        self.data
            .server_time_ms
            .checked_add(i64::try_from(self.round_trip_ms / 2).ok()?)?
            .checked_add(i64::try_from(elapsed).ok()?)
    }
    pub fn stale(&self, tick: u64) -> bool {
        tick.checked_sub(self.received_tick_ms)
            .is_none_or(|elapsed| elapsed > 60_000)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_clock_is_monotonic_and_identity_changes_reject_cached_data() {
        let command: Command = serde_json::from_value(serde_json::json!({
            "protocol_version":1,"command_id":"command","request_id":"request","sequence":1,
            "device_id":"device","generation":1,"device_version":1,"instance_id":"host",
            "session_key":"session","action":"enter","expires_at":100,
            "identity":{"contest_id":"contest","contest_name":"test","contest_version":1,
            "participant_id":"participant","name":"name","school":"school","seat_id":"seat",
            "seat_label":"1","room_id":"room","room_version":1}
        }))
        .unwrap();
        let data = Data {
            schema_version: 1,
            command_id: command.command_id.clone(),
            device_id: command.device_id.clone(),
            generation: 1,
            identity: command.identity.clone(),
            configuration_version: 1,
            configuration_digest: "a".repeat(64),
            server_time_ms: 100_000,
            starts_at_ms: Some(200_000),
            domjudge_api: None,
            domjudge_contest: None,
            logo: None,
        };
        let connection = Connection {
            state: "online".into(),
            source: "test".into(),
            checked_at_ms: 100_000,
            detail: "test".into(),
        };
        let mut display = Display {
            data,
            received_tick_ms: 10_000,
            round_trip_ms: 100,
            clock_offset_ms: -999_000,
            wccs: connection.clone(),
            oj: connection,
        };
        assert_eq!(display.server_now(11_000), Some(101_050));
        display.clock_offset_ms = 999_000; // calendar rollback cannot affect extrapolation
        assert_eq!(display.server_now(11_000), Some(101_050));
        assert_eq!(display.server_now(9_000), None); // reboot/reset never extends cached time
        assert!(display.stale(9_000));
        assert!(display.stale(70_001));
        assert!(display.data.matches(&command));
        let mut changed = command.clone();
        changed.identity.contest_version += 1;
        assert!(!display.data.matches(&changed));
        changed = command.clone();
        changed.generation += 1;
        assert!(!display.data.matches(&changed));
        changed = command;
        changed.identity.seat_id = "other".into();
        assert!(!display.data.matches(&changed));
    }
    #[test]
    fn malformed_logo_and_external_source_payload_are_rejected() {
        let logo = super::Logo {
            school: "school".into(),
            source: "upload".into(),
            sha256: "0".repeat(64),
            bmp_hex: "ff".repeat(60),
        };
        assert!(logo.pixels().is_none());
    }
    #[test]
    fn embedded_logo_requires_exact_bytes_hash_and_bounded_bitmap() {
        let mut bytes = vec![0u8; 58];
        bytes[..2].copy_from_slice(b"BM");
        for (offset, value) in [(2, 58u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[26..30].copy_from_slice(&[1, 0, 24, 0]);
        let mut logo = Logo {
            school: "school".into(),
            source: "upload-test.bmp".into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bmp_hex: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        };
        assert_eq!(logo.pixels().unwrap().0, 1);
        logo.bmp_hex.push_str("ff");
        assert!(logo.pixels().is_none());
        logo.bmp_hex.truncate(116);
        logo.sha256 = "0".repeat(64);
        assert!(logo.pixels().is_none());
    }
}
