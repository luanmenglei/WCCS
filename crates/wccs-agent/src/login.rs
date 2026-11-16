//! File-backed fault simulator. This module never calls Windows account or logon APIs.
//! The journal models write-ahead compensation and remains separate from live authority.
use crate::host::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use wccs_protocol::{control::Command, login::AccountState, token};

/// The controller validates the current authenticated connection and host instance
/// before calling an account adapter. A replacement server/database can restart
/// its sequence, so a durable account journal must not compare different scopes.
pub(crate) fn sequence_is_new(
    previous: Option<&Command>,
    sequence: i64,
    command: &Command,
) -> bool {
    command.sequence > 0
        && (command.sequence > sequence
            || previous.is_some_and(|previous| {
                previous.device_id == command.device_id
                    && previous
                        .login
                        .as_ref()
                        .zip(command.login.as_ref())
                        .is_some_and(|(old, new)| {
                            previous.instance_id != command.instance_id
                                || old.connection_id != new.connection_id
                        })
            }))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Machine {
    incarnation: String,
    account_sid: Option<String>,
    account_owner: Option<String>,
    account_admin: bool,
    account_enabled: bool,
    teaching_sid: String,
    teaching_enabled: bool,
    teaching_revision: u64,
    session: String,
    observed_sid: Option<String>,
    desktop_confirmed: bool,
    recovery_ready: bool,
    tools_ready: bool,
    fault: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    device: String,
    incarnation: String,
    owner: String,
    #[serde(default)]
    account_sid: Option<String>,
    revision: u64,
    sequence: i64,
    command: Option<Command>,
    phase: String,
    detail: String,
    step: u8,
    teaching_sid: Option<String>,
    original_enabled: Option<bool>,
    before_revision: Option<u64>,
}

pub struct Simulator {
    directory: PathBuf,
    _lock: fs::File,
    journal: Journal,
    // A permit is deliberately memory-only and expires against a caller-supplied monotonic clock.
    permit: Option<(String, String, String, u64)>,
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::windows_agent::persist(path, &serde_json::to_vec_pretty(value)?)
        .map_err(|_| "login_journal_write_failed".into())
}

impl Simulator {
    pub fn open(directory: PathBuf, device: &str) -> Result<Self> {
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
        let machine_path = directory.join("machine.json");
        if !journal_path.exists() && !machine_path.exists() {
            let machine = Machine {
                incarnation: token(),
                account_sid: None,
                account_owner: None,
                account_admin: false,
                account_enabled: true,
                teaching_sid: "S-1-5-21-100-100-100-500".into(),
                teaching_enabled: true,
                teaching_revision: 0,
                session: "none".into(),
                observed_sid: None,
                desktop_confirmed: false,
                recovery_ready: true,
                tools_ready: true,
                fault: String::new(),
            };
            let journal = Journal {
                schema: 1,
                device: device.into(),
                incarnation: machine.incarnation.clone(),
                owner: token(),
                account_sid: None,
                revision: 0,
                sequence: 0,
                command: None,
                phase: "teaching".into(),
                detail: "standby".into(),
                step: 0,
                teaching_sid: None,
                original_enabled: None,
                before_revision: None,
            };
            save(&machine_path, &machine)?;
            save(&journal_path, &journal)?;
        }
        // A missing/corrupt peer record is never silently recreated after image restoration.
        let journal: Journal = serde_json::from_slice(&fs::read(journal_path)?)?;
        if journal.schema != 1 || journal.device != device {
            return Err("login_record_mismatch".into());
        }
        let mut result = Self {
            directory,
            _lock: lock,
            journal,
            permit: None,
        };
        let _ = result.recover();
        Ok(result)
    }
    fn machine(&self) -> Result<Machine> {
        let machine: Machine =
            serde_json::from_slice(&fs::read(self.directory.join("machine.json"))?)?;
        if machine.incarnation != self.journal.incarnation {
            return Err("login_record_mismatch".into());
        }
        Ok(machine)
    }
    fn persist(&mut self) -> Result<()> {
        self.journal.revision += 1;
        save(&self.directory.join("journal.json"), &self.journal)
    }
    fn write_machine(&self, machine: &Machine) -> Result<()> {
        save(&self.directory.join("machine.json"), machine)
    }
    pub fn state(&self) -> AccountState {
        let Ok(m) = self.machine() else {
            return AccountState {
                adapter: "simulation".into(),
                phase: "recovery_failed".into(),
                detail: "record_mismatch".into(),
                ..AccountState::unavailable()
            };
        };
        let account = if m.account_sid.is_none() {
            "absent"
        } else if m.account_owner.as_deref() == Some(&self.journal.owner)
            && m.account_sid == self.journal.account_sid
            && !m.account_admin
            && m.account_enabled
        {
            "owned"
        } else {
            "conflict"
        };
        AccountState {
            adapter: "simulation".into(),
            revision: self.journal.revision,
            phase: self.journal.phase.clone(),
            account: account.into(),
            target_sid: m.account_sid,
            session: m.session,
            observed_sid: m.observed_sid,
            desktop_confirmed: m.desktop_confirmed,
            recovery_ready: m.recovery_ready,
            tools_ready: m.tools_ready,
            teaching_enabled: m.teaching_enabled,
            detail: self.journal.detail.clone(),
        }
    }
    pub fn accept(&mut self, command: &Command) -> Result<()> {
        if self.journal.command.as_ref() == Some(command) {
            return Ok(());
        }
        if !sequence_is_new(
            self.journal.command.as_ref(),
            self.journal.sequence,
            command,
        ) {
            return Err("login_replay_rejected".into());
        }
        let plan = command.login.as_ref().ok_or("login_plan_missing")?;
        let state = self.state();
        if state != plan.snapshot
            || state
                .skip_reason(&command.action, plan.confirm_session_end)
                .is_some()
        {
            return Err("login_precondition_changed".into());
        }
        self.journal.command = Some(command.clone());
        self.journal.sequence = command.sequence;
        self.journal.step = 0;
        self.journal.phase = if command.action == "restore_teaching" {
            "recovering"
        } else {
            "preparing"
        }
        .into();
        self.journal.detail = "prepared".into();
        self.persist()
    }
    /// Called serially only after a fresh authenticated response. Each poll advances one step.
    pub fn advance(&mut self, command: &Command, clock_ms: u64) -> Result<bool> {
        if self.journal.command.as_ref() != Some(command) {
            return Err("login_operation_mismatch".into());
        }
        if command.action == "restore_teaching" {
            self.recover()?;
            return Ok(self.state().phase == "teaching");
        }
        if self.journal.phase == "controlled" {
            return Ok(self.state().login_complete());
        }
        if !["preparing", "login_pending"].contains(&self.journal.phase.as_str()) {
            return Err("login_authority_revoked".into());
        }
        let mut m = self.machine()?;
        if !m.recovery_ready || !m.tools_ready {
            return self.abort("precondition_failed");
        }
        match self.journal.step {
            0 => {
                if m.account_sid.is_some()
                    && (m.account_owner.as_deref() != Some(&self.journal.owner)
                        || m.account_sid != self.journal.account_sid
                        || m.account_admin
                        || !m.account_enabled)
                {
                    return self.abort("precondition_failed");
                }
                // Ownership was durably allocated before account creation. Retried creation checks it.
                m.account_sid
                    .get_or_insert_with(|| "S-1-5-21-100-100-100-1001".into());
                m.account_owner = Some(self.journal.owner.clone());
                self.journal.account_sid = m.account_sid.clone();
                self.persist()?;
                self.write_machine(&m)?;
            }
            1 => {
                if m.session == "teaching"
                    && command
                        .login
                        .as_ref()
                        .is_some_and(|p| p.confirm_session_end)
                {
                    m.session = "none".into();
                    m.observed_sid = None;
                    m.desktop_confirmed = false;
                    self.write_machine(&m)?;
                } else if !["none", "contest"].contains(&m.session.as_str()) {
                    return self.abort("precondition_failed");
                }
            }
            2 => {
                // Write-ahead intent permits recovery after a crash between the following writes.
                self.journal.teaching_sid = Some(m.teaching_sid.clone());
                self.journal.original_enabled = Some(m.teaching_enabled);
                self.journal.before_revision = Some(m.teaching_revision);
                self.persist()?;
                m.teaching_enabled = false;
                m.teaching_revision += 1;
                self.write_machine(&m)?;
            }
            3 => {
                self.permit = Some((
                    command.device_id.clone(),
                    command.instance_id.clone(),
                    command.command_id.clone(),
                    clock_ms + 5_000,
                ));
                self.journal.phase = "login_pending".into();
                self.journal.detail = "login_submitted".into();
            }
            4 => {
                let permit = self.permit.take().ok_or("login_permit_missing")?;
                if permit.0 != command.device_id
                    || permit.1 != command.instance_id
                    || permit.2 != command.command_id
                    || clock_ms >= permit.3
                {
                    return self.abort("login_failed");
                }
                if m.fault == "wrong_password" || m.fault == "account_unavailable" {
                    return self.abort("login_failed");
                }
                m.session = "contest".into();
                m.observed_sid = m.account_sid.clone();
                m.desktop_confirmed = m.fault != "no_desktop";
                self.write_machine(&m)?;
            }
            _ => {
                if m.observed_sid != m.account_sid
                    || m.account_sid.is_none()
                    || !m.desktop_confirmed
                    || m.session != "contest"
                {
                    return self.abort("login_failed");
                }
                self.journal.phase = "controlled".into();
                self.journal.detail = "desktop_observed".into();
            }
        }
        self.journal.step += 1;
        self.persist()?;
        Ok(self.state().login_complete())
    }
    fn abort(&mut self, detail: &str) -> Result<bool> {
        self.recover()?;
        self.journal.detail = detail.into();
        self.persist()?;
        Err("login_execution_failed".into())
    }
    pub fn recover(&mut self) -> Result<()> {
        self.permit = None;
        // No online authority survives process startup or this method, including completed login.
        if self.journal.phase == "teaching" && self.journal.original_enabled.is_none() {
            self.machine()?;
            return Ok(());
        }
        self.journal.phase = "recovering".into();
        self.persist()?;
        let outcome = self.restore_owned_field();
        if outcome.is_err() {
            self.journal.phase = "recovery_failed".into();
            self.journal.detail = "recovery_conflict".into();
            self.persist()?;
            return outcome;
        }
        self.journal.original_enabled = None;
        self.journal.before_revision = None;
        self.journal.teaching_sid = None;
        self.journal.phase = "teaching".into();
        self.journal.detail = "restored".into();
        self.persist()
    }
    fn restore_owned_field(&mut self) -> Result<()> {
        let mut m = self.machine()?;
        let Some(original) = self.journal.original_enabled else {
            return Ok(());
        };
        if m.fault == "recovery_failure" {
            return Err("login_recovery_failed".into());
        }
        if self.journal.teaching_sid.as_deref() != Some(&m.teaching_sid) {
            return Err("login_recovery_conflict".into());
        }
        let before = self
            .journal
            .before_revision
            .ok_or("login_record_mismatch")?;
        if m.teaching_revision == before && m.teaching_enabled == original {
            return Ok(());
        }
        // The second case handles a crash after restoration but before journal completion.
        if m.teaching_revision == before + 2 && m.teaching_enabled == original {
            return Ok(());
        }
        if m.teaching_revision != before + 1 || m.teaching_enabled {
            return Err("login_recovery_conflict".into());
        }
        m.teaching_enabled = original;
        m.teaching_revision += 1;
        self.write_machine(&m)
    }
}

pub trait Runtime {
    fn state(&self) -> AccountState;
    fn recover(&mut self) -> Result<()>;
    fn accept(&mut self, command: &Command) -> Result<()>;
    fn advance(&mut self, command: &Command, clock_ms: u64) -> Result<bool>;
}
impl Runtime for Simulator {
    fn state(&self) -> AccountState {
        Simulator::state(self)
    }
    fn recover(&mut self) -> Result<()> {
        Simulator::recover(self)
    }
    fn accept(&mut self, command: &Command) -> Result<()> {
        Simulator::accept(self, command)
    }
    fn advance(&mut self, command: &Command, clock_ms: u64) -> Result<bool> {
        Simulator::advance(self, command, clock_ms)
    }
}
impl Runtime for crate::login_native::Native {
    fn state(&self) -> AccountState {
        Self::state(self)
    }
    fn recover(&mut self) -> Result<()> {
        Self::recover(self)
    }
    fn accept(&mut self, command: &Command) -> Result<()> {
        Self::accept(self, command)
    }
    fn advance(&mut self, command: &Command, clock_ms: u64) -> Result<bool> {
        Self::advance(self, command, clock_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture {
        directory: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                directory: std::env::temp_dir().join(format!("wccs-login-{}", token())),
            }
        }
        fn open(&self) -> Simulator {
            Simulator::open(self.directory.clone(), "synthetic").unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
    fn command(s: &Simulator, sequence: i64) -> Command {
        serde_json::from_value(json!({"protocol_version":1,"command_id":format!("operation-{sequence}"),"request_id":"request","sequence":sequence,"device_id":"synthetic","generation":1,"device_version":1,"instance_id":"boot","session_key":"session","action":"prepare_login","expires_at":0,
            "identity":{"contest_id":"contest","contest_name":"合成比赛","contest_version":1,"participant_id":"person","name":"合成选手","school":"合成学校","seat_id":"seat","seat_label":"A1","room_id":"A","room_version":1},
            "login":{"connection_id":"connection","snapshot":s.state(),"confirm_session_end":false}})).unwrap()
    }
    fn finish(s: &mut Simulator, c: &Command) {
        s.accept(c).unwrap();
        for _ in 0..6 {
            s.advance(c, 100).unwrap();
        }
        assert!(s.state().login_complete());
    }
    #[test]
    fn new_authenticated_connection_can_restart_server_sequence() {
        let fixture = Fixture::new();
        let mut simulator = fixture.open();
        let previous = command(&simulator, 94);
        finish(&mut simulator, &previous);
        simulator.recover().unwrap();
        let mut next = command(&simulator, 1);
        assert!(simulator.accept(&next).is_err());
        next.instance_id = "new-host".into();
        next.login.as_mut().unwrap().connection_id = "new-connection".into();
        finish(&mut simulator, &next);
        simulator.recover().unwrap();
        let mut replay = next.clone();
        replay.command_id = "different-command-same-sequence".into();
        replay.login.as_mut().unwrap().snapshot = simulator.state();
        assert!(simulator.accept(&replay).is_err());
        assert!(simulator.advance(&previous, 102).is_err());
        assert!(simulator.state().teaching_enabled);
    }
    #[test]
    fn copied_owner_marker_cannot_replace_the_recorded_sid() {
        let fixture = Fixture::new();
        let mut simulator = fixture.open();
        let operation = command(&simulator, 1);
        finish(&mut simulator, &operation);
        simulator.recover().unwrap();
        let mut machine = simulator.machine().unwrap();
        machine.account_sid = Some("S-1-5-21-100-100-100-9999".into());
        simulator.write_machine(&machine).unwrap();
        assert_eq!(simulator.state().account, "conflict");
        assert!(simulator.accept(&command(&simulator, 2)).is_err());
    }

    #[test]
    fn offline_start_login_disconnect_retains_desktop_and_account() {
        let f = Fixture::new();
        let mut s = f.open();
        assert_eq!(s.state().phase, "teaching");
        let c = command(&s, 1);
        finish(&mut s, &c);
        let sid = s.state().target_sid;
        s.recover().unwrap();
        s.recover().unwrap();
        assert!(s.state().teaching_enabled);
        assert_eq!(s.state().target_sid, sid);
        assert!(s.state().desktop_confirmed);
        assert_eq!(s.state().session, "contest");
        assert!(s.advance(&c, 101).is_err());
        drop(s);
        let mut restarted = f.open();
        assert_eq!(restarted.state().phase, "teaching");
        assert!(restarted.advance(&c, 102).is_err());
        let next = command(&restarted, 2);
        finish(&mut restarted, &next);
    }
    #[test]
    fn restart_at_every_step_recovers_without_replaying_login() {
        for steps in 0..=6 {
            let f = Fixture::new();
            let mut s = f.open();
            let c = command(&s, 1);
            s.accept(&c).unwrap();
            for _ in 0..steps {
                s.advance(&c, 100).unwrap();
            }
            let desktop = s.state().desktop_confirmed;
            drop(s);
            let s = f.open();
            assert_eq!(s.state().phase, "teaching");
            assert!(s.state().teaching_enabled);
            assert_eq!(s.state().desktop_confirmed, desktop);
        }
    }
    #[test]
    fn write_ahead_and_compensation_crash_windows_are_idempotent() {
        for applied in [false, true] {
            let f = Fixture::new();
            let mut s = f.open();
            let mut m = s.machine().unwrap();
            s.journal.phase = "preparing".into();
            s.journal.teaching_sid = Some(m.teaching_sid.clone());
            s.journal.original_enabled = Some(m.teaching_enabled);
            s.journal.before_revision = Some(m.teaching_revision);
            s.persist().unwrap();
            if applied {
                m.teaching_enabled = false;
                m.teaching_revision += 1;
                s.write_machine(&m).unwrap();
            }
            drop(s);
            let mut s = f.open();
            assert!(s.state().teaching_enabled);
            s.recover().unwrap();
        }
        let f = Fixture::new();
        let mut s = f.open();
        let c = command(&s, 1);
        finish(&mut s, &c);
        s.restore_owned_field().unwrap();
        drop(s);
        assert_eq!(f.open().state().phase, "teaching");
    }
    #[test]
    fn foreign_accounts_tools_and_sessions_block_before_any_change() {
        for scenario in [
            "foreign", "admin", "disabled", "tools", "recovery", "teaching", "locked", "multiple",
            "rdp",
        ] {
            let f = Fixture::new();
            let mut s = f.open();
            let mut m = s.machine().unwrap();
            match scenario {
                "foreign" => {
                    m.account_sid = Some("foreign-sid".into());
                    m.account_owner = Some("foreign-owner".into());
                }
                "admin" => {
                    m.account_sid = Some("owned-sid".into());
                    m.account_owner = Some(s.journal.owner.clone());
                    m.account_admin = true;
                }
                "disabled" => {
                    m.account_sid = Some("owned-sid".into());
                    m.account_owner = Some(s.journal.owner.clone());
                    m.account_enabled = false;
                }
                "tools" => m.tools_ready = false,
                "recovery" => m.recovery_ready = false,
                session => m.session = session.into(),
            }
            s.write_machine(&m).unwrap();
            let c = command(&s, 1);
            assert!(s.accept(&c).is_err(), "{scenario}");
            assert!(s.state().teaching_enabled);
        }
    }
    #[test]
    fn teaching_session_requires_confirmation_and_is_not_resurrected() {
        let f = Fixture::new();
        let mut s = f.open();
        let mut m = s.machine().unwrap();
        m.session = "teaching".into();
        s.write_machine(&m).unwrap();
        let mut c = command(&s, 1);
        c.login.as_mut().unwrap().confirm_session_end = true;
        finish(&mut s, &c);
        s.recover().unwrap();
        assert_eq!(s.state().session, "contest");
    }
    #[test]
    fn wrong_password_account_failure_and_missing_desktop_recover() {
        for fault in ["wrong_password", "account_unavailable", "no_desktop"] {
            let f = Fixture::new();
            let mut s = f.open();
            let mut m = s.machine().unwrap();
            m.fault = fault.into();
            s.write_machine(&m).unwrap();
            let c = command(&s, 1);
            s.accept(&c).unwrap();
            let mut failed = false;
            for _ in 0..6 {
                if s.advance(&c, 100).is_err() {
                    failed = true;
                    break;
                }
            }
            assert!(failed);
            assert_eq!(s.state().phase, "teaching");
            assert!(s.state().teaching_enabled);
        }
    }
    #[test]
    fn permit_expiry_and_disconnect_during_submission_cannot_log_in_later() {
        for disconnect in [false, true] {
            let f = Fixture::new();
            let mut s = f.open();
            let c = command(&s, 1);
            s.accept(&c).unwrap();
            for _ in 0..4 {
                s.advance(&c, 100).unwrap();
            }
            if disconnect {
                s.recover().unwrap();
            }
            assert!(s.advance(&c, 5_100).is_err());
            assert!(s.state().teaching_enabled);
            assert!(!s.state().desktop_confirmed);
        }
    }
    #[test]
    fn recovery_failure_and_external_revision_changes_block_new_enable() {
        for conflict in [false, true] {
            let f = Fixture::new();
            let mut s = f.open();
            let c = command(&s, 1);
            finish(&mut s, &c);
            let mut m = s.machine().unwrap();
            if conflict {
                m.teaching_revision += 1;
            } else {
                m.fault = "recovery_failure".into();
            }
            s.write_machine(&m).unwrap();
            assert!(s.recover().is_err());
            assert_eq!(s.state().phase, "recovery_failed");
            assert!(s.accept(&command(&s, 2)).is_err());
            if !conflict {
                m.fault.clear();
                s.write_machine(&m).unwrap();
                s.recover().unwrap();
                assert!(s.state().teaching_enabled);
            }
        }
    }
    #[test]
    fn snapshot_binding_replay_lock_and_image_mismatch() {
        let f = Fixture::new();
        let mut s = f.open();
        let c = command(&s, 1);
        assert!(Simulator::open(f.directory.clone(), "synthetic").is_err());
        let mut m = s.machine().unwrap();
        m.tools_ready = false;
        s.write_machine(&m).unwrap();
        assert!(s.accept(&c).is_err());
        m.tools_ready = true;
        s.write_machine(&m).unwrap();
        finish(&mut s, &c);
        s.recover().unwrap();
        let mut changed = c.clone();
        changed.action = "restore_teaching".into();
        assert!(s.accept(&changed).is_err());
        m.incarnation = token();
        s.write_machine(&m).unwrap();
        assert!(s.recover().is_err());
        assert_eq!(s.state().detail, "record_mismatch");
        drop(s);
        assert!(Simulator::open(f.directory.clone(), "different-device").is_err());
    }
}
