//! SYSTEM-owned write-ahead recovery for the waiting user's Task Manager policy.
//! Registry handles are uniquely owned here. Values retain their exact type/bytes.
use crate::host::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use wccs_session::ipc::wide;
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{Registry::*, Threading::*},
    },
    core::{PCWSTR, w},
};

const SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\System";
const VALUE: PCWSTR = w!("DisableTaskMgr");

pub(crate) struct Privilege {
    token: crate::login_windows::Handle,
    previous: TOKEN_PRIVILEGES,
}
impl Privilege {
    pub(crate) fn enable(name: PCWSTR) -> Result<Self> {
        let mut handle = HANDLE::default();
        // SAFETY: borrowed current process, owned output token, one-entry privilege buffers.
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
                &mut handle,
            )?;
            let token = crate::login_windows::Handle(handle);
            let mut luid = LUID::default();
            LookupPrivilegeValueW(None, name, &mut luid)?;
            let requested = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut previous = TOKEN_PRIVILEGES::default();
            let mut returned = 0;
            AdjustTokenPrivileges(
                token.0,
                false,
                Some(&requested),
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                Some(&mut previous),
                Some(&mut returned),
            )?;
            if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
                return Err("waiting_hive_privilege_missing".into());
            }
            Ok(Self { token, previous })
        }
    }
}
impl Drop for Privilege {
    fn drop(&mut self) {
        // SAFETY: token and previous privileges remain owned until after restoration.
        unsafe {
            let _ = AdjustTokenPrivileges(self.token.0, false, Some(&self.previous), 0, None, None);
        }
    }
}
struct MountedHive {
    name: Vec<u16>,
    _backup: Privilege,
    _restore: Privilege,
}
struct Key(HKEY, Option<MountedHive>);
impl Key {
    fn finish(&mut self) -> Result<()> {
        // SAFETY: close children before root, then unmount only our dedicated recovery mount.
        unsafe {
            if !self.0.is_invalid() {
                RegCloseKey(self.0).ok()?;
                self.0 = HKEY::default();
            }
            if let Some(mount) = &self.1 {
                RegUnLoadKeyW(HKEY_USERS, PCWSTR(mount.name.as_ptr())).ok()?;
            }
        }
        self.1 = None;
        Ok(())
    }
}
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: each successful open/load/create transfers one handle to Key.
        let _ = self.finish();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Value {
    pub kind: u32,
    pub bytes: Vec<u8>,
}
fn applied() -> Option<Value> {
    Some(Value {
        kind: REG_DWORD.0,
        bytes: 1u32.to_le_bytes().to_vec(),
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    machine: String,
    sid: String,
    original: Option<Value>,
}

fn root(sid: &str) -> Result<Key> {
    if !sid.starts_with("S-1-5-21-")
        || !sid
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-' || b == b'S')
    {
        return Err("waiting_policy_sid_invalid".into());
    }
    let name = wide(sid);
    let mut key = HKEY::default();
    // SAFETY: terminated SID and writable output, HKEY_USERS is borrowed.
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_USERS,
            PCWSTR(name.as_ptr()),
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut key,
        )
    };
    if result == ERROR_SUCCESS {
        return Ok(Key(key, None));
    }
    if result != ERROR_FILE_NOT_FOUND {
        result.ok()?;
    }
    // An unloaded profile after reboot still needs restoration before next login.
    let profile = wide(format!(
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}"
    ));
    let mut path = [0u16; 32768];
    let mut size = std::mem::size_of_val(&path) as u32;
    // SAFETY: bounded UTF-16 output. RegGetValue expands ProfileImagePath.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(profile.as_ptr()),
            w!("ProfileImagePath"),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(path.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()?;
    }
    let path = String::from_utf16(
        &path[..path
            .iter()
            .position(|c| *c == 0)
            .ok_or("profile_path_invalid")?],
    )?;
    let path = Path::new(&path).join("NTUSER.DAT");
    if !path.is_file() {
        return Err("waiting_profile_hive_missing".into());
    }
    let path = wide(path);
    let mount = MountedHive {
        name: wide(format!("WCCS.WaitingRecovery.{sid}")),
        _backup: Privilege::enable(SE_BACKUP_NAME)?,
        _restore: Privilege::enable(SE_RESTORE_NAME)?,
    };
    // SAFETY: profile hive supports distinct per-key ACLs. Stable private mount
    // allows the next service instance to finish an interrupted recovery.
    unsafe {
        let result = RegOpenKeyExW(
            HKEY_USERS,
            PCWSTR(mount.name.as_ptr()),
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut key,
        );
        if result == ERROR_FILE_NOT_FOUND {
            RegLoadKeyW(
                HKEY_USERS,
                PCWSTR(mount.name.as_ptr()),
                PCWSTR(path.as_ptr()),
            )
            .ok()?;
            let mut root = Key(HKEY::default(), Some(mount));
            RegOpenKeyExW(
                HKEY_USERS,
                PCWSTR(root.1.as_ref().unwrap().name.as_ptr()),
                Some(0),
                KEY_READ | KEY_WRITE,
                &mut root.0,
            )
            .ok()?;
            return Ok(root);
        }
        result.ok()?;
    }
    Ok(Key(key, Some(mount)))
}

