//! All Win32 calls live here. The UI thread owns every HWND, HFONT and timer.
//! Callbacks have static lifetimes, keep no Rust references, and post scalar
//! notifications to the message loop. No Rust borrow crosses a reentrant call.

mod control;
mod presentation;
mod snapshot;
mod window;

use control::Control;
pub use control::controlled;
use snapshot::{recovered_previous_run, save_snapshot};
use window::{Label, NativeWindow};

use crate::state::{InputKind, SessionEvent, Snapshot, WaitingState};
use presentation::{BACKGROUND, CARD, FOREGROUND, MUTED};
use std::{
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{
            LibraryLoader::*,
            RemoteDesktop::*,
            StationsAndDesktops::*,
            SystemInformation::{GetLocalTime, GetTickCount64},
            Threading::*,
        },
        UI::{
            Controls::DRAWITEMSTRUCT,
            HiDpi::*,
            Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_ESCAPE, VK_F1, VK_RETURN, VK_TAB},
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, PCWSTR, Result as WinResult, w},
};

use wccs_protocol::control::Command;

const REBUILD: u32 = WM_APP + 1;
const LAYOUT: u32 = WM_APP + 2;
const SESSION: u32 = WM_APP + 3;
const GUARDED_INPUT: u32 = WM_APP + 4;
static GUARDED: AtomicBool = AtomicBool::new(false);
static INTERNAL_EXIT: AtomicBool = AtomicBool::new(false);

