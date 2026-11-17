//! WSS owns waiting commands. The existing HTTP worker exclusively owns tool jobs.
use futures_util::{SinkExt, StreamExt};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
use wccs_protocol::{control::*, token};
use wccs_session::ipc::{self, Child};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn run(
    path: PathBuf,
    executable: PathBuf,
    test_mode: bool,
    simulate_login: bool,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("lock"))?;
    let path = path.canonicalize()?;
    let executable = executable.canonicalize()?;
    if !test_mode && !ipc::is_system()? {
        return Err("service_requires_local_system".into());
    }
    if simulate_login && !test_mode {
        return Err("simulation_requires_test_mode".into());
    }
    let accounts: Option<Box<dyn crate::login::Runtime>> = if simulate_login {
        let identity: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        Some(Box::new(crate::login::Simulator::open(
            path.with_extension("login-simulation"),
            identity["device_id"].as_str().ok_or("device_missing")?,
        )?))
    } else if !test_mode && path.with_extension("login-native.json").exists() {
        let identity: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        Some(Box::new(crate::login_native::Native::open(
            &path.with_extension("login-native.json"),
            identity["device_id"].as_str().ok_or("device_missing")?,
            executable.clone(),
        )?))
    } else {
        None
    };
    log(&path, "host_started")?;
    let runtime = tokio::runtime::Runtime::new()?;
    let instance = token();
    let worker_path = path.clone();
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || {
        crate::windows_agent::run_worker(&worker_path, worker_stop);
    });
    let diagnostic_path = path.clone();
    let diagnostic_executable = executable.clone();
    let diagnostic_stop = stop.clone();
    let diagnostic_worker = std::thread::spawn(move || {
        crate::diagnostics::service_worker(
            &diagnostic_path,
            &diagnostic_executable,
            test_mode,
            diagnostic_stop,
        );
    });
    let presentation: crate::presentation::Shared = Default::default();
    let presentation_state = presentation.clone();
    let presentation_path = path.clone();
    let presentation_instance = instance.clone();
    let presentation_stop = stop.clone();
    let presentation_worker = std::thread::spawn(move || {
        crate::presentation::worker(
            &presentation_path,
            &presentation_instance,
            test_mode,
            presentation_state,
            presentation_stop,
        )
    });
    let health_path = path.clone();
    let health_instance = instance.clone();
    let health_stop = stop.clone();
    let health_worker = std::thread::spawn(move || {
        crate::health::worker(&health_path, &health_instance, health_stop)
    });
    let maintenance_path = path.clone();
    let distribution_path = path.clone();
    let distribution_stop = stop.clone();
    let distribution_worker = std::thread::spawn(move || {
        crate::distribution::worker(&distribution_path, test_mode, distribution_stop)
    });
    let maintenance_executable = executable.clone();
    let maintenance_stop = stop.clone();
    let maintenance_instance = instance.clone();
    let maintenance_worker = std::thread::spawn(move || {
        crate::maintenance::worker(
            &maintenance_path,
            &maintenance_executable,
            &maintenance_instance,
            test_mode,
            maintenance_stop,
        );
    });
    runtime.block_on(async {
        if test_mode {
            let signal = stop.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                signal.store(true, Ordering::Relaxed);
            });
        }
        let mut host = Controller {
            presentation,
            path,
            executable,
            accounts,
            test_mode,
            instance,
            child: None,
            command: None,
            report: None,
            actual: Actual::default(),
            last_ui: Instant::now(),
            stage_started: Instant::now(),
            command_deadline: Instant::now(),
        };
        let mut backoff = 1;
        // Startup never replays waiting authority. Recovery failures remain observable.
        let _ = host.recover_accounts();
        while !stop.load(Ordering::Relaxed) {
            let start = Instant::now();
            if let Err(error) = host.connection(&stop).await {
                eprintln!("control_connection_unavailable");
                log_error(&host.path, "control_connection_unavailable", error.as_ref());
            }
            let _ = host.fail("control_disconnected");
            // A disconnected completed command cannot silently create another window.
            if start.elapsed() > Duration::from_secs(60) {
                backoff = 1;
            } else {
                backoff = (backoff * 2).min(30);
            }
            let delay = backoff + rand::random::<u64>() % 3;
            for _ in 0..delay * 5 {
                let _ = host.recover_accounts();
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        let _ = host.fail("control_disconnected");
        host.child = None;
        let _ = log(&host.path, "host_stopped");
    });
    stop.store(true, Ordering::Relaxed);
    worker.join().map_err(|_| "http_worker_panicked")?;
    presentation_worker
        .join()
        .map_err(|_| "presentation_worker_panicked")?;
    health_worker.join().map_err(|_| "health_worker_panicked")?;
    diagnostic_worker
        .join()
        .map_err(|_| "diagnostic_worker_panicked")?;
    maintenance_worker
        .join()
        .map_err(|_| "maintenance_worker_panicked")?;
    distribution_worker
        .join()
        .map_err(|_| "distribution_worker_panicked")?;
    Ok(())
}

