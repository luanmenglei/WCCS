//! Local power requests from the recovered contest console. Authority is owned by Native.
use crate::{
    host::Result,
    login_windows::{Handle, token_sid},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use wccs_session::ipc::wide;
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Pipes::*, RemoteDesktop::*, Shutdown::*, Threading::*},
        UI::WindowsAndMessaging::*,
    },
    core::{PCWSTR, w},
};
const PIPE: &str = r"\\.\pipe\WCCS-Local-Power-v1";

pub struct Broker {
    allowed: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Broker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Broker {
    pub fn start() -> Result<Self> {
        let allowed = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let state = allowed.clone();
        let stopping = stop.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let pipe = create_pipe();
            let _ = sender.send(pipe.is_ok());
            let Ok(pipe) = pipe else { return };
            while !stopping.load(Ordering::Acquire) {
                let _ = serve(pipe.0, &state, &stopping);
                thread::sleep(Duration::from_millis(25));
            }
        });
        if !receiver.recv()? {
            let _ = worker.join();
            return Err("power_pipe_unavailable".into());
        }
        Ok(Self {
            allowed,
            stop,
            worker: Some(worker),
        })
    }
    pub fn set_account(&self, sid: Option<&str>) -> Result<()> {
        *self.allowed.lock().map_err(|_| "power_state_poisoned")? = sid.map(String::from);
        Ok(())
    }
}
fn create_pipe() -> Result<Handle> {
    let name = wide(PIPE);
    let sddl = wide("O:SYG:SYD:P(A;;GA;;;SY)(A;;GRGW;;;IU)");
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: terminated strings and initialized descriptor outlive creation. Pipe copies the ACL.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let pipe = CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            16,
            16,
            0,
            Some(&attributes),
        );
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        if pipe.is_invalid() {
            return Err("power_pipe_create_failed".into());
        }
        Ok(Handle(pipe))
    }
}
fn permitted(allowed: Option<&str>, sid: &str, session: u32, console: u32, action: u8) -> bool {
    allowed == Some(sid)
        && session != 0
        && console != u32::MAX
        && session == console
        && [1, 2].contains(&action)
}
fn serve(pipe: HANDLE, shared: &Mutex<Option<String>>, stop: &AtomicBool) -> Result<()> {
    // SAFETY: the worker owns the nonblocking pipe and all I/O uses initialized bounded buffers.
    unsafe {
        if ConnectNamedPipe(pipe, None).is_err() && GetLastError() != ERROR_PIPE_CONNECTED {
            return Ok(());
        }
        let result = (|| -> Result<()> {
            let started = Instant::now();
            let mut available = 0;
            while available == 0 {
                PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None)?;
                if stop.load(Ordering::Acquire) || started.elapsed() > Duration::from_millis(500) {
                    return Err("power_request_timeout".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            if available != 2 {
                return Err("power_request_invalid".into());
            }
            let mut input = [0u8; 2];
            let mut count = 0;
            ReadFile(pipe, Some(&mut input), Some(&mut count), None)?;
            if count != 2 || input[0] != 1 {
                return Err("power_request_version".into());
            }
            // Impersonation ties authorization to the actual connected token, including its session.
            ImpersonateNamedPipeClient(pipe)?;
            let peer = (|| -> Result<(String, u32)> {
                let mut raw = HANDLE::default();
                OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut raw)?;
                let token = Handle(raw);
                let mut session = 0u32;
                let mut length = 0;
                GetTokenInformation(
                    token.0,
                    TokenSessionId,
                    Some((&mut session as *mut u32).cast()),
                    size_of::<u32>() as u32,
                    &mut length,
                )?;
                Ok((token_sid(token.0)?, session))
            })();
            // Never execute a privileged action while impersonating. Failure terminates this worker.
            if RevertToSelf().is_err() {
                stop.store(true, Ordering::Release);
                return Err("power_revert_failed".into());
            }
            let (sid, session) = peer?;
            // Hold authority through request submission, serialized with restriction activation.
            let allowed = shared.lock().map_err(|_| "power_state_poisoned")?;
            let mut response = [0u8];
            if permitted(
                allowed.as_deref(),
                &sid,
                session,
                WTSGetActiveConsoleSessionId(),
                input[1],
            ) {
                let _privilege = crate::waiting_policy::Privilege::enable(SE_SHUTDOWN_NAME)?;
                InitiateSystemShutdownExW(
                    PCWSTR::null(),
                    w!("WCCS 本机电源操作，请保存工作"),
                    15,
                    false,
                    input[1] == 2,
                    SHTDN_REASON_FLAG_PLANNED | SHTDN_REASON_MAJOR_APPLICATION,
                )?;
                response[0] = 1;
            }
            WriteFile(pipe, Some(&response), Some(&mut count), None)?;
            drop(allowed);
            let sent = Instant::now();
            while !stop.load(Ordering::Acquire) && sent.elapsed() < Duration::from_millis(500) {
                if PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        })();
        let _ = DisconnectNamedPipe(pipe);
        result
    }
}
pub fn request(action: &str) -> Result<()> {
    let code = match action {
        "shutdown" => 1,
        "restart" => 2,
        _ => return Err("power_action_invalid".into()),
    };
    let message = wide(if code == 1 {
        "保存工作后，确定关闭这台电脑？"
    } else {
        "保存工作后，确定重启这台电脑？"
    });
    let pipe = wide(PIPE);
    // SAFETY: local interactive confirmation and fixed-size synchronous pipe transaction.
    unsafe {
        if MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("WCCS 电源"),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        ) != IDYES
        {
            return Ok(());
        }
        let input = [1u8, code];
        let mut response = [0u8; 1];
        let mut count = 0;
        let result = CallNamedPipeW(
            PCWSTR(pipe.as_ptr()),
            Some(input.as_ptr().cast()),
            2,
            Some(response.as_mut_ptr().cast()),
            1,
            &mut count,
            2000,
        );
        if !result.as_bool() || count != 1 || response[0] != 1 {
            MessageBoxW(
                None,
                w!("当前尚未恢复教学，或本机服务不可用。请联系工作人员。"),
                w!("WCCS 电源"),
                MB_OK | MB_ICONINFORMATION,
            );
            return Err("power_request_rejected".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_require_recovered_account_active_console_and_known_action() {
        assert!(permitted(Some("contest"), "contest", 1, 1, 1));
        assert!(permitted(Some("contest"), "contest", 1, 1, 2));
        assert!(!permitted(None, "contest", 1, 1, 1));
        assert!(!permitted(Some("contest"), "other", 1, 1, 1));
        assert!(!permitted(Some("contest"), "contest", 2, 1, 1));
        assert!(!permitted(Some("contest"), "contest", 0, 0, 1));
        assert!(!permitted(Some("contest"), "contest", 1, u32::MAX, 1));
        assert!(!permitted(Some("contest"), "contest", 1, 1, 3));
    }
}
