//! Apply the explicitly approved file set once, before acknowledging a contest start.
use crate::{
    file_transaction::guard_directory, host::Result, login_native, login_windows,
    preparation::known_folder,
};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};
use wccs_protocol::{
    control::{Command, DesktopFiles, DesktopMaterial},
    distribution::CHUNK_SIZE,
};
use windows::Win32::{
    Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT},
    UI::Shell::{FOLDERID_Desktop, FOLDERID_Profile, FOLDERID_PublicDesktop},
};

pub struct Worker {
    deadline: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<Result<()>>>,
}
impl Worker {
    pub fn start(path: PathBuf, command: Command, sid: String, lease: u64) -> Self {
        let deadline = Arc::new(AtomicU64::new(
            crate::service::monotonic_ms() + lease * 1000,
        ));
        let expires = deadline.clone();
        let thread = thread::spawn(move || {
            apply(&path, &command, &sid, || {
                if crate::service::monotonic_ms() >= expires.load(Ordering::Acquire) {
                    Err("desktop_authorization_ended".into())
                } else {
                    Ok(())
                }
            })
        });
        Self {
            deadline,
            thread: Some(thread),
        }
    }
    pub fn progress(&mut self, lease: u64) -> Result<bool> {
        let now = crate::service::monotonic_ms();
        if now >= self.deadline.load(Ordering::Acquire) {
            return Err("desktop_authorization_ended".into());
        }
        self.deadline.store(now + lease * 1000, Ordering::Release);
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            return Ok(false);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "desktop_worker_failed")??;
        }
        Ok(true)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.deadline.store(0, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn entries(root: &Path) -> Result<Vec<String>> {
    fs::read_dir(root)?
        .map(|entry| {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| "desktop_name_invalid")?;
            // Validate the enumerated component before using it in a Win32 path.
            if !wccs_protocol::recovery::relative(&name) {
                return Err("desktop_name_invalid".into());
            }
            Ok(name)
        })
        .collect()
}

struct Download {
    client: reqwest::blocking::Client,
    url: String,
    device: String,
    credential: String,
}

impl Download {
    fn open(path: &Path, command: &Command) -> Result<Self> {
        let identity = serde_json::from_slice(&fs::read(path)?)?;
        let server = crate::connection::server(path, &identity).map_err(|e| e.to_string())?;
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(6))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            credential: crate::identity::credential(path, &identity).map_err(|e| e.to_string())?,
            url: format!("{server}/api/device/desktop-files/{}", command.command_id),
            device: command.device_id.clone(),
        })
    }
    fn chunk(
        &self,
        artifact: &str,
        index: usize,
        buffer: &mut [u8],
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        loop {
            check()?;
            let response = self
                .client
                .get(format!("{}/{artifact}/{index}", self.url))
                .bearer_auth(&self.credential)
                .header("X-WCCS-Device-ID", &self.device)
                .send()?;
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                thread::sleep(std::time::Duration::from_millis(
                    100 + rand::random::<u64>() % 400,
                ));
                continue;
            }
            let mut response = response.error_for_status()?;
            response.read_exact(buffer)?;
            if response.read(&mut [0])? != 0 {
                return Err("desktop_chunk_length_invalid".into());
            }
            return Ok(());
        }
    }
}

fn stage_file(
    work: &Path,
    staged: &Path,
    file: &DesktopMaterial,
    download: Option<&Download>,
    check: &impl Fn() -> Result<()>,
) -> Result<()> {
    let version = work.join("WCCS-resources").join(&file.id);
    let mut local = if version.join(&file.name).try_exists()? {
        let guards = guard_directory(&version, false)?;
        let source = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(version.join(&file.name))?;
        let metadata = source.metadata()?;
        if metadata.len() != file.size
            || !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        {
            return Err("desktop_material_changed".into());
        }
        Some((source, guards))
    } else {
        None
    };
    let mut output = fs::File::create_new(staged.join(&file.name))?;
    let mut hashes = String::new();
    let mut buffer = vec![0; CHUNK_SIZE];
    let mut index = 0;
    let mut remaining = file.size;
    while remaining > 0 {
        check()?;
        let n = remaining.min(CHUNK_SIZE as u64) as usize;
        if let Some((source, _)) = &mut local {
            source.read_exact(&mut buffer[..n])?;
        } else {
            download.ok_or("desktop_material_missing")?.chunk(
                &file.id,
                index,
                &mut buffer[..n],
                check,
            )?;
        }
        index += 1;
        output.write_all(&buffer[..n])?;
        hashes.push_str(&wccs_protocol::bytes_digest(&buffer[..n]));
        remaining -= n as u64;
    }
    if wccs_protocol::digest(&hashes) != file.chunks_digest {
        return Err("desktop_material_changed".into());
    }
    output.sync_all()?;
    Ok(())
}