struct Controller {
    presentation: crate::presentation::Shared,
    path: PathBuf,
    accounts: Option<Box<dyn crate::login::Runtime>>,
    executable: PathBuf,
    test_mode: bool,
    instance: String,
    child: Option<Child>,
    command: Option<Command>,
    report: Option<Report>,
    actual: Actual,
    last_ui: Instant,
    stage_started: Instant,
    command_deadline: Instant,
}
impl Controller {
    fn observe_accounts(&mut self) {
        self.actual.account = Some(
            self.accounts
                .as_ref()
                .map_or_else(wccs_protocol::login::AccountState::unavailable, |s| {
                    s.state()
                }),
        );
    }
    fn recover_accounts(&mut self) -> Result<()> {
        let policy = self.recover_waiting_policy();
        let result = self.accounts.as_mut().map_or(Ok(()), |s| s.recover());
        self.observe_accounts();
        policy.and(result)
    }
    fn recover_waiting_policy(&mut self) -> Result<()> {
        if self.test_mode {
            return Ok(());
        }
        let result =
            crate::waiting_policy::recover(&self.path.with_extension("waiting-policy.json"));
        if let Err(error) = &result {
            self.actual.state = "failed".into();
            self.actual.detail = "waiting_policy_recovery_failed".into();
            log_error(&self.path, "waiting_policy_recovery_failed", error.as_ref());
            self.persist()?;
        }
        result
    }
    fn stop_child(&mut self) -> Result<()> {
        if let Some(child) = &mut self.child {
            child.stop()?;
        }
        self.child = None;
        Ok(())
    }
    fn persist(&self) -> Result<()> {
        // Audit journal is never replayed as authority on a new host instance.
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema_version":1,"instance_id":self.instance,"command":self.command,"report":self.report,"actual":self.actual}),
        )?;
        crate::windows_agent::persist(&self.path.with_extension("control.json"), &bytes)
            .map_err(|_| "control_persist_failed")?;
        Ok(())
    }
    async fn connection(&mut self, stop: &AtomicBool) -> Result<()> {
        let identity: serde_json::Value = serde_json::from_slice(&fs::read(&self.path)?)?;
        let typed_identity = serde_json::from_value(identity.clone())?;
        let server = crate::connection::server(&self.path, &typed_identity)
            .map_err(|_| "server_not_configured")?;
        let mut url =
            crate::windows_agent::validate_server(&server).map_err(|_| "invalid_server_origin")?;
        let device = identity["device_id"].as_str().ok_or("device_missing")?;
        let credential = fs::read_to_string(self.path.with_extension("credential"))?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| "invalid_control_scheme")?;
        url.set_path("/api/device/control");
        let mut request = url.as_str().into_client_request()?;
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {credential}").parse()?);
        request
            .headers_mut()
            .insert("x-wccs-device-id", device.parse()?);
        request
            .headers_mut()
            .insert("sec-websocket-protocol", CONTROL_PROTOCOL.parse()?);
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE))
            .write_buffer_size(0)
            .max_write_buffer_size(MAX_MESSAGE * 2);
        let (mut socket, response) = tokio::time::timeout(
            Duration::from_secs(5),
            connect_async_with_config(request, Some(config), false),
        )
        .await??;
        if response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|h| h.to_str().ok())
            != Some(CONTROL_PROTOCOL)
        {
            return Err("control_protocol_mismatch".into());
        }
        let mut last_sync = crate::service::monotonic_ms();
        while !stop.load(Ordering::Relaxed) {
            if crate::connection::server(&self.path, &typed_identity)
                .ok()
                .as_deref()
                != Some(&server)
            {
                return Err("server_configuration_changed".into());
            }
            if crate::service::monotonic_ms().saturating_sub(last_sync) > 5_000 {
                return Err("control_heartbeat_expired".into());
            }
            self.observe_accounts();
            if self.actual.detail == "waiting_policy_recovery_failed"
                && self.recover_accounts().is_ok()
            {
                self.actual = Actual::default();
                self.observe_accounts();
                self.persist()?;
            }
            if self
                .actual
                .account
                .as_ref()
                .is_some_and(|s| s.phase == "controlled" && !s.login_complete())
            {
                self.stop_child()?;
                let _ = self.recover_accounts();
            }
            let session = ipc::discover(self.test_mode).ok();
            let session_key = session.as_ref().map(|s| s.key.clone());
            let poll = ControlPoll {
                protocol_version: CONTROL_VERSION,
                instance_id: self.instance.clone(),
                session_key: session_key.clone(),
                capabilities: if self.accounts.is_some() {
                    vec![
                        "waiting-v1".into(),
                        if self.test_mode {
                            "contest-login-simulation-v1"
                        } else {
                            "contest-login-windows-test-v1"
                        }
                        .into(),
                    ]
                } else {
                    vec!["waiting-v1".into()]
                },
                actual: self.actual.clone(),
                report: self.report.clone(),
            };
            tokio::time::timeout(
                Duration::from_secs(3),
                socket.send(Message::Text(serde_json::to_string(&poll)?.into())),
            )
            .await
            .map_err(|_| "control_send_timeout")??;
            let Some(Ok(Message::Text(text))) =
                tokio::time::timeout(Duration::from_secs(5), socket.next())
                    .await
                    .map_err(|_| "control_receive_timeout")?
            else {
                return Err("control_transport_closed".into());
            };
            let received_at = crate::service::monotonic_ms();
            if received_at.saturating_sub(last_sync) > 8_000 {
                return Err("control_heartbeat_expired".into());
            }
            last_sync = received_at;
            let reply: ControlReply = serde_json::from_str(&text)?;
            if reply.protocol_version != CONTROL_VERSION
                || reply.lease_seconds == 0
                || reply.lease_seconds > LEASE_SECONDS
            {
                return Err("control_protocol_mismatch".into());
            }
            if let Some(command) = reply.command {
                if command.device_id != device
                    || command.generation != reply.generation
                    || command.instance_id != self.instance
                    || command.protocol_version != CONTROL_VERSION
                    || command.expires_at <= reply.server_time
                    || !["enter", "leave", "prepare_login", "restore_teaching"]
                        .contains(&command.action.as_str())
                    || command
                        .login
                        .as_ref()
                        .is_some_and(|p| p.connection_id != reply.connection_id)
                {
                    return Err("control_command_rejected".into());
                }
                let result = if command.action == "enter"
                    && session_key.as_deref() != Some(&command.session_key)
                {
                    self.fail("session_changed")
                } else {
                    self.apply(
                        command.clone(),
                        session.as_ref(),
                        reply
                            .lease_seconds
                            .min((command.expires_at - reply.server_time) as u64),
                        (command.expires_at - reply.server_time) as u64,
                    )
                };
                if let Err(error) = result
                    && self.actual.detail != "waiting_policy_recovery_failed"
                {
                    return Err(error);
                }
            } else {
                self.stop_child()?;
                if self.recover_accounts().is_ok() {
                    self.actual = if session.is_some() {
                        Actual::default()
                    } else {
                        Actual {
                            state: "no_session".into(),
                            detail: "no_available_session".into(),
                            ..Default::default()
                        }
                    };
                    self.report = None;
                }
                self.observe_accounts();
                self.persist()?;
            }
            for _ in 0..5 {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        let _ = socket.close(None).await;
        Ok(())
    }
    fn stage(&mut self, stage: &str) -> Result<()> {
        self.observe_accounts();
        if let Some(command) = &self.command {
            self.report = Some(Report {
                command_id: command.command_id.clone(),
                sequence: command.sequence,
                stage: stage.into(),
                actual: self.actual.clone(),
            });
        }
        self.persist()
    }
    fn fail(&mut self, detail: &str) -> Result<()> {
        let stopped = self.stop_child();
        let recovery = self.recover_accounts();
        self.observe_accounts();
        self.actual = Actual {
            state: "failed".into(),
            detail:
                if recovery.is_err() && self.actual.detail == "waiting_policy_recovery_failed" {
                    "waiting_policy_recovery_failed"
                } else {
                    detail
                }
                .into(),
            account: self.actual.account.clone(),
            ..Default::default()
        };
        if self
            .report
            .as_ref()
            .is_some_and(|r| r.stage == "succeeded" || r.stage == "failed")
        {
            self.persist()?;
        } else {
            self.stage("failed")?;
        }
        stopped.and(recovery)
    }
    fn apply(
        &mut self,
        command: Command,
        session: Option<&ipc::Session>,
        lease: u64,
        remaining: u64,
    ) -> Result<()> {
        if self
            .command
            .as_ref()
            .is_none_or(|c| c.command_id != command.command_id)
        {
            if self
                .command
                .as_ref()
                .is_some_and(|c| c.sequence >= command.sequence)
            {
                return Err("control_sequence_regression".into());
            }
            if ["prepare_login", "restore_teaching"].contains(&command.action.as_str()) {
                self.stop_child()?;
                self.recover_waiting_policy()?;
                self.accounts
                    .as_mut()
                    .ok_or("native_login_unavailable")?
                    .accept(&command)?;
            }
            self.command = Some(command);
            self.stage_started = Instant::now();
            self.command_deadline = Instant::now() + Duration::from_secs(remaining.min(3600));
            self.stage("accepted")?;
            return Ok(());
        }
        if Instant::now() >= self.command_deadline {
            return self.fail("command_expired");
        }
        let lease = lease.min(
            self.command_deadline
                .saturating_duration_since(Instant::now())
                .as_secs()
                .max(1),
        );
        if self.command.as_ref() != Some(&command) {
            return Err("control_command_conflict".into());
        }
        if self.report.as_ref().is_some_and(|r| r.stage == "failed") {
            return Ok(());
        }
        if self.report.as_ref().is_some_and(|r| r.stage == "accepted") {
            self.stage("executing")?;
            if command.action == "enter" {
                self.stop_child()?;
                let Some(session) = session else {
                    return self.fail("no_session");
                };
                if !self.test_mode {
                    self.recover_waiting_policy()?;
                    self.observe_accounts();
                    if !self.actual.account.as_ref().is_some_and(|a| {
                        a.phase == "controlled"
                            && a.login_complete()
                            && a.target_sid.as_deref() == Some(session.sid())
                    }) {
                        return self.fail("waiting_account_required");
                    }
                    if let Err(error) = crate::waiting_policy::apply(
                        &self.path.with_extension("waiting-policy.json"),
                        session.sid(),
                    ) {
                        log_error(&self.path, "waiting_policy_apply_failed", error.as_ref());
                        return self.fail("waiting_policy_apply_failed");
                    }
                }
                match Child::start(&self.executable, session, self.test_mode) {
                    Ok(child) => self.child = Some(child),
                    Err(_) => return self.fail("session_launch_failed"),
                }
            }
            return Ok(());
        }
        if ["prepare_login", "restore_teaching"].contains(&command.action.as_str()) {
            if self.report.as_ref().is_some_and(|r| r.stage == "succeeded") {
                return Ok(());
            }
            let result = self
                .accounts
                .as_mut()
                .ok_or("native_login_unavailable")?
                .advance(&command, crate::service::monotonic_ms());
            self.observe_accounts();
            match result {
                Ok(true) => {
                    self.actual.state = "desktop".into();
                    self.actual.detail = "standby".into();
                    self.stage("succeeded")?;
                }
                Ok(false) => self.persist()?,
                Err(error) => {
                    // Only bounded machine codes enter the diagnostic log, never arbitrary error text.
                    let detail = error.to_string();
                    if detail.len() <= 80
                        && detail
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    {
                        let _ = log(&self.path, &format!("account_operation_failed {detail}"));
                    } else if let Some(error) = error.downcast_ref::<windows::core::Error>() {
                        let _ = log(
                            &self.path,
                            &format!("account_operation_failed hresult={:08x}", error.code().0),
                        );
                    }
                    self.fail("ipc_or_session_failed")?;
                }
            }
            return Ok(());
        }
        if session.is_none() {
            return self.fail("no_session");
        }
        if command.action == "leave" && self.child.is_none() {
            self.recover_waiting_policy()?;
            self.actual = Actual::default();
            if self.report.as_ref().is_none_or(|r| r.stage != "succeeded") {
                self.stage("succeeded")?;
            }
            return Ok(());
        }
        let Some(child) = &mut self.child else {
            return self.fail("session_process_unavailable");
        };
        if command.action == "enter"
            && !self.test_mode
            && crate::waiting_policy::verify(&self.path.with_extension("waiting-policy.json"))
                .is_err()
        {
            return self.fail("waiting_policy_changed");
        }
        let exchange = (|| -> Result<Option<Actual>> {
            if !child.ready()? {
                return Ok(None);
            }
            let actual = child.pipe.receive::<Actual>()?;
            if !actual
                .as_ref()
                .is_some_and(|a| command.action == "leave" && a.state == "desktop")
            {
                child.pipe.send(&IpcRequest {
                    protocol_version: CONTROL_VERSION,
                    command: command.clone(),
                    lease_seconds: lease,
                    presentation: self
                        .presentation
                        .lock()
                        .ok()
                        .and_then(|s| s.as_ref().filter(|p| p.data.matches(&command)).cloned()),
                })?;
            }
            Ok(actual)
        })();
        match exchange {
            Ok(Some(actual)) if actual.valid() => {
                let observation_changed = self.actual.state != actual.state
                    || self.actual.detail != actual.detail
                    || self.actual.monitors != actual.monitors;
                self.actual = actual;
                self.observe_accounts();
                self.last_ui = Instant::now();
                let complete = if command.action == "enter" {
                    self.actual.state == "waiting"
                        && self.actual.command_id.as_deref() == Some(&command.command_id)
                        && self.actual.session_key.as_deref() == Some(&command.session_key)
                } else {
                    self.actual.state == "desktop"
                };
                if command.action == "leave" && complete {
                    self.stop_child()?;
                    self.recover_waiting_policy()?;
                }
                if complete && self.report.as_ref().is_none_or(|r| r.stage != "succeeded") {
                    self.stage("succeeded")?;
                } else if observation_changed {
                    self.persist()?;
                }
            }
            Ok(None)
                if self.last_ui.elapsed() < Duration::from_secs(8)
                    || self.stage_started.elapsed() < Duration::from_secs(8) => {}
            _ => return self.fail("ipc_or_session_failed"),
        }
        Ok(())
    }
}

