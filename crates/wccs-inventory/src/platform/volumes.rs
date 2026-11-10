use super::{observation, text, wide};
use crate::model::{Observation, Volume, VolumeIdentity, VolumeInventory, VolumeSpace};
use windows::{
    Win32::{Foundation::*, Storage::FileSystem::*},
    core::{PCWSTR, Result as WinResult},
};

struct VolumeSearch(HANDLE);
impl Drop for VolumeSearch {
    fn drop(&mut self) {
        // SAFETY: owns a successful FindFirstVolumeW search handle, closed once.
        unsafe {
            let _ = FindVolumeClose(self.0);
        }
    }
}

pub fn collect() -> VolumeInventory {
    let mut inventory = VolumeInventory {
        entries: Vec::new(),
        enumeration: Observation::available(()),
    };
    // Volume GUID names have a documented fixed format well within this buffer.
    let mut name = [0u16; 1024];
    // SAFETY: name is writable UTF-16 storage. The handle is owned by VolumeSearch.
    let search = match unsafe { FindFirstVolumeW(&mut name) } {
        Ok(handle) => VolumeSearch(handle),
        Err(error) => {
            inventory.enumeration = observation(Err(error));
            return inventory;
        }
    };
    loop {
        inventory.entries.push(volume(&text(&name)));
        // SAFETY: search is still live and name has the declared capacity.
        match unsafe { FindNextVolumeW(search.0, &mut name) } {
            Ok(()) => {}
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(error) => {
                // Retain entries gathered before an enumeration failure.
                inventory.enumeration = observation(Err(error));
                break;
            }
        }
    }
    inventory
}

fn volume(name: &str) -> Volume {
    let path = wide(name.as_ref());
    let root = PCWSTR(path.as_ptr());
    // SAFETY: root is a NUL-terminated GUID path valid for this entire function.
    let drive_type = unsafe { GetDriveTypeW(root) };
    Volume {
        volume_guid_path: name.into(),
        mount_paths: observation(mount_paths(root)),
        drive_type,
        identity: observation(identity(root)),
        space: observation(space(root)),
    }
}

fn mount_paths(root: PCWSTR) -> WinResult<Vec<String>> {
    let mut size = 256u32;
    for _ in 0..3 {
        if size > 1024 * 1024 {
            return Err(ERROR_INSUFFICIENT_BUFFER.into());
        }
        let mut buffer = vec![0u16; size as usize];
        // SAFETY: root belongs to volume() and remains live. buffer is writable
        // and the slice communicates its capacity. Returned data is MULTI_SZ.
        let result =
            unsafe { GetVolumePathNamesForVolumeNameW(root, Some(&mut buffer), &mut size) };
        match result {
            Ok(()) => {
                return Ok(buffer
                    .split(|value| *value == 0)
                    .take_while(|part| !part.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect());
            }
            Err(error) if error.code() == ERROR_MORE_DATA.to_hresult() => {}
            Err(error) => return Err(error),
        }
    }
    Err(ERROR_MORE_DATA.into())
}

fn identity(root: PCWSTR) -> WinResult<VolumeIdentity> {
    let mut label = [0u16; 261];
    let mut file_system = [0u16; 261];
    let mut serial = 0;
    let mut flags = 0;
    // SAFETY: root stays live in volume(); all output buffers and DWORD pointers
    // have the exact required capacity. The API only reads volume metadata.
    unsafe {
        GetVolumeInformationW(
            root,
            Some(&mut label),
            Some(&mut serial),
            None,
            Some(&mut flags),
            Some(&mut file_system),
        )?
    };
    Ok(VolumeIdentity {
        label: text(&label),
        serial_number: format!("{serial:08X}"),
        file_system: text(&file_system),
        file_system_flags: flags,
    })
}

fn space(root: PCWSTR) -> WinResult<VolumeSpace> {
    let mut result = VolumeSpace {
        total_bytes: 0,
        free_bytes: 0,
        available_to_caller_bytes: 0,
    };
    // SAFETY: root stays live in volume(); each output points to a writable u64.
    unsafe {
        GetDiskFreeSpaceExW(
            root,
            Some(&mut result.available_to_caller_bytes),
            Some(&mut result.total_bytes),
            Some(&mut result.free_bytes),
        )?
    };
    Ok(result)
}
