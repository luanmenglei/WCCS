//! Public diagnostic evidence contains fixed outcomes, never credentials or raw errors.
use crate::valid_id;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SeatClaim {
    pub room_id: String,
    pub seat_number: i32,
    pub interface_index: u32,
    pub mac_address: String,
}

impl SeatClaim {
    pub fn valid(&self) -> bool {
        valid_id(&self.room_id)
            && self.seat_number > 0
            && self.interface_index > 0
            && (12..=32).contains(&self.mac_address.len())
            && self
                .mac_address
                .bytes()
                .all(|c| c.is_ascii_hexdigit() || c == b':' || c == b'-')
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Configuration,
    Installation,
    Server,
    Authentication,
    HttpHeartbeat,
    Websocket,
    Service,
    SessionIpc,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Pending,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub step: Step,
    pub outcome: Outcome,
    pub checked_at: i64,
    pub last_success_at: Option<i64>,
    pub code: Code,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    ReceiptVerified,
    SavedOffline,
    NotConfigured,
    InvalidConfiguration,
    ConnectionUnavailable,
    AuthenticationRejected,
    RegistrationPending,
    RegistrationRejected,
    IncompatibleReply,
    ServiceRunning,
    ServiceStopped,
    ServiceMissing,
    PermissionDenied,
    SessionUnavailable,
    IpcFailed,
    FileMissing,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub request_id: String,
    pub device_id: String,
    pub agent_version: String,
    pub evidence_context: EvidenceContext,
    pub configuration_revision: u64,
    pub configuration_digest: String,
    pub server_origin: Option<String>,
    pub collected_at: i64,
    pub seat_claim: Option<SeatClaim>,
    pub checks: Vec<Check>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceContext {
    Service,
    LocalTest,
}

impl Report {
    pub fn valid(&self) -> bool {
        self.schema_version == 1
            && valid_id(&self.request_id)
            && valid_id(&self.device_id)
            && self.agent_version.len() <= 80
            && self.configuration_digest.len() == 64
            && self.server_origin.as_ref().is_none_or(|origin| {
                origin.len() <= 2048
                    && (origin.starts_with("https://")
                        || origin.starts_with("http://127.0.0.1:")
                        || origin.starts_with("http://localhost:")
                        || origin.starts_with("http://[::1]:"))
                    && !origin
                        .chars()
                        .any(|c| c.is_control() || c.is_whitespace() || "@?#".contains(c))
            })
            && self
                .configuration_digest
                .bytes()
                .all(|c| c.is_ascii_hexdigit())
            && self.seat_claim.as_ref().is_none_or(SeatClaim::valid)
            && self.collected_at >= 0
            && self.checks.len() == 8
            && self.checks.iter().enumerate().all(|(i, check)| {
                check.checked_at >= 0
                    && check.last_success_at.is_none_or(|time| time >= 0)
                    && !self.checks[..i]
                        .iter()
                        .any(|other| other.step == check.step)
                    && (check.outcome != Outcome::Succeeded
                        || check.last_success_at == Some(check.checked_at))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_rejects_secrets_unknown_steps_and_duplicate_evidence() {
        let check = serde_json::json!({"step":"server","outcome":"failed","checked_at":1,"last_success_at":null,"code":"connection_unavailable"});
        let mut secret = check.clone();
        secret["error"] = "Bearer secret".into();
        assert!(serde_json::from_value::<Check>(secret).is_err());
        let report = Report {
            schema_version: 1,
            request_id: "diagnostic".into(),
            device_id: "device".into(),
            agent_version: "0.3.0".into(),
            evidence_context: EvidenceContext::LocalTest,
            configuration_revision: 1,
            configuration_digest: "a".repeat(64),
            server_origin: None,
            collected_at: 1,
            seat_claim: None,
            checks: vec![serde_json::from_value(check).unwrap(); 8],
        };
        assert!(!report.valid());
    }
}
