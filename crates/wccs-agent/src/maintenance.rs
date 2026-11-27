//! 维护线程先补交旧回执，再领取一项新任务。控制连接的续租由独立线程负责。
mod channel;
mod identification;
mod journal;
mod outcome;

use crate::{AgentResult as Result, connection};
use channel::Channel;
pub use identification::identify_local;
use journal::{Record, recover};
pub use journal::{diagnostic_summary, quarantined_count};
use reqwest::blocking::Client;
use serde_json::json;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wccs_protocol::{
    digest,
    maintenance::{Command, Result as TaskResult, Template},
};

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
    let channel = Channel::open(client, path, instance, test_mode, simulation)?;
    let binding = connection::binding(&channel.identity);
    journal::resend_pending(path, &channel.server, &binding, |record| {
        channel.post_result(record)
    })?;
    crate::file_transaction::recover_pending(&path.with_extension("preparation"))
        .map_err(|_| "maintenance_rollback_pending")?;
    let reply = channel.poll()?;
    let Some(command) = reply.command else {
        return Ok(());
    };
    if command.schema_version != 1
        || command.device_id != channel.identity.device_id
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
        path.with_extension("maintenance")
    };
    let record_path = directory.join(format!("{}.json", command.job_id));
    if let Some(receipt) =
        journal::previous_receipt(&record_path, &command, &channel.server, &binding)?
    {
        return channel.post_result(&receipt);
    }
    let mut record = Record {
        server: channel.server.clone(),
        device_binding: binding,
        result: result(&command, "accepted", "persisted"),
        command,
        acknowledged: false,
    };
    persist_and_report(&channel, &record_path, &record)?;
    execute(&channel, executable, stop, &record_path, &mut record)?;
    persist_and_report(&channel, &record_path, &record)?;
    journal::archive_ack(&record_path, &mut record)
}

fn persist_and_report(channel: &Channel<'_>, path: &Path, record: &Record) -> Result<()> {
    // 先落盘再发回执。响应丢失或进程退出时，下一轮从日志继续补报。
    journal::save(path, record)?;
    channel.post_result(record)
}

fn execute(
    channel: &Channel<'_>,
    executable: &Path,
    stop: &AtomicBool,
    record_path: &Path,
    record: &mut Record,
) -> Result<()> {
    let command = record.command.clone();
    let mut report_executing = |detail: &str| {
        record.result = result(&command, "executing", detail);
        persist_and_report(channel, record_path, record)
    };
    record.result = match &command.template {
        Template::Shutdown {} => {
            report_executing("shutdown_started")?;
            if stop.load(Ordering::Relaxed) || !channel.still_offered(&command)? {
                result(&command, "cancelled", "connection_lost")
            } else if channel.simulation
                || (!channel.test_mode && crate::power::shutdown_machine().is_ok())
            {
                result(&command, "succeeded", "shutdown_requested")
            } else {
                result(&command, "failed", "execution_failed")
            }
        }
        Template::Recovery { options } => {
            report_executing("recovery_started")?;
            execute_recovery(
                channel,
                &command,
                options,
                &record.result.command_digest,
                stop,
            )
        }
        Template::Identify { .. } => {
            match identification::display(
                executable,
                channel.test_mode,
                &command,
                stop,
                || channel.still_offered(&command).unwrap_or(false),
                || report_executing("display_observed"),
            ) {
                Ok((stage, detail)) => result(&command, stage, detail),
                Err(_) => result(&command, "failed", "execution_failed"),
            }
        }
        Template::PrepareDesktop { .. }
        | Template::CheckWorkspace { .. }
        | Template::DeployScripts { .. } => {
            report_executing("preparation_started")?;
            let execution = if channel.test_mode {
                Err("preparation_requires_system_service".into())
            } else {
                crate::preparation::execute(channel.path, &command, || {
                    !stop.load(Ordering::Relaxed)
                        && channel.still_offered(&command).unwrap_or(false)
                })
            };
            outcome::preparation(&command, execution)
        }
    };
    Ok(())
}

fn execute_recovery(
    channel: &Channel<'_>,
    command: &Command,
    options: &wccs_protocol::recovery::Options,
    command_digest: &str,
    stop: &AtomicBool,
) -> TaskResult {
    let started = Instant::now();
    let deadline = options
        .limits
        .as_ref()
        .map_or(900, |limits| limits.timeout_seconds);
    let mut lease = None;
    let execution = crate::recovery::execute(
        channel.path,
        command,
        crate::recovery::Transfer {
            client: channel.client,
            server: &channel.server,
            credential: &channel.credential,
        },
        channel.simulation,
        || {
            if stop.load(Ordering::Relaxed) || started.elapsed() >= Duration::from_secs(deadline) {
                return false;
            }
            if lease.is_some_and(|last: Instant| last.elapsed() < Duration::from_secs(1)) {
                return true;
            }
            let valid = if options.limits.is_some() {
                channel.same_server() && channel.authorize_recovery(command, command_digest)
            } else {
                channel.still_offered(command).unwrap_or(false)
            };
            if valid {
                lease = Some(Instant::now());
            }
            valid
        },
    );
    outcome::recovery(command, execution)
}
