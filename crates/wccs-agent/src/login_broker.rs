//! System-only, bounded Credential Provider transport. No network or filesystem credentials.
use crate::{
    host::Result,
    login_windows::{Handle, Secret, token_sid},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use wccs_session::ipc::wide;
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Pipes::*, RemoteDesktop::*, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};

pub struct Offer {
    pub device: String,
    pub instance: String,
    pub operation: String,
    pub username: String,
    pub sid: String,
    pub password: Secret,
    pub deadline: u64,
}
#[derive(Default)]
struct State {
    offer: Option<Offer>,
    consumed: Option<String>,
}
pub struct Broker {
    state: Arc<Mutex<State>>,
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
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let pipe = create_pipe();
            let _ = sender.send(pipe.is_ok());
            let Ok(pipe) = pipe else {
                return;
            };
            while !worker_stop.load(Ordering::Acquire) {
                let _ = serve(pipe.0, &worker_state, &worker_stop);
                thread::sleep(Duration::from_millis(25));
            }
        });
        if !receiver.recv()? {
            let _ = worker.join();
            return Err("login_pipe_unavailable".into());
        }
        Ok(Self {
            state,
            stop,
            worker: Some(worker),
        })
    }
    pub fn issue(&self, offer: Offer) -> Result<()> {
        if [&offer.device, &offer.instance, &offer.operation]
            .iter()
            .any(|s| s.len() > 64)
            || offer.username.encode_utf16().count() > 63
            || offer.sid.len() > 191
            || offer.password.0.len() > 128
        {
            return Err("login_offer_bounds".into());
        }
        let mut state = self.state.lock().map_err(|_| "login_broker_poisoned")?;
        state.consumed = None;
        state.offer = Some(offer);
        Ok(())
    }
    pub fn revoke(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.offer = None;
            state.consumed = None;
        }
    }
    pub fn consumed(&self, operation: &str) -> bool {
        self.state
            .lock()
            .is_ok_and(|s| s.consumed.as_deref() == Some(operation))
    }
}
fn create_pipe() -> Result<Handle> {
    let name = wide(r"\\.\pipe\WCCS-Contest-Login-v1");
    let sddl = wide("O:SYG:SYD:P(A;;GA;;;SY)");
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: descriptor and name outlive creation; pipe copies the descriptor. Nonblocking mode bounds servicing.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let pipe = CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1024,
            1024,
            0,
            Some(&attributes),
        );
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        if pipe.is_invalid() {
            return Err("login_pipe_create_failed".into());
        }
        Ok(Handle(pipe))
    }
}
fn peer(pipe: HANDLE) -> Result<bool> {
    let mut pid = 0;
    let mut session = 0;
    // SAFETY: kernel-owned pipe PID is immediately tied to a live process handle, token and image path.
    unsafe {
        GetNamedPipeClientProcessId(pipe, &mut pid)?;
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
        ProcessIdToSessionId(pid, &mut session)?;
        if session != WTSGetActiveConsoleSessionId() {
            return Ok(false);
        }
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token)?;
        let token = Handle(token);
        if token_sid(token.0)? != "S-1-5-18" {
            return Ok(false);
        }
        let mut path = [0u16; 32768];
        let mut count = path.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut count,
        )?;
        let mut system = [0u16; 32768];
        let size =
            windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut system));
        if size == 0 || size as usize >= system.len() {
            return Ok(false);
        }
        let expected = format!(
            "{}\\LogonUI.exe",
            String::from_utf16(&system[..size as usize])?
        );
        Ok(String::from_utf16(&path[..count as usize])?.eq_ignore_ascii_case(&expected))
    }
}
fn serve(pipe: HANDLE, shared: &Mutex<State>, stop: &AtomicBool) -> Result<()> {
    // SAFETY: the worker exclusively owns this pipe. All transfers use bounded initialized slices.
    unsafe {
        let connected = ConnectNamedPipe(pipe, None);
        if connected.is_err() && GetLastError() != ERROR_PIPE_CONNECTED {
            return Ok(());
        }
        let result = (|| -> Result<()> {
            if !peer(pipe)? {
                return Err("login_peer_rejected".into());
            }
            let mut available = 0;
            let started = std::time::Instant::now();
            loop {
                PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None)?;
                if available != 0 {
                    break;
                }
                if stop.load(Ordering::Acquire) || started.elapsed() >= Duration::from_millis(500) {
                    return Err("login_request_timeout".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            if available != 203 {
                return Err("login_request_bounds".into());
            }
            let mut input = [0u8; 203];
            let mut count = 0;
            ReadFile(pipe, Some(&mut input), Some(&mut count), None)?;
            if count != 203 || u32::from_le_bytes(input[..4].try_into().unwrap()) != 1 {
                return Err("login_request_version".into());
            }
            let verb = u32::from_le_bytes(input[4..8].try_into().unwrap());
            let mut output = [0u8; 980];
            output[..4].copy_from_slice(&1u32.to_le_bytes());
            let mut state = shared.lock().map_err(|_| "login_broker_poisoned")?;
            if state
                .offer
                .as_ref()
                .is_some_and(|o| crate::service::monotonic_ms() >= o.deadline)
            {
                state.offer = None;
            }
            if let Some(offer) = &state.offer {
                let mut binding = [0u8; 195];
                for (index, value) in [&offer.device, &offer.instance, &offer.operation]
                    .iter()
                    .enumerate()
                {
                    binding[index * 65..index * 65 + value.len()].copy_from_slice(value.as_bytes());
                }
                if verb == 0 || (verb == 1 && input[8..] == binding) {
                    output[4..8].copy_from_slice(&1u32.to_le_bytes());
                    output[8..16].copy_from_slice(&offer.deadline.to_le_bytes());
                    output[16..211].copy_from_slice(&binding);
                    for (start, value) in [(212, wide(&offer.username)), (340, wide(&offer.sid))] {
                        for (i, c) in value.iter().enumerate() {
                            output[start + i * 2..start + i * 2 + 2]
                                .copy_from_slice(&c.to_le_bytes());
                        }
                    }
                    if verb == 1 {
                        for (i, c) in offer.password.0.iter().enumerate() {
                            output[724 + i * 2..726 + i * 2].copy_from_slice(&c.to_le_bytes());
                        }
                        state.consumed = Some(offer.operation.clone());
                        state.offer = None;
                    }
                }
            }
            drop(state);
            let written = WriteFile(pipe, Some(&output), Some(&mut count), None);
            for byte in &mut output {
                std::ptr::write_volatile(byte, 0);
            }
            written?;
            if count != 980 {
                return Err("login_response_incomplete".into());
            }
            // Keep the response available until the authenticated client closes its handle.
            // FlushFileBuffers would block indefinitely on a stalled LogonUI process.
            let sent = std::time::Instant::now();
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
