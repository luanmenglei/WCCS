//! Opt-in native adapter for an explicitly provisioned, recoverable Windows test machine.
use crate::{
    host::Result,
    login_broker::{Broker, Offer},
    login_windows as platform,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use wccs_protocol::{
    control::{Actual, CONTROL_VERSION, Command, IpcRequest},
    login::AccountState,
    token,
};
use wccs_session::ipc::{self, Child};
const DISABLED: u32 = 2;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    machine_guid: String,
    machine_name: String,
    account_name: String,
    teaching_name: String,
    recovery_name: String,
    recovery_sid: String,
    work_directory: PathBuf,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    device: String,
    machine_guid: String,
    owner: String,
    account_sid: Option<String>,
    creating_account: bool,
    teaching_sid: String,
    revision: u64,
    sequence: i64,
    command: Option<Command>,
    phase: String,
    detail: String,
    step: u8,
    original_flags: Option<u32>,
    session_key: Option<String>,
}
pub struct Native {
    config: Config,
    directory: PathBuf,
    journal: Journal,
    _lock: fs::File,
    broker: Broker,
    probe: Option<Child>,
    executable: PathBuf,
    stage_started: u64,
    recovery_ready: bool,
    tools_ready: bool,
}
fn persist(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::windows_agent::persist(path, &serde_json::to_vec_pretty(value)?)
        .map_err(|_| "native_journal_failed".into())
}
fn config(path: &Path) -> Result<Config> {
    if !ipc::is_system()? {
        return Err("native_login_requires_system".into());
    }
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    if platform::machine_guid()? != config.machine_guid
        || std::env::var("COMPUTERNAME")?.to_lowercase() != config.machine_name.to_lowercase()
    {
        return Err("native_login_wrong_machine".into());
    }
    if config.account_name.len() > 32
        || !config.account_name.starts_with("Wccs")
        || config
            .account_name
            .eq_ignore_ascii_case(&config.teaching_name)
        || config
            .account_name
            .eq_ignore_ascii_case(&config.recovery_name)
        || config
            .teaching_name
            .eq_ignore_ascii_case(&config.recovery_name)
    {
        return Err("native_account_configuration_invalid".into());
    }
    Ok(config)
}

