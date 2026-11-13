//! Windows account and credential primitives. All outputs are credential-free.
use crate::host::Result;
use std::{fs, path::Path};
use wccs_session::ipc::wide;
use windows::{
    Win32::{
        Foundation::*,
        NetworkManagement::NetManagement::*,
        Security::{Authorization::*, Cryptography::*, *},
        System::{Registry::*, RemoteDesktop::*},
    },
    core::{PCWSTR, PWSTR},
};

pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: exclusively owned non-pseudo kernel handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct NetBuffer(*mut u8);
impl Drop for NetBuffer {
    fn drop(&mut self) {
        // SAFETY: NetUser APIs allocate this buffer, and it is freed exactly once.
        unsafe {
            NetApiBufferFree(Some(self.0.cast()));
        }
    }
}
pub struct Secret(pub Vec<u16>);
impl Drop for Secret {
    fn drop(&mut self) {
        for item in &mut self.0 {
            // SAFETY: unique writable elements. Volatile stores prevent dead-store removal.
            unsafe {
                std::ptr::write_volatile(item, 0);
            }
        }
    }
}
pub fn machine_guid() -> Result<String> {
    let mut data = [0u16; 128];
    let mut bytes = std::mem::size_of_val(&data) as u32;
    // SAFETY: bounded UTF-16 registry query, no handle is retained.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::w!("SOFTWARE\\Microsoft\\Cryptography"),
            windows::core::w!("MachineGuid"),
            RRF_RT_REG_SZ,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()?;
    }
    Ok(String::from_utf16(
        &data[..data
            .iter()
            .position(|c| *c == 0)
            .ok_or("machine_guid_invalid")?],
    )?)
}
pub fn sid_text(sid: PSID) -> Result<String> {
    let mut text = PWSTR::null();
    // SAFETY: callers supply a SID backed by a live API-owned token or user buffer.
    unsafe {
        ConvertSidToStringSidW(sid, &mut text)?;
        let result = text.to_string();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        Ok(result?)
    }
}
pub fn token_sid(token: HANDLE) -> Result<String> {
    let mut size = 0;
    // SAFETY: first call queries size, second uses pointer-aligned owned storage.
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
        if size == 0 {
            return Err("token_user_unavailable".into());
        }
        let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token,
            TokenUser,
            Some(data.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
        sid_text((&*data.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}
pub struct User {
    pub sid: String,
    pub comment: String,
    pub flags: u32,
}
pub fn user(name: &str) -> Result<Option<User>> {
    let name = wide(name);
    let mut buffer = std::ptr::null_mut();
    // SAFETY: level 4 determines the returned layout. The buffer stays live while copying.
    unsafe {
        let status = NetUserGetInfo(PCWSTR::null(), PCWSTR(name.as_ptr()), 4, &mut buffer);
        if status == 2221 {
            return Ok(None);
        } // NERR_UserNotFound
        if status != 0 {
            return Err("account_query_failed".into());
        }
        let buffer = NetBuffer(buffer);
        let info = &*buffer.0.cast::<USER_INFO_4>();
        Ok(Some(User {
            sid: sid_text(info.usri4_user_sid)?,
            comment: if info.usri4_comment.is_null() {
                String::new()
            } else {
                info.usri4_comment.to_string()?
            },
            flags: info.usri4_flags.0,
        }))
    }
}
pub fn create_user(name: &str, owner: &str, password: &Secret) -> Result<User> {
    if user(name)?.is_some() {
        return Err("account_already_exists".into());
    }
    let mut name = wide(name);
    let mut comment = wide(owner);
    let info = USER_INFO_1 {
        usri1_name: PWSTR(name.as_mut_ptr()),
        usri1_password: PWSTR(password.0.as_ptr().cast_mut()),
        usri1_priv: USER_PRIV_USER,
        usri1_comment: PWSTR(comment.as_mut_ptr()),
        usri1_flags: USER_ACCOUNT_FLAGS(UF_NORMAL_ACCOUNT) | UF_SCRIPT | UF_DONT_EXPIRE_PASSWD,
        ..Default::default()
    };
    // SAFETY: terminated buffers outlive the call. NetUserAdd copies the password into SAM.
    unsafe {
        if NetUserAdd(
            PCWSTR::null(),
            1,
            (&info as *const USER_INFO_1).cast(),
            None,
        ) != 0
        {
            return Err("account_create_failed".into());
        }
    }
    let account_name = String::from_utf16(&name[..name.len() - 1])?;
    user(&account_name)?.ok_or_else(|| "account_create_unconfirmed".into())
}
pub fn set_flags(name: &str, sid: &str, expected: u32, desired: u32) -> Result<()> {
    let current = user(name)?.ok_or("account_missing")?;
    if current.sid != sid || current.flags != expected {
        return Err("account_external_change".into());
    }
    let name = wide(name);
    let info = USER_INFO_1008 {
        usri1008_flags: USER_ACCOUNT_FLAGS(desired),
    };
    // SAFETY: level 1008 accepts only flags, preserving all other account fields.
    unsafe {
        if NetUserSetInfo(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            1008,
            (&info as *const USER_INFO_1008).cast(),
            None,
        ) != 0
        {
            return Err("account_flags_failed".into());
        }
    }
    Ok(())
}
pub fn ensure_users_member(account_sid: &str) -> Result<()> {
    let text = wide(account_sid);
    let mut account = PSID::default();
    let mut group = [0usize; 16];
    let mut size = std::mem::size_of_val(&group) as u32;
    let mut name = [0u16; 256];
    let mut name_size = name.len() as u32;
    let mut domain = [0u16; 256];
    let mut domain_size = domain.len() as u32;
    let mut kind = SID_NAME_USE::default();
    // SAFETY: aligned SID storage and bounded name buffers remain live throughout the calls.
    // The localized Users group is resolved from its well-known SID, never from guessed text.
    unsafe {
        let group_sid = PSID(group.as_mut_ptr().cast());
        CreateWellKnownSid(WinBuiltinUsersSid, None, Some(group_sid), &mut size)?;
        LookupAccountSidW(
            PCWSTR::null(),
            group_sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_size,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_size,
            &mut kind,
        )?;
        ConvertStringSidToSidW(PCWSTR(text.as_ptr()), &mut account)?;
        let member = LOCALGROUP_MEMBERS_INFO_0 {
            lgrmi0_sid: account,
        };
        let status = NetLocalGroupAddMembers(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            0,
            (&member as *const LOCALGROUP_MEMBERS_INFO_0).cast(),
            1,
        );
        let _ = LocalFree(Some(HLOCAL(account.0)));
        if status != 0 && status != 1378 {
            return Err("contest_users_membership_failed".into());
        }
    }
    Ok(())
}
pub fn authenticate(name: &str, password: &Secret) -> Result<Handle> {
    let name = wide(name);
    let mut handle = HANDLE::default();
    // SAFETY: terminated local credentials; the returned token is owned and never logged.
    unsafe {
        LogonUserW(
            PCWSTR(name.as_ptr()),
            windows::core::w!("."),
            PCWSTR(password.0.as_ptr()),
            LOGON32_LOGON_INTERACTIVE,
            LOGON32_PROVIDER_DEFAULT,
            &mut handle,
        )?;
    }
    Ok(Handle(handle))
}
pub fn administrator(token: HANDLE) -> Result<bool> {
    let mut sid = [0usize; (SECURITY_MAX_SID_SIZE as usize).div_ceil(std::mem::size_of::<usize>())];
    let mut size = std::mem::size_of_val(&sid) as u32;
    let mut member = windows::core::BOOL::default();
    // SAFETY: fixed aligned buffers and borrowed input token. A UAC-limited token's
    // linked handle is owned here and closed after the membership check. This checks
    // the account's administrative ability, including a filtered administrator.
    unsafe {
        let mut elevation = TOKEN_ELEVATION_TYPE::default();
        let mut returned = 0;
        GetTokenInformation(
            token,
            TokenElevationType,
            Some((&mut elevation as *mut TOKEN_ELEVATION_TYPE).cast()),
            std::mem::size_of_val(&elevation) as u32,
            &mut returned,
        )?;
        let linked = if elevation == TokenElevationTypeLimited {
            let mut info = TOKEN_LINKED_TOKEN::default();
            GetTokenInformation(
                token,
                TokenLinkedToken,
                Some((&mut info as *mut TOKEN_LINKED_TOKEN).cast()),
                std::mem::size_of_val(&info) as u32,
                &mut returned,
            )?;
            Some(Handle(info.LinkedToken))
        } else {
            None
        };
        let membership_token = linked.as_ref().map_or(token, |handle| handle.0);
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            None,
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut size,
        )?;
        let mut duplicate = HANDLE::default();
        DuplicateToken(membership_token, SecurityIdentification, &mut duplicate)?;
        let duplicate = Handle(duplicate);
        CheckTokenMembership(
            Some(duplicate.0),
            PSID(sid.as_mut_ptr().cast()),
            &mut member,
        )?;
    }
    Ok(member.as_bool())
}
pub fn protect(path: &Path, secret: &Secret) -> Result<()> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: (secret.0.len() * 2) as u32,
        pbData: secret.0.as_ptr().cast::<u8>().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input is borrowed, DPAPI allocates output using LocalAlloc. Machine scope is protected by the deployment DACL.
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )?;
        let result = crate::windows_agent::persist(
            path,
            std::slice::from_raw_parts(output.pbData, output.cbData as usize),
        )
        .map_err(|_| "secret_persist_failed");
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        result?;
    }
    Ok(())
}
pub fn unprotect(path: &Path) -> Result<Secret> {
    let mut bytes = fs::read(path)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: byte buffer lives for the call; output is copied then explicitly wiped before LocalFree.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )?;
        if output.pbData.is_null() {
            return Err("secret_format_invalid".into());
        }
        let raw = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = if raw.len() <= 256 && raw.len().is_multiple_of(2) {
            Some(Secret(
                raw.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect(),
            ))
        } else {
            None
        };
        for item in raw {
            std::ptr::write_volatile(item, 0);
        }
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        let secret = result.ok_or("secret_format_invalid")?;
        if secret.0.last() != Some(&0) {
            return Err("secret_format_invalid".into());
        }
        Ok(secret)
    }
}
struct Revert;
impl Drop for Revert {
    fn drop(&mut self) {
        // SAFETY: paired with successful impersonation on this same synchronous thread.
        unsafe {
            if RevertToSelf().is_err() {
                std::process::abort();
            }
        }
    }
}
pub fn as_user<T>(token: HANDLE, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    // SAFETY: token remains live, operation is synchronous and guard always reverts on this thread.
    unsafe {
        ImpersonateLoggedOnUser(token)?;
    }
    let _revert = Revert;
    operation()
}
pub fn check_access(token: HANDLE, tools: &[String], work: &Path) -> Result<()> {
    if tools.is_empty() || administrator(token)? {
        return Err("ordinary_account_required".into());
    }
    // SAFETY: borrowed token remains live; guard reverts before any async yield or privileged operation.
    unsafe {
        ImpersonateLoggedOnUser(token)?;
    }
    let _revert = Revert;
    for tool in tools {
        use std::os::windows::fs::OpenOptionsExt;
        // Request both read and execute access under the ordinary account token.
        fs::OpenOptions::new()
            .access_mode(0x80000000 | 0x20000000)
            .open(tool)?;
    }
    let path = work.join(format!("wccs-access-{}", wccs_protocol::token()));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    drop(file);
    fs::remove_file(path)?;
    Ok(())
}
pub struct Console {
    pub kind: String,
    pub id: Option<u32>,
    pub sid: Option<String>,
}
pub fn console(contest_sid: Option<&str>, teaching_sid: &str) -> Result<Console> {
    let mut pointer = std::ptr::null_mut();
    let mut count = 0;
    // SAFETY: WTS-owned arrays and strings are copied and freed on all paths before returning.
    unsafe {
        WTSEnumerateSessionsW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            0,
            1,
            &mut pointer,
            &mut count,
        )?;
        let rows = if count == 0 {
            &[]
        } else {
            if pointer.is_null() {
                return Err("session_inventory_invalid".into());
            }
            std::slice::from_raw_parts(pointer, count as usize)
        };
        let mut sessions = Vec::new();
        let mut query_failed = false;
        for row in rows {
            let mut user = PWSTR::null();
            let mut bytes = 0;
            let query = WTSQuerySessionInformationW(
                Some(WTS_CURRENT_SERVER_HANDLE),
                row.SessionId,
                WTSUserName,
                &mut user,
                &mut bytes,
            );
            query_failed |= query.is_err();
            let present = query.is_ok() && bytes > 2;
            if !user.is_null() {
                WTSFreeMemory(user.0.cast());
            }
            if present {
                sessions.push(row.SessionId);
            }
        }
        if !pointer.is_null() {
            WTSFreeMemory(pointer.cast());
        }
        if query_failed {
            return Err("session_inventory_failed".into());
        }
        if sessions.is_empty() {
            return Ok(Console {
                kind: "none".into(),
                id: None,
                sid: None,
            });
        }
        if sessions.len() != 1 {
            return Ok(Console {
                kind: "multiple".into(),
                id: None,
                sid: None,
            });
        }
        let id = sessions[0];
        if id != WTSGetActiveConsoleSessionId() {
            return Ok(Console {
                kind: "rdp".into(),
                id: Some(id),
                sid: None,
            });
        }
        let mut token = HANDLE::default();
        WTSQueryUserToken(id, &mut token)?;
        let token = Handle(token);
        let sid = token_sid(token.0)?;
        let kind = if wccs_session::ipc::discover(false).is_err() {
            "locked"
        } else if Some(sid.as_str()) == contest_sid {
            "contest"
        } else if sid == teaching_sid {
            "teaching"
        } else {
            "unknown"
        };
        Ok(Console {
            kind: kind.into(),
            id: Some(id),
            sid: Some(sid),
        })
    }
}
pub fn logoff(id: u32) -> Result<()> {
    // SAFETY: caller has just rechecked the exact console SID and explicit session-end confirmation.
    unsafe {
        WTSLogoffSession(Some(WTS_CURRENT_SERVER_HANDLE), id, false)?;
    }
    Ok(())
}
