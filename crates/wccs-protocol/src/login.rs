//! Public account observations contain no credentials. Native logon remains capability gated.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccountState {
    pub adapter: String,
    pub revision: u64,
    pub phase: String,
    pub account: String,
    pub target_sid: Option<String>,
    pub session: String,
    pub observed_sid: Option<String>,
    pub desktop_confirmed: bool,
    pub recovery_ready: bool,
    pub tools_ready: bool,
    pub teaching_enabled: bool,
    pub detail: String,
}

impl AccountState {
    pub fn unavailable() -> Self {
        Self {
            adapter: "unavailable".into(),
            revision: 0,
            phase: "teaching".into(),
            account: "unavailable".into(),
            target_sid: None,
            session: "unknown".into(),
            observed_sid: None,
            desktop_confirmed: false,
            recovery_ready: false,
            tools_ready: false,
            teaching_enabled: true,
            detail: "native_adapter_unavailable".into(),
        }
    }
    pub fn valid(&self) -> bool {
        ["unavailable", "simulation", "windows_test"].contains(&self.adapter.as_str())
            && [
                "teaching",
                "preparing",
                "login_pending",
                "controlled",
                "recovering",
                "recovery_failed",
            ]
            .contains(&self.phase.as_str())
            && ["unavailable", "absent", "owned", "conflict"].contains(&self.account.as_str())
            && [
                "none", "contest", "teaching", "locked", "multiple", "rdp", "unknown",
            ]
            .contains(&self.session.as_str())
            && [
                "native_adapter_unavailable",
                "standby",
                "prepared",
                "login_submitted",
                "desktop_observed",
                "restored",
                "recovery_conflict",
                "recovery_failed",
                "record_mismatch",
                "login_failed",
                "precondition_failed",
            ]
            .contains(&self.detail.as_str())
            && self.target_sid.as_ref().is_none_or(|s| crate::valid_id(s))
            && self
                .observed_sid
                .as_ref()
                .is_none_or(|s| crate::valid_id(s))
    }
    pub fn skip_reason(&self, action: &str, confirm_session_end: bool) -> Option<&'static str> {
        if !["simulation", "windows_test"].contains(&self.adapter.as_str()) {
            return Some("login_unavailable");
        }
        if action == "restore_teaching" {
            return None;
        }
        if self.phase != "teaching" {
            return Some("recovery_required");
        }
        if self.account == "conflict" {
            return Some("account_conflict");
        }
        if !self.recovery_ready || !self.teaching_enabled {
            return Some("recovery_unavailable");
        }
        // Native preparation checks the newly previewed paths under the ordinary token
        // before ending a session or disabling the teaching account. Allow a repaired
        // tool configuration to be checked even when the previous observation failed.
        if !self.tools_ready && self.adapter != "windows_test" {
            return Some("tools_unavailable");
        }
        match self.session.as_str() {
            "none" | "contest" => None,
            "teaching" if confirm_session_end => None,
            "teaching" => Some("session_confirmation_required"),
            _ => Some("unsupported_session"),
        }
    }
    pub fn login_complete(&self) -> bool {
        self.valid()
            && ["simulation", "windows_test"].contains(&self.adapter.as_str())
            && self.phase == "controlled"
            && self.account == "owned"
            && self.session == "contest"
            && self.desktop_confirmed
            && self.target_sid.is_some()
            && self.target_sid == self.observed_sid
            && !self.teaching_enabled
            && self.recovery_ready
            && self.tools_ready
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LoginPlan {
    #[serde(default)]
    pub tool_version: i64,
    #[serde(default)]
    pub tool_paths: Vec<String>,
    pub connection_id: String,
    pub snapshot: AccountState,
    pub confirm_session_end: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_preparation_can_recheck_repaired_tool_paths() {
        let mut state = AccountState::unavailable();
        state.adapter = "windows_test".into();
        state.account = "owned".into();
        state.session = "none".into();
        state.recovery_ready = true;
        assert_eq!(state.skip_reason("prepare_login", false), None);
        state.adapter = "simulation".into();
        assert_eq!(
            state.skip_reason("prepare_login", false),
            Some("tools_unavailable")
        );
    }
    #[test]
    fn unavailable_adapter_cannot_claim_login_or_accept_sensitive_fields() {
        let state = AccountState::unavailable();
        assert!(state.valid());
        assert!(!state.login_complete());
        assert_eq!(
            state.skip_reason("prepare_login", true),
            Some("login_unavailable")
        );
        let mut value = serde_json::to_value(state).unwrap();
        value["password"] = "secret".into();
        assert!(serde_json::from_value::<AccountState>(value).is_err());
    }
}
