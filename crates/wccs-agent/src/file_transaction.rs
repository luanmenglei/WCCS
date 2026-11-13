//! Small, durable file replacements used by the fixed preparation templates.
use crate::{host::Result, windows_agent::persist};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use wccs_protocol::digest;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};

#[derive(Deserialize, Serialize)]
pub struct Update {
    pub path: PathBuf,
    pub content: Vec<u8>,
    pub previous: Option<Vec<u8>>,
}
#[derive(Deserialize, Serialize)]
pub struct Journal {
    pub schema: u32,
    pub job_id: String,
    pub phase: String,
    pub completion: String,
    pub files: Vec<Update>,
}
pub fn bytes_digest(bytes: &[u8]) -> String {
    // Packages contain UTF-8, while shortcut transactions may contain binary data.
    wccs_protocol::bytes_digest(bytes)
}
pub fn guard_directory(path: &Path, create: bool) -> Result<Vec<fs::File>> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    if !path
        .to_str()
        .is_some_and(wccs_protocol::preparation::directory)
    {
        return Err("target_path_invalid".into());
    }
    let mut current = PathBuf::new();
    let mut guards = Vec::new();
    for component in path.components() {
        current.push(component);
        if !current.has_root() {
            continue;
        }
        if create && !current.exists() {
            fs::create_dir(&current)?;
        }
        // Directory handles deny concurrent rename, deletion and reparse-point writes.
        // OPEN_REPARSE_POINT opens the entry itself so metadata never follows a junction.
        let file = fs::OpenOptions::new()
            .access_mode(0x80)
            .share_mode(1)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&current)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err("target_reparse_point".into());
        }
        guards.push(file);
    }
    Ok(guards)
}
fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    use std::os::windows::fs::MetadataExt;
    let metadata = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || metadata.len() > 262144
    {
        return Err("target_file_invalid".into());
    }
    Ok(Some(fs::read(path)?))
}
fn save(path: &Path, journal: &Journal) -> Result<()> {
    persist(path, &serde_json::to_vec(journal)?).map_err(|_| "preparation_journal_failed".into())
}
fn replace(path: &Path, bytes: &[u8], job: &str) -> Result<()> {
    let temporary = path.with_extension(format!("wccs-{job}.tmp"));
    // A previous interrupted write owns this exact temporary name. Verify entry type before reuse.
    if read_file(&temporary)?.is_some() {
        fs::remove_file(&temporary)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    use std::os::windows::ffi::OsStrExt;
    let source: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both NUL-terminated paths remain alive. The temporary file is on the
    // same guarded directory and closed before atomic replacement with write-through.
    unsafe {
        windows::Win32::Storage::FileSystem::MoveFileExW(
            windows::core::PCWSTR(source.as_ptr()),
            windows::core::PCWSTR(target.as_ptr()),
            windows::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}
pub fn apply(
    journal_path: &Path,
    job: &str,
    completion: &str,
    files: Vec<(PathBuf, Vec<u8>)>,
    mut authorized: impl FnMut() -> bool,
    verify: impl FnOnce(&[Update]) -> Result<()>,
) -> Result<Journal> {
    if files.is_empty()
        || files.len() > 96
        || files.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() > 2 * 1024 * 1024
    {
        return Err("preparation_output_limit".into());
    }
    let targets: std::collections::HashSet<_> = files
        .iter()
        .map(|(path, _)| path.to_string_lossy().to_ascii_lowercase())
        .collect();
    if targets.len() != files.len() {
        return Err("duplicate_output_path".into());
    }
    let started = Instant::now();
    let mut guards = Vec::new();
    let mut updates = Vec::new();
    for (path, content) in files {
        guards.extend(guard_directory(
            path.parent().ok_or("target_parent_missing")?,
            true,
        )?);
        let previous = read_file(&path)?;
        updates.push(Update {
            path,
            content,
            previous,
        });
    }
    let mut journal = Journal {
        schema: 1,
        job_id: job.into(),
        phase: "prepared".into(),
        completion: completion.into(),
        files: updates,
    };
    save(journal_path, &journal)?;
    let operation = (|| -> Result<()> {
        journal.phase = "applying".into();
        save(journal_path, &journal)?;
        for update in &journal.files {
            if started.elapsed() > Duration::from_secs(60) || !authorized() {
                return Err("preparation_authorization_ended".into());
            }
            if read_file(&update.path)? != update.previous {
                return Err("preparation_file_changed".into());
            }
            replace(&update.path, &update.content, job)?;
            if read_file(&update.path)?.as_deref() != Some(&update.content) {
                return Err("preparation_verify_failed".into());
            }
        }
        verify(&journal.files)?;
        if started.elapsed() > Duration::from_secs(60) || !authorized() {
            return Err("preparation_authorization_ended".into());
        }
        journal.phase = "committed".into();
        save(journal_path, &journal)?;
        Ok(())
    })();
    if operation.is_err() {
        rollback(journal_path, &mut journal).map_err(|_| "rollback_failed")?;
        return Err("rollback_complete".into());
    }
    Ok(journal)
}
pub fn rollback(path: &Path, journal: &mut Journal) -> Result<()> {
    journal.phase = "rolling_back".into();
    save(path, journal)?;
    for update in journal.files.iter().rev() {
        let _guards = guard_directory(update.path.parent().ok_or("target_parent_missing")?, false)?;
        let current = read_file(&update.path)?;
        if current == update.previous {
            continue;
        }
        if current.as_deref() != Some(&update.content) {
            return Err("rollback_external_change".into());
        }
        if let Some(previous) = &update.previous {
            replace(&update.path, previous, &journal.job_id)?;
        } else {
            fs::remove_file(&update.path)?;
        }
    }
    journal.phase = "rolled_back".into();
    save(path, journal)
}
pub fn recover(path: &Path) -> Result<Option<Journal>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("preparation_journal_size".into());
    }
    let mut journal: Journal = serde_json::from_slice(&bytes)?;
    if journal.schema != 1
        || !wccs_protocol::valid_id(&journal.job_id)
        || journal.files.len() > 96
        || journal.files.iter().any(|f| {
            f.content.len() > 262144 || f.previous.as_ref().is_some_and(|b| b.len() > 262144)
        })
        || ![
            "prepared",
            "applying",
            "committed",
            "rolling_back",
            "rolled_back",
        ]
        .contains(&journal.phase.as_str())
        || !["desktop_updated", "scripts_activated"].contains(&journal.completion.as_str())
    {
        return Err("preparation_journal_invalid".into());
    }
    if journal.phase == "committed" {
        for update in &journal.files {
            let _guards =
                guard_directory(update.path.parent().ok_or("target_parent_missing")?, false)?;
            if read_file(&update.path)?.as_deref() != Some(&update.content) {
                return Err("committed_output_changed".into());
            }
        }
    } else if journal.phase != "rolled_back" {
        rollback(path, &mut journal)?;
    }
    Ok(Some(journal))
}
/// Failed terminal receipts remain immutable. Retry their unfinished rollback
/// before accepting another job; never overwrite an externally modified file.
pub fn recover_pending(directory: &Path) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
            return Err("preparation_journal_size".into());
        }
        let journal: Journal = serde_json::from_slice(&fs::read(&path)?)?;
        if !["committed", "rolled_back"].contains(&journal.phase.as_str()) {
            recover(&path)?;
        }
    }
    Ok(())
}
pub fn owned_root(root: &Path, owner: &str) -> Result<(PathBuf, Vec<u8>)> {
    let _guards = guard_directory(root, true)?;
    let marker = root.join(".wccs-owner.json");
    let bytes = serde_json::to_vec(&serde_json::json!({"schema_version":1,"owner":digest(owner)}))?;
    match read_file(&marker)? {
        Some(previous) if previous != bytes => return Err("target_ownership_conflict".into()),
        None if fs::read_dir(root)?.next().is_some() => return Err("target_not_empty".into()),
        None => {
            persist(&marker, &bytes).map_err(|_| "ownership_marker_failed")?;
        }
        _ => {}
    }
    Ok((marker, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_restart_verifies_content_and_failed_verifier_restores_old_bytes() {
        let root =
            std::env::temp_dir().join(format!("wccs-file-commit-{}", wccs_protocol::token()));
        fs::create_dir(&root).unwrap();
        let file = root.join("active.json");
        let state = root.join("journal.json");
        fs::write(&file, b"old").unwrap();
        assert!(
            apply(
                &state,
                "job",
                "scripts_activated",
                vec![(file.clone(), b"new".to_vec())],
                || true,
                |_| Err("account_access_denied".into())
            )
            .is_err()
        );
        assert_eq!(fs::read(&file).unwrap(), b"old");
        apply(
            &state,
            "job",
            "scripts_activated",
            vec![(file.clone(), b"new".to_vec())],
            || true,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(recover(&state).unwrap().unwrap().phase, "committed");
        fs::write(&file, b"external").unwrap();
        assert!(recover(&state).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"external");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn interrupted_apply_rolls_back_and_retry_preserves_external_edits() {
        let root = std::env::temp_dir().join(format!("wccs-files-{}", wccs_protocol::token()));
        fs::create_dir(&root).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        let state = root.join("journal.json");
        fs::write(&a, b"before").unwrap();
        let mut calls = 0;
        assert!(
            apply(
                &state,
                "job",
                "desktop_updated",
                vec![(a.clone(), b"after".to_vec()), (b.clone(), b"new".to_vec())],
                || {
                    calls += 1;
                    calls < 2
                },
                |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(fs::read(&a).unwrap(), b"before");
        assert!(!b.exists());
        assert_eq!(recover(&state).unwrap().unwrap().phase, "rolled_back");
        let mut journal = Journal {
            schema: 1,
            job_id: "job".into(),
            phase: "applying".into(),
            completion: "desktop_updated".into(),
            files: vec![Update {
                path: a.clone(),
                content: b"after".to_vec(),
                previous: Some(b"before".to_vec()),
            }],
        };
        fs::write(&a, b"external").unwrap();
        save(&state, &journal).unwrap();
        assert!(rollback(&state, &mut journal).is_err());
        assert_eq!(fs::read(&a).unwrap(), b"external");
        assert!(recover_pending(&root).is_err());
        fs::write(&a, b"after").unwrap();
        recover_pending(&root).unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"before");
        fs::remove_dir_all(root).unwrap();
    }
}
