use super::{observation, text};
use crate::{
    health::{Display, Health, Memory, Sample},
    model::{Observation, Reason},
};
use windows::{
    Win32::{
        Foundation::FILETIME,
        Graphics::Gdi::*,
        System::{SystemInformation::*, Threading::GetSystemTimes},
        UI::Input::*,
    },
    core::{PCWSTR, Result as WinResult},
};

#[derive(Default)]
pub struct HealthCollector {
    previous_cpu: Option<(u64, u64)>,
}

impl HealthCollector {
    pub fn collect(&mut self) -> Health {
        let cpu = match cpu_times() {
            Ok(current) => {
                let previous = self.previous_cpu.replace(current);
                previous
                    .and_then(|old| crate::health::cpu_percent(old, current))
                    .map(Observation::available)
                    .unwrap_or_else(|| Observation::unavailable(Reason::NotApplicable))
            }
            Err(error) => observation(Err(error)),
        };
        let cpu_percent = Sample::new(cpu);
        let memory = Sample::new(observation(memory()));
        let displays = Sample::new(observation(displays()));
        let (keyboards, mice) = match input_devices() {
            Ok((keyboards, mice)) => (
                Observation::available(keyboards),
                Observation::available(mice),
            ),
            Err(error) => (observation(Err(error.clone())), observation(Err(error))),
        };
        Health {
            // SAFETY: GetTickCount64 takes no pointers and returns monotonic system uptime.
            uptime_seconds: Sample::new(Observation::available(unsafe { GetTickCount64() } / 1000)),
            cpu_percent,
            memory,
            displays,
            keyboards: Sample::new(keyboards),
            mice: Sample::new(mice),
        }
    }
}

fn cpu_times() -> WinResult<(u64, u64)> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: three distinct, initialized FILETIME outputs remain alive for the synchronous API.
    unsafe {
        GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user))?;
    }
    let value = |v: FILETIME| (u64::from(v.dwHighDateTime) << 32) | u64::from(v.dwLowDateTime);
    Ok((value(kernel).saturating_add(value(user)), value(idle)))
}
fn memory() -> WinResult<Memory> {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: dwLength matches the writable structure. The call owns no resources.
    unsafe {
        GlobalMemoryStatusEx(&mut status)?;
    }
    Ok(Memory {
        total_bytes: status.ullTotalPhys,
        available_bytes: status.ullAvailPhys,
        used_percent: status.dwMemoryLoad,
    })
}
fn displays() -> WinResult<Vec<Display>> {
    let mut result = Vec::new();
    for index in 0..64 {
        let mut device = DISPLAY_DEVICEW {
            cb: size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: null selects desktop display adapters, device has the exact size and lifetime.
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
            break;
        }
        if !device
            .StateFlags
            .contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
        {
            continue;
        }
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: DeviceName is API-populated NUL-terminated UTF-16. mode stays writable through the call.
        if !unsafe {
            EnumDisplaySettingsW(
                PCWSTR(device.DeviceName.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut mode,
            )
        }
        .as_bool()
        {
            return Err(windows::core::Error::from_win32());
        }
        result.push(Display {
            name: text(&device.DeviceName),
            description: text(&device.DeviceString),
            width: mode.dmPelsWidth,
            height: mode.dmPelsHeight,
            refresh_hz: mode.dmDisplayFrequency,
        });
    }
    if result.is_empty() {
        return Err(windows::Win32::Foundation::ERROR_NOT_FOUND.into());
    }
    Ok(result)
}
fn input_devices() -> WinResult<(u32, u32)> {
    let mut count = 0;
    // SAFETY: count is a writable output, null asks only for the number of records.
    if unsafe { GetRawInputDeviceList(None, &mut count, size_of::<RAWINPUTDEVICELIST>() as u32) }
        == u32::MAX
    {
        return Err(windows::core::Error::from_win32());
    }
    if count > 1024 {
        return Err(windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER.into());
    }
    let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
    // SAFETY: buffer contains count aligned records. Windows never receives ownership of their borrowed handles.
    let received = unsafe {
        GetRawInputDeviceList(
            Some(devices.as_mut_ptr()),
            &mut count,
            size_of::<RAWINPUTDEVICELIST>() as u32,
        )
    };
    if received == u32::MAX || received as usize > devices.len() {
        return Err(windows::core::Error::from_win32());
    }
    devices.truncate(received as usize);
    Ok((
        devices
            .iter()
            .filter(|d| d.dwType == RIM_TYPEKEYBOARD)
            .count() as u32,
        devices.iter().filter(|d| d.dwType == RIM_TYPEMOUSE).count() as u32,
    ))
}
