use super::{Buffer, observation, text};
use crate::model::{Account, Observation};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::ConvertSidToStringSidW, *},
        System::{Threading::*, WindowsProgramming::GetUserNameW},
    },
    core::{PWSTR, Result as WinResult},
};

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns a valid OpenProcessToken handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn collect() -> Account {
    account_from_token(open_token(), observation(user_name()))
}

fn account_from_token(token: WinResult<Token>, user_name: Observation<String>) -> Account {
    match token {
        Ok(token) => Account {
            user_name,
            sid: observation(sid(&token)),
            elevated: observation(elevation(&token)),
            elevation_type: observation(elevation_type(&token)),
            administrators_group: observation(administrators_group(&token)),
            session_id: observation(session_id(&token)),
        },
        Err(error) => Account {
            user_name,
            sid: observation(Err(error.clone())),
            elevated: observation(Err(error.clone())),
            elevation_type: observation(Err(error.clone())),
            administrators_group: observation(Err(error.clone())),
            session_id: observation(Err(error)),
        },
    }
}

fn open_token() -> WinResult<Token> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle. TOKEN_QUERY
    // requests read access only, and ownership of the returned handle goes to Token.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    Ok(Token(token))
}

fn user_name() -> WinResult<String> {
    let mut buffer = [0u16; 257];
    let mut size = buffer.len() as u32;
    // SAFETY: buffer length includes the terminator and covers UNLEN + 1.
    unsafe { GetUserNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size)? };
    Ok(text(&buffer))
}

fn token_buffer(token: &Token, class: TOKEN_INFORMATION_CLASS) -> WinResult<Buffer> {
    let mut size = 0;
    // SAFETY: token is a live owned handle. Null output with zero size requests capacity.
    let result = unsafe { GetTokenInformation(token.0, class, None, 0, &mut size) };
    if let Err(error) = result
        && error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult()
    {
        return Err(error);
    }
    for _ in 0..3 {
        let mut buffer = Buffer::new(size)?;
        // SAFETY: buffer is aligned and has at least size writable bytes. All
        // pointers returned by TokenUser/TokenGroups remain within the owned buffer.
        let result = unsafe {
            GetTokenInformation(token.0, class, Some(buffer.as_mut_ptr()), size, &mut size)
        };
        match result {
            Ok(()) => return Ok(buffer),
            Err(error) if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
            Err(error) => return Err(error),
        }
    }
    Err(ERROR_INSUFFICIENT_BUFFER.into())
}

fn sid(token: &Token) -> WinResult<String> {
    let mut buffer = token_buffer(token, TokenUser)?;
    let mut sid_string = PWSTR::null();
    // SAFETY: TokenUser initialized a TOKEN_USER at the aligned buffer start.
    // Its SID remains valid throughout conversion. Conversion allocates with LocalAlloc.
    unsafe {
        let user = &*buffer.as_mut_ptr().cast::<TOKEN_USER>();
        ConvertSidToStringSidW(user.User.Sid, &mut sid_string)?;
        let value = sid_string.to_string();
        // Release the allocated string even when UTF-16 conversion fails.
        LocalFree(Some(HLOCAL(sid_string.0.cast())));
        value.map_err(|_| ERROR_INVALID_DATA.into())
    }
}

fn elevation(token: &Token) -> WinResult<bool> {
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: output points to a writable TOKEN_ELEVATION of the specified size.
    // Some Windows builds reject a zero-length sizing query with ERROR_BAD_LENGTH,
    // so this fixed-size information class is queried directly.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )?
    };
    Ok(elevation.TokenIsElevated != 0)
}

fn elevation_type(token: &Token) -> WinResult<String> {
    let mut buffer = token_buffer(token, TokenElevationType)?;
    // SAFETY: successful TokenElevationType query initialized this exact structure.
    let value = unsafe { *buffer.as_mut_ptr().cast::<TOKEN_ELEVATION_TYPE>() };
    Ok(match value.0 {
        1 => "default",
        2 => "full",
        3 => "limited",
        _ => "unknown",
    }
    .into())
}

fn session_id(token: &Token) -> WinResult<u32> {
    let mut buffer = token_buffer(token, TokenSessionId)?;
    // SAFETY: successful TokenSessionId query initialized a DWORD.
    Ok(unsafe { *buffer.as_mut_ptr().cast::<u32>() })
}

fn administrators_group(token: &Token) -> WinResult<String> {
    let mut buffer = token_buffer(token, TokenGroups)?;
    // SAFETY: TokenGroups supplies the group count followed by that many aligned
    // SID_AND_ATTRIBUTES records. Every SID is borrowed from the live buffer.
    unsafe {
        let groups = &*buffer.as_mut_ptr().cast::<TOKEN_GROUPS>();
        let entries =
            std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize);
        for entry in entries {
            if IsWellKnownSid(entry.Sid, WinBuiltinAdministratorsSid).as_bool() {
                // SE_GROUP_USE_FOR_DENY_ONLY (0x10) and SE_GROUP_ENABLED (0x4).
                return Ok(if entry.Attributes & 0x10 != 0 {
                    "deny_only"
                } else if entry.Attributes & 4 != 0 {
                    "enabled"
                } else {
                    "disabled"
                }
                .into());
            }
        }
    }
    Ok("absent".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_fixed_size_elevation_query_succeeds() {
        let token = open_token().expect("current process token must be queryable");
        elevation(&token).expect("fixed-size query must avoid ERROR_BAD_LENGTH");
    }

    #[test]
    fn denied_token_keeps_name_and_marks_every_permission_unknown() {
        let account = account_from_token(
            Err(ERROR_ACCESS_DENIED.into()),
            Observation::available("synthetic-user".into()),
        );
        assert_eq!(account.user_name.value().unwrap(), "synthetic-user");
        let json = serde_json::to_value(account).unwrap();
        for name in [
            "sid",
            "elevated",
            "elevation_type",
            "administrators_group",
            "session_id",
        ] {
            assert_eq!(json[name]["status"], "unavailable");
            assert_eq!(json[name]["reason"], "permission_denied");
            assert_eq!(json[name]["code"], 5);
        }
    }
}
