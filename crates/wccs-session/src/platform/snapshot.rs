//! 开发预览的运行记录，用于在下次启动时提示上一次异常退出。
use super::wide;
use crate::state::Snapshot;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};
use windows::{
    Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    core::PCWSTR,
};

pub(super) fn recovered_previous_run(directory: &Path) -> io::Result<bool> {
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

pub(super) fn save_snapshot(directory: &Path, snapshot: &Snapshot) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
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
}
