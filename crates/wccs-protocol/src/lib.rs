#![forbid(unsafe_code)]

pub mod control;
pub mod diagnostics;
pub mod distribution;
pub mod health;
pub mod login;
pub mod maintenance;
pub mod preparation;
pub mod presentation;
pub mod recovery;

mod device;
mod tool_checks;
pub use device::{Heartbeat, HeartbeatReply, ImageRegistration, Registration, image_device_id};
pub use tool_checks::{
    CheckResult, Job, PublishRequest, RecheckRequest, validate_result, validate_tools,
    windows_absolute_file,
};

use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROTOCOL_VERSION: u32 = 1;
pub const ONLINE_SECONDS: i64 = 35;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn bytes_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub fn digest(value: &str) -> String {
    bytes_digest(value.as_bytes())
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}
