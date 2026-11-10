//! All Win32 calls live here. The UI thread owns every HWND, HFONT and timer.
//! Callbacks have static lifetimes, keep no Rust references, and post scalar
//! notifications to the message loop. No Rust borrow crosses a reentrant call.

use crate::state::{InputKind, SessionEvent, Snapshot, WaitingState, content_scale};
use std::{
    fs::{self, File},
    io::{self, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
        System::{LibraryLoader::*, RemoteDesktop::*, Threading::*},
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_ESCAPE, VK_RETURN, VK_TAB},
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, PCWSTR, Result as WinResult, w},
};

use wccs_protocol::control::{Actual, CONTROL_VERSION, Command, IpcRequest, LEASE_SECONDS};
use wccs_session::ipc;

const REBUILD: u32 = WM_APP + 1;
const LAYOUT: u32 = WM_APP + 2;
const SESSION: u32 = WM_APP + 3;
const EXIT_BUTTON: usize = 100;
const CLASS_NAME: PCWSTR = w!("WCCS.WaitingPrototype.v1");

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

struct Instance {
    handle: HANDLE,
    owned: bool,
}

impl Instance {
    fn acquire(name: PCWSTR) -> WinResult<Option<Self>> {
        // SAFETY: name is terminated and borrowed for this call. Default security
        // uses the current token's DACL. The handle is non-inheritable and owned here.
        let handle = unsafe { CreateMutexW(None, false, name)? };
        let mut instance = Self {
            handle,
            owned: false,
        };
        // SAFETY: handle is a live mutex. Zero timeout never blocks the UI thread.
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => {
                instance.owned = true;
                Ok(Some(instance))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(windows::core::Error::from_win32()),
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the creating UI thread drops its unique handle once. Release is
        // only called after successful acquisition, including abandoned ownership.
        unsafe {
            if self.owned {
                let _ = ReleaseMutex(self.handle);
            }
            let _ = CloseHandle(self.handle);
        }
    }
}

struct Font(HFONT);

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: controls have been destroyed or switched to a replacement font.
        // This owned font is never selected into an application-owned DC.
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

struct NativeWindow {
    handle: HWND,
    labels: Vec<HWND>,
    exit: HWND,
    font: Option<Font>,
    session_registered: bool,
    dpi: u32,
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        // SAFETY: this UI thread owns the parent and all its children. Unregister
        // before destruction. DestroyWindow synchronously ends their callbacks,
        // then Rust drops the font. No callback owns or frees a Rust object.
        unsafe {
            if self.session_registered {
                let _ = WTSUnRegisterSessionNotification(self.handle);
            }
            let _ = DestroyWindow(self.handle);
        }
    }
}

