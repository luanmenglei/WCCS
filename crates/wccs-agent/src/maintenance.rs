//! Fixed maintenance templates execute outside the WSS renewal thread.
use crate::{
    connection, diagnostics,
    windows_agent::{Identity, Result, persist},
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wccs_protocol::{
    control, digest,
    maintenance::{Command, Poll, Reply, Result as TaskResult, Template},
    now, token,
};
use wccs_session::ipc;

#[derive(Deserialize, Serialize)]
struct Record {
    server: String,
    device_binding: String,
    command: Command,
    result: TaskResult,
    acknowledged: bool,
}
fn save(path: &Path, record: &Record) -> Result<()> {
    persist(path, &serde_json::to_vec(record)?)
}
fn history_path(path: &Path, group: &str) -> Result<std::path::PathBuf> {
    let directory = path.parent().ok_or("maintenance_path_invalid")?.join(group);
    fs::create_dir_all(&directory)?;
    Ok(directory.join(path.file_name().ok_or("maintenance_path_invalid")?))
}

fn archive_ack(path: &Path, record: &mut Record) -> Result<()> {
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
fn result(command: &Command, stage: &str, detail: &str) -> TaskResult {
    TaskResult {
        recovery: None,
        steps: Vec::new(),
        schema_version: 1,
        job_id: command.job_id.clone(),
        command_digest: digest(&json!(command).to_string()),
        stage: stage.into(),
        detail: detail.into(),
    }
}
fn post_result(
    client: &Client,
    identity: &Identity,
    credential: &str,
    record: &Record,
) -> Result<()> {
    let receipt: serde_json::Value = client
        .post(format!("{}/api/device/maintenance/result", record.server))
        .bearer_auth(credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&record.result)
        .send()?
        .error_for_status()?
        .json()?;
    if receipt["accepted"] != true
        || receipt["job_id"] != record.command.job_id
        || receipt["stage"] != record.result.stage
    {
        return Err("maintenance_ack_mismatch".into());
    }
    Ok(())
}
fn poll(
    client: &Client,
    server: &str,
    identity: &Identity,
    credential: &str,
    instance: &str,
    test_mode: bool,
    simulation: bool,
) -> Result<Reply> {
    let session = if simulation {
        None
    } else {
        ipc::discover(test_mode).ok()
    };
    let mut input = Poll {
        schema_version: 1,
        instance_id: instance.into(),
        session_key: if simulation {
            Some("recovery-simulation".into())
        } else {
            session.map(|s| s.key)
        },
        adapter: if test_mode { "local_test" } else { "service" }.into(),
        capabilities: if simulation {
            vec!["recovery-v1".into(), "recovery-v2".into()]
        } else if test_mode {
            vec!["identify-v1".into()]
        } else {
            vec![
                "identify-v1".into(),
                "preparation-v1".into(),
                "competition-v1".into(),
                "recovery-v1".into(),
                "recovery-v2".into(),
            ]
        },
    };
    let send = |input: &Poll| {
        client
            .post(format!("{server}/api/device/maintenance/poll"))
            .header("x-wccs-volume-policy", "1")
            .header(
                "x-wccs-shutdown",
                if !test_mode || simulation { "1" } else { "0" },
            )
            .header("x-wccs-desktop-files", if !test_mode { "1" } else { "0" })
            .bearer_auth(credential)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .json(input)
            .send()
    };
    let mut response = send(&input)?;
    // Servers predating v2 reject additional capabilities. Keep their v1 operations usable.
    if response.status() == reqwest::StatusCode::BAD_REQUEST {
        input
            .capabilities
            .retain(|capability| capability != "recovery-v2");
        response = send(&input)?;
    }
    let reply: Reply = response.error_for_status()?.json()?;
    if reply.schema_version != 1 {
        return Err("maintenance_protocol_mismatch".into());
    }
    Ok(reply)
}

fn recover(directory: &Path, identity_path: &Path) -> Result<()> {
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

pub fn simulate_recovery_once(path: &Path, instance: &str) -> Result<()> {
    if !wccs_protocol::valid_id(instance) {
        return Err("simulation_instance_invalid".into());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    recover(&path.with_extension("maintenance"), path)?;
    recover(&path.with_extension("maintenance-v2"), path)?;
    cycle(
        &client,
        path,
        Path::new("unused-simulation-session.exe"),
        instance,
        true,
        true,
        &AtomicBool::new(false),
    )
}

pub fn worker(
    path: &Path,
    executable: &Path,
    instance: &str,
    test_mode: bool,
    stop: Arc<AtomicBool>,
) {
    let directory = path.with_extension("maintenance");
    if recover(&directory, path)
        .and_then(|()| recover(&path.with_extension("maintenance-v2"), path))
        .is_err()
    {
        eprintln!("maintenance_recovery_failed");
        return;
    }
    let Ok(client) = Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    while !stop.load(Ordering::Relaxed) {
        if cycle(&client, path, executable, instance, test_mode, false, &stop).is_err() {
            eprintln!("maintenance_sync_failed");
        }
        for _ in 0..10 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn cycle(
    client: &Client,
    path: &Path,
    executable: &Path,
    instance: &str,
    test_mode: bool,
    simulation: bool,
    stop: &AtomicBool,
) -> Result<()> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let server = connection::server(path, &identity)?;
    let credential = diagnostics::credential(path, &identity)?;
    let binding = connection::binding(&identity);
    let directory = path.with_extension("maintenance");
    let directories = [directory.clone(), path.with_extension("maintenance-v2")];
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
        if !record.acknowledged && record.server == server && record.device_binding == binding {
            match post_result(client, &identity, &credential, &record) {
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
    crate::file_transaction::recover_pending(&path.with_extension("preparation"))
        .map_err(|_| "maintenance_rollback_pending")?;
    let reply = poll(
        client,
        &server,
        &identity,
        &credential,
        instance,
        test_mode,
        simulation,
    )?;
    let Some(command) = reply.command else {
        return Ok(());
    };
    if command.schema_version != 1
        || command.device_id != identity.device_id
        || command.instance_id != instance
        || !wccs_protocol::valid_id(&command.job_id)
        || !command.template.valid()
        || command.expires_at <= reply.server_time
    {
        return Err("maintenance_command_rejected".into());
    }
    // Historical binaries only read .maintenance. Extended records survive downgrade untouched.
    let directory = if matches!(command.template.capability(), "recovery-v2" | "shutdown-v1") {
        path.with_extension("maintenance-v2")
    } else {
        directory
    };
    let record_path = directory.join(format!("{}.json", command.job_id));
    if record_path.exists() {
        let record: Record = serde_json::from_slice(&fs::read(record_path)?)?;
        if record.command != command {
            return Err("maintenance_identifier_conflict".into());
        }
        // Durable tombstones remain after ACK, so redelivery never repeats a side effect.
        post_result(client, &identity, &credential, &record)?;
        return Ok(());
    }
    let confirmed = history_path(&record_path, "confirmed")?;
    let quarantined = history_path(&record_path, "quarantine")?.try_exists()?;
    if confirmed.try_exists()? || quarantined {
        let mut receipt = result(&command, "interrupted", "restart_interrupted");
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
        return post_result(
            client,
            &identity,
            &credential,
            &Record {
                server,
                device_binding: binding,
                command,
                result: receipt,
                acknowledged: false,
            },
        );
    }
    let mut record = Record {
        server: server.clone(),
        device_binding: binding,
        result: result(&command, "accepted", "persisted"),
        command,
        acknowledged: false,
    };
    save(&record_path, &record)?;
    post_result(client, &identity, &credential, &record)?;
    let command = record.command.clone();
    if matches!(command.template, Template::Shutdown {}) {
        record.result = result(&command, "executing", "shutdown_started");
        save(&record_path, &record)?;
        post_result(client, &identity, &credential, &record)?;
        let authorized = !stop.load(Ordering::Relaxed)
            && connection::server(path, &identity).ok().as_deref() == Some(&server)
            && poll(
                client,
                &server,
                &identity,
                &credential,
                instance,
                test_mode,
                simulation,
            )?
            .command
            .as_ref()
                == Some(&command);
        record.result = if !authorized {
            result(&command, "cancelled", "connection_lost")
        } else if simulation || (!test_mode && crate::power::shutdown_machine().is_ok()) {
            result(&command, "succeeded", "shutdown_requested")
        } else {
            result(&command, "failed", "execution_failed")
        };
    } else if let Template::Recovery { options } = &command.template {
        record.result = result(&command, "executing", "recovery_started");
        save(&record_path, &record)?;
        post_result(client, &identity, &credential, &record)?;
        let started = Instant::now();
        let deadline = options
            .limits
            .as_ref()
            .map_or(900, |limits| limits.timeout_seconds);
        let command_digest = record.result.command_digest.clone();
        let mut lease = None;
        let execution = crate::recovery::execute(
            path,
            &command,
            crate::recovery::Transfer {
                client,
                server: &server,
                credential: &credential,
            },
            simulation,
            || {
                if stop.load(Ordering::Relaxed)
                    || started.elapsed() >= Duration::from_secs(deadline)
                {
                    return false;
                }
                if lease.is_some_and(|last: Instant| last.elapsed() < Duration::from_secs(1)) {
                    return true;
                }
                if connection::server(path, &identity).ok().as_deref() != Some(&server) {
                    return false;
                }
                let valid = if options.limits.is_some() {
                    let input = Poll {
                        schema_version: 1,
                        instance_id: instance.into(),
                        session_key: Some(command.session_key.clone()),
                        adapter: if test_mode { "local_test" } else { "service" }.into(),
                        capabilities: vec!["recovery-v1".into(), "recovery-v2".into()],
                    };
                    client
                        .post(format!("{server}/api/device/maintenance/authorize"))
                        .header("x-wccs-volume-policy", "1")
                        .bearer_auth(&credential)
                        .header("X-WCCS-Device-ID", &identity.device_id)
                        .json(&input)
                        .send()
                        .ok()
                        .and_then(|response| response.error_for_status().ok())
                        .and_then(|response| response.json::<serde_json::Value>().ok())
                        .is_some_and(|reply| {
                            reply["schema_version"] == 1
                                && reply["command_digest"] == command_digest
                        })
                } else {
                    poll(
                        client,
                        &server,
                        &identity,
                        &credential,
                        instance,
                        test_mode,
                        simulation,
                    )
                    .ok()
                    .and_then(|reply| reply.command)
                    .as_ref()
                        == Some(&command)
                };
                if valid {
                    lease = Some(Instant::now());
                }
                valid
            },
        );
        record.result = match execution {
            Ok((detail, manifest)) => {
                let mut value = result(&command, "succeeded", detail);
                value.recovery = Some(manifest);
                value
            }
            Err(error) => {
                let mut value = result(&command, "failed", "execution_failed");
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
        };
    } else if matches!(command.template, Template::Identify { .. }) {
        let outcome = display(
            executable,
            test_mode,
            &command,
            stop,
            || {
                connection::server(path, &identity).ok().as_deref() == Some(&server)
                    && poll(
                        client,
                        &server,
                        &identity,
                        &credential,
                        instance,
                        test_mode,
                        simulation,
                    )
                    .ok()
                    .and_then(|r| r.command)
                    .as_ref()
                        == Some(&command)
            },
            || {
                record.result = result(&command, "executing", "display_observed");
                save(&record_path, &record)?;
                post_result(client, &identity, &credential, &record)
            },
        );
        record.result = match outcome {
            Ok((stage, detail)) => result(&command, stage, detail),
            Err(_) => result(&command, "failed", "execution_failed"),
        };
    } else {
        record.result = result(&command, "executing", "preparation_started");
        save(&record_path, &record)?;
        post_result(client, &identity, &credential, &record)?;
        let execution = if test_mode {
            Err("preparation_requires_system_service".into())
        } else {
            crate::preparation::execute(path, &command, || {
                !stop.load(Ordering::Relaxed)
                    && connection::server(path, &identity).ok().as_deref() == Some(&server)
                    && poll(
                        client,
                        &server,
                        &identity,
                        &credential,
                        instance,
                        test_mode,
                        simulation,
                    )
                    .ok()
                    .and_then(|r| r.command)
                    .as_ref()
                        == Some(&command)
            })
        };
        record.result = match execution {
            Ok(steps) => {
                let detail = match command.template {
                    Template::PrepareDesktop { .. } => "desktop_updated",
                    Template::CheckWorkspace { .. } => "workspace_verified",
                    _ => "scripts_activated",
                };
                let mut completed = result(&command, "succeeded", detail);
                completed.steps = steps;
                completed
            }
            Err(error) => {
                let message = error.to_string();
                let detail = match error.to_string().as_str() {
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
                let mut failed = result(&command, stage, detail);
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
        };
    }
    save(&record_path, &record)?;
    post_result(client, &identity, &credential, &record)?;
    archive_ack(&record_path, &mut record)
}

fn display(
    executable: &Path,
    test_mode: bool,
    command: &Command,
    stop: &AtomicBool,
    mut authorized: impl FnMut() -> bool,
    mut observed: impl FnMut() -> Result<()>,
) -> Result<(&'static str, &'static str)> {
    let session = ipc::discover(test_mode).map_err(|_| "maintenance_session_unavailable")?;
    if session.key != command.session_key {
        return Ok(("cancelled", "identity_changed"));
    }
    let mut child = ipc::Child::start(executable, &session, test_mode)
        .map_err(|_| "maintenance_child_failed")?;
    let start = Instant::now();
    while !child.ready().map_err(|_| "maintenance_ipc_failed")? {
        if start.elapsed() > Duration::from_secs(5) || stop.load(Ordering::Relaxed) {
            return Ok(("failed", "session_unavailable"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut request = control::IpcRequest {
        presentation: None,
        protocol_version: 1,
        lease_seconds: 5,
        command: control::Command {
            protocol_version: 1,
            command_id: command.job_id.clone(),
            request_id: command.request_id.clone(),
            sequence: 0,
            device_id: command.device_id.clone(),
            generation: command.generation,
            device_version: command.device_version,
            instance_id: command.instance_id.clone(),
            session_key: command.session_key.clone(),
            action: "identify".into(),
            expires_at: command.expires_at,
            identity: command.identity.clone(),
            desktop_files: None,
            login: None,
        },
    };
    let Template::Identify { duration_seconds } = command.template else {
        return Err("identification_template_required".into());
    };
    let deadline = Instant::now() + Duration::from_secs(duration_seconds);
    let mut seen = false;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) || !authorized() {
            return Ok(("cancelled", "connection_lost"));
        }
        if ipc::discover(test_mode)
            .ok()
            .is_none_or(|s| s.key != session.key)
        {
            return Ok(("cancelled", "identity_changed"));
        }
        if child.pipe.send(&request).is_err() {
            return Ok(("cancelled", "cancelled_locally"));
        }
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            match child.pipe.receive::<control::Actual>() {
                Ok(Some(actual))
                    if actual.command_id.as_deref() == Some(&command.job_id)
                        && actual.session_key.as_deref() == Some(&session.key)
                        && actual.state == "waiting"
                        && actual.detail == "ui_observed"
                        && actual.monitors > 0 =>
                {
                    if !seen {
                        observed()?;
                        seen = true;
                    }
                }
                Err(_) => return Ok(("cancelled", "cancelled_locally")),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    if !seen {
        return Ok(("failed", "session_unavailable"));
    }
    request.command.action = "leave".into();
    child
        .pipe
        .send(&request)
        .map_err(|_| "maintenance_close_failed")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(Some(actual)) = child.pipe.receive::<control::Actual>()
            && actual.state == "desktop"
        {
            child.stop().map_err(|_| "maintenance_close_unconfirmed")?;
            return Ok(("succeeded", "display_closed"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err("maintenance_close_unconfirmed".into())
}

pub fn identify_local(path: &Path, executable: &Path, seconds: u64) -> Result<()> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let config = connection::load(path, &identity)?;
    let fingerprint = connection::configuration_digest(&config);
    let template = Template::Identify {
        duration_seconds: seconds,
    };
    if !template.valid() {
        return Err("识别显示时长为 5～120 秒".into());
    }
    let session = ipc::discover(true).map_err(|_| "session_unavailable")?;
    let network = wccs_inventory::network();
    let selected = config.configuration.seat_claim.as_ref().and_then(|c| {
        network.value()?.iter().find(|n| {
            n.interface_index == c.interface_index
                && n.mac_address.eq_ignore_ascii_case(&c.mac_address)
        })
    });
    let addresses = selected
        .map(|n| {
            n.addresses
                .iter()
                .filter_map(|a| a.address.value().cloned())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_else(|| "请在配置中选择目标网卡".into());
    let label = config
        .configuration
        .seat_claim
        .as_ref()
        .map(|c| format!("{} / {} 待核对", c.room_id, c.seat_number))
        .unwrap_or_else(|| "未配置座位".into());
    let command = Command {
        recovery: None,
        preparation: None,
        schema_version: 1,
        job_id: token(),
        request_id: token(),
        device_id: identity.device_id.clone(),
        generation: 0,
        device_version: 0,
        address_version: 0,
        session_key: session.key,
        instance_id: token(),
        expires_at: now() + seconds as i64,
        template,
        identity: control::WaitingIdentity {
            contest_id: "identification".into(),
            contest_name: "工作人员本机识别".into(),
            contest_version: 0,
            participant_id: String::new(),
            name: identity.device_id.clone(),
            school: addresses,
            seat_id: String::new(),
            seat_label: label,
            room_id: String::new(),
            room_version: 0,
        },
    };
    let outcome = display(
        executable,
        true,
        &command,
        &AtomicBool::new(false),
        || {
            connection::load(path, &identity)
                .ok()
                .is_some_and(|saved| connection::configuration_digest(&saved) == fingerprint)
        },
        || Ok(()),
    )?;
    println!("{} {}", outcome.0, outcome.1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_server_capability_rejection_falls_back_to_legacy_poll() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() < 8192);
                }
                let header = String::from_utf8(header).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut bytes = vec![0; length];
                stream.read_exact(&mut bytes).unwrap();
                let input: Poll = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(
                    input.capabilities.iter().any(|cap| cap == "recovery-v2"),
                    attempt == 0
                );
                assert!(input.capabilities.iter().any(|cap| cap == "recovery-v1"));
                let (status, body) = if attempt == 0 {
                    ("400 Bad Request", "{}")
                } else {
                    (
                        "200 OK",
                        "{\"schema_version\":1,\"server_time\":1,\"command\":null}",
                    )
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let identity: Identity = serde_json::from_value(
            json!({"server":server,"device_id":"synthetic","enrollment_token":"synthetic"}),
        )
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        assert!(
            poll(
                &client,
                &server,
                &identity,
                "synthetic",
                "instance",
                true,
                true
            )
            .unwrap()
            .command
            .is_none()
        );
        worker.join().unwrap();
    }
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
        fs::remove_dir_all(directory).unwrap();
    }
}
