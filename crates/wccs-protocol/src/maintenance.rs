use crate::{control::WaitingIdentity, valid_id};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Template {
    Shutdown {},
    Recovery { options: crate::recovery::Options },
    Identify { duration_seconds: u64 },
    PrepareDesktop { profile_version: i64 },
    CheckWorkspace { profile_version: i64 },
    DeployScripts { profile_version: i64 },
}
impl Template {
    pub fn capability(&self) -> &str {
        if matches!(self, Self::Shutdown {}) {
            "shutdown-v1"
        } else if matches!(self, Self::Recovery { options } if options.limits.is_some()) {
            "recovery-v2"
        } else if matches!(self, Self::Recovery { .. }) {
            "recovery-v1"
        } else if matches!(self, Self::Identify { .. }) {
            "identify-v1"
        } else {
            "preparation-v1"
        }
    }
    pub fn profile_version(&self) -> Option<i64> {
        match self {
            Self::Identify { .. } | Self::Recovery { .. } | Self::Shutdown {} => None,
            Self::PrepareDesktop { profile_version }
            | Self::CheckWorkspace { profile_version }
            | Self::DeployScripts { profile_version } => Some(*profile_version),
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Shutdown {} => true,
            Self::Recovery { options } => options.valid(),
            Self::Identify { duration_seconds } => (5..=120).contains(duration_seconds),
            Self::PrepareDesktop { profile_version }
            | Self::CheckWorkspace { profile_version }
            | Self::DeployScripts { profile_version } => *profile_version > 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<crate::recovery::Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<crate::preparation::Snapshot>,
    pub schema_version: u32,
    pub job_id: String,
    pub request_id: String,
    pub device_id: String,
    pub generation: i64,
    pub device_version: i64,
    pub address_version: i64,
    pub session_key: String,
    pub instance_id: String,
    pub identity: WaitingIdentity,
    pub expires_at: i64,
    pub template: Template,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Result {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<crate::recovery::Manifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<crate::preparation::Step>,
    pub schema_version: u32,
    pub job_id: String,
    pub command_digest: String,
    pub stage: String,
    pub detail: String,
}
impl Result {
    pub fn valid(&self) -> bool {
        self.recovery.as_ref().is_none_or(|m| m.valid())
            && self.steps.len() <= 128
            && self.steps.iter().all(|step| {
                step.path.len() <= 1024
                    && step.detail.len() <= 200
                    && [
                        "preflight",
                        "staging",
                        "applying",
                        "verification",
                        "rollback",
                        "activation",
                    ]
                    .contains(&step.phase.as_str())
                    && ["succeeded", "failed", "skipped"].contains(&step.outcome.as_str())
            })
            && self.schema_version == 1
            && valid_id(&self.job_id)
            && self.command_digest.len() == 64
            && self.command_digest.bytes().all(|c| c.is_ascii_hexdigit())
            && [
                "accepted",
                "executing",
                "succeeded",
                "failed",
                "cancelled",
                "interrupted",
            ]
            .contains(&self.stage.as_str())
            && [
                "recovery_inspected",
                "archive_verified",
                "archive_skipped",
                "cleanup_complete",
                "reboot_requested",
                "shutdown_requested",
                "shutdown_started",
                "recovery_started",
                "persisted",
                "display_observed",
                "display_closed",
                "timed_out",
                "cancelled_locally",
                "identity_changed",
                "connection_lost",
                "session_unavailable",
                "execution_failed",
                "restart_interrupted",
                "desktop_updated",
                "workspace_verified",
                "scripts_activated",
                "rollback_complete",
                "rollback_failed",
                "preparation_started",
            ]
            .contains(&self.detail.as_str())
    }
    pub fn terminal(&self) -> bool {
        !["accepted", "executing"].contains(&self.stage.as_str())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Reply {
    pub schema_version: u32,
    pub server_time: i64,
    pub command: Option<Command>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    pub schema_version: u32,
    pub instance_id: String,
    pub session_key: Option<String>,
    pub adapter: String,
    pub capabilities: Vec<String>,
}
