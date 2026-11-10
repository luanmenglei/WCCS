//! Named-pipe and process handles are owned here. Every peer is checked against
//! the exact live process handle, preventing PID reuse and replacement clients.
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::Path,
    time::{Duration, Instant},
};
use wccs_protocol::control::MAX_MESSAGE;
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Environment::*, Pipes::*, RemoteDesktop::*, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle exclusively owns a valid non-pseudo kernel handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Descriptor {
    fn new(sddl: &str) -> Result<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let text = wide(sddl);
        // SAFETY: terminated input and writable output. LocalFree owns the allocation.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )?;
        }
        Ok(Self(sd))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: false.into(),
        }
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSecurityDescriptor allocated this block exactly once.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

pub fn process_session() -> Result<u32> {
    let mut id = 0;
    // SAFETY: current PID is valid and id is writable.
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut id)?;
    }
    Ok(id)
}

fn token_information(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<usize>> {
    let mut size = 0;
    // SAFETY: size query with null buffer followed by pointer-aligned storage.
    unsafe {
        let _ = GetTokenInformation(token, class, None, 0, &mut size);
    }
    if size == 0 {
        return Err("token_query_failed".into());
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: buffer has at least size bytes and correct alignment for TOKEN structs.
    unsafe {
        GetTokenInformation(
            token,
            class,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
    }
    Ok(buffer)
}

fn user_sid(token: HANDLE) -> Result<String> {
    let data = token_information(token, TokenUser)?;
    let mut text = PWSTR::null();
    // SAFETY: successful TokenUser data contains an in-buffer SID for the call.
    unsafe {
        let user = &*(data.as_ptr().cast::<TOKEN_USER>());
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let value = text.to_string()?;
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        Ok(value)
    }
}

fn current_token() -> Result<Handle> {
    let mut token = HANDLE::default();
    // SAFETY: current process pseudo handle is borrowed, output token is owned.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
    }
    Ok(Handle(token))
}

pub fn is_system() -> Result<bool> {
    Ok(user_sid(current_token()?.0)? == "S-1-5-18")
}

pub struct Session {
    pub key: String,
    token: Handle,
    sid: String,
}

pub fn discover(test_mode: bool) -> Result<Session> {
    let current = process_session()?;
    let (id, token) = if test_mode {
        if current == 0 {
            return Err("no_interactive_session".into());
        }
        (current, current_token()?)
    } else {
        if !is_system()? {
            return Err("service_requires_local_system".into());
        }
        // SAFETY: WTS returns an owned array; copied IDs are used after freeing it.
        let id = unsafe {
            let mut pointer = std::ptr::null_mut();
            let mut count = 0;
            WTSEnumerateSessionsW(
                Some(WTS_CURRENT_SERVER_HANDLE),
                0,
                1,
                &mut pointer,
                &mut count,
            )?;
            let sessions = if count == 0 {
                &[]
            } else {
                if pointer.is_null() {
                    return Err("session_list_invalid".into());
                }
                std::slice::from_raw_parts(pointer, count as usize)
            };
            let active: Vec<u32> = sessions
                .iter()
                .filter(|s| s.State == WTSActive)
                .map(|s| s.SessionId)
                .collect();
            WTSFreeMemory(pointer.cast());
            let console = WTSGetActiveConsoleSessionId();
            if active != [console] || console == u32::MAX {
                return Err("ambiguous_or_remote_session".into());
            }
            console
        };
        let mut handle = HANDLE::default();
        // SAFETY: LocalSystem has SeTcbPrivilege. Token belongs to exact console session.
        unsafe {
            WTSQueryUserToken(id, &mut handle)?;
        }
        (id, Handle(handle))
    };
    // SAFETY: WTS owns the returned block until WTSFreeMemory. Validate level and
    // byte length before reading the union. Locked/unknown sessions fail closed.
    unsafe {
        let mut buffer = PWSTR::null();
        let mut bytes = 0;
        WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            id,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes,
        )?;
        let available = if bytes as usize >= std::mem::size_of::<WTSINFOEXW>() {
            let info = &*buffer.0.cast::<WTSINFOEXW>();
            info.Level == 1
                && info.Data.WTSInfoExLevel1.SessionState == WTSActive
                && info.Data.WTSInfoExLevel1.SessionFlags == WTS_SESSIONSTATE_UNLOCK as i32
        } else {
            false
        };
        WTSFreeMemory(buffer.0.cast());
        if !available {
            return Err("locked_or_unknown_session".into());
        }
    }
    let sid = user_sid(token.0)?;
    let data = token_information(token.0, TokenStatistics)?;
    // SAFETY: successful TokenStatistics buffer is aligned and sufficiently sized.
    let statistics = unsafe { &*(data.as_ptr().cast::<TOKEN_STATISTICS>()) };
    let key = format!(
        "s{id}-{}-{}",
        statistics.AuthenticationId.HighPart, statistics.AuthenticationId.LowPart
    );
    Ok(Session { key, token, sid })
}

pub struct Pipe {
    handle: Handle,
}
impl Pipe {
    pub fn receive<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>> {
        let mut available = 0;
        // SAFETY: live pipe, no data buffer for a bounded availability query.
        unsafe {
            PeekNamedPipe(self.handle.0, None, 0, None, Some(&mut available), None)?;
        }
        if available == 0 {
            return Ok(None);
        }
        if available as usize > MAX_MESSAGE {
            return Err("ipc_message_too_large".into());
        }
        let mut bytes = vec![0; MAX_MESSAGE];
        let mut read = 0;
        // SAFETY: exclusive reader owns pipe, buffer bounds passed by slice.
        unsafe {
            ReadFile(self.handle.0, Some(&mut bytes), Some(&mut read), None)?;
        }
        Ok(Some(serde_json::from_slice(&bytes[..read as usize])?))
    }
    pub fn send<T: serde::Serialize>(&self, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > MAX_MESSAGE {
            return Err("ipc_message_too_large".into());
        }
        let mut written = 0;
        // SAFETY: message-mode nonblocking pipe, bytes remain borrowed for the call.
        unsafe {
            WriteFile(self.handle.0, Some(&bytes), Some(&mut written), None)?;
        }
        if written as usize != bytes.len() {
            return Err("ipc_backpressure".into());
        }
        Ok(())
    }
}

// Constants from Win32 UpdateProcThreadAttribute's mitigation policy contract.
// Apply before user32 loads, closing the startup race for injected desktop hooks.
const BLOCK_NON_MICROSOFT_DLLS: u64 = 1u64 << 44;
const DISABLE_EXTENSION_POINTS: u64 = 1u64 << 32;
struct Mitigations {
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Vec<usize>,
    _policy: Box<u64>,
}
impl Mitigations {
    fn new() -> Result<Self> {
        let mut size = 0;
        // SAFETY: first call queries bytes; the second receives aligned owned storage.
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        }
        if size == 0 {
            return Err("process_mitigation_size_failed".into());
        }
        let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let pointer = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        // SAFETY: storage remains alive and fixed until DeleteProcThreadAttributeList.
        unsafe {
            InitializeProcThreadAttributeList(Some(pointer), 1, None, &mut size)?;
        }
        let attributes = Self {
            pointer,
            _storage: storage,
            _policy: Box::new(BLOCK_NON_MICROSOFT_DLLS | DISABLE_EXTENSION_POINTS),
        };
        // SAFETY: the policy Box has a stable address through CreateProcess, including moves.
        unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY as usize,
                Some((&*attributes._policy as *const u64).cast()),
                std::mem::size_of::<u64>(),
                None,
                None,
            )?;
        }
        Ok(attributes)
    }
}
impl Drop for Mitigations {
    fn drop(&mut self) {
        // SAFETY: exactly one deletion precedes release of the attribute storage and policy.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

pub struct Child {
    pub pipe: Pipe,
    process: Handle,
    connected: bool,
    launched: Instant,
}
impl Child {
    pub fn start(exe: &Path, session: &Session, test_mode: bool) -> Result<Self> {
        let name = format!(r"\\.\pipe\WCCS.Waiting.{}", wccs_protocol::token());
        let pipe = server_pipe(&name, &session.sid)?;
        let executable = exe.canonicalize()?;
        let executable_text = executable.to_str().ok_or("invalid_executable_path")?;
        if executable_text.contains('"') {
            return Err("invalid_executable_path".into());
        }
        let command = format!(
            "\"{executable_text}\" --controlled \"{name}\" {} {}",
            std::process::id(),
            if test_mode { "test" } else { "service" }
        );
        let mut command = wide(command);
        let application = wide(executable);
        let mut desktop = wide("winsta0\\default");
        let mitigations = Mitigations::new()?;
        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
                lpDesktop: PWSTR(desktop.as_mut_ptr()),
                ..Default::default()
            },
            lpAttributeList: mitigations.pointer,
        };
        let mut process = PROCESS_INFORMATION::default();
        // Production child is owned by SYSTEM with no ordinary-user handle access.
        // This protects IPC results from same-user OpenProcess/DuplicateHandle injection.
        let process_descriptor =
            Descriptor::new("O:SYG:SYD:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;RC;;;OW)")?;
        let process_attributes = process_descriptor.attributes();
        // SAFETY: explicit application path, mutable terminated command, no inherited handles.
        // CreateProcessAsUser creates a separate console user's desktop process from Session 0.
        unsafe {
            if test_mode {
                CreateProcessW(
                    PCWSTR(application.as_ptr()),
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    false,
                    CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                    None,
                    None,
                    &startup.StartupInfo,
                    &mut process,
                )?;
            } else {
                let mut environment = std::ptr::null_mut();
                CreateEnvironmentBlock(&mut environment, Some(session.token.0), false)?;
                let result = CreateProcessAsUserW(
                    Some(session.token.0),
                    PCWSTR(application.as_ptr()),
                    Some(PWSTR(command.as_mut_ptr())),
                    Some(&process_attributes),
                    Some(&process_attributes),
                    false,
                    CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                    Some(environment),
                    None,
                    &startup.StartupInfo,
                    &mut process,
                );
                let _ = DestroyEnvironmentBlock(environment);
                result?;
            }
            let _ = CloseHandle(process.hThread);
        }
        Ok(Self {
            pipe,
            process: Handle(process.hProcess),
            connected: false,
            launched: Instant::now(),
        })
    }
    pub fn stop(&mut self) -> Result<()> {
        // SAFETY: retained exact child handle owns termination/synchronization rights.
        unsafe {
            if WaitForSingleObject(self.process.0, 0) == WAIT_TIMEOUT {
                TerminateProcess(self.process.0, 1)?;
            }
            if WaitForSingleObject(self.process.0, 3000) != WAIT_OBJECT_0 {
                return Err("session_stop_unconfirmed".into());
            }
        }
        Ok(())
    }
    pub fn ready(&mut self) -> Result<bool> {
        // SAFETY: process handle retained for lifetime, preventing PID-reuse ambiguity.
        unsafe {
            if self.connected {
                return Ok(true);
            }
            if WaitForSingleObject(self.process.0, 0) != WAIT_TIMEOUT {
                return Err("session_process_exited".into());
            }
            let result = ConnectNamedPipe(self.pipe.handle.0, None);
            if let Err(error) = result {
                if error.code() == ERROR_PIPE_LISTENING.to_hresult() {
                    if self.launched.elapsed() > Duration::from_secs(8) {
                        return Err("ipc_connect_timeout".into());
                    }
                    return Ok(false);
                }
                if error.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                    return Err(error.into());
                }
            }
            if check_peer(&self.pipe, GetProcessId(self.process.0)).is_err() {
                DisconnectNamedPipe(self.pipe.handle.0)?;
                return Err("ipc_peer_rejected".into());
            }
            self.connected = true;
            Ok(true)
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        // SAFETY: only the exact child process handle is terminated. No PID lookup.
        // Normal leave is cooperative; this fallback handles crashed/hung UI and service stop.
        unsafe {
            if WaitForSingleObject(self.process.0, 0) == WAIT_TIMEOUT {
                let _ = TerminateProcess(self.process.0, 1);
                let _ = WaitForSingleObject(self.process.0, 3000);
            }
        }
    }
}