/// Reuse the provisioned account identity and DPAPI secret without changing login state.
pub fn maintenance_account(
    identity_path: &Path,
    device: &str,
) -> Result<(PathBuf, platform::Handle)> {
    let path = identity_path.with_extension("login-native.json");
    let config = config(&path)?;
    let state = path.with_extension("state");
    let journal: Journal = serde_json::from_slice(&fs::read(state.join("journal.json"))?)?;
    if journal.device != device
        || journal.machine_guid != config.machine_guid
        || journal.creating_account
        || journal.phase != "controlled"
    {
        return Err("maintenance_identity_changed".into());
    }
    let user = platform::user(&config.account_name)?.ok_or("contest_account_missing")?;
    if user.comment != journal.owner
        || journal.account_sid.as_deref() != Some(&user.sid)
        || user.flags & DISABLED != 0
    {
        return Err("contest_account_ownership".into());
    }
    let secret = platform::unprotect(&state.join("contest.dpapi"))?;
    let token = platform::authenticate(&config.account_name, &secret)?;
    if platform::token_sid(token.0)? != user.sid || platform::administrator(token.0)? {
        return Err("ordinary_account_required".into());
    }
    Ok((config.work_directory, token))
}
pub fn maintenance_active(identity: &Path, device: &str, session: &str, sid: &str) -> bool {
    let state = identity
        .with_extension("login-native.json")
        .with_extension("state");
    fs::read(state.join("journal.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Journal>(&bytes).ok())
        .is_some_and(|journal| {
            journal.device == device
                && journal.phase == "controlled"
                && !journal.creating_account
                && journal.account_sid.as_deref() == Some(sid)
                && journal.session_key.as_deref() == Some(session)
        })
}
/// Provisioning consumes a local UTF-16 recovery password file. Passwords never enter argv or JSON responses.
pub fn provision(path: &Path, secret_path: &Path) -> Result<()> {
    let config = config(path)?;
    let directory = path.with_extension("state");
    fs::create_dir_all(&directory)?;
    let mut bytes = fs::read(secret_path)?;
    let secret = platform::Secret(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect(),
    );
    for b in &mut bytes {
        // SAFETY: unique mutable allocation, wipe before releasing source material.
        unsafe {
            std::ptr::write_volatile(b, 0);
        }
    }
    if !bytes.len().is_multiple_of(2) || secret.0.last() != Some(&0) || secret.0.len() > 128 {
        return Err("recovery_secret_format".into());
    }
    let token = platform::authenticate(&config.recovery_name, &secret)?;
    if platform::token_sid(token.0)? != config.recovery_sid || !platform::administrator(token.0)? {
        return Err("recovery_identity_invalid".into());
    }
    platform::protect(&directory.join("recovery.dpapi"), &secret)?;
    Ok(())
}
impl Native {
    pub fn open(path: &Path, device: &str, executable: PathBuf) -> Result<Self> {
        let config = config(path)?;
        let directory = path.with_extension("state");
        fs::create_dir_all(&directory)?;
        use std::os::windows::fs::OpenOptionsExt;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(directory.join("recovery.lock"))?;
        let journal_path = directory.join("journal.json");
        if !journal_path.exists() {
            if platform::user(&config.account_name)?.is_some() {
                return Err("native_account_ownership_missing".into());
            }
            let teaching =
                platform::user(&config.teaching_name)?.ok_or("teaching_account_missing")?;
            let journal = Journal {
                schema: 1,
                device: device.into(),
                machine_guid: config.machine_guid.clone(),
                owner: format!("WCCS:{}", token()),
                account_sid: None,
                creating_account: false,
                teaching_sid: teaching.sid,
                revision: 0,
                sequence: 0,
                command: None,
                phase: "teaching".into(),
                detail: "standby".into(),
                step: 0,
                original_flags: None,
                session_key: None,
            };
            persist(&journal_path, &journal)?;
        }
        let journal: Journal = serde_json::from_slice(&fs::read(journal_path)?)?;
        if journal.schema != 1
            || journal.machine_guid != config.machine_guid
            || journal.device != device
        {
            return Err("native_record_mismatch".into());
        }
        let broker = Broker::start()?;
        let mut result = Self {
            config,
            directory,
            journal,
            _lock: lock,
            broker,
            probe: None,
            executable,
            stage_started: 0,
            recovery_ready: false,
            tools_ready: false,
        };
        let _ = result.recover();
        result.reconcile_account_creation()?;
        result.recovery_ready = result.check_recovery().is_ok();
        result.tools_ready = result.check_tools().is_ok();
        Ok(result)
    }
    fn persist(&mut self) -> Result<()> {
        self.journal.revision += 1;
        persist(&self.directory.join("journal.json"), &self.journal)
    }
    fn check_recovery(&self) -> Result<()> {
        let secret = platform::unprotect(&self.directory.join("recovery.dpapi"))?;
        let token = platform::authenticate(&self.config.recovery_name, &secret)?;
        if platform::token_sid(token.0)? != self.config.recovery_sid
            || !platform::administrator(token.0)?
        {
            return Err("recovery_identity_invalid".into());
        }
        Ok(())
    }
    fn reconcile_account_creation(&mut self) -> Result<()> {
        if self.journal.account_sid.is_some() || !self.journal.creating_account {
            return Ok(());
        }
        let Some(user) = platform::user(&self.config.account_name)? else {
            return Ok(());
        };
        let secret = platform::unprotect(&self.directory.join("contest.dpapi"))?;
        let token = platform::authenticate(&self.config.account_name, &secret)?;
        if user.comment != self.journal.owner || platform::token_sid(token.0)? != user.sid {
            return Err("account_creation_conflict".into());
        }
        self.journal.account_sid = Some(user.sid);
        self.journal.creating_account = false;
        self.persist()
    }
    fn check_tools(&self) -> Result<()> {
        let user = platform::user(&self.config.account_name)?.ok_or("contest_account_missing")?;
        if user.comment != self.journal.owner
            || self.journal.account_sid.as_deref() != Some(&user.sid)
            || user.flags & DISABLED != 0
        {
            return Err("account_ownership_conflict".into());
        }
        let password = platform::unprotect(&self.directory.join("contest.dpapi"))?;
        let token = platform::authenticate(&self.config.account_name, &password)?;
        if platform::token_sid(token.0)? != user.sid {
            return Err("account_sid_changed".into());
        }
        let paths = &self
            .journal
            .command
            .as_ref()
            .and_then(|c| c.login.as_ref())
            .ok_or("tool_snapshot_missing")?
            .tool_paths;
        platform::check_access(token.0, paths, &self.config.work_directory)
    }
    pub fn state(&self) -> AccountState {
        let teaching = platform::user(&self.config.teaching_name).ok().flatten();
        let target = platform::user(&self.config.account_name).ok().flatten();
        let account = match &target {
            None if self.journal.account_sid.is_none() => "absent",
            Some(u)
                if u.comment == self.journal.owner
                    && self.journal.account_sid.as_deref() == Some(&u.sid)
                    && u.flags & DISABLED == 0 =>
            {
                "owned"
            }
            _ => "conflict",
        };
        let session = platform::console(
            self.journal.account_sid.as_deref(),
            &self.journal.teaching_sid,
        )
        .ok();
        let session_key = ipc::discover(false).ok().map(|s| s.key);
        AccountState {
            adapter: "windows_test".into(),
            revision: self.journal.revision,
            phase: self.journal.phase.clone(),
            account: account.into(),
            target_sid: target.map(|u| u.sid),
            session: session
                .as_ref()
                .map_or("unknown".into(), |s| s.kind.clone()),
            observed_sid: session.and_then(|s| s.sid),
            desktop_confirmed: self.journal.phase == "controlled"
                && self.journal.session_key.is_some()
                && self.journal.session_key == session_key,
            recovery_ready: self.recovery_ready,
            tools_ready: self.tools_ready,
            teaching_enabled: teaching
                .is_some_and(|u| u.sid == self.journal.teaching_sid && u.flags & DISABLED == 0),
            detail: self.journal.detail.clone(),
        }
    }
    pub fn accept(&mut self, command: &Command) -> Result<()> {
        if self.journal.command.as_ref() == Some(command) {
            return Ok(());
        }
        let plan = command.login.as_ref().ok_or("native_login_plan_missing")?;
        if command.sequence <= self.journal.sequence
            || self.state() != plan.snapshot
            || self
                .state()
                .skip_reason(&command.action, plan.confirm_session_end)
                .is_some()
        {
            return Err("native_login_preview_changed".into());
        }
        self.journal.sequence = command.sequence;
        self.journal.command = Some(command.clone());
        self.journal.phase = if command.action == "restore_teaching" {
            "recovering"
        } else {
            "preparing"
        }
        .into();
        self.journal.detail = "prepared".into();
        self.journal.step = 0;
        self.stage_started = crate::service::monotonic_ms();
        self.persist()
    }
    pub fn advance(&mut self, command: &Command, clock: u64) -> Result<bool> {
        if self.journal.command.as_ref() != Some(command) {
            return Err("native_login_operation_mismatch".into());
        }
        if command.action == "restore_teaching" {
            self.recover()?;
            return Ok(true);
        }
        if !["preparing", "login_pending"].contains(&self.journal.phase.as_str()) {
            return Err("native_login_authority_revoked".into());
        }
        if clock.saturating_sub(self.stage_started) > 120_000 {
            return Err("native_login_timeout".into());
        }
        match self.journal.step {
            0 => {
                self.check_recovery()?;
                self.recovery_ready = true;
                self.reconcile_account_creation()?;
                let existing = platform::user(&self.config.account_name)?;
                if existing.is_none() {
                    if self.journal.account_sid.is_some() {
                        return Err("owned_account_missing".into());
                    }
                    self.journal.creating_account = true;
                    self.persist()?;
                    // Credential and ownership intent are durable before NetUserAdd.
                    let secret_path = self.directory.join("contest.dpapi");
                    if !secret_path.exists() {
                        let mut bytes = format!("Aa1!{}", token()).into_bytes();
                        let secret = platform::Secret(
                            bytes.iter().map(|b| u16::from(*b)).chain([0]).collect(),
                        );
                        for byte in &mut bytes {
                            // SAFETY: unique allocation holding only the generated password.
                            unsafe {
                                std::ptr::write_volatile(byte, 0);
                            }
                        }
                        platform::protect(&secret_path, &secret)?;
                    }
                    let secret = platform::unprotect(&secret_path)?;
                    let user = platform::create_user(
                        &self.config.account_name,
                        &self.journal.owner,
                        &secret,
                    )?;
                    self.journal.account_sid = Some(user.sid);
                    self.journal.creating_account = false;
                    self.persist()?;
                }
                platform::ensure_users_member(
                    self.journal
                        .account_sid
                        .as_deref()
                        .ok_or("contest_sid_missing")?,
                )?;
                self.check_tools()?;
                self.tools_ready = true;
            }
            1 => {
                let session = platform::console(
                    self.journal.account_sid.as_deref(),
                    &self.journal.teaching_sid,
                )?;
                if session.kind == "teaching"
                    && command
                        .login
                        .as_ref()
                        .is_some_and(|p| p.confirm_session_end)
                {
                    self.persist()?;
                    platform::logoff(session.id.ok_or("teaching_session_missing")?)?;
                    return Ok(false);
                }
                if !["none", "contest"].contains(&session.kind.as_str()) {
                    return Err("native_session_changed".into());
                }
            }
            2 => {
                let session = platform::console(
                    self.journal.account_sid.as_deref(),
                    &self.journal.teaching_sid,
                )?;
                if !["none", "contest"].contains(&session.kind.as_str()) {
                    return Err("native_session_changed".into());
                }
                let account = platform::user(&self.config.teaching_name)?
                    .ok_or("teaching_account_missing")?;
                if account.sid != self.journal.teaching_sid {
                    return Err("teaching_account_changed".into());
                }
                self.journal.original_flags = Some(account.flags);
                self.persist()?;
                platform::set_flags(
                    &self.config.teaching_name,
                    &account.sid,
                    account.flags,
                    account.flags | DISABLED,
                )?;
            }
            3 => {
                let current = platform::console(
                    self.journal.account_sid.as_deref(),
                    &self.journal.teaching_sid,
                )?;
                if !["none", "contest"].contains(&current.kind.as_str()) {
                    return Err("native_session_changed".into());
                }
                let session = ipc::discover(false).ok();
                if session
                    .as_ref()
                    .is_some_and(|s| Some(s.sid()) == self.journal.account_sid.as_deref())
                {
                    self.journal.step = 4;
                } else {
                    self.broker.issue(Offer {
                        device: command.device_id.clone(),
                        instance: command.instance_id.clone(),
                        operation: command.command_id.clone(),
                        username: self.config.account_name.clone(),
                        sid: self
                            .journal
                            .account_sid
                            .clone()
                            .ok_or("contest_sid_missing")?,
                        password: platform::unprotect(&self.directory.join("contest.dpapi"))?,
                        deadline: clock + 5_000,
                    })?;
                }
                self.journal.phase = "login_pending".into();
                self.journal.detail = "login_submitted".into();
            }
            4 => {
                if !self.broker.consumed(&command.command_id) {
                    if clock.saturating_sub(self.stage_started) > 20_000 {
                        return Err("native_login_not_consumed".into());
                    }
                    return Ok(false);
                }
            }
            _ => {
                let current = platform::console(
                    self.journal.account_sid.as_deref(),
                    &self.journal.teaching_sid,
                )?;
                if ["multiple", "rdp", "teaching", "unknown"].contains(&current.kind.as_str()) {
                    return Err("native_session_changed".into());
                }
                let Ok(session) = ipc::discover(false) else {
                    return Ok(false);
                };
                if Some(session.sid()) != self.journal.account_sid.as_deref() {
                    return Err("native_login_sid_mismatch".into());
                }
                if self.probe.is_none() {
                    self.probe = Some(Child::start(&self.executable, &session, false)?);
                }
                let probe = self.probe.as_mut().unwrap();
                if !probe.ready()? {
                    return Ok(false);
                }
                if let Some(actual) = probe.pipe.receive::<Actual>()? {
                    if actual.state != "desktop"
                        || actual.command_id.as_deref() != Some(&command.command_id)
                        || actual.session_key.as_deref() != Some(&session.key)
                    {
                        return Err("native_desktop_evidence_invalid".into());
                    }
                    self.journal.session_key = Some(session.key);
                    self.journal.phase = "controlled".into();
                    self.journal.detail = "desktop_observed".into();
                    self.probe = None;
                    self.persist()?;
                    return Ok(true);
                }
                let mut probe_command = command.clone();
                probe_command.action = "probe_desktop".into();
                probe_command.session_key = session.key;
                probe_command.login.as_mut().unwrap().snapshot.target_sid =
                    self.journal.account_sid.clone();
                probe.pipe.send(&IpcRequest {
                    presentation: None,
                    protocol_version: CONTROL_VERSION,
                    command: probe_command,
                    lease_seconds: 5,
                })?;
                return Ok(false);
            }
        }
        self.journal.step += 1;
        self.persist()?;
        Ok(false)
    }
    pub fn recover(&mut self) -> Result<()> {
        self.broker.revoke();
        self.probe = None;
        if self.journal.phase == "teaching" && self.journal.original_flags.is_none() {
            return Ok(());
        }
        self.journal.phase = "recovering".into();
        self.persist()?;
        let result = (|| -> Result<()> {
            if platform::machine_guid()? != self.journal.machine_guid {
                return Err("native_restore_machine_changed".into());
            }
            if let Some(original) = self.journal.original_flags {
                let current = platform::user(&self.config.teaching_name)?
                    .ok_or("teaching_account_missing")?;
                if current.sid != self.journal.teaching_sid {
                    return Err("teaching_sid_changed".into());
                }
                if current.flags != original {
                    platform::set_flags(
                        &self.config.teaching_name,
                        &current.sid,
                        original | DISABLED,
                        original,
                    )?;
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.journal.phase = "recovery_failed".into();
            self.journal.detail = "recovery_conflict".into();
            self.persist()?;
            return result;
        }
        self.journal.original_flags = None;
        self.journal.phase = "teaching".into();
        self.journal.detail = "restored".into();
        self.journal.session_key = None;
        self.persist()
    }
}