impl NativeWindow {
    fn create(
        rect: RECT,
        instance: HINSTANCE,
        index: usize,
        recovered: bool,
        identity: Option<&Command>,
        test_mode: bool,
    ) -> WinResult<Self> {
        let rect = if test_mode {
            RECT {
                left: rect.left + 40,
                top: rect.top + 40,
                right: (rect.left + 900).min(rect.right),
                bottom: (rect.top + 740).min(rect.bottom),
            }
        } else {
            rect
        };
        let title = wide(format!("WCCS 等待界面 · 开发测试 · 屏幕 {}", index + 1).as_ref());
        // SAFETY: class is registered on this thread. Strings remain alive through
        // creation and Windows copies them. No user-data pointer is passed.
        let handle = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS_NAME,
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance),
                None,
            )?
        };
        let mut window = Self {
            handle,
            labels: Vec::new(),
            exit: HWND::default(),
            font: None,
            session_registered: false,
            dpi: 96,
        };
        let recovery = if recovered {
            "上次运行异常结束，已重新建立等待界面"
        } else {
            "开发测试 · 合成身份信息"
        };
        for text in [
            "WCCS 竞赛等待",
            "姓名　林小禾",
            "学校　示例实验学校",
            "座位　A-023",
            "等待比赛开始，请等候工作人员通知。",
            "",
            "",
            recovery,
            "退出方式　点击按钮 / Esc / Alt+F4",
        ] {
            window
                .labels
                .push(window.control(w!("STATIC"), text, 0, instance)?);
        }
        if let Some(command) = identity {
            window.set_label(0, &format!("{} · 等待", command.identity.contest_name))?;
            window.set_label(1, &format!("姓名　{}", command.identity.name))?;
            window.set_label(2, &format!("学校　{}", command.identity.school))?;
            window.set_label(3, &format!("座位　{}", command.identity.seat_label))?;
            window.set_label(
                7,
                if test_mode {
                    "受控开发测试 · 可随时退出"
                } else {
                    "身份已由管理端确认"
                },
            )?;
        }
        window.exit = window.control(w!("BUTTON"), "退出开发测试", EXIT_BUTTON, instance)?;
        window.layout()?;
        Ok(window)
    }

    fn control(
        &self,
        class: PCWSTR,
        text: &str,
        id: usize,
        instance: HINSTANCE,
    ) -> WinResult<HWND> {
        let text = wide(text.as_ref());
        // SAFETY: parent is live and owned by this thread. Child ownership transfers
        // to the parent. ID is an integer control identifier, never dereferenced.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR(text.as_ptr()),
                WS_CHILD
                    | WS_VISIBLE
                    | if id == EXIT_BUTTON {
                        WS_TABSTOP
                    } else {
                        WINDOW_STYLE::default()
                    },
                0,
                0,
                1,
                1,
                Some(self.handle),
                Some(HMENU(id as *mut _)),
                Some(instance),
                None,
            )
        }
    }

    fn layout(&mut self) -> WinResult<()> {
        let mut rect = RECT::default();
        // SAFETY: window remains live for both queries; rect is writable.
        unsafe {
            GetClientRect(self.handle, &mut rect)?;
            self.dpi = GetDpiForWindow(self.handle);
        }
        let scale = content_scale(rect.right, rect.bottom, self.dpi);
        let pixels = |value: f64| (value * scale).round() as i32;
        // SAFETY: face name is static and terminated. Returned HFONT is exclusively
        // owned by Font and assigned to live controls before replacing the old font.
        let font = unsafe {
            CreateFontW(
                -pixels(23.0),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                DEFAULT_PITCH.0 as u32,
                w!("Microsoft YaHei UI"),
            )
        };
        if font.is_invalid() {
            return Err(windows::core::Error::from_win32());
        }
        let font = Font(font);
        let left = (rect.right - pixels(680.0)) / 2;
        let top = (rect.bottom - pixels(580.0)) / 2;
        // SAFETY: all handles are live children. WM_SETFONT borrows the font until
        // replacement/destruction. Synchronous SendMessage completes before return.
        unsafe {
            for (index, label) in self.labels.iter().enumerate() {
                SetWindowPos(
                    *label,
                    None,
                    left,
                    top + pixels(index as f64 * 53.0),
                    pixels(680.0),
                    pixels(48.0),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
            SetWindowPos(
                self.exit,
                None,
                left,
                top + pixels(505.0),
                pixels(260.0),
                pixels(58.0),
                SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
            // Complete every fallible layout operation before any control borrows
            // the replacement font, so a failure cannot leave a dangling HFONT.
            for control in self.labels.iter().chain(std::iter::once(&self.exit)) {
                SendMessageW(
                    *control,
                    WM_SETFONT,
                    Some(WPARAM(font.0.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        self.font = Some(font);
        Ok(())
    }

    fn set_label(&self, index: usize, text: &str) -> WinResult<()> {
        let text = wide(text.as_ref());
        // SAFETY: label is a live child; Windows copies the terminated text.
        unsafe { SetWindowTextW(self.labels[index], PCWSTR(text.as_ptr())) }
    }
}

// SAFETY contract: Windows invokes this static callback on the creating thread.
// hwnd/message parameters are borrowed only during the call. No Rust state or
// pointer is retained. catch_unwind prevents unwinding across the system ABI.
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let result = std::panic::catch_unwind(|| {
        // SAFETY: parameters originate in Windows. Only WM_DPICHANGED dereferences
        // lp, whose RECT is valid for that message's synchronous callback lifetime.
        unsafe {
            match message {
                WM_CLOSE => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_COMMAND if wp.0 & 0xffff == EXIT_BUTTON && wp.0 >> 16 == BN_CLICKED as usize => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_QUERYENDSESSION => LRESULT(1),
                WM_CTLCOLORSTATIC => {
                    // The borrowed DC is supplied by Windows for this paint only.
                    let dc = HDC(wp.0 as *mut _);
                    SetBkColor(dc, COLORREF(GetSysColor(COLOR_WINDOW)));
                    SetTextColor(dc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
                    LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
                }
                WM_ENDSESSION if wp.0 != 0 => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_DISPLAYCHANGE => {
                    post(REBUILD, WPARAM(0));
                    LRESULT(0)
                }
                WM_DPICHANGED => {
                    let rect = &*(lp.0 as *const RECT);
                    if SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .is_err()
                    {
                        PostQuitMessage(1);
                    }
                    post(REBUILD, WPARAM(0));
                    LRESULT(0)
                }
                WM_SIZE => {
                    post(LAYOUT, WPARAM(0));
                    LRESULT(0)
                }
                WM_WTSSESSION_CHANGE => {
                    post(SESSION, wp);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, message, wp, lp),
            }
        }
    });
    result.unwrap_or_else(|_| {
        // SAFETY: terminates only this UI thread's message loop on callback panic.
        unsafe {
            PostQuitMessage(1);
        }
        LRESULT(0)
    })
}

fn post(message: u32, parameter: WPARAM) {
    // SAFETY: UI thread queue exists because a window has already been created.
    // Only scalar values are enqueued, with no borrowed callback pointers.
    unsafe {
        if PostThreadMessageW(GetCurrentThreadId(), message, parameter, LPARAM(0)).is_err() {
            PostQuitMessage(1);
        }
    }
}

// SAFETY contract: EnumDisplayMonitors synchronously supplies valid rect and a
// pointer to our exclusive Vec. It retains neither pointer after enumeration.
unsafe extern "system" fn monitor_proc(_: HMONITOR, _: HDC, rect: *mut RECT, data: LPARAM) -> BOOL {
    let result = std::panic::catch_unwind(|| {
        // SAFETY: these pointers come from the synchronous call in monitor_rects.
        unsafe {
            (&mut *(data.0 as *mut Vec<RECT>)).push(*rect);
        }
    });
    BOOL::from(result.is_ok())
}

fn monitor_rects() -> WinResult<Vec<RECT>> {
    let mut rects = Vec::<RECT>::new();
    // SAFETY: exclusive Vec outlives enumeration. Callback is static and synchronous.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut rects as *mut _ as isize),
        )
        .ok()?;
    }
    if rects.is_empty() {
        return Err(ERROR_NOT_FOUND.into());
    }
    Ok(rects)
}

struct WindowClass(HINSTANCE);

impl Drop for WindowClass {
    fn drop(&mut self) {
        // SAFETY: run's windows are dropped before their registered class.
        unsafe {
            let _ = UnregisterClassW(CLASS_NAME, Some(self.0));
        }
    }
}

struct Timer(usize);

impl Drop for Timer {
    fn drop(&mut self) {
        // SAFETY: ID belongs to this UI thread's windowless timer.
        unsafe {
            let _ = KillTimer(None, self.0);
        }
    }
}

struct App {
    windows: Vec<NativeWindow>,
    state: WaitingState,
    started: Instant,
    started_at_unix_ms: u128,
    recovered: bool,
    directory: Option<PathBuf>,
    command: Option<Command>,
    test_mode: bool,
    last_save_ms: u64,
    last_text: (String, String),
}

impl App {
    fn rebuild(&mut self, instance: HINSTANCE) -> WinResult<()> {
        let mut windows = Vec::new();
        for (index, rect) in monitor_rects()?.into_iter().enumerate() {
            windows.push(NativeWindow::create(
                rect,
                instance,
                index,
                self.recovered,
                self.command.as_ref(),
                self.test_mode,
            )?);
        }
        // SAFETY: first window is live. Only it subscribes to this session, so
        // counters remain independent of monitor count. Drop unregisters it.
        unsafe {
            WTSRegisterSessionNotification(windows[0].handle, NOTIFY_FOR_THIS_SESSION)?;
        }
        windows[0].session_registered = true;
        self.windows = windows;
        self.last_text = Default::default();
        self.render()?;
        // SAFETY: all windows belong to this UI thread. Showing never makes them
        // topmost. OS focus policy remains in effect. Hidden sessions stay hidden.
        unsafe {
            for window in &self.windows {
                let _ = ShowWindow(
                    window.handle,
                    if self.state.visible() {
                        SW_SHOWNOACTIVATE
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
        Ok(())
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    fn render(&mut self) -> WinResult<()> {
        let remaining = self.state.remaining_ms(self.elapsed_ms());
        let prompt = if remaining > 0 {
            format!(
                "已检测到触碰，请稍候。提示冷却 {} 秒",
                remaining.div_ceil(1000)
            )
        } else {
            "等待中 · 触碰键盘或鼠标可查看提示".into()
        };
        let counts = format!(
            "键盘 {} 次　鼠标 {} 次　提示 {} 次",
            self.state.keyboard_events, self.state.mouse_events, self.state.prompts
        );
        if self.last_text != (prompt.clone(), counts.clone()) {
            for window in &self.windows {
                window.set_label(5, &prompt)?;
                window.set_label(6, &counts)?;
            }
            self.last_text = (prompt, counts);
        }
        Ok(())
    }

    fn save(&mut self, clean_exit: bool) -> io::Result<()> {
        let Some(directory) = &self.directory else {
            return Ok(());
        };
        let elapsed_ms = self.elapsed_ms();
        let snapshot = Snapshot {
            schema_version: 1,
            process_id: std::process::id(),
            started_at_unix_ms: self.started_at_unix_ms,
            elapsed_ms,
            clean_exit,
            recovered_previous_run: self.recovered,
            keyboard_events: self.state.keyboard_events,
            mouse_events: self.state.mouse_events,
            prompts: self.state.prompts,
            session_events: self.state.session_events,
            visible: !clean_exit && self.state.visible(),
            monitor_count: self.windows.len(),
            monitor_dpi: self.windows.iter().map(|window| window.dpi).collect(),
        };
        save_snapshot(directory, &snapshot)?;
        self.last_save_ms = elapsed_ms;
        Ok(())
    }
}

fn recovered_previous_run(directory: &Path) -> io::Result<bool> {
    match fs::read(directory.join("status.json")) {
        Ok(bytes) => {
            let previous: Snapshot = serde_json::from_slice(&bytes)?;
            if previous.schema_version != 1 {
                return Err(io::Error::other(
                    "运行记录版本不支持，请使用新的 --state-dir",
                ));
            }
            Ok(!previous.clean_exit)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn save_snapshot(directory: &Path, snapshot: &Snapshot) -> io::Result<()> {
    let temporary = directory.join("status.tmp");
    let mut file = File::create(&temporary)?;
    serde_json::to_writer_pretty(&mut file, snapshot)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    let source = wide(temporary.as_os_str());
    let destination = wide(directory.join("status.json").as_os_str());
    // SAFETY: both paths are terminated and live for the call. Same-directory
    // replacement publishes one complete snapshot, including after interrupted writes.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(io::Error::other)
    }
}

pub fn run(directory: &Path) -> Result<u8, Box<dyn std::error::Error>> {
    run_ui(Some(directory), None)
}

struct Control {
    pipe: ipc::Pipe,
    command: Command,
    deadline: Instant,
    test_mode: bool,
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
        || request.command.action != "enter"
        || request.command.session_key != session.key
        || request.lease_seconds == 0
        || request.lease_seconds > LEASE_SECONDS
    {
        return Err("ipc_command_rejected".into());
    }
    let control = Control {
        pipe,
        command: request.command,
        deadline: Instant::now() + std::time::Duration::from_secs(request.lease_seconds),
        test_mode,
    };
    run_ui(None, Some(control))
}

fn run_ui(
    directory: Option<&Path>,
    mut control: Option<Control>,
) -> Result<u8, Box<dyn std::error::Error>> {
    let Some(_instance) = Instance::acquire(w!("Local\\WCCS.WaitingPrototype.v1"))? else {
        eprintln!("本会话的等待界面已经运行，请使用已有窗口或先退出该进程。");
        return Ok(2);
    };
    let mut session_id = 0;
    // SAFETY: called before any window creation. Output session ID is writable.
    // PMv2 requires Windows 10 1703+, checked by the API's failure result.
    let module = unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id)?;
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        HINSTANCE(GetModuleHandleW(None)?.0)
    };
    let directory = directory.map(|d| d.join(format!("session-{session_id}")));
    if let Some(directory) = &directory {
        fs::create_dir_all(directory)?;
    }
    let app = App {
        windows: Vec::new(),
        state: WaitingState::default(),
        started: Instant::now(),
        started_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        recovered: directory
            .as_ref()
            .map(|d| recovered_previous_run(d))
            .transpose()?
            .unwrap_or(false),
        command: control.as_ref().map(|c| c.command.clone()),
        test_mode: control.as_ref().is_some_and(|c| c.test_mode),
        directory,
        last_save_ms: 0,
        last_text: Default::default(),
    };
    // SAFETY: module is borrowed for process lifetime. Cursor/brush are shared
    // system resources. The static callback remains valid until class unregistration.
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: CLASS_NAME,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
    }
    let _class = WindowClass(module);
    // Declare the app after the class guard so all exit paths destroy windows first.
    let mut app = app;
    app.save(false)?;
    app.rebuild(module)?;
    app.save(false)?;
    if let Some(control) = &control {
        report_control(control, &app)?;
    }
    // SAFETY: this thread owns the first visible window. Focus failure is permitted
    // by Windows policy and does not prevent clicking or closing the prototype.
    unsafe {
        let _ = SetForegroundWindow(app.windows[0].handle);
    }
    // SAFETY: windowless UI-thread timer, with no callback or borrowed pointers.
    let timer_id = unsafe { SetTimer(None, 0, 250, None) };
    if timer_id == 0 {
        return Err(windows::core::Error::from_win32().into());
    }
    let _timer = Timer(timer_id);
    let mut message = MSG::default();
    loop {
        // SAFETY: message is writable, and all queued callbacks belong to this UI thread.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            return Err(windows::core::Error::from_win32().into());
        }
        if result == 0 {
            let code = u8::from(message.wParam.0 != 0);
            if code == 0 {
                app.save(true)?;
            }
            return Ok(code);
        }
        match message.message {
            REBUILD => {
                drain_notifications(REBUILD);
                app.rebuild(module)?;
                app.save(false)?;
            }
            LAYOUT => {
                drain_notifications(LAYOUT);
                for window in &mut app.windows {
                    window.layout()?;
                }
            }
            SESSION => {
                if control.is_some() {
                    return Err("controlled_session_changed".into());
                }
                let event = match message.wParam.0 as u32 {
                    WTS_SESSION_LOCK => Some(SessionEvent::Lock),
                    WTS_SESSION_UNLOCK => Some(SessionEvent::Unlock),
                    WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
                        Some(SessionEvent::Disconnect)
                    }
                    WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => Some(SessionEvent::Connect),
                    _ => None,
                };
                if let Some(event) = event {
                    app.state.session(event);
                    app.rebuild(module)?;
                    app.save(false)?;
                }
            }
            WM_TIMER if message.hwnd.is_invalid() && message.wParam.0 == timer_id => {
                if let Some(control) = &mut control {
                    if Instant::now() >= control.deadline {
                        return Err("control_lease_expired".into());
                    }
                    if let Some(request) = control
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
                            control
                                .pipe
                                .send(&Actual::default())
                                .map_err(|_| "ipc_result_failed")?;
                            return Ok(0);
                        }
                        if request.command != control.command {
                            return Err("ipc_command_changed".into());
                        }
                        control.deadline =
                            Instant::now() + std::time::Duration::from_secs(request.lease_seconds);
                        report_control(control, &app)?;
                    }
                }
                app.render()?;
                if app.elapsed_ms().saturating_sub(app.last_save_ms) >= 1000 {
                    app.save(false)?;
                }
            }
            _ => {
                let kind = match message.message {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(InputKind::Keyboard),
                    WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
                    | WM_XBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => Some(InputKind::Mouse),
                    _ => None,
                };
                if let Some(kind) = kind {
                    app.state.touch(kind, app.elapsed_ms());
                    if message.message == WM_KEYDOWN && message.wParam.0 == VK_ESCAPE.0 as usize {
                        app.save(true)?;
                        return Ok(0);
                    }
                    if message.message == WM_KEYDOWN {
                        // SAFETY: queued input targets this thread's live windows.
                        // Tab selects the sole actionable child on its own monitor.
                        unsafe {
                            if message.wParam.0 == VK_TAB.0 as usize {
                                let root = GetAncestor(message.hwnd, GA_ROOT);
                                if let Some(window) =
                                    app.windows.iter().find(|window| window.handle == root)
                                {
                                    let _ = SetFocus(Some(window.exit));
                                }
                                continue;
                            }
                            if message.wParam.0 == VK_RETURN.0 as usize
                                && app.windows.iter().any(|window| window.exit == GetFocus())
                            {
                                app.save(true)?;
                                return Ok(0);
                            }
                        }
                    }
                }
                // SAFETY: GetMessage initialized this message. Dispatch invokes only
                // live Windows procedures. No mutable Rust reference is exposed to them.
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }
}

fn drain_notifications(kind: u32) {
    let mut message = MSG::default();
    // SAFETY: removes only our pointer-free, coalescible layout notifications.
    unsafe { while PeekMessageW(&mut message, None, kind, kind, PM_REMOVE).as_bool() {} }
}

fn report_control(control: &Control, app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let actual = Actual {
        state: if app.state.visible() && !app.windows.is_empty() {
            "waiting"
        } else {
            "unknown"
        }
        .into(),
        command_id: Some(control.command.command_id.clone()),
        session_key: Some(control.command.session_key.clone()),
        monitors: app.windows.len() as u32,
        detail: "ui_observed".into(),
    };
    control
        .pipe
        .send(&actual)
        .map_err(|_| "ipc_report_failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "wccs-session-{}-{unique}-中文 空格",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            // This unique directory was exclusively created by this test.
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn snapshot(clean_exit: bool) -> Snapshot {
        Snapshot {
            schema_version: 1,
            process_id: 0,
            started_at_unix_ms: 0,
            elapsed_ms: 42,
            clean_exit,
            recovered_previous_run: false,
            keyboard_events: 2,
            mouse_events: 3,
            prompts: 1,
            session_events: 0,
            visible: true,
            monitor_count: 1,
            monitor_dpi: vec![144],
        }
    }

    #[test]
    fn interrupted_snapshot_and_clean_restart_are_distinct() {
        let directory = TestDirectory::new();
        assert!(!recovered_previous_run(&directory.0).unwrap());
        save_snapshot(&directory.0, &snapshot(false)).unwrap();
        assert!(recovered_previous_run(&directory.0).unwrap());
        fs::write(directory.0.join("status.tmp"), b"{interrupted").unwrap();
        assert!(recovered_previous_run(&directory.0).unwrap());
        save_snapshot(&directory.0, &snapshot(true)).unwrap();
        assert!(!recovered_previous_run(&directory.0).unwrap());
        assert!(!directory.0.join("status.tmp").exists());
        let saved: Snapshot =
            serde_json::from_slice(&fs::read(directory.0.join("status.json")).unwrap()).unwrap();
        assert_eq!((saved.keyboard_events, saved.mouse_events), (2, 3));
    }

    #[test]
    fn failed_snapshot_write_preserves_previous_publication() {
        let directory = TestDirectory::new();
        save_snapshot(&directory.0, &snapshot(false)).unwrap();
        let previous = fs::read(directory.0.join("status.json")).unwrap();
        fs::create_dir(directory.0.join("status.tmp")).unwrap();
        assert!(save_snapshot(&directory.0, &snapshot(true)).is_err());
        assert_eq!(fs::read(directory.0.join("status.json")).unwrap(), previous);
    }

    #[test]
    fn unknown_or_corrupt_record_is_reported_without_overwrite() {
        let directory = TestDirectory::new();
        let mut future = snapshot(false);
        future.schema_version = 2;
        save_snapshot(&directory.0, &future).unwrap();
        assert!(recovered_previous_run(&directory.0).is_err());
        fs::write(directory.0.join("status.json"), b"invalid").unwrap();
        assert!(recovered_previous_run(&directory.0).is_err());
        assert_eq!(
            fs::read(directory.0.join("status.json")).unwrap(),
            b"invalid"
        );
    }

    #[test]
    fn mutex_excludes_other_thread_and_releases_after_drop() {
        let name = format!("Local\\WCCS.SessionTest.{}", std::process::id());
        let wide_name = wide(name.as_ref());
        let first = Instance::acquire(PCWSTR(wide_name.as_ptr()))
            .unwrap()
            .unwrap();
        let other_name = name.clone();
        assert!(
            std::thread::spawn(move || {
                let name = wide(other_name.as_ref());
                Instance::acquire(PCWSTR(name.as_ptr())).unwrap().is_none()
            })
            .join()
            .unwrap()
        );
        drop(first);
        assert!(
            std::thread::spawn(move || {
                let name = wide(name.as_ref());
                Instance::acquire(PCWSTR(name.as_ptr())).unwrap().is_some()
            })
            .join()
            .unwrap()
        );
    }
}