pub fn connect(name: &str, parent: u32, test_mode: bool) -> Result<Pipe> {
    if !name.starts_with(r"\\.\pipe\WCCS.Waiting.") {
        return Err("ipc_name_rejected".into());
    }
    let path = wide(name);
    // SAFETY: local pipe path and no inherited handle. SQOS prevents server impersonation.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )?
    };
    let pipe = Pipe {
        handle: Handle(handle),
    };
    let mode = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
    let mut server = 0;
    // SAFETY: query kernel-authenticated peer PID and keep the process open during validation.
    unsafe {
        SetNamedPipeHandleState(pipe.handle.0, Some(&mode), None, None)?;
        GetNamedPipeServerProcessId(pipe.handle.0, &mut server)?;
        if server != parent {
            return Err("ipc_server_rejected".into());
        }
        let process = Handle(OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            parent,
        )?);
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token)?;
        let token = Handle(token);
        if !test_mode && user_sid(token.0)? != "S-1-5-18" {
            return Err("ipc_server_not_system".into());
        }
        if test_mode && user_sid(token.0)? != user_sid(current_token()?.0)? {
            return Err("ipc_test_user_mismatch".into());
        }
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )?;
        let image = std::path::PathBuf::from(String::from_utf16(&buffer[..length as usize])?);
        let expected = std::env::current_exe()?.with_file_name("wccs-agent.exe");
        if image.canonicalize()? != expected.canonicalize()? {
            return Err("ipc_server_image_mismatch".into());
        }
    }
    Ok(pipe)
}

