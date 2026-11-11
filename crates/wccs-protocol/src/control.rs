//! Waiting control v1. Tool jobs retain their independent HTTP ownership.
use serde::{Deserialize, Serialize};

pub const CONTROL_VERSION: u32 = 1;
pub const CONTROL_PROTOCOL: &str = "wccs-control-v1";
pub const MAX_MESSAGE: usize = 65_536;
pub const LEASE_SECONDS: u64 = 15;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WaitingIdentity {
    pub contest_id: String,
    pub contest_name: String,
    pub contest_version: i64,
    pub participant_id: String,
    pub name: String,
    pub school: String,
    pub seat_id: String,
    pub seat_label: String,
    pub room_id: String,
    pub room_version: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub protocol_version: u32,
    pub command_id: String,
    pub request_id: String,
    pub sequence: i64,
    pub device_id: String,
    pub generation: i64,
    pub device_version: i64,
    pub instance_id: String,
    pub session_key: String,
    pub action: String,
    pub expires_at: i64,
    pub identity: WaitingIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<crate::login::LoginPlan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Actual {
    pub state: String,
    pub command_id: Option<String>,
    pub session_key: Option<String>,
    pub monitors: u32,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<crate::login::AccountState>,
}
impl Default for Actual {
    fn default() -> Self {
        Self {
            state: "desktop".into(),
            command_id: None,
            session_key: None,
            monitors: 0,
            detail: "teaching_standby".into(),
            account: None,
        }
    }
}
impl Actual {
    pub fn valid(&self) -> bool {
        self.account.as_ref().is_none_or(|s| s.valid())
            && ["desktop", "waiting", "unknown", "failed", "no_session"]
                .contains(&self.state.as_str())
            && [
                "teaching_standby",
                "ui_observed",
                "control_disconnected",
                "session_changed",
                "session_launch_failed",
                "no_session",
                "no_available_session",
                "session_process_unavailable",
                "ipc_or_session_failed",
                "command_expired",
                "session_process_exited",
                "ipc_rejected",
                "standby",
                "ended",
            ]
            .contains(&self.detail.as_str())
            && self.monitors <= 64
            && self.command_id.as_ref().is_none_or(|s| crate::valid_id(s))
            && self.session_key.as_ref().is_none_or(|s| crate::valid_id(s))
            && (self.state != "waiting"
                || (self.monitors > 0 && self.command_id.is_some() && self.session_key.is_some()))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub command_id: String,
    pub sequence: i64,
    pub stage: String,
    pub actual: Actual,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPoll {
    pub protocol_version: u32,
    pub instance_id: String,
    pub session_key: Option<String>,
    pub capabilities: Vec<String>,
    pub actual: Actual,
    pub report: Option<Report>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReply {
    pub protocol_version: u32,
    pub connection_id: String,
    pub generation: i64,
    pub server_time: i64,
    pub lease_seconds: u64,
    pub command: Option<Command>,
    pub acknowledged: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IpcRequest {
    pub protocol_version: u32,
    pub command: Command,
    pub lease_seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_needs_display_and_session_evidence() {
        let mut actual = Actual {
            state: "waiting".into(),
            ..Default::default()
        };
        assert!(!actual.valid());
        actual.command_id = Some("command".into());
        actual.session_key = Some("session".into());
        actual.monitors = 1;
        assert!(actual.valid());
        actual.detail = "unexpected payload with credentials".into();
        assert!(!actual.valid());
    }
}
