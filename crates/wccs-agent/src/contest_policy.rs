//! Write-ahead machine policy recovery. Invoked only by the native SYSTEM login adapter.
use crate::{host::Result, login_windows, waiting_policy};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use wccs_protocol::login::{PolicyState, Restrictions};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authentication::Identity::*, Authorization::*, *},
    },
    core::{PCWSTR, PWSTR},
};

const STORAGE_KEY: &str = r"SOFTWARE\Policies\Microsoft\Windows\RemovableStorageDevices";
const STORAGE_VALUE: &str = "Deny_All";
const SHUTDOWN_RIGHTS: [&str; 2] = ["SeShutdownPrivilege", "SeRemoteShutdownPrivilege"];

struct Policy(LSA_HANDLE);
impl Drop for Policy {
    fn drop(&mut self) {
        // SAFETY: this object exclusively owns the handle returned by LsaOpenPolicy.
        unsafe {
            let _ = LsaClose(self.0);
        }
    }
}
struct LsaMemory(*mut std::ffi::c_void);
impl Drop for LsaMemory {
    fn drop(&mut self) {
        // SAFETY: only allocations returned by LsaEnumerateAccountsWithUserRight enter this guard.
        unsafe {
            let _ = LsaFreeMemory(Some(self.0));
        }
    }
}
fn check(status: NTSTATUS) -> Result<()> {
    if status.0 != 0 {
        // SAFETY: conversion of an NTSTATUS value does not access caller memory.
        return Err(std::io::Error::from_raw_os_error(
            unsafe { LsaNtStatusToWinError(status) } as i32
        )
        .into());
    }
    Ok(())
}
impl Policy {
    fn open(write: bool) -> Result<Self> {
        let mut handle = LSA_HANDLE::default();
        let attributes = LSA_OBJECT_ATTRIBUTES {
            Length: size_of::<LSA_OBJECT_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        let access = POLICY_LOOKUP_NAMES
            | POLICY_VIEW_LOCAL_INFORMATION
            | if write { POLICY_CREATE_ACCOUNT } else { 0 };
        // SAFETY: local policy, initialized attributes, uniquely owned output handle.
        check(unsafe { LsaOpenPolicy(None, &attributes, access as u32, &mut handle) })?;
        Ok(Self(handle))
    }
    fn holders(&self, right: &str) -> Result<Vec<String>> {
        let mut text = wccs_session::ipc::wide(right);
        let name = unicode(&mut text);
        let mut buffer = std::ptr::null_mut();
        let mut count = 0;
        // SAFETY: live policy/name and writable output slots. Allocation is guarded immediately.
        let status = unsafe {
            LsaEnumerateAccountsWithUserRight(self.0, Some(&name), &mut buffer, &mut count)
        };
        // SAFETY: scalar NTSTATUS conversion.
        if unsafe { LsaNtStatusToWinError(status) } == ERROR_NO_MORE_ITEMS.0 {
            return Ok(vec![]);
        }
        check(status)?;
        let _memory = LsaMemory(buffer);
        if count > 4096 || (count > 0 && buffer.is_null()) {
            return Err("policy_holders_invalid".into());
        }
        let mut result = Vec::new();
        if count > 0 {
            // SAFETY: LSA returns count initialized entries valid until _memory is dropped.
            for item in unsafe {
                std::slice::from_raw_parts(
                    buffer.cast::<LSA_ENUMERATION_INFORMATION>(),
                    count as usize,
                )
            } {
                result.push(login_windows::sid_text(item.Sid)?);
            }
        }
        result.sort();
        Ok(result)
    }
    fn change(&self, sid: &str, right: &str, add: bool) -> Result<()> {
        let text = wccs_session::ipc::wide(sid);
        let mut account = PSID::default();
        let mut right_text = wccs_session::ipc::wide(right);
        let rights = [unicode(&mut right_text)];
        // SAFETY: validated SID conversion allocates a SID, both LSA calls synchronously borrow it.
        // LocalFree runs before propagating the LSA status, including its failure path.
        unsafe {
            ConvertStringSidToSidW(PCWSTR(text.as_ptr()), &mut account)?;
            let status = if add {
                LsaAddAccountRights(self.0, account, &rights)
            } else {
                LsaRemoveAccountRights(self.0, account, false, Some(&rights))
            };
            let _ = LocalFree(Some(HLOCAL(account.0)));
            check(status)
        }
    }
}
fn unicode(text: &mut [u16]) -> LSA_UNICODE_STRING {
    LSA_UNICODE_STRING {
        Length: ((text.len() - 1) * 2) as u16,
        MaximumLength: (text.len() * 2) as u16,
        Buffer: PWSTR(text.as_mut_ptr()),
    }
}
fn retained(sid: &str) -> bool {
    ["S-1-5-18", "S-1-5-32-544"].contains(&sid)
}
fn blocked_storage() -> Option<waiting_policy::Value> {
    Some(waiting_policy::Value {
        kind: 4,
        bytes: 1u32.to_le_bytes().to_vec(),
    })
}

#[derive(Deserialize, Serialize)]
struct Grant {
    right: String,
    holders: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    machine: String,
    requested: Restrictions,
    storage: Option<waiting_policy::Value>,
    rights: Vec<Grant>,
}
fn load(path: &Path) -> Result<Option<Journal>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let journal: Journal = serde_json::from_slice(&bytes)?;
    if journal.version != 1
        || journal.machine != login_windows::machine_guid()?
        || journal.rights.len()
            != if journal.requested.block_shutdown {
                2
            } else {
                0
            }
        || (journal.requested.block_shutdown
            && SHUTDOWN_RIGHTS.iter().any(|right| {
                journal
                    .rights
                    .iter()
                    .filter(|grant| grant.right == *right)
                    .count()
                    != 1
            }))
        || journal
            .rights
            .iter()
            .any(|g| !SHUTDOWN_RIGHTS.contains(&g.right.as_str()) || g.holders.len() > 4096)
    {
        return Err("contest_policy_record_mismatch".into());
    }
    Ok(Some(journal))
}
pub fn apply(path: &Path, requested: &Restrictions) -> Result<()> {
    if !requested.enabled() {
        return Ok(());
    }
    if load(path)?.is_some() {
        return Err("contest_policy_recovery_pending".into());
    }
    let policy = Policy::open(true)?;
    let mut rights = Vec::new();
    if requested.block_shutdown {
        for right in SHUTDOWN_RIGHTS {
            rights.push(Grant {
                right: right.into(),
                holders: policy.holders(right)?,
            });
        }
    }
    let journal = Journal {
        version: 1,
        machine: login_windows::machine_guid()?,
        requested: requested.clone(),
        storage: waiting_policy::machine_read(STORAGE_KEY, STORAGE_VALUE)?,
        rights,
    };
    crate::local_state::persist(path, &serde_json::to_vec(&journal)?)
        .map_err(|_| "contest_policy_persist_failed")?;
    if requested.block_removable_storage {
        waiting_policy::machine_write(STORAGE_KEY, STORAGE_VALUE, &blocked_storage())?;
    }
    for grant in &journal.rights {
        for sid in grant.holders.iter().filter(|sid| !retained(sid)) {
            policy.change(sid, &grant.right, false)?;
        }
    }
    verify(path)
}
pub fn verify(path: &Path) -> Result<()> {
    let Some(journal) = load(path)? else {
        return Ok(());
    };
    if journal.requested.block_removable_storage
        && waiting_policy::machine_read(STORAGE_KEY, STORAGE_VALUE)? != blocked_storage()
    {
        return Err("contest_storage_policy_changed".into());
    }
    if journal.requested.block_shutdown {
        let policy = Policy::open(false)?;
        for right in SHUTDOWN_RIGHTS {
            if policy.holders(right)?.iter().any(|sid| !retained(sid)) {
                return Err("contest_shutdown_policy_changed".into());
            }
        }
    }
    Ok(())
}
pub fn recover(path: &Path) -> Result<()> {
    let Some(journal) = load(path)? else {
        return Ok(());
    };
    // Restore the original grants even when storage restoration needs staff review.
    let policy = Policy::open(true)?;
    for grant in &journal.rights {
        let current = policy.holders(&grant.right)?;
        for sid in grant.holders.iter().filter(|sid| !current.contains(sid)) {
            policy.change(sid, &grant.right, true)?;
        }
    }
    if journal.requested.block_removable_storage {
        let current = waiting_policy::machine_read(STORAGE_KEY, STORAGE_VALUE)?;
        if current != journal.storage && current != blocked_storage() {
            return Err("contest_policy_restore_conflict".into());
        }
        waiting_policy::machine_write(STORAGE_KEY, STORAGE_VALUE, &journal.storage)?;
        if waiting_policy::machine_read(STORAGE_KEY, STORAGE_VALUE)? != journal.storage {
            return Err("contest_policy_restore_unconfirmed".into());
        }
    }
    fs::remove_file(path)?;
    Ok(())
}
pub fn state(path: &Path) -> PolicyState {
    match load(path) {
        Ok(None) => PolicyState {
            requested: Restrictions::default(),
            state: "ready".into(),
        },
        Ok(Some(journal)) => PolicyState {
            requested: journal.requested,
            state: if verify(path).is_ok() {
                "applied"
            } else {
                "recovery_failed"
            }
            .into(),
        },
        Err(_) => PolicyState {
            requested: Restrictions::default(),
            state: "recovery_failed".into(),
        },
    }
}
