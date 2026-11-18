//! Post-contest file operations. Simulation is confined to the identity's sibling test directory.
use crate::{file_transaction, host::Result, login_native};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use wccs_protocol::{
    bytes_digest,
    distribution::CHUNK_SIZE,
    maintenance::{Command, Template},
    recovery::{File, MAX_BYTES, MAX_FILES, Manifest, Options, Volume},
};
use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};

pub struct Transfer<'a> {
    pub client: &'a reqwest::blocking::Client,
    pub server: &'a str,
    pub credential: &'a str,
}
struct OpenTree {
    manifest: Manifest,
    files: Vec<fs::File>,
    guards: Vec<fs::File>,
}
fn volume(path: &Path, simulation: bool) -> Result<Volume> {
    if simulation {
        return Ok(Volume {
            guid: "simulation".into(),
            serial: "simulation".into(),
            label: "独立临时目录".into(),
            system: false,
        });
    }
    let windows = std::env::var("SystemRoot")?;
    let target = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let system = windows.to_lowercase();
    for volume in wccs_inventory::volumes().entries {
        if volume.drive_type != 3 {
            continue;
        }
        let mounts = volume
            .mount_paths
            .value()
            .ok_or("recovery_volume_unknown")?;
        if mounts
            .iter()
            .any(|mount| target.starts_with(&mount.to_lowercase()))
        {
            let id = volume.identity.value().ok_or("recovery_volume_unknown")?;
            return Ok(Volume {
                guid: volume.volume_guid_path,
                serial: id.serial_number.clone(),
                label: id.label.clone(),
                system: mounts.iter().any(|m| system.starts_with(&m.to_lowercase())),
            });
        }
    }
    Err("recovery_volume_unknown".into())
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentVolume {
    device_id: String,
    drive: String,
    guid: String,
    serial: String,
}

impl DeploymentVolume {
    fn permits(&self, device: &str, volume: &Volume) -> bool {
        !volume.system
            && self.device_id == device
            && wccs_protocol::recovery::deployment_volume(&self.drive)
            && !self.guid.is_empty()
            && self.guid.eq_ignore_ascii_case(&volume.guid)
            && !self.serial.is_empty()
            && self.serial.eq_ignore_ascii_case(&volume.serial)
    }
}

fn target(
    identity: &Path,
    command: &Command,
    options: &Options,
    simulation: bool,
) -> Result<PathBuf> {
    let target = PathBuf::from(&options.target);
    let _guard = file_transaction::guard_directory(&target, false)?;
    if simulation {
        if options.scope != "workspace"
            || target.canonicalize()?
                != identity.with_extension("recovery-target").canonicalize()?
        {
            return Err("recovery_simulation_target_mismatch".into());
        }
        return Ok(target);
    }
    if !wccs_session::ipc::is_system()? {
        return Err("recovery_requires_system".into());
    }
    let work = login_native::recovery_target(
        identity,
        &command.device_id,
        ["clean", "reboot"].contains(&options.action.as_str()),
    )?;
    if options.scope == "workspace" {
        if target.canonicalize()? != work.canonicalize()? || target.parent().is_none() {
            return Err("recovery_workspace_mismatch".into());
        }
    } else if !wccs_protocol::recovery::deployment_volume(&options.target)
        || target.parent().is_some()
        || volume(&target, false)?.system
    {
        return Err("recovery_system_volume_rejected".into());
    }
    if options.scope == "volume" && options.action != "inspect" {
        let policy = fs::read(identity.with_extension("recovery-volume.json"))
            .map_err(|_| "recovery_volume_not_provisioned")?;
        let policy: DeploymentVolume =
            serde_json::from_slice(&policy).map_err(|_| "recovery_volume_policy_invalid")?;
        if !policy.permits(&command.device_id, &volume(&target, false)?) {
            return Err("recovery_volume_identity_mismatch".into());
        }
    }
    let canonical = target.canonicalize()?;
    for protected in [
        identity.to_path_buf(),
        std::env::current_exe()?,
        PathBuf::from(std::env::var("SystemRoot")?),
    ] {
        if protected.canonicalize()?.starts_with(&canonical) {
            return Err("recovery_protected_path".into());
        }
    }
    Ok(target)
}
fn excluded(options: &Options, relative: &str) -> bool {
    options.scope == "volume"
        && ["$recycle.bin", "system volume information"].contains(&relative.to_lowercase().as_str())
}
fn scan(
    root: &Path,
    options: &Options,
    simulation: bool,
    deleting: bool,
    authorized: &mut impl FnMut() -> bool,
) -> Result<OpenTree> {
    let mut tree = OpenTree {
        manifest: Manifest {
            target: options.target.clone(),
            volume: volume(root, simulation)?,
            files: vec![],
            directories: vec![],
        },
        files: vec![],
        guards: file_transaction::guard_directory(root, false)?,
    };
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0u64;
    let max_entries = options
        .limits
        .as_ref()
        .map_or(MAX_FILES, |limits| limits.max_entries);
    let archive_limit = options
        .limits
        .as_ref()
        .map_or(MAX_BYTES, |limits| limits.archive_bytes);
    while let Some(directory) = pending.pop() {
        if !authorized() {
            return Err("recovery_authorization_ended".into());
        }
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("recovery_filename_invalid")?
                .replace('\\', "/");
            if excluded(options, &relative) {
                continue;
            }
            if !wccs_protocol::recovery::relative(&relative) {
                return Err("recovery_filename_invalid".into());
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                return Err("target_reparse_point".into());
            }
            if tree.manifest.files.len() + tree.manifest.directories.len() >= max_entries {
                return Err("recovery_capacity_exceeded".into());
            }
            if metadata.is_dir() {
                // Ancestors are already retained. Keep one additional guard for this directory.
                tree.guards.push(
                    file_transaction::guard_directory(&path, false)?
                        .pop()
                        .ok_or("recovery_path_invalid")?,
                );
                tree.manifest.directories.push(relative);
                pending.push(path);
                continue;
            }
            let mut file = fs::OpenOptions::new()
                .access_mode(0x80000000 | if deleting { 0x10000 } else { 0 })
                .share_mode(1)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(&path)?;
            let metadata = file.metadata()?;
            let collect = options.limits.is_none() || options.collects(&relative);
            if collect {
                total = total
                    .checked_add(metadata.len())
                    .ok_or("recovery_capacity_exceeded")?;
            }
            if !metadata.is_file()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            {
                return Err("target_reparse_point".into());
            }
            if total > archive_limit {
                return Err("recovery_capacity_exceeded".into());
            }
            use sha2::{Digest, Sha256};
            let mut digest = Sha256::new();
            let mut hashes = vec![];
            let mut remaining = metadata.len();
            while remaining > 0 {
                if !authorized() {
                    return Err("recovery_authorization_ended".into());
                }
                let mut bytes = vec![0; remaining.min(CHUNK_SIZE as u64) as usize];
                file.read_exact(&mut bytes)?;
                remaining -= bytes.len() as u64;
                digest.update(&bytes);
                if collect {
                    hashes.push(bytes_digest(&bytes));
                }
            }
            tree.manifest.files.push(File {
                path: relative,
                size: metadata.len(),
                sha256: format!("{:x}", digest.finalize()),
                hashes,
            });
            tree.files.push(file);
        }
    }
    let mut files: Vec<_> = tree
        .manifest
        .files
        .drain(..)
        .zip(tree.files.drain(..))
        .collect();
    files.sort_by(|left, right| left.0.path.cmp(&right.0.path));
    (tree.manifest.files, tree.files) = files.into_iter().unzip();
    tree.manifest.directories.sort();
    if !tree.manifest.valid_for(options) {
        return Err("recovery_manifest_invalid".into());
    }
    Ok(tree)
}
fn archive_request(
    request: reqwest::blocking::RequestBuilder,
    authorized: &mut impl FnMut() -> bool,
) -> Result<serde_json::Value> {
    let mut delay = std::time::Duration::from_millis(300);
    loop {
        if !authorized() {
            return Err("recovery_authorization_ended".into());
        }
        let response = request
            .try_clone()
            .ok_or("recovery_request_invalid")?
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json());
        match response {
            Ok(value) => return Ok(value),
            Err(error)
                if error.is_timeout()
                    || error.is_connect()
                    || error.is_body()
                    || error
                        .status()
                        .is_some_and(|s| matches!(s.as_u16(), 429 | 502 | 503 | 504)) =>
            {
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_secs(2));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub fn execute(
    identity: &Path,
    command: &Command,
    transfer: Transfer<'_>,
    simulation: bool,
    mut authorized: impl FnMut() -> bool,
) -> Result<(&'static str, Manifest)> {
    let Template::Recovery { options } = &command.template else {
        return Err("recovery_template_required".into());
    };
    if !options.valid() {
        return Err("recovery_options_invalid".into());
    }
    let root = target(identity, command, options, simulation)?;
    let mut tree = scan(
        &root,
        options,
        simulation,
        options.action == "clean",
        &mut authorized,
    )?;
    if options.action == "inspect" {
        return Ok(("recovery_inspected", tree.manifest));
    }
    let expected = &command
        .recovery
        .as_ref()
        .ok_or("recovery_snapshot_missing")?
        .manifest;
    if options.action == "reboot" {
        if !tree.manifest.files.is_empty()
            || !tree.manifest.directories.is_empty()
            || tree.manifest.volume != expected.volume
        {
            return Err("recovery_target_changed".into());
        }
        if !authorized() {
            return Err("recovery_authorization_ended".into());
        }
        if !simulation {
            reboot_boot_marker(true)?;
            crate::windows_agent::persist(
                &identity.with_extension("requested-reboot"),
                command.job_id.as_bytes(),
            )
            .map_err(|_| "recovery_reboot_marker_failed")?;
        }
        request_reboot(identity, &command.job_id, simulation)?;
        return Ok(("reboot_requested", expected.clone()));
    }
    if &tree.manifest != expected {
        return Err("recovery_target_changed".into());
    }
    if options.action == "archive" {
        for (index, (entry, file)) in tree
            .manifest
            .files
            .iter()
            .zip(&mut tree.files)
            .enumerate()
            .filter(|(_, (entry, _))| options.collects(&entry.path))
        {
            file.seek(SeekFrom::Start(0))?;
            for chunk in 0..entry.hashes.len() {
                if !authorized() {
                    return Err("recovery_authorization_ended".into());
                }
                let mut bytes = vec![0; entry.chunk_length(chunk).ok_or("recovery_chunk_invalid")?];
                file.read_exact(&mut bytes)?;
                let request = transfer
                    .client
                    .post(format!(
                        "{}/api/device/recovery/{}/{index}/{chunk}",
                        transfer.server, command.job_id
                    ))
                    .bearer_auth(transfer.credential)
                    .header("X-WCCS-Device-ID", &command.device_id)
                    .timeout(std::time::Duration::from_secs(30))
                    .body(bytes);
                let response = archive_request(request, &mut authorized)?;
                if response["accepted"] != true
                    || response["file_index"] != index
                    || response["chunk_index"] != chunk
                {
                    return Err("recovery_archive_ack_mismatch".into());
                }
            }
        }
        if !authorized() {
            return Err("recovery_authorization_ended".into());
        }
        let request = transfer
            .client
            .post(format!(
                "{}/api/device/recovery/{}/finish",
                transfer.server, command.job_id
            ))
            .bearer_auth(transfer.credential)
            .header("X-WCCS-Device-ID", &command.device_id)
            .timeout(std::time::Duration::from_secs(120));
        let response = archive_request(request, &mut authorized)?;
        if response["verified"] != true || response["job_id"] != command.job_id {
            return Err("recovery_archive_ack_mismatch".into());
        }
        return Ok((
            if options.extensions.is_empty() {
                "archive_skipped"
            } else {
                "archive_verified"
            },
            tree.manifest,
        ));
    }
    if options.action != "clean"
        || !options.confirm_session_end
        || command
            .recovery
            .as_ref()
            .and_then(|s| s.archive_job.as_ref())
            .is_none()
    {
        return Err("recovery_cleanup_unapproved".into());
    }
    for file in tree.files.drain(..) {
        if !authorized() {
            return Err("recovery_authorization_ended".into());
        }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: this exact file was opened with DELETE, without following reparse points,
        // and denies writers/renames. The borrowed handle and fixed-size buffer stay live.
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )?;
        }
    }
    drop(tree.guards);
    let mut directories = tree.manifest.directories.clone();
    directories.sort_by_key(|p| std::cmp::Reverse(p.split('/').count()));
    for relative in directories {
        if !authorized() {
            return Err("recovery_authorization_ended".into());
        }
        let path = root.join(relative);
        let _parents = file_transaction::guard_directory(
            path.parent().ok_or("recovery_path_invalid")?,
            false,
        )?;
        // remove_dir only removes an empty directory or the junction itself. It never traverses it.
        fs::remove_dir(path)?;
    }
    let remaining = scan(&root, options, simulation, false, &mut authorized)?;
    if !remaining.manifest.files.is_empty() || !remaining.manifest.directories.is_empty() {
        return Err("recovery_target_changed".into());
    }
    Ok(("cleanup_complete", tree.manifest))
}
/// A volatile machine key survives service restarts and disappears when the system hive unloads.
pub fn reboot_boot_marker(create: bool) -> Result<bool> {
    use windows::{
        Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*},
        core::w,
    };
    let path = w!("SOFTWARE\\WCCS-RequestedRebootThisBoot");
    let mut key = HKEY::default();
    // SAFETY: fixed machine subkey, live NUL-terminated name, writable output handle.
    // On success this function owns and closes the returned handle exactly once.
    unsafe {
        let status = if create {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                path,
                None,
                None,
                REG_OPTION_VOLATILE,
                KEY_QUERY_VALUE,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, Some(0), KEY_QUERY_VALUE, &mut key)
        };
        if !create && status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        status.ok()?;
        RegCloseKey(key).ok()?;
    }
    Ok(true)
}

