//! 维护日志区分待补报、已确认和需人工核对的记录。只有新任务可以进入执行。
use super::result;
use crate::{AgentResult as Result, connection, identity::Identity, local_state::persist};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{fs, path::Path};
use wccs_protocol::maintenance::{Command, Result as TaskResult, Template};
#[derive(Deserialize, Serialize)]
pub(super) struct Record {
    pub(super) server: String,
    pub(super) device_binding: String,
    pub(super) command: Command,
    pub(super) result: TaskResult,
    pub(super) acknowledged: bool,
}
pub(super) fn save(path: &Path, record: &Record) -> Result<()> {
    persist(path, &serde_json::to_vec(record)?)
}
fn history_path(path: &Path, group: &str) -> Result<std::path::PathBuf> {
    let directory = path.parent().ok_or("maintenance_path_invalid")?.join(group);
    fs::create_dir_all(&directory)?;
    Ok(directory.join(path.file_name().ok_or("maintenance_path_invalid")?))
}

pub(super) fn archive_ack(path: &Path, record: &mut Record) -> Result<()> {
    record.acknowledged = true;
    // Keep the digest of the complete command; large immutable payloads have
    // already been accepted by the server. Commit the marker before unlinking.
    record.command.recovery = None;
    record.command.preparation = None;
    record.result.recovery = None;
    save(&history_path(path, "confirmed")?, record)?;
    fs::remove_file(path)?;
    Ok(())
}

fn load_pending(path: &Path) -> Result<Option<Record>> {
    match serde_json::from_slice(&fs::read(path)?) {
        Ok(record) => Ok(Some(record)),
        Err(_) => {
            fs::rename(path, history_path(path, "quarantine")?)?;
            eprintln!("maintenance_journal_quarantined");
            Ok(None)
        }
    }
}

pub fn quarantined_count(path: &Path) -> usize {
    ["maintenance", "maintenance-v2"]
        .iter()
        .filter_map(|ext| fs::read_dir(path.with_extension(ext).join("quarantine")).ok())
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .count()
}

pub fn diagnostic_summary(path: &Path, identity: &Identity) -> Vec<serde_json::Value> {
    let entries = ["maintenance", "maintenance-v2"]
        .into_iter()
        .flat_map(|extension| {
            let base = path.with_extension(extension);
            [
                base.clone(),
                base.join("confirmed"),
                base.join("quarantine"),
            ]
        })
        .filter_map(|directory| fs::read_dir(directory).ok())
        .flatten();
    let binding = connection::binding(identity);
    let mut records: Vec<Record> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "json"
                || fs::metadata(&path).ok()?.len() > 2 * 1024 * 1024
            {
                return None;
            }
            let record: Record = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
            (record.device_binding == binding
                && record.result.valid()
                && wccs_protocol::valid_id(&record.command.request_id))
            .then_some(record)
        })
        .collect();
    records.sort_by_key(|record| std::cmp::Reverse(record.command.expires_at));
    records.into_iter().take(32).map(|record|json!({
        "job_id":record.result.job_id,"request_id":record.command.request_id,
        "stage":record.result.stage,"detail":record.result.detail,
        "server_acknowledged":record.acknowledged,
        "steps":record.result.steps.iter().map(|s|json!({"phase":s.phase,"outcome":s.outcome})).collect::<Vec<_>>()
    })).collect()
}
pub(super) fn recover(directory: &Path, identity_path: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(mut record) = load_pending(&path)? else {
            continue;
        };
        if record.acknowledged {
            archive_ack(&path, &mut record)?;
            continue;
        }
        if !record.result.terminal() {
            record.result = result(&record.command, "interrupted", "restart_interrupted");
            if record.command.preparation.is_some() {
                match crate::file_transaction::recover(&crate::preparation::journal_path(
                    identity_path,
                    &record.command.job_id,
                )) {
                    Ok(Some(journal)) if journal.phase == "committed" => {
                        record.result = result(&record.command, "succeeded", &journal.completion);
                        record.result.steps = journal
                            .files
                            .iter()
                            .map(|file| wccs_protocol::preparation::Step {
                                phase: "verification".into(),
                                path: file.path.to_string_lossy().into(),
                                outcome: "succeeded".into(),
                                detail: "recovered_committed_content".into(),
                            })
                            .collect();
                    }
                    Ok(Some(_)) => {
                        record.result = result(&record.command, "interrupted", "rollback_complete")
                    }
                    Ok(None) => {}
                    Err(_) => record.result = result(&record.command, "failed", "rollback_failed"),
                }
            }
            record.acknowledged = false;
            save(&path, &record)?;
        }
    }
    Ok(())
}