fn log_error(path: &std::path::Path, operation: &str, error: &(dyn std::error::Error + 'static)) {
    let detail = error.to_string();
    if let Some(error) = error.downcast_ref::<windows::core::Error>() {
        let _ = log(path, &format!("{operation} hresult={:08x}", error.code().0));
    } else if detail.len() <= 80
        && detail
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        let _ = log(path, &format!("{operation} {detail}"));
    } else {
        let _ = log(path, operation);
    }
}

fn log(path: &std::path::Path, event: &str) -> Result<()> {
    use std::io::Write;
    let path = path.with_extension("host.log");
    let truncate = fs::metadata(&path).is_ok_and(|m| m.len() > 1_048_576);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(path)?;
    writeln!(file, "{} {event}", wccs_protocol::now())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Controller, Command) {
        let directory = std::env::temp_dir().join(format!("wccs-control-{}", token()));
        fs::create_dir(&directory).unwrap();
        let command = Command {
            protocol_version: 1,
            command_id: "command-one".into(),
            request_id: "request-one".into(),
            sequence: 1,
            device_id: "synthetic".into(),
            generation: 1,
            device_version: 1,
            instance_id: "instance".into(),
            session_key: "session".into(),
            action: "enter".into(),
            expires_at: wccs_protocol::now() + 900,
            login: None,
            identity: WaitingIdentity {
                contest_id: "contest".into(),
                contest_name: "Synthetic".into(),
                contest_version: 1,
                participant_id: "person".into(),
                name: "Synthetic".into(),
                school: "School".into(),
                seat_id: "seat".into(),
                seat_label: "A1".into(),
                room_id: "A".into(),
                room_version: 1,
            },
        };
        (
            Controller {
                presentation: Default::default(),
                path: directory.join("device.json"),
                accounts: None,
                executable: directory.join("unused.exe"),
                test_mode: true,
                instance: "instance".into(),
                child: None,
                command: None,
                report: None,
                actual: Actual::default(),
                last_ui: Instant::now(),
                stage_started: Instant::now(),
                command_deadline: Instant::now(),
            },
            command,
        )
    }
    fn cleanup(host: Controller) {
        fs::remove_file(host.path.with_extension("control.json")).unwrap();
        fs::remove_dir(host.path.parent().unwrap()).unwrap();
    }
    #[test]
    fn accepted_is_durable_and_no_session_cannot_complete() {
        let (mut host, command) = fixture();
        host.apply(command.clone(), None, 15, 900).unwrap();
        assert_eq!(host.report.as_ref().unwrap().stage, "accepted");
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(host.path.with_extension("control.json")).unwrap())
                .unwrap();
        assert_eq!(saved["command"]["command_id"], "command-one");
        host.apply(command.clone(), None, 15, 900).unwrap();
        assert_eq!(host.report.as_ref().unwrap().stage, "failed");
        host.apply(command, None, 15, 900).unwrap();
        assert!(host.child.is_none());
        cleanup(host);
    }
    #[test]
    fn monotonic_deadline_and_sequence_fence_prevent_revival() {
        let (mut host, command) = fixture();
        host.apply(command.clone(), None, 15, 900).unwrap();
        host.command_deadline = Instant::now() - Duration::from_secs(1);
        host.apply(command.clone(), None, 15, 3600).unwrap();
        assert_eq!(host.actual.detail, "command_expired");
        let mut older = command.clone();
        older.command_id = "older".into();
        older.sequence = 0;
        assert!(host.apply(older, None, 15, 900).is_err());
        let mut newer = command;
        newer.command_id = "newer".into();
        newer.sequence = 2;
        host.apply(newer, None, 15, 900).unwrap();
        assert_eq!(host.report.as_ref().unwrap().stage, "accepted");
        cleanup(host);
    }
    #[test]
    fn lost_window_keeps_original_receipt_and_reports_actual_failure() {
        let (mut host, command) = fixture();
        host.apply(command.clone(), None, 15, 900).unwrap();
        host.stage("succeeded").unwrap();
        host.fail("session_process_exited").unwrap();
        assert_eq!(host.report.as_ref().unwrap().stage, "succeeded");
        assert_eq!(host.actual.state, "failed");
        let mut changed = command;
        changed.identity.name = "Changed".into();
        assert!(host.apply(changed, None, 15, 900).is_err());
        cleanup(host);
    }
    #[test]
    fn corrupt_policy_journal_reports_recovery_failure_and_preserves_evidence() {
        let (mut host, command) = fixture();
        host.apply(command, None, 15, 900).unwrap();
        let journal = host.path.with_extension("waiting-policy.json");
        fs::write(&journal, b"interrupted journal").unwrap();
        host.test_mode = false;
        assert!(host.fail("control_disconnected").is_err());
        assert_eq!(host.actual.detail, "waiting_policy_recovery_failed");
        assert_eq!(host.report.as_ref().unwrap().stage, "failed");
        assert!(host.actual.valid());
        assert_eq!(fs::read(&journal).unwrap(), b"interrupted journal");
        fs::remove_file(journal).unwrap();
        fs::remove_file(host.path.with_extension("host.log")).unwrap();
        cleanup(host);
    }
}
