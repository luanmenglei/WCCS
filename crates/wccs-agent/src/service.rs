//! SCM callback storage is process-lifetime. Callback handles are borrowed from SCM.
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use wccs_session::ipc::wide;
use windows::{
    Win32::{Foundation::NO_ERROR, System::Services::*},
    core::{PCWSTR, PWSTR},
};

struct Settings {
    name: String,
    path: PathBuf,
    executable: PathBuf,
    stop: Arc<AtomicBool>,
}
static SETTINGS: OnceLock<Settings> = OnceLock::new();
static STATUS: Mutex<Option<usize>> = Mutex::new(None);

fn status(state: SERVICE_STATUS_CURRENT_STATE, exit: u32) {
    if let Ok(guard) = STATUS.lock()
        && let Some(handle) = *guard
    {
        let value = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
            } else {
                0
            },
            dwWin32ExitCode: exit,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: 15000,
        };
        // SAFETY: SCM owns a status handle valid until service_main returns.
        unsafe {
            let _ = SetServiceStatus(SERVICE_STATUS_HANDLE(handle as *mut _), &value);
        }
    }
}
unsafe extern "system" fn handler(
    control: u32,
    _event: u32,
    _data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    if control == SERVICE_CONTROL_STOP || control == SERVICE_CONTROL_SHUTDOWN {
        if let Some(settings) = SETTINGS.get() {
            settings.stop.store(true, Ordering::Relaxed);
        }
        status(SERVICE_STOP_PENDING, 0);
    }
    NO_ERROR.0
}
unsafe extern "system" fn main_callback(_count: u32, _args: *mut PWSTR) {
    let result = std::panic::catch_unwind(|| {
        let settings = SETTINGS.get().expect("SCM settings initialized");
        let name = wide(&settings.name);
        // SAFETY: static handler, no context pointer. Name valid for registration call.
        let handle =
            unsafe { RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()), Some(handler), None) };
        let Ok(handle) = handle else {
            return;
        };
        if let Ok(mut stored) = STATUS.lock() {
            *stored = Some(handle.0 as usize);
        }
        status(SERVICE_START_PENDING, 0);
        status(SERVICE_RUNNING, 0);
        let result = crate::host::run(
            settings.path.clone(),
            settings.executable.clone(),
            false,
            false,
            settings.stop.clone(),
        );
        status(SERVICE_STOPPED, if result.is_ok() { 0 } else { 1 });
    });
    if result.is_err() {
        status(SERVICE_STOPPED, 1);
    }
}
pub fn dispatch(name: String, path: PathBuf, executable: PathBuf) -> crate::host::Result<()> {
    if !name.starts_with("WCCS-") || name.len() > 80 {
        return Err("service_name_invalid".into());
    }
    SETTINGS
        .set(Settings {
            name: name.clone(),
            path,
            executable,
            stop: Arc::new(AtomicBool::new(false)),
        })
        .map_err(|_| "service_already_initialized")?;
    let mut name = wide(name);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_mut_ptr()),
            lpServiceProc: Some(main_callback),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    // SAFETY: table and name outlive the blocking dispatcher; callbacks are static.
    unsafe {
        StartServiceCtrlDispatcherW(table.as_ptr())?;
    }
    Ok(())
}

/// Boot-relative Windows clock includes sleep and ignores wall-clock changes.
pub fn monotonic_ms() -> u64 {
    // SAFETY: GetTickCount64 takes no pointers, owns no handles, and is callable on any thread.
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

pub fn query(name: &str) -> windows::core::Result<(bool, u32)> {
    struct ServiceHandle(SC_HANDLE);
    impl Drop for ServiceHandle {
        fn drop(&mut self) {
            // SAFETY: each successful SCM open is owned by this wrapper exactly once.
            unsafe {
                let _ = CloseServiceHandle(self.0);
            }
        }
    }
    let name = wide(name);
    // SAFETY: name is terminated. Both SCM handles and output storage remain live through the calls.
    unsafe {
        let manager = ServiceHandle(OpenSCManagerW(None, None, SC_MANAGER_CONNECT)?);
        let service = ServiceHandle(OpenServiceW(
            manager.0,
            PCWSTR(name.as_ptr()),
            SERVICE_QUERY_STATUS,
        )?);
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut needed = 0;
        let buffer = std::slice::from_raw_parts_mut(
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
            size_of::<SERVICE_STATUS_PROCESS>(),
        );
        QueryServiceStatusEx(service.0, SC_STATUS_PROCESS_INFO, Some(buffer), &mut needed)?;
        Ok((status.dwCurrentState == SERVICE_RUNNING, status.dwProcessId))
    }
}