fn move_entry(from: &Path, to: &Path) -> Result<()> {
    use wccs_session::ipc::wide;
    use windows::{
        Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW},
        core::PCWSTR,
    };
    let source = wide(from.to_str().ok_or("desktop_path_invalid")?);
    let target = wide(to.to_str().ok_or("desktop_path_invalid")?);
    // SAFETY: both UTF-16 paths are live and terminated. No replace and no cross-volume copy.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}

fn apply(
    identity: &Path,
    command: &Command,
    sid: &str,
    check: impl Fn() -> Result<()>,
) -> Result<()> {
    let files = command
        .desktop_files
        .as_ref()
        .filter(|f| f.valid())
        .ok_or("desktop_plan_invalid")?;
    let (work, account) = login_native::maintenance_account(identity, &command.device_id)?;
    if login_windows::token_sid(account.0)? != sid
        || !login_native::maintenance_active(
            identity,
            &command.device_id,
            &command.session_key,
            sid,
        )
    {
        return Err("desktop_account_changed".into());
    }
    let private = known_folder(&FOLDERID_Desktop, Some(account.0))?;
    let profile = known_folder(&FOLDERID_Profile, Some(account.0))?;
    let public = known_folder(&FOLDERID_PublicDesktop, None)?;
    let _priv_guard = guard_directory(&private, false)?;
    let _pub_guard = guard_directory(&public, false)?;
    let _profile_guard = guard_directory(&profile, false)?;
    if !private.canonicalize()?.starts_with(profile.canonicalize()?)
        || private.canonicalize()? == profile.canonicalize()?
        || private.canonicalize()?.starts_with(public.canonicalize()?)
        || public.canonicalize()?.starts_with(private.canonicalize()?)
    {
        return Err("desktop_folder_scope_invalid".into());
    }
    let retained = profile.join(format!("WCCS-desktop-{}", wccs_protocol::token()));
    let staged = retained.join("materials");
    let old = retained.join("previous");
    let common_old = identity
        .with_extension("desktop-image")
        .join(wccs_protocol::token());
    let _backup_guard = guard_directory(&common_old, true)?;
    let download = (!files.materials.is_empty())
        .then(|| Download::open(identity, command))
        .transpose()?;
    let _user_guards = login_windows::as_user(account.0, || {
        fs::create_dir(&retained)?;
        let mut guards = guard_directory(&staged, true)?;
        guards.extend(guard_directory(&old, true)?);
        for file in &files.materials {
            stage_file(&work, &staged, file, download.as_ref(), &check)?;
        }
        Ok(guards)
    })?;
    let move_as_owner = |from: &Path, to: &Path, common| {
        if common {
            move_entry(from, to)
        } else {
            login_windows::as_user(account.0, || move_entry(from, to))
        }
    };
    reconcile(
        files,
        Locations {
            private: &private,
            public: &public,
            old: &old,
            common_old: &common_old,
            staged: &staged,
        },
        &check,
        move_as_owner,
    )
}

struct Locations<'a> {
    private: &'a Path,
    public: &'a Path,
    old: &'a Path,
    common_old: &'a Path,
    staged: &'a Path,
}

fn reconcile(
    files: &DesktopFiles,
    dirs: Locations<'_>,
    check: &impl Fn() -> Result<()>,
    move_as_owner: impl Fn(&Path, &Path, bool) -> Result<()>,
) -> Result<()> {
    let Locations {
        private,
        public,
        old,
        common_old,
        staged,
    } = dirs;
    let private_entries = entries(private)?;
    let public_entries = entries(public)?;
    let keep = check_keep(files, &private_entries, &public_entries)?;
    let mut moves = Vec::new();
    for (root, backup, names, common) in [
        (private, old, private_entries, false),
        (public, common_old, public_entries, true),
    ] {
        for name in names {
            if !keep.contains(&name.to_lowercase()) {
                moves.push((root.join(&name), backup.join(name), common));
            }
        }
    }
    for file in &files.materials {
        moves.push((staged.join(&file.name), private.join(&file.name), false));
    }
    let mut completed = 0;
    let result = (|| -> Result<()> {
        for (from, to, common) in &moves {
            check()?;
            move_as_owner(from, to, *common)?;
            completed += 1;
        }
        check()?;
        let actual: Vec<_> = entries(private)?
            .into_iter()
            .chain(entries(public)?)
            .map(|s| s.to_lowercase())
            .collect();
        let expected: HashSet<_> = keep
            .into_iter()
            .chain(files.materials.iter().map(|f| f.name.to_lowercase()))
            .collect();
        if actual.len() != expected.len() || !actual.iter().all(|n| expected.contains(n)) {
            return Err("desktop_contents_changed".into());
        }
        Ok(())
    })();
    if result.is_err() {
        // Undo only this attempt. A colliding or occupied file remains at its recovery location.
        for (from, to, common) in moves[..completed].iter().rev() {
            let _ = move_as_owner(to, from, *common);
        }
    }
    result
}

