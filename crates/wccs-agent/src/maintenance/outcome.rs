//! 维护回执只向服务端暴露约定的阶段和错误码，系统错误可能含有本机路径或账户信息。
use super::result;
use crate::host::Result;
use wccs_protocol::{
    maintenance::{Command, Result as TaskResult, Template},
    preparation::Step,
    recovery::Manifest,
};

pub(super) fn recovery(command: &Command, execution: Result<(&str, Manifest)>) -> TaskResult {
    match execution {
        Ok((detail, manifest)) => {
            let mut value = result(command, "succeeded", detail);
            value.recovery = Some(manifest);
            value
        }
        Err(error) => {
            let mut value = result(command, "failed", "execution_failed");
            let message = error.to_string();
            let safe = [
                "recovery_simulation_target_mismatch",
                "recovery_requires_system",
                "recovery_workspace_mismatch",
                "recovery_system_volume_rejected",
                "recovery_protected_path",
                "recovery_volume_unknown",
                "recovery_volume_not_provisioned",
                "recovery_volume_policy_invalid",
                "recovery_volume_identity_mismatch",
                "recovery_filename_invalid",
                "recovery_capacity_exceeded",
                "target_reparse_point",
                "recovery_authorization_ended",
                "recovery_target_changed",
                "recovery_archive_ack_mismatch",
                "restore_teaching_required",
            ];
            value.steps.push(wccs_protocol::preparation::Step {
                phase: "verification".into(),
                path: String::new(),
                outcome: "failed".into(),
                detail: if safe.contains(&message.as_str()) {
                    message
                } else {
                    "recovery_io_or_transfer_failed".into()
                },
            });
            value
        }
    }
}
pub(super) fn preparation(command: &Command, execution: Result<Vec<Step>>) -> TaskResult {
    match execution {
        Ok(steps) => {
            let detail = match command.template {
                Template::PrepareDesktop { .. } => "desktop_updated",
                Template::CheckWorkspace { .. } => "workspace_verified",
                _ => "scripts_activated",
            };
            let mut completed = result(command, "succeeded", detail);
            completed.steps = steps;
            completed
        }
        Err(error) => {
            let message = error.to_string();
            let detail = match message.as_str() {
                "rollback_complete" => "rollback_complete",
                "rollback_failed" => "rollback_failed",
                "maintenance_identity_changed" => "identity_changed",
                "preparation_authorization_ended" => "connection_lost",
                _ => "execution_failed",
            };
            let stage = if ["identity_changed", "connection_lost"].contains(&detail) {
                "cancelled"
            } else {
                "failed"
            };
            let mut failed = result(command, stage, detail);
            let safe = [
                "maintenance_requires_system",
                "preparation_requires_system_service",
                "preparation_version_mismatch",
                "work_directory_mismatch",
                "desktop_account_path_mismatch",
                "script_runtime_version_mismatch",
                "immutable_package_version_conflict",
                "target_ownership_conflict",
                "target_not_empty",
                "target_reparse_point",
                "script_directory_writable_by_contest_account",
                "ordinary_account_required",
                "contest_account_missing",
                "contest_account_ownership",
                "maintenance_identity_changed",
                "preparation_authorization_ended",
                "rollback_complete",
                "rollback_failed",
            ];
            failed.steps.push(wccs_protocol::preparation::Step {
                phase: if detail.starts_with("rollback_") {
                    "rollback"
                } else {
                    "preflight"
                }
                .into(),
                path: String::new(),
                outcome: "failed".into(),
                detail: if safe.contains(&message.as_str()) {
                    message
                } else {
                    "system_operation_failed".into()
                },
            });
            failed
        }
    }
}