fn policy(root: &Key, create: bool) -> Result<Option<Key>> {
    let name = wide(SUBKEY);
    let mut key = HKEY::default();
    // SAFETY: live root, terminated subkey and writable output. Only this fixed key is accessed.
    let result = unsafe {
        if create {
            RegCreateKeyExW(
                root.0,
                PCWSTR(name.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(
                root.0,
                PCWSTR(name.as_ptr()),
                Some(0),
                KEY_QUERY_VALUE,
                &mut key,
            )
        }
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    result.ok()?;
    Ok(Some(Key(key, None)))
}

fn read(key: Option<&Key>) -> Result<Option<Value>> {
    read_named(key, VALUE)
}
fn read_named(key: Option<&Key>, name: PCWSTR) -> Result<Option<Value>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 4096;
    let mut bytes = vec![0u8; size as usize];
    // SAFETY: bounded raw output preserves arbitrary pre-existing registry types.
    let result = unsafe {
        RegQueryValueExW(
            key.0,
            name,
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    result.ok()?;
    bytes.truncate(size as usize);
    Ok(Some(Value {
        kind: kind.0,
        bytes,
    }))
}

fn write(key: &Key, value: &Option<Value>) -> Result<()> {
    write_named(key, VALUE, value)
}
fn write_named(key: &Key, name: PCWSTR, value: &Option<Value>) -> Result<()> {
    // SAFETY: owned writable key, bytes borrowed for synchronous registry call.
    unsafe {
        match value {
            Some(value) => RegSetValueExW(
                key.0,
                name,
                Some(0),
                REG_VALUE_TYPE(value.kind),
                Some(&value.bytes),
            )
            .ok()?,
            None => {
                let result = RegDeleteValueW(key.0, name);
                if result != ERROR_FILE_NOT_FOUND {
                    result.ok()?;
                }
            }
        }
        RegFlushKey(key.0).ok()?;
    }
    Ok(())
}

fn load(path: &Path) -> Result<Option<Journal>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let journal: Journal = serde_json::from_slice(&bytes)?;
    if journal.version != 1 || journal.machine != crate::login_windows::machine_guid()? {
        return Err("waiting_policy_machine_mismatch".into());
    }
    Ok(Some(journal))
}

pub fn apply(path: &Path, sid: &str) -> Result<()> {
    if load(path)?.is_some() {
        return Err("waiting_policy_recovery_pending".into());
    }
    let root = root(sid)?;
    let journal = Journal {
        version: 1,
        machine: crate::login_windows::machine_guid()?,
        sid: sid.into(),
        original: read(policy(&root, false)?.as_ref())?,
    };
    // The service's identity lock serializes all owners. Persist before first mutation.
    crate::local_state::persist(path, &serde_json::to_vec(&journal)?)
        .map_err(|_| "waiting_policy_persist_failed")?;
    let key = policy(&root, true)?.ok_or("waiting_policy_key_missing")?;
    write(&key, &applied())?;
    verify(path)
}

pub fn verify(path: &Path) -> Result<()> {
    let journal = load(path)?.ok_or("waiting_policy_journal_missing")?;
    let root = root(&journal.sid)?;
    if read(policy(&root, false)?.as_ref())? != applied() {
        return Err("waiting_policy_changed".into());
    }
    Ok(())
}

fn can_restore(current: &Option<Value>, original: &Option<Value>) -> bool {
    current == original || *current == applied()
}

pub fn recover(path: &Path) -> Result<()> {
    let Some(journal) = load(path)? else {
        return Ok(());
    };
    let mut root = root(&journal.sid)?;
    let key = policy(&root, false)?;
    let current = read(key.as_ref())?;
    if !can_restore(&current, &journal.original) {
        return Err("waiting_policy_restore_conflict".into());
    }
    if current != journal.original {
        let writable = policy(&root, true)?.ok_or("waiting_policy_key_missing")?;
        write(&writable, &journal.original)?;
    }
    if read(key.as_ref())? != journal.original {
        return Err("waiting_policy_restore_unconfirmed".into());
    }
    drop(key);
    root.finish()?;
    fs::remove_file(path)?;
    Ok(())
}

fn machine_key(subkey: &str, create: bool) -> Result<Option<Key>> {
    let name = wide(subkey);
    let mut key = HKEY::default();
    // SAFETY: fixed caller-owned subkey strings, borrowed predefined root, owned output key.
    let status = unsafe {
        if create {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(name.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(name.as_ptr()),
                Some(0),
                KEY_QUERY_VALUE,
                &mut key,
            )
        }
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    status.ok()?;
    Ok(Some(Key(key, None)))
}
pub(crate) fn machine_read(subkey: &str, name: &str) -> Result<Option<Value>> {
    let name = wide(name);
    read_named(machine_key(subkey, false)?.as_ref(), PCWSTR(name.as_ptr()))
}
pub(crate) fn machine_write(subkey: &str, name: &str, value: &Option<Value>) -> Result<()> {
    let name = wide(name);
    let key = machine_key(subkey, true)?.ok_or("machine_policy_key_missing")?;
    write_named(&key, PCWSTR(name.as_ptr()), value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privilege_guard_supplies_previous_state_and_length_buffers() {
        // Traverse privilege is present in ordinary interactive tokens as well as SYSTEM.
        let privilege = Privilege::enable(SE_CHANGE_NOTIFY_NAME).unwrap();
        drop(privilege);
    }

    #[test]
    fn restoration_accepts_partial_apply_and_repeated_restore_but_preserves_external_changes() {
        for original in [
            None,
            applied(),
            Some(Value {
                kind: REG_BINARY.0,
                bytes: vec![0, 7, 255],
            }),
        ] {
            assert!(can_restore(&original, &original));
            assert!(can_restore(&applied(), &original));
            assert!(!can_restore(
                &Some(Value {
                    kind: REG_SZ.0,
                    bytes: vec![9]
                }),
                &original
            ));
        }
    }
}