pub(super) fn resend_pending(
    identity_path: &Path,
    server: &str,
    binding: &str,
    send: impl Fn(&Record) -> Result<()>,
) -> Result<()> {
    let directories = [
        identity_path.with_extension("maintenance"),
        identity_path.with_extension("maintenance-v2"),
    ];
    for entry in directories
        .iter()
        .map(fs::read_dir)
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
    {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(mut record) = load_pending(&path)? else {
            continue;
        };
        if record.acknowledged {
            archive_ack(&path, &mut record)?;
            continue;
        }
        if !record.result.terminal() {
            record.result = result(&record.command, "failed", "execution_failed");
            record.acknowledged = false;
            save(&path, &record)?;
        }
        if record.server == server && record.device_binding == binding {
            match send(&record) {
                Ok(()) => archive_ack(&path, &mut record)?,
                Err(error)
                    if error
                        .downcast_ref::<reqwest::Error>()
                        .and_then(|e| e.status())
                        .is_some_and(|s| matches!(s.as_u16(), 400 | 403 | 404 | 409 | 422)) =>
                {
                    fs::rename(&path, history_path(&path, "quarantine")?)?;
                    eprintln!("maintenance_receipt_requires_review");
                }
                Err(_) => eprintln!("maintenance_receipt_retry_pending"),
            }
        }
    }
    Ok(())
}
pub(super) fn previous_receipt(
    record_path: &Path,
    command: &Command,
    server: &str,
    binding: &str,
) -> Result<Option<Record>> {
    if record_path.exists() {
        let record: Record = serde_json::from_slice(&fs::read(record_path)?)?;
        if &record.command != command {
            return Err("maintenance_identifier_conflict".into());
        }
        // Durable tombstones remain after ACK, so redelivery never repeats a side effect.
        return Ok(Some(record));
    }
    let confirmed = history_path(record_path, "confirmed")?;
    let quarantined = history_path(record_path, "quarantine")?.try_exists()?;
    if confirmed.try_exists()? || quarantined {
        let mut receipt = result(command, "interrupted", "restart_interrupted");
        if !quarantined {
            let saved: Record = serde_json::from_slice(&fs::read(confirmed)?)?;
            if saved.server != server
                || saved.device_binding != binding
                || saved.result.command_digest != receipt.command_digest
            {
                return Err("maintenance_identifier_conflict".into());
            }
            if !matches!(command.template, Template::Recovery { .. })
                || saved.result.stage != "succeeded"
            {
                receipt = saved.result;
            }
        }
        // A restored server may request work already performed. For a damaged
        // journal or compacted manifest only interruption can be attested.
        return Ok(Some(Record {
            server: server.into(),
            device_binding: binding.into(),
            command: command.clone(),
            result: receipt,
            acknowledged: false,
        }));
    }
    Ok(None)
}
#[cfg(test)]
mod tests {
    use super::*;
    use wccs_protocol::token;
    #[test]
    fn restart_converts_running_journal_without_replaying_side_effects() {
        let directory = std::env::temp_dir().join(format!("wccs-maintenance-{}", token()));
        fs::create_dir(&directory).unwrap();
        let command:Command=serde_json::from_value(json!({"schema_version":1,"job_id":"job","request_id":"request","device_id":"device","generation":1,"device_version":1,"address_version":1,"session_key":"session","instance_id":"instance","expires_at":10,"template":{"operation":"identify","duration_seconds":5},
            "identity":{"contest_id":"identification","contest_name":"test","contest_version":0,"participant_id":"","name":"device","school":"127.0.0.1","seat_id":"","seat_label":"1","room_id":"","room_version":0}})).unwrap();
        let path = directory.join("job.json");
        let record = Record {
            server: "https://example.test".into(),
            device_binding: "binding".into(),
            result: result(&command, "executing", "display_observed"),
            command,
            acknowledged: false,
        };
        save(&path, &record).unwrap();
        recover(&directory, &directory.join("device.json")).unwrap();
        let first = fs::read(&path).unwrap();
        let recovered: Record = serde_json::from_slice(&first).unwrap();
        assert_eq!(recovered.result.stage, "interrupted");
        assert!(!recovered.acknowledged);
        recover(&directory, &directory.join("device.json")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), first);
        let confirmed = directory.join("confirmed");
        fs::create_dir(&confirmed).unwrap();
        for id in 0..4097 {
            fs::write(confirmed.join(format!("done-{id}.json")), b"historical").unwrap();
        }
        fs::write(directory.join("corrupt.json"), b"{").unwrap();
        let mut recovered: Record = serde_json::from_slice(&first).unwrap();
        recovered.acknowledged = true;
        save(&path, &recovered).unwrap();
        recover(&directory, &directory.join("device.json")).unwrap();
        assert!(directory.join("quarantine/corrupt.json").exists());
        assert!(confirmed.join("job.json").exists());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&confirmed).unwrap().count(), 4098);

        // 已确认任务再次下发时只返回旧回执。复用编号的其他命令必须拒绝。
        let receipt = previous_receipt(&path, &recovered.command, &recovered.server, "binding")
            .unwrap()
            .unwrap();
        assert_eq!(receipt.result, recovered.result);
        let mut conflicting = recovered.command.clone();
        conflicting.request_id = "another-request".into();
        assert!(previous_receipt(&path, &conflicting, &recovered.server, "binding").is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