fn check_peer(pipe: &Pipe, expected: u32) -> Result<()> {
    let mut pid = 0;
    // SAFETY: pipe owns a live server endpoint; kernel writes the connected client's PID.
    unsafe {
        GetNamedPipeClientProcessId(pipe.handle.0, &mut pid)?;
    }
    if pid != expected {
        return Err("ipc_peer_rejected".into());
    }
    Ok(())
}

fn server_pipe(name: &str, sid: &str) -> Result<Pipe> {
    let path = wide(name);
    let descriptor = Descriptor::new(&format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{})", sid))?;
    let attributes = descriptor.attributes();
    // SAFETY: unique first instance, local-only pipe. Security allocation outlives creation.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(path.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_MESSAGE as u32,
            MAX_MESSAGE as u32,
            0,
            Some(&attributes),
        )
    };
    if handle.is_invalid() {
        return Err(windows::core::Error::from_win32().into());
    }
    Ok(Pipe {
        handle: Handle(handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (String, Pipe) {
        let name = format!(r"\\.\pipe\WCCS.Waiting.{}", wccs_protocol::token());
        let pipe = server_pipe(&name, &user_sid(current_token().unwrap().0).unwrap()).unwrap();
        (name, pipe)
    }

    #[test]
    fn first_instance_and_wrong_server_pid_are_rejected() {
        let (name, _server) = fixture();
        let sid = user_sid(current_token().unwrap().0).unwrap();
        assert!(server_pipe(&name, &sid).is_err());
        assert!(connect(&name, std::process::id().wrapping_add(1), true).is_err());
    }

    #[test]
    fn same_user_process_cannot_impersonate_service_or_agent_image() {
        let (name, _server) = fixture();
        assert!(connect(&name, std::process::id(), true).is_err());
        let (name, _server) = fixture();
        assert!(connect(&name, std::process::id(), false).is_err());
        assert!(
            connect(
                r"\\remote\pipe\WCCS.Waiting.invalid",
                std::process::id(),
                true
            )
            .is_err()
        );
    }

    #[test]
    fn message_boundaries_backpressure_and_peer_exit() {
        let (name, server) = fixture();
        let path = wide(name);
        // SAFETY: test owns both ends of this unique local pipe. Handles are closed by RAII.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
            .unwrap()
        };
        let client = Pipe {
            handle: Handle(handle),
        };
        // SAFETY: nonblocking message mode on the exclusively owned test client handle.
        unsafe {
            SetNamedPipeHandleState(
                client.handle.0,
                Some(&(PIPE_READMODE_MESSAGE | PIPE_NOWAIT)),
                None,
                None,
            )
            .unwrap();
        }
        assert!(check_peer(&server, std::process::id()).is_ok());
        assert!(check_peer(&server, std::process::id().wrapping_add(1)).is_err());
        assert!(server.receive::<serde_json::Value>().unwrap().is_none());
        client.send(&serde_json::json!({"sequence":1})).unwrap();
        client.send(&serde_json::json!({"sequence":2})).unwrap();
        assert_eq!(
            server.receive::<serde_json::Value>().unwrap().unwrap()["sequence"],
            1
        );
        assert_eq!(
            server.receive::<serde_json::Value>().unwrap().unwrap()["sequence"],
            2
        );
        assert!(client.send(&"x".repeat(MAX_MESSAGE)).is_err());
        drop(client);
        assert!(server.receive::<serde_json::Value>().is_err());
    }
}
