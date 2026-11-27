//! 现场人员和远程维护共用同一个座位识别窗口，分别提供本机配置核对和服务器续权。
use crate::{AgentResult as Result, connection, identity::Identity};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use wccs_protocol::{
    control,
    maintenance::{Command, Template},
    now, token,
};
use wccs_session::ipc;

pub(super) fn display(
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
