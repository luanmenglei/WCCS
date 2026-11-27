//! 单个显示器的等待窗口拥有其子控件和字体。窗口销毁和 DPI 重排都在 UI 线程执行。
use super::{CLASS_NAME, EXIT_BUTTON, Font, LABEL_ID, WARNING_ID, presentation::Layout, wide};
use wccs_protocol::control::Command;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{
            RemoteDesktop::{
                NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
                WTSUnRegisterSessionNotification,
            },
            SystemServices::*,
        },
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, Result as WinResult, w},
};

// 控件 ID 和展示布局按这个顺序对应。更新文字通过名称定位，DPI 重排后仍使用同一个控件。
#[repr(usize)]
pub(super) enum Label {
    Contest = 0,
    Name = 1,
    School = 2,
    Seat = 3,
    Connection = 4,
    Timing = 5,
    InputCounts = 6,
    StaffHint = 7,
    Clock = 8,
    ClockCaption = 9,
    Diagnostics = 10,
}

pub(super) struct NativeWindow {
    pub(super) handle: HWND,
    labels: Vec<HWND>,
    pub(super) exit: HWND,
    warning: HWND,
    pub(super) warning_visible: bool,
    fonts: Vec<Font>,
    pub(super) development: bool,
    session_registered: bool,
    pub(super) dpi: u32,
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
    pub(super) fn create(
        rect: RECT,
        instance: HINSTANCE,
        index: usize,
        recovered: bool,
        identity: Option<&Command>,
        test_mode: bool,
    ) -> WinResult<Self> {
        let identification = identity.is_some_and(|c| c.action == "identify");
        let rect = if test_mode && !identification {
            RECT {
                left: rect.left + 40,
                top: rect.top + 40,
                right: (rect.left + 900).min(rect.right),
                bottom: (rect.top + 740).min(rect.bottom),
            }
        } else {
            rect
        };
        let title = wide(format!("WCCS 等待界面 · 屏幕 {}", index + 1).as_ref());
        // SAFETY: class is registered on this thread. Strings remain alive through
        // creation and Windows copies them. No user-data pointer is passed.
        let handle = unsafe {
            CreateWindowExW(
                if identity.is_some() && !test_mode {
                    WS_EX_TOPMOST
                } else {
                    WINDOW_EX_STYLE::default()
                },
                CLASS_NAME,
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_CLIPCHILDREN,
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
            warning: HWND::default(),
            warning_visible: false,
            fonts: Vec::new(),
            development: identity.is_none() || test_mode || identification,
            session_registered: false,
            dpi: 96,
        };
        let recovery = if recovered {
            "上次运行异常结束，等待界面已恢复"
        } else {
            "开发预览 · 合成身份"
        };
        for (index, text) in [
            "WCCS 程序设计竞赛",
            "林小禾",
            "示例实验学校",
            "A-023",
            "等待比赛开始",
            "请等待工作人员统一开放",
            "",
            recovery,
            "00:00:00",
            "当前时间",
            "",
            "WCCS",
        ]
        .into_iter()
        .enumerate()
        {
            window
                .labels
                .push(window.control(w!("STATIC"), text, LABEL_ID + index, instance)?);
        }
        if let Some(command) = identity {
            window.set_label(Label::Contest, &command.identity.contest_name)?;
            window.set_label(Label::Name, &command.identity.name)?;
            window.set_label(Label::School, &command.identity.school)?;
            window.set_label(Label::Seat, &command.identity.seat_label)?;
            window.set_label(
                Label::StaffHint,
                if test_mode { "受控开发预览" } else { "" },
            )?;
        }
        if window.development {
            window.exit = window.control(
                w!("BUTTON"),
                if identification {
                    "关闭设备识别"
                } else {
                    "退出开发测试"
                },
                EXIT_BUTTON,
                instance,
            )?;
        }
        if identification {
            window.set_label(Label::Connection, "设备与物理座位核对")?;
            window.set_label(Label::Timing, "Esc 关闭 · 到期自动关闭")?;
            window.set_label(Label::StaffHint, "工作人员主动识别")?;
        }
        window.warning =
            window.control(w!("STATIC"), "请勿触碰鼠标和键盘", WARNING_ID, instance)?;
        // SAFETY: child belongs to this window and starts hidden until input.
        unsafe {
            let _ = ShowWindow(window.warning, SW_HIDE);
        }
        unsafe {
            let _ = ShowWindow(window.labels[10], SW_HIDE);
        }
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
                    | WS_CLIPSIBLINGS
                    | if id == EXIT_BUTTON {
                        WS_TABSTOP
                    } else {
                        let style = match id {
                            WARNING_ID => SS_OWNERDRAW.0,
                            id if id == LABEL_ID + 10 => SS_LEFT.0,
                            id if id == LABEL_ID + 3 => {
                                SS_CENTER.0 | SS_CENTERIMAGE.0 | SS_ENDELLIPSIS.0
                            }
                            id if [5, 8, 9].contains(&id.saturating_sub(LABEL_ID)) => {
                                SS_RIGHT.0 | SS_ENDELLIPSIS.0
                            }
                            _ => SS_LEFTNOWORDWRAP.0 | SS_ENDELLIPSIS.0,
                        };
                        WINDOW_STYLE(SS_NOPREFIX.0 | style)
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

    pub(super) fn layout(&mut self) -> WinResult<()> {
        let mut rect = RECT::default();
        // SAFETY: window remains live for both queries; rect is writable.
        unsafe {
            GetClientRect(self.handle, &mut rect)?;
            self.dpi = GetDpiForWindow(self.handle);
        }
        let layout = Layout::new(rect.right, rect.bottom, self.development);
        let mut fonts = Vec::new();
        for (index, (label, (area, size, weight))) in
            self.labels.iter().zip(layout.labels).enumerate()
        {
            fonts.push(fitted_font(
                *label,
                area.right - area.left,
                area.bottom - area.top,
                size,
                weight,
                if index == 8 {
                    w!("Consolas")
                } else {
                    w!("Microsoft YaHei UI")
                },
            )?);
            // SAFETY: controls are owned by this live parent on the UI thread.
            unsafe {
                SetWindowPos(
                    *label,
                    None,
                    area.left,
                    area.top,
                    area.right - area.left,
                    area.bottom - area.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
        }
        if self.development {
            let area = layout.exit;
            fonts.push(fitted_font(
                self.exit,
                area.right - area.left,
                area.bottom - area.top,
                (18. * layout.unit) as i32,
                400,
                w!("Microsoft YaHei UI"),
            )?);
            // SAFETY: development-only button exists and belongs to this window.
            unsafe {
                SetWindowPos(
                    self.exit,
                    None,
                    area.left,
                    area.top,
                    area.right - area.left,
                    area.bottom - area.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
        }
        // SAFETY: warning is an owned child; positioning preserves visibility.
        unsafe {
            SetWindowPos(
                self.warning,
                Some(HWND_TOP),
                0,
                0,
                rect.right,
                rect.bottom,
                SWP_NOACTIVATE,
            )?;
        }
        // All fallible work completes before any control borrows a replacement
        // font. The previous fonts remain owned until every control has switched.
        // SAFETY: handles/fonts live through synchronous WM_SETFONT and beyond.
        unsafe {
            let controls = self
                .labels
                .iter()
                .copied()
                .chain(self.development.then_some(self.exit));
            for (control, font) in controls.zip(&fonts) {
                SendMessageW(
                    control,
                    WM_SETFONT,
                    Some(WPARAM(font.0.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
            let _ = InvalidateRect(Some(self.handle), None, true);
        }
        self.fonts = fonts;
        Ok(())
    }

    pub(super) fn subscribe_to_session(&mut self) -> WinResult<()> {
        // SAFETY: this thread owns the live HWND. Drop unregisters a successful subscription.
        unsafe {
            WTSRegisterSessionNotification(self.handle, NOTIFY_FOR_THIS_SESSION)?;
        }
        self.session_registered = true;
        Ok(())
    }

    pub(super) fn show_warning(&mut self, warning: bool) {
        if self.warning_visible == warning {
            return;
        }
        // SAFETY: owned child, shown above identity labels without taking
        // activation from the root window used by the control protocol.
        unsafe {
            let _ = ShowWindow(
                self.warning,
                if warning { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
            let _ = RedrawWindow(
                Some(self.handle),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        }
        self.warning_visible = warning;
    }

    pub(super) fn show_diagnostics(&self, show: bool) {
        // SAFETY: this label is a live child owned by the window on the UI thread.
        unsafe {
            let _ = ShowWindow(
                self.labels[Label::Diagnostics as usize],
                if show { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
        }
    }

    pub(super) fn set_label(&self, label: Label, text: &str) -> WinResult<()> {
        let text = wide(text.as_ref());
        // SAFETY: label is a live child; Windows copies the terminated text.
        unsafe { SetWindowTextW(self.labels[label as usize], PCWSTR(text.as_ptr())) }
    }
}

// Match native single-line ellipsis. Preserve readable text at a minimum size;
// controls retain the full identity string for accessibility and inspection.
fn fitted_font(
    control: HWND,
    width: i32,
    height: i32,
    size: i32,
    weight: i32,
    family: PCWSTR,
) -> WinResult<Font> {
    // SAFETY: the UI thread owns control. The DC is released after the closure on
    // both success and failure. Every selected font is restored before dropping it.
    unsafe {
        let dc = GetDC(Some(control));
        if dc.is_invalid() {
            return Err(windows::core::Error::from_win32());
        }
        let result = (|| {
            let mut text = vec![0; GetWindowTextLengthW(control) as usize + 1];
            let length = GetWindowTextW(control, &mut text) as usize;
            text.truncate(length);
            let minimum = size.clamp(12, 16);
            let mut size = size.max(minimum);
            loop {
                let font = CreateFontW(
                    -size,
                    0,
                    0,
                    0,
                    weight,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    DEFAULT_PITCH.0 as u32,
                    family,
                );
                if font.is_invalid() {
                    return Err(windows::core::Error::from_win32());
                }
                let font = Font(font);
                let previous = SelectObject(dc, font.0.into());
                let mut bounds = RECT {
                    right: width,
                    ..Default::default()
                };
                DrawTextW(
                    dc,
                    &mut text,
                    &mut bounds,
                    DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
                );
                SelectObject(dc, previous);
                if (bounds.bottom <= height && bounds.right <= width) || size <= minimum {
                    return Ok(font);
                }
                size = (size * 9 / 10).max(minimum);
            }
        })();
        ReleaseDC(Some(control), dc);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::{WindowClass, window_proc};
    use super::*;
    use wccs_protocol::control::CONTROL_VERSION;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    #[test]
    fn native_layout_preserves_identity_with_readable_ellipsis() {
        let mut command: Command = serde_json::from_value(serde_json::json!({
            "protocol_version": CONTROL_VERSION, "command_id": "visual-test",
            "request_id": "visual-test", "sequence": 1, "device_id": "test",
            "generation": 1, "device_version": 1, "instance_id": "test",
            "session_key": "test", "action": "enter", "expires_at": 1,
            "identity": {
                "contest_id": "test", "contest_name": "程序设计竞赛 & 现场验证",
                "contest_version": 1, "participant_id": "test", "name": "林".repeat(100),
                "school": "校".repeat(200), "seat_id": "test", "seat_label": "A-023",
                "room_id": "test", "room_version": 1
            }
        }))
        .unwrap();
        // SAFETY: this test thread registers its class and owns all hidden windows.
        // Windows are destroyed before the class guard. No UI is shown or focused.
        let instance = unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            instance
        };
        let _class = WindowClass(instance);
        for (case, (width, height)) in [
            (640, 480),
            (800, 600),
            (1080, 1920),
            (1920, 1080),
            (3840, 2160),
            (640, 480),
        ]
        .into_iter()
        .enumerate()
        {
            if case == 5 {
                command.identity.school = "W".repeat(200);
                command.identity.name = "W".repeat(100);
            }
            let window = NativeWindow::create(
                RECT {
                    right: width,
                    bottom: height,
                    ..Default::default()
                },
                instance,
                0,
                false,
                Some(&command),
                false,
            )
            .unwrap();
            assert!(!window.development);
            assert!(window.exit.is_invalid());
            for (index, expected) in [
                (0, command.identity.contest_name.as_str()),
                (1, command.identity.name.as_str()),
                (2, command.identity.school.as_str()),
                (3, command.identity.seat_label.as_str()),
                (6, ""),
                (7, ""),
            ] {
                // SAFETY: label/font belong to window. Restore the borrowed DC's
                // font before releasing the DC and eventually destroying window.
                unsafe {
                    let label = window.labels[index];
                    let mut text = vec![0; GetWindowTextLengthW(label) as usize + 1];
                    let length = GetWindowTextW(label, &mut text) as usize;
                    text.truncate(length);
                    assert_eq!(String::from_utf16(&text).unwrap(), expected);
                    let mut area = RECT::default();
                    GetClientRect(label, &mut area).unwrap();
                    let dc = GetDC(Some(label));
                    assert!(!dc.is_invalid());
                    let previous = SelectObject(dc, window.fonts[index].0.into());
                    let mut measured = RECT {
                        right: area.right,
                        ..Default::default()
                    };
                    DrawTextW(
                        dc,
                        &mut text,
                        &mut measured,
                        DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
                    );
                    SelectObject(dc, previous);
                    ReleaseDC(Some(label), dc);
                    assert!(
                        measured.bottom <= area.bottom,
                        "height {width}×{height}, label {index}"
                    );
                    let mut font = LOGFONTW::default();
                    assert_ne!(
                        GetObjectW(
                            window.fonts[index].0.into(),
                            std::mem::size_of::<LOGFONTW>() as i32,
                            Some((&mut font as *mut LOGFONTW).cast())
                        ),
                        0
                    );
                    assert!(font.lfHeight.unsigned_abs() >= 12);
                    if measured.right > area.right {
                        assert!(font.lfHeight.unsigned_abs() <= 16);
                        assert_ne!(
                            GetWindowLongW(label, GWL_STYLE) as u32 & SS_ENDELLIPSIS.0,
                            0
                        );
                    }
                }
            }
        }
        let mut preview = NativeWindow::create(
            RECT {
                right: 1920,
                bottom: 1080,
                ..Default::default()
            },
            instance,
            0,
            false,
            None,
            true,
        )
        .unwrap();
        assert!(preview.development);
        assert!(!preview.exit.is_invalid());
        preview.layout().unwrap();
        assert_eq!(preview.fonts.len(), preview.labels.len() + 1);
    }
}
