use super::{observation, text};
use crate::model::{Observation, System};
use windows::{
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::*,
        System::{Registry::*, SystemInformation::*},
    },
    core::{PCWSTR, PWSTR, Result as WinResult, w},
};

pub fn collect() -> System {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: info points to writable storage of the exact API structure size.
    unsafe { GetNativeSystemInfo(&mut info) };
    // SAFETY: GetNativeSystemInfo initializes the processor-architecture union arm.
    let architecture = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture };
    let name = match architecture {
        PROCESSOR_ARCHITECTURE_AMD64 => "x86_64",
        PROCESSOR_ARCHITECTURE_INTEL => "x86",
        PROCESSOR_ARCHITECTURE_ARM64 => "aarch64",
        _ => "unknown",
    };
    System {
        host_name: observation(host_name()),
        version: version(),
        edition: observation(registry_string(w!("EditionID"))),
        display_version: observation(registry_string(w!("DisplayVersion"))),
        update_build_revision: observation(revision()),
        native_architecture: name.into(),
        process_architecture: std::env::consts::ARCH.into(),
    }
}

fn host_name() -> WinResult<String> {
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    // SAFETY: buffer and size describe writable UTF-16 storage. 256 accommodates DNS host names.
    unsafe {
        GetComputerNameExW(
            ComputerNameDnsHostname,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut size,
        )?
    };
    Ok(text(&buffer))
}

fn version() -> Observation<String> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: initialized size matches the writable structure. RtlGetVersion is
    // queried directly to avoid manifest-dependent GetVersionEx compatibility values.
    let status = unsafe { RtlGetVersion(&mut version) };
    if status.is_err() {
        return Observation::from_win32(status.0 as u32);
    }
    Observation::available(format!(
        "{}.{}.{}",
        version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
    ))
}

const VERSION_KEY: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");

fn registry_string(name: PCWSTR) -> WinResult<String> {
    let mut buffer = vec![0u16; 1024];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: predefined HKLM handle is borrowed, query is read-only, UTF-16
    // buffer has the declared byte capacity and all strings are NUL-terminated.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            VERSION_KEY,
            name,
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()?
    };
    let value = text(&buffer);
    if value.is_empty() {
        return Err(ERROR_INVALID_DATA.into());
    }
    Ok(value)
}

fn revision() -> WinResult<u32> {
    let mut value = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    // SAFETY: requested DWORD type matches value and its byte size. HKLM is borrowed.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            VERSION_KEY,
            w!("UBR"),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut bytes),
        )
        .ok()?
    };
    Ok(value)
}
