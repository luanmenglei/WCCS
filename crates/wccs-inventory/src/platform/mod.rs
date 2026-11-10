mod account;
mod network;
mod system;
mod volumes;

use crate::{
    model::{EvidenceContext, Observation, Report},
    tools,
};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::Storage::FileSystem::*,
    core::{PCWSTR, Result as WinResult, w},
};

pub fn collect(
    context: EvidenceContext,
    configuration: Option<&tools::ToolConfiguration>,
) -> std::io::Result<Report> {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    let (tool_configuration, tools) =
        tools::check_configuration(configuration, tools::inspect_file, file_version)?;
    Ok(Report {
        schema_version: 3,
        collector_version: env!("CARGO_PKG_VERSION").into(),
        evidence_context: context,
        collected_at_unix_ms: started.as_millis(),
        collection_mode: "read_only".into(),
        system: system::collect(),
        account: account::collect(),
        volumes: volumes::collect(),
        network: observation(network::collect()),
        tool_configuration: Some(tool_configuration),
        tool_search_directories: Vec::new(),
        tools,
    })
}

fn observation<T>(result: WinResult<T>) -> Observation<T> {
    match result {
        Ok(value) => Observation::available(value),
        Err(error) => {
            let hresult = error.code().0 as u32;
            let code = if hresult & 0xffff0000 == 0x80070000 {
                hresult & 0xffff
            } else {
                hresult
            };
            Observation::from_win32(code)
        }
    }
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn text(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

// Windows variable-length API structures require pointer alignment. u128 storage
// provides 16-byte alignment and remains alive until all embedded pointers are read.
struct Buffer(Vec<u128>);

impl Buffer {
    fn new(bytes: u32) -> WinResult<Self> {
        if bytes == 0 || bytes > 16 * 1024 * 1024 {
            return Err(windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER.into());
        }
        Ok(Self(vec![0; (bytes as usize).div_ceil(16)]))
    }

    fn as_mut_ptr(&mut self) -> *mut std::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
}

pub fn file_version(path: &Path) -> Observation<String> {
    observation(read_file_version(path))
}

fn read_file_version(path: &Path) -> WinResult<String> {
    let path = wide(path.as_os_str());
    // SAFETY: path is NUL-terminated and lives through the call. This API reads a PE resource.
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
    if size == 0 {
        return Err(windows::core::Error::from_win32());
    }
    let mut buffer = Buffer::new(size)?;
    let mut value = std::ptr::null_mut();
    let mut length = 0;
    // SAFETY: aligned buffer has at least size writable bytes. VerQueryValueW returns
    // a borrowed pointer into that buffer, consumed below before buffer is dropped.
    unsafe {
        GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, buffer.as_mut_ptr())?;
        // VerQueryValueW does not promise to set last-error on failure.
        if !VerQueryValueW(buffer.as_mut_ptr(), w!("\\"), &mut value, &mut length).as_bool() {
            return Err(windows::Win32::Foundation::ERROR_RESOURCE_DATA_NOT_FOUND.into());
        }
    }
    if value.is_null() || (length as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return Err(windows::Win32::Foundation::ERROR_INVALID_DATA.into());
    }
    // SAFETY: API returned a VS_FIXEDFILEINFO block with sufficient length. Use
    // unaligned read because a resource's internal layout need not align Rust types.
    let info = unsafe { value.cast::<VS_FIXEDFILEINFO>().read_unaligned() };
    if info.dwSignature != 0xfeef04bd {
        return Err(windows::Win32::Foundation::ERROR_INVALID_DATA.into());
    }
    Ok(format!(
        "{}.{}.{}.{}",
        info.dwFileVersionMS >> 16,
        info.dwFileVersionMS & 0xffff,
        info.dwFileVersionLS >> 16,
        info.dwFileVersionLS & 0xffff
    ))
}