struct KeyboardGuard(HHOOK);
impl KeyboardGuard {
    fn install(module: HINSTANCE) -> WinResult<Self> {
        // SAFETY: static callback, UI-thread message loop owns the hook until Drop.
        let hook =
            unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(module), 0)? };
        GUARDED.store(true, Ordering::Relaxed);
        Ok(Self(hook))
    }
}
impl Drop for KeyboardGuard {
    fn drop(&mut self) {
        GUARDED.store(false, Ordering::Relaxed);
        // SAFETY: this thread owns the hook. No callback retains borrowed state.
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}
unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && GUARDED.load(Ordering::Relaxed) {
        if wp.0 as u32 == WM_KEYDOWN || wp.0 as u32 == WM_SYSKEYDOWN {
            post(GUARDED_INPUT, WPARAM(0));
        }
        // Waiting accepts no keyboard actions. No key values are read or persisted.
        return LRESULT(1);
    }
    // SAFETY: forwarding untouched Windows hook parameters, including negative codes.
    unsafe { CallNextHookEx(None, code, wp, lp) }
}
const EXIT_BUTTON: usize = 100;
const WARNING_ID: usize = 300;
static ANIMATE: AtomicBool = AtomicBool::new(true);
const LABEL_ID: usize = 200;
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
                    if !GUARDED.load(Ordering::Relaxed) {
                        PostQuitMessage(0);
                    }
                    LRESULT(0)
                }
                WM_COMMAND if wp.0 & 0xffff == EXIT_BUTTON && wp.0 >> 16 == BN_CLICKED as usize => {
                    if !GUARDED.load(Ordering::Relaxed) {
                        PostQuitMessage(0);
                    }
                    LRESULT(0)
                }
                WM_QUERYENDSESSION => LRESULT(1),
                WM_CTLCOLORSTATIC => {
                    // The borrowed DC is supplied by Windows for this paint only.
                    let dc = HDC(wp.0 as *mut _);
                    let id = GetDlgCtrlID(HWND(lp.0 as *mut _)) as usize;
                    let color = match id.saturating_sub(LABEL_ID) {
                        0 | 1 | 3 | 8 | 11 => FOREGROUND,
                        _ => MUTED,
                    };
                    let background = if (1..=3).contains(&id.saturating_sub(LABEL_ID)) {
                        CARD
                    } else {
                        BACKGROUND
                    };
                    SetBkColor(dc, background);
                    SetTextColor(dc, color);
                    SetDCBrushColor(dc, background);
                    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
                }
                WM_ERASEBKGND => LRESULT(1),
                WM_PAINT => {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut paint);
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    let development = GetDlgItem(Some(hwnd), EXIT_BUTTON as i32).is_ok();
                    presentation::paint(dc, rect, development, ANIMATE.load(Ordering::Relaxed));
                    let _ = EndPaint(hwnd, &paint);
                    LRESULT(0)
                }
                WM_DRAWITEM if wp.0 == WARNING_ID => {
                    // DRAWITEMSTRUCT is borrowed for this synchronous notification.
                    let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                    presentation::paint_warning(item.hDC, item.rcItem);
                    LRESULT(1)
                }
                WM_SETTINGCHANGE => {
                    update_animation_preference();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    LRESULT(0)
                }
                WM_ENDSESSION if wp.0 != 0 => {
                    INTERNAL_EXIT.store(true, Ordering::Relaxed);
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
                        INTERNAL_EXIT.store(true, Ordering::Relaxed);
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
            INTERNAL_EXIT.store(true, Ordering::Relaxed);
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
            INTERNAL_EXIT.store(true, Ordering::Relaxed);
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
    display: Option<wccs_protocol::presentation::Display>,
    show_diagnostics: bool,
    windows: Vec<NativeWindow>,
    state: WaitingState,
    started: Instant,
    started_at_unix_ms: u128,
    recovered: bool,
    directory: Option<PathBuf>,
    command: Option<Command>,
    test_mode: bool,
    last_save_ms: u64,
    last_counts: String,
    last_clock: String,
    last_pointer: POINT,
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
        // 只订阅一个窗口，避免多屏重复计数同一次会话切换。
        windows[0].subscribe_to_session()?;
        self.windows = windows;
        self.last_counts.clear();
        self.last_clock.clear();
        self.render()?;
        // SAFETY: all windows belong to this UI thread. Creation sets the formal
        // mode's topmost style. Activation is handled by the foreground check.
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
        let identification = self
            .command
            .as_ref()
            .is_some_and(|c| c.action == "identify");
        let warning = !identification && self.state.remaining_ms(self.elapsed_ms()) > 0;
        let counts = format!(
            "键盘 {} 次　鼠标 {} 次　提示 {} 次",
            self.state.keyboard_events, self.state.mouse_events, self.state.prompts
        );
        // Wall time is display-only. Control leases and prompts use Instant.
        let now = unsafe { GetLocalTime() };
        let clock = format!("{:02}:{:02}:{:02}", now.wHour, now.wMinute, now.wSecond);
        for window in &mut self.windows {
            if self.last_clock != clock {
                window.set_label(Label::Clock, &clock)?;
            }
            if self.last_counts != counts && window.development && !identification {
                window.set_label(Label::InputCounts, &counts)?;
            }
            window.show_warning(warning);
        }
        if !identification {
            self.render_presentation()?;
        }
        self.last_clock = clock;
        self.last_counts = counts;
        Ok(())
    }

    fn render_presentation(&mut self) -> WinResult<()> {
        if self
            .command
            .as_ref()
            .is_none_or(|command| command.action != "enter")
        {
            return Ok(());
        }
        // SAFETY: the boot tick query has no borrowed pointers or owned resources.
        let tick = unsafe { GetTickCount64() };
        let display = self
            .display
            .as_ref()
            .filter(|p| self.command.as_ref().is_some_and(|c| p.data.matches(c)));
        presentation::set_logo(display.and_then(|p| p.data.logo.as_ref()));
        for window in &mut self.windows {
            window.set_label(Label::StaffHint, "工作人员诊断 F1")?;
            let Some(display) = display else {
                window.set_label(Label::Connection, "WCCS 等待展示数据 · OJ 未知")?;
                window.set_label(Label::Diagnostics,"展示数据尚未到达。工作人员可在本机配置入口执行分步骤诊断，并在管理端核对展示配置与设备回执。")?;
                window.show_diagnostics(self.show_diagnostics);
                continue;
            };
            let stale = display.stale(tick);
            let status = |state: &str| match state {
                "online" => "已连接",
                "offline" => "连接中断",
                _ => "未知",
            };
            window.set_label(
                Label::Connection,
                &format!(
                    "WCCS {} · OJ {}",
                    if stale {
                        "数据过期"
                    } else {
                        status(&display.wccs.state)
                    },
                    if stale {
                        "数据过期"
                    } else {
                        status(&display.oj.state)
                    }
                ),
            )?;
            let now = display.server_now(tick);
            if let (Some(now), Some(start)) = (now, display.data.starts_at_ms) {
                let seconds = start.saturating_sub(now).max(0).saturating_add(999) / 1000;
                window.set_label(
                    Label::Clock,
                    &format!(
                        "{:02}:{:02}:{:02}",
                        seconds / 3600,
                        seconds / 60 % 60,
                        seconds % 60
                    ),
                )?;
                window.set_label(
                    Label::ClockCaption,
                    if now >= start {
                        "已到开始时间，等待开放"
                    } else {
                        "距离比赛开始"
                    },
                )?;
            } else {
                window.set_label(Label::Clock, "--:--:--")?;
                window.set_label(Label::ClockCaption, "开赛时间待配置")?;
            }
            window.set_label(
                Label::Timing,
                &format!(
                    "显示校时 {:+} ms · RTT {} ms{}",
                    display.clock_offset_ms,
                    display.round_trip_ms,
                    if stale { " · 样本过期" } else { "" }
                ),
            )?;
            let logo = display
                .data
                .logo
                .as_ref()
                .map(|l| format!("{} / {}", l.source, l.sha256))
                .unwrap_or_else(|| "校徽缺失，使用默认图标，下轮刷新重试".into());
            window.set_label(Label::Diagnostics,&format!("工作人员诊断（F1 收起）\n设备 {} · 座位 {}\n比赛 {} · 身份版本 {} / {}\n展示配置 v{}\n摘要 {}\nWCCS {} · {} · 更新 {}\nOJ {} · {} · 更新 {}\n服务器时间样本 {} · RTT {} ms · 偏差 {:+} ms\n校徽 {}",
                display.data.device_id,display.data.identity.seat_label,display.data.identity.contest_id,display.data.identity.contest_version,display.data.identity.room_version,
                display.data.configuration_version,display.data.configuration_digest,display.wccs.source,display.wccs.detail,display.wccs.checked_at_ms,
                display.oj.source,display.oj.detail,display.oj.checked_at_ms,display.data.server_time_ms,display.round_trip_ms,display.clock_offset_ms,logo))?;
            window.show_diagnostics(self.show_diagnostics);
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

pub fn run(directory: &Path) -> Result<u8, Box<dyn std::error::Error>> {
    run_ui(Some(directory), None)
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
        display: control.as_ref().and_then(|c| c.display.clone()),
        show_diagnostics: false,
        test_mode: control.as_ref().is_some_and(|c| c.test_mode),
        directory,
        last_save_ms: 0,
        last_counts: String::new(),
        last_clock: String::new(),
        last_pointer: pointer_position(),
    };
    // SAFETY: module is borrowed for process lifetime. Cursor/brush are shared
    // system resources. The static callback remains valid until class unregistration.
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: CLASS_NAME,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
    }
    let _class = WindowClass(module);
    let guarded = control
        .as_ref()
        .is_some_and(|c| !c.test_mode && c.command.action != "identify");
    let _keyboard = if guarded {
        Some(KeyboardGuard::install(module)?)
    } else {
        None
    };
    // Declare the app after the class guard so all exit paths destroy windows first.
    let mut app = app;
    app.save(false)?;
    app.rebuild(module)?;
    app.save(false)?;
    if let Some(control) = &control {
        control.report(&app)?;
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
    update_animation_preference();
    // Separate visual frames from the existing lease/report timer.
    let animation_timer_id = unsafe { SetTimer(None, 0, 50, None) };
    if animation_timer_id == 0 {
        return Err(windows::core::Error::from_win32().into());
    }
    let _animation_timer = Timer(animation_timer_id);
    let mut message = MSG::default();
    loop {
        // SAFETY: message is writable, and all queued callbacks belong to this UI thread.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            return Err(windows::core::Error::from_win32().into());
        }
        if result == 0 {
            if guarded && !INTERNAL_EXIT.load(Ordering::Relaxed) {
                // Local WM_QUIT is not authority to end a controlled wait.
                continue;
            }
            let code = u8::from(message.wParam.0 != 0);
            if code == 0 {
                app.save(true)?;
            }
            return Ok(code);
        }
        match message.message {
            GUARDED_INPUT => {
                drain_notifications(GUARDED_INPUT);
                app.state.touch(InputKind::Keyboard, app.elapsed_ms());
                if app.windows.iter().any(|window| !window.warning_visible) {
                    app.render()?;
                }
            }
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
            WM_TIMER if message.hwnd.is_invalid() && message.wParam.0 == animation_timer_id => {
                if ANIMATE.load(Ordering::Relaxed) && app.state.visible() {
                    for window in &app.windows {
                        if !window.warning_visible {
                            // SAFETY: invalidate only our owned window, never activate it.
                            unsafe {
                                let _ = InvalidateRect(Some(window.handle), None, false);
                            }
                        }
                    }
                }
            }
            WM_TIMER if message.hwnd.is_invalid() && message.wParam.0 == timer_id => {
                if guarded {
                    // SAFETY: UI thread owns every window. Reassert z-order without
                    // activating each monitor, then request focus under Windows policy.
                    unsafe {
                        for window in &app.windows {
                            SetWindowPos(
                                window.handle,
                                Some(HWND_TOPMOST),
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                            )?;
                        }
                        if !app
                            .windows
                            .iter()
                            .any(|w| w.handle == GetForegroundWindow())
                        {
                            restore_foreground(app.windows[0].handle);
                        }
                    }
                }
                if let Some(control) = &mut control
                    && control.poll_host(&mut app)?.is_break()
                {
                    return Ok(0);
                }
                app.render()?;
                if app.elapsed_ms().saturating_sub(app.last_save_ms) >= 1000 {
                    app.save(false)?;
                }
            }
            _ => {
                // Showing/hiding the warning causes synthetic WM_MOUSEMOVE messages
                // even while idle. Compare screen coordinates across child windows
                // so those messages cannot reopen the warning indefinitely.
                if message.message == WM_MOUSEMOVE {
                    let pointer = pointer_position();
                    if pointer == app.last_pointer {
                        continue;
                    }
                    app.last_pointer = pointer;
                }
                let kind = match message.message {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(InputKind::Keyboard),
                    WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
                    | WM_XBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => Some(InputKind::Mouse),
                    _ => None,
                };
                if let Some(kind) = kind {
                    app.state.touch(kind, app.elapsed_ms());
                    if app.windows.iter().any(|window| !window.warning_visible) {
                        app.render()?;
                    }
                    if message.message == WM_KEYDOWN
                        && message.wParam.0 == VK_F1.0 as usize
                        && !identification_mode(&app)
                    {
                        app.show_diagnostics = !app.show_diagnostics;
                        app.render()?;
                    }
                    if !guarded
                        && message.message == WM_KEYDOWN
                        && message.wParam.0 == VK_ESCAPE.0 as usize
                    {
                        app.save(true)?;
                        return Ok(0);
                    }
                    if message.message == WM_KEYDOWN {
                        // SAFETY: queued input targets this thread's live windows.
                        // Tab selects the sole actionable child on its own monitor.
                        unsafe {
                            if message.wParam.0 == VK_TAB.0 as usize {
                                let root = GetAncestor(message.hwnd, GA_ROOT);
                                if let Some(window) = app
                                    .windows
                                    .iter()
                                    .find(|window| window.handle == root && window.development)
                                {
                                    let _ = SetFocus(Some(window.exit));
                                }
                                continue;
                            }
                            if message.wParam.0 == VK_RETURN.0 as usize
                                && app
                                    .windows
                                    .iter()
                                    .any(|window| window.development && window.exit == GetFocus())
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

fn pointer_position() -> POINT {
    let mut point = POINT::default();
    // SAFETY: writable POINT, used transiently for input deduplication only.
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}

fn update_animation_preference() {
    let mut enabled = BOOL(0);
    // SAFETY: SystemParametersInfoW writes one BOOL for this action. On failure
    // keep a static background. This only reads the user's animation preference.
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ANIMATE.store(result.is_ok() && enabled.as_bool(), Ordering::Relaxed);
}

fn input_desktop_active() -> bool {
    // SAFETY: the returned input desktop is owned until CloseDesktop. A secure
    // desktop denies this ordinary token, which is reported as unobserved.
    unsafe {
        let Ok(desktop) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        else {
            return false;
        };
        let mut name = [0u16; 128];
        let result = GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            None,
        );
        let _ = CloseDesktop(desktop);
        result.is_ok()
            && String::from_utf16_lossy(
                &name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())],
            )
            .eq_ignore_ascii_case("default")
    }
}

fn restore_foreground(window: HWND) {
    if !input_desktop_active() {
        return;
    }
    // SAFETY: window belongs to this UI thread. The foreground HWND is borrowed
    // only to obtain a thread ID. Input queues are joined for this synchronous
    // activation attempt and always detached before returning to the message loop.
    // Failure to attach or activate is represented by the next observed report.
    unsafe {
        if SetForegroundWindow(window).as_bool() {
            return;
        }
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        if foreground != 0
            && foreground != current
            && AttachThreadInput(current, foreground, true).as_bool()
        {
            let _ = SetForegroundWindow(window);
            let _ = SetFocus(Some(window));
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
}

fn identification_mode(app: &App) -> bool {
    app.command.as_ref().is_some_and(|c| c.action == "identify")
}

#[cfg(test)]
mod tests {
    use super::*;

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