fn check_keep(
    files: &DesktopFiles,
    private: &[String],
    public: &[String],
) -> Result<HashSet<String>> {
    let names: Vec<_> = private
        .iter()
        .chain(public)
        .map(|s| s.to_lowercase())
        .collect();
    let keep: HashSet<_> = files.keep.iter().map(|s| s.to_lowercase()).collect();
    if keep
        .iter()
        .any(|s| names.iter().filter(|n| *n == s).count() != 1)
    {
        return Err("desktop_image_file_missing_or_ambiguous".into());
    }
    Ok(keep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpublished_file_is_downloaded_and_checked_before_use() -> Result<()> {
        let root = std::env::temp_dir().join(format!("wccs-download-{}", wccs_protocol::token()));
        let staged = root.join("staged");
        fs::create_dir_all(&staged)?;
        let server = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = server.local_addr()?;
        let peer = thread::spawn(move || -> Result<()> {
            use std::io::BufRead;
            for _ in 0..2 {
                let (mut socket, _) = server.accept()?;
                let mut reader = std::io::BufReader::new(&socket);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                assert!(line.starts_with("GET /command/material/0 "));
                loop {
                    line.clear();
                    if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                        break;
                    }
                }
                socket.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\npdf",
                )?;
            }
            Ok(())
        });
        let remote = Download {
            client: reqwest::blocking::Client::new(),
            url: format!("http://{address}/command"),
            device: "synthetic".into(),
            credential: "synthetic".into(),
        };
        let mut file = DesktopMaterial {
            name: "比赛说明.pdf".into(),
            id: "material".into(),
            size: 3,
            chunks_digest: wccs_protocol::digest(&wccs_protocol::bytes_digest(b"pdf")),
        };
        stage_file(&root, &staged, &file, Some(&remote), &|| Ok(()))?;
        assert_eq!(fs::read(staged.join(&file.name))?, b"pdf");
        file.name = "changed.pdf".into();
        file.chunks_digest = "0".repeat(64);
        assert!(stage_file(&root, &staged, &file, Some(&remote), &|| Ok(())).is_err());
        peer.join().unwrap()?;
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn exact_files_on_merged_desktop_and_interrupted_apply() -> Result<()> {
        let root = std::env::temp_dir().join(format!("wccs-desktop-{}", wccs_protocol::token()));
        fs::create_dir(&root)?;
        let paths = ["private", "public", "old", "common-old", "staged"].map(|n| root.join(n));
        for path in &paths {
            fs::create_dir(path)?;
        }
        let [private, public, old, common_old, staged] = &paths;
        let dirs = || Locations {
            private,
            public,
            old,
            common_old,
            staged,
        };
        fs::write(private.join("练习.cpp"), b"original")?;
        fs::write(private.join("说明.pdf"), b"old pdf")?;
        fs::write(public.join("编辑器.lnk"), b"shortcut")?;
        fs::create_dir(public.join("课程"))?;
        fs::write(public.join("课程/unit.txt"), b"lesson")?;
        let material = DesktopMaterial {
            id: "material".into(),
            name: "说明.pdf".into(),
            size: 3,
            chunks_digest: wccs_protocol::digest(&wccs_protocol::bytes_digest(b"pdf")),
        };
        let source = root.join("WCCS-resources/material");
        fs::create_dir_all(&source)?;
        fs::write(source.join("说明.pdf"), b"pdf")?;
        stage_file(&root, staged, &material, None, &|| Ok(()))?;
        let files = DesktopFiles {
            keep: vec!["编辑器.lnk".into()],
            materials: vec![material],
        };
        assert!(files.valid());
        let mut bad = files.clone();
        bad.keep.push("missing".into());
        assert!(reconcile(&bad, dirs(), &|| Ok(()), |a, b, _| move_entry(a, b)).is_err());
        assert!(
            reconcile(&files, dirs(), &|| Ok(()), |a, b, _| {
                if a.parent() == Some(staged.as_path()) {
                    return Err("synthetic_failure".into());
                }
                move_entry(a, b)
            })
            .is_err()
        );
        assert_eq!(fs::read(private.join("说明.pdf"))?, b"old pdf");
        assert!(public.join("课程/unit.txt").is_file());
        reconcile(&files, dirs(), &|| Ok(()), |a, b, _| move_entry(a, b))?;
        assert_eq!(entries(private)?, vec!["说明.pdf"]);
        assert_eq!(fs::read(private.join("说明.pdf"))?, b"pdf");
        assert_eq!(entries(public)?, vec!["编辑器.lnk"]);
        assert_eq!(fs::read(old.join("练习.cpp"))?, b"original");
        assert!(common_old.join("课程/unit.txt").is_file());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