fn request_reboot(identity: &Path, job: &str, simulation: bool) -> Result<()> {
    if simulation {
        crate::windows_agent::persist(
            &identity.with_extension("reboot-simulation.json"),
            serde_json::json!({"job_id":job,"reboot_requested":true})
                .to_string()
                .as_bytes(),
        )
        .map_err(|_| "recovery_simulation_write_failed")?;
        return Ok(());
    }
    let _privilege =
        crate::waiting_policy::Privilege::enable(windows::Win32::Security::SE_SHUTDOWN_NAME)?;
    use windows::{
        Win32::System::Shutdown::*,
        core::{PCWSTR, w},
    };
    // SAFETY: local machine only. The caller checked SYSTEM, teaching recovery, exact target,
    // online authorization and explicit session-end confirmation. Applications are not forced closed.
    unsafe {
        InitiateSystemShutdownExW(
            PCWSTR::null(),
            w!("WCCS 赛后恢复，工作人员已确认重启"),
            15,
            false,
            true,
            SHTDN_REASON_FLAG_PLANNED | SHTDN_REASON_MAJOR_APPLICATION,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deployed_volume_requires_matching_device_mount_and_identity() {
        let mut policy = DeploymentVolume {
            device_id: "one".into(),
            drive: "D:\\".into(),
            guid: "GUID".into(),
            serial: "001".into(),
        };
        let mut volume = Volume {
            guid: "guid".into(),
            serial: "001".into(),
            label: String::new(),
            system: false,
        };
        assert!(policy.permits("one", &volume));
        assert!(!policy.permits("two", &volume));
        volume.serial = "002".into();
        assert!(!policy.permits("one", &volume));
        volume.serial = "001".into();
        policy.drive = "E:\\".into();
        assert!(!policy.permits("one", &volume));
        policy.drive = "D:\\".into();
        volume.system = true;
        assert!(!policy.permits("one", &volume));
    }
    #[test]
    fn extended_scan_accepts_large_cleanup_and_more_than_legacy_entries() {
        let root =
            std::env::temp_dir().join(format!("wccs-large-recovery-{}", wccs_protocol::token()));
        fs::create_dir(&root).unwrap();
        let file = fs::File::create(root.join("large.bin")).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        drop(file);
        for index in 0..4100 {
            fs::create_dir(root.join(format!("directory-{index:04}"))).unwrap();
        }
        let mut options = Options {
            limits: Some(wccs_protocol::recovery::Limits {
                max_entries: 5000,
                archive_bytes: MAX_BYTES,
                timeout_seconds: 3600,
            }),
            action: "inspect".into(),
            contest_id: "test".into(),
            target: root.to_string_lossy().into(),
            scope: "workspace".into(),
            extensions: vec!["cpp".into()],
            skip_reason: String::new(),
            previous_request: None,
            confirm_session_end: false,
        };
        let tree = scan(&root, &options, true, false, &mut || true).unwrap();
        assert_eq!(tree.manifest.directories.len(), 4100);
        assert_eq!(tree.manifest.files[0].size, MAX_BYTES + 1);
        assert!(tree.manifest.files[0].hashes.is_empty());
        assert!(fs::write(root.join("large.bin"), b"changed").is_err());
        drop(tree);
        options.limits = None;
        assert!(scan(&root, &options, true, false, &mut || true).is_err());
        options.limits = Some(wccs_protocol::recovery::Limits {
            max_entries: 5000,
            archive_bytes: MAX_BYTES,
            timeout_seconds: 3600,
        });
        options.extensions = vec!["bin".into()];
        assert!(scan(&root, &options, true, false, &mut || true).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_stops_after_authority_loss_and_preserves_remaining_files() {
        let directory =
            std::env::temp_dir().join(format!("wccs-cleanup-{}", wccs_protocol::token()));
        fs::create_dir(&directory).unwrap();
        let identity = directory.join("device.json");
        let root = identity.with_extension("recovery-target");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.cpp"), b"a").unwrap();
        fs::write(root.join("b.cpp"), b"b").unwrap();
        let options = Options {
            limits: None,
            action: "clean".into(),
            contest_id: "test".into(),
            target: root.to_string_lossy().into(),
            scope: "workspace".into(),
            extensions: vec!["cpp".into()],
            skip_reason: String::new(),
            previous_request: Some("archive".into()),
            confirm_session_end: true,
        };
        let manifest = scan(&root, &options, true, false, &mut || true)
            .unwrap()
            .manifest;
        let command:Command=serde_json::from_value(serde_json::json!({"schema_version":1,"job_id":"job","request_id":"request","device_id":"device","generation":1,"device_version":1,"address_version":1,"session_key":"session","instance_id":"instance","expires_at":100,"template":{"operation":"recovery","options":options},"recovery":{"inspection_job":"inspection","archive_job":"archive","manifest":manifest},"identity":{"contest_id":"test","contest_name":"test","contest_version":1,"participant_id":"participant","name":"test","school":"school","seat_id":"seat","seat_label":"1","room_id":"room","room_version":1}})).unwrap();
        let client = reqwest::blocking::Client::new();
        let transfer = Transfer {
            client: &client,
            server: "http://127.0.0.1:1",
            credential: "synthetic",
        };
        let mut calls = 0;
        assert!(
            execute(&identity, &command, transfer, true, || {
                calls += 1;
                calls <= 4
            })
            .is_err()
        );
        assert!(!root.join("a.cpp").exists());
        assert_eq!(fs::read(root.join("b.cpp")).unwrap(), b"b");
        let remaining = scan(&root, &options, true, false, &mut || true)
            .unwrap()
            .manifest;
        assert_eq!(remaining.files.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_holds_files_against_changes_and_rejects_modified_scope() {
        let root = std::env::temp_dir().join(format!("wccs-recovery-{}", wccs_protocol::token()));
        fs::create_dir(&root).unwrap();
        let name = format!("{}.cpp", "汉".repeat(62));
        fs::write(root.join(&name), b"int main(){}").unwrap();
        let options = Options {
            limits: None,
            action: "inspect".into(),
            contest_id: "test".into(),
            target: root.to_string_lossy().into(),
            scope: "workspace".into(),
            extensions: vec!["cpp".into()],
            skip_reason: String::new(),
            previous_request: None,
            confirm_session_end: false,
        };
        let tree = scan(&root, &options, true, false, &mut || true).unwrap();
        assert_eq!(tree.manifest.files.len(), 1);
        assert!(tree.manifest.valid_for(&options));
        assert_eq!(tree.manifest.files[0].path, name);
        assert_eq!(
            tree.manifest.files[0].hashes,
            vec![bytes_digest(b"int main(){}")]
        );
        assert!(fs::write(root.join(&name), b"changed").is_err());
        assert!(fs::rename(root.join(&name), root.join("moved.cpp")).is_err());
        drop(tree);
        assert!(scan(&root, &options, true, false, &mut || false).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
