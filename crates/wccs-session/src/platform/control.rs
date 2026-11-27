//! 宿主通过已认证的管道授予等待权限。窗口用单调时钟判断续租，过期后退出本轮控制。
use super::{App, input_desktop_active, run_ui};
use std::{ops::ControlFlow, time::Instant};
use wccs_protocol::control::{Actual, CONTROL_VERSION, Command, IpcRequest, LEASE_SECONDS};
use wccs_session::ipc;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetShellWindow};

pub(super) struct Control {
    pub(super) display: Option<wccs_protocol::presentation::Display>,
    pipe: ipc::Pipe,
    pub(super) command: Command,
    deadline: Instant,
    pub(super) test_mode: bool,
}

pub fn controlled() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 || !["test", "service"].contains(&args[4].as_str()) {
        return Err("controlled_arguments".into());
    }
    let test_mode = args[4] == "test";
    let pipe =
        ipc::connect(&args[2], args[3].parse()?, test_mode).map_err(|_| "ipc_auth_failed")?;
    let start = Instant::now();
    let request = loop {
        if let Some(request) = pipe
            .receive::<IpcRequest>()
            .map_err(|_| "ipc_read_failed")?
        {
            break request;
        }
        if start.elapsed().as_secs() > 8 {
            return Err("ipc_timeout".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let session = ipc::discover(true).map_err(|_| "session_unavailable")?;
    if request.protocol_version != CONTROL_VERSION
        || request.command.protocol_version != CONTROL_VERSION
        || !["enter", "probe_desktop", "probe_ipc", "identify"]
            .contains(&request.command.action.as_str())
        || request.command.session_key != session.key
        || request.lease_seconds == 0
        || request.lease_seconds > LEASE_SECONDS
    {
        return Err("ipc_command_rejected".into());
    }
    if request.command.action == "probe_ipc" {
        pipe.send(&Actual {
            command_id: Some(request.command.command_id),
            session_key: Some(session.key),
            detail: "ui_observed".into(),
            ..Default::default()
        })
        .map_err(|_| "ipc_probe_reply_failed")?;
        return Ok(0);
    }
    if request.command.action == "probe_desktop" {
        if request
            .command
            .login
            .as_ref()
            .and_then(|p| p.snapshot.target_sid.as_deref())
            != Some(session.sid())
            || !ipc::default_desktop().map_err(|_| "desktop_query_failed")?
        {
            return Err("desktop_identity_mismatch".into());
        }
        let started = Instant::now();
        // SAFETY: GetShellWindow borrows a window handle owned by the shell in this session.
        while !input_desktop_active() || unsafe { GetShellWindow().0.is_null() } {
            if started.elapsed() > std::time::Duration::from_secs(90) {
                return Err("desktop_shell_initialization_timeout".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        pipe.send(&Actual {
            state: "desktop".into(),
            command_id: Some(request.command.command_id),
            session_key: Some(session.key),
            detail: "ui_observed".into(),
            ..Default::default()
        })
        .map_err(|_| "desktop_report_failed")?;
        return Ok(0);
    }
    let control = Control {
        pipe,
        display: request
            .presentation
            .filter(|p| p.data.matches(&request.command)),
        command: request.command,
        deadline: Instant::now() + std::time::Duration::from_secs(request.lease_seconds),
        test_mode,
    };
    run_ui(None, Some(control))
}

impl Control {
    pub(super) fn poll_host(
        &mut self,
        app: &mut App,
    ) -> Result<ControlFlow<()>, Box<dyn std::error::Error>> {
        if Instant::now() >= self.deadline {
            return Err("control_lease_expired".into());
        }
        if let Some(request) = self
            .pipe
            .receive::<IpcRequest>()
            .map_err(|_| "ipc_disconnected")?
        {
            if request.protocol_version != CONTROL_VERSION
                || request.lease_seconds == 0
                || request.lease_seconds > LEASE_SECONDS
            {
                return Err("ipc_protocol_rejected".into());
            }
            if request.command.action == "leave" {
                app.windows.clear();
                self.pipe
                    .send(&Actual::default())
                    .map_err(|_| "ipc_result_failed")?;
                return Ok(ControlFlow::Break(()));
            }
            self.display = request
                .presentation
                .filter(|p| p.data.matches(&self.command));
            app.display = self.display.clone();
            if request.command != self.command {
                return Err("ipc_command_changed".into());
            }
            self.deadline = Instant::now() + std::time::Duration::from_secs(request.lease_seconds);
            self.report(app)?;
        }
        Ok(ControlFlow::Continue(()))
    }
    pub(super) fn report(&self, app: &App) -> Result<(), Box<dyn std::error::Error>> {
        // SAFETY: borrowed foreground HWND is compared only, never dereferenced.
        let foreground = self.test_mode
            || (input_desktop_active()
                && unsafe {
                    app.windows
                        .iter()
                        .any(|w| w.handle == GetForegroundWindow())
                });
        let actual = Actual {
            state: if app.state.visible() && !app.windows.is_empty() && foreground {
                "waiting"
            } else {
                "unknown"
            }
            .into(),
            command_id: Some(self.command.command_id.clone()),
            session_key: Some(self.command.session_key.clone()),
            monitors: app.windows.len() as u32,
            detail: if foreground {
                "ui_observed"
            } else {
                "waiting_focus_lost"
            }
            .into(),
            account: None,
        };
        self.pipe.send(&actual).map_err(|_| "ipc_report_failed")?;
        Ok(())
    }
}
