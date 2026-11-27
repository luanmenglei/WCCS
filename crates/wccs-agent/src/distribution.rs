//! Resumable data transfer worker, isolated from the control lease and native session thread.
use crate::{connection, file_transaction, host::Result, identity::Identity, local_state::persist};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use wccs_protocol::{
    bytes_digest, digest,
    distribution::{CHUNK_SIZE, Command, Manifest, Progress},
};

#[derive(Deserialize)]
struct Reply {
    jobs: Vec<Command>,
}
#[derive(Deserialize, Serialize)]
struct Receipt {
    binding: String,
    progress: Progress,
}

pub fn diagnostic_summary(path: &Path, identity: &Identity) -> Vec<serde_json::Value> {
    let Ok(server) = connection::server(path, identity) else {
        return vec![];
    };
    let binding = format!("{server}:{}", connection::binding(identity));
    let Ok(entries) = fs::read_dir(path.with_extension("distribution")) else {
        return vec![];
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let bytes = read_regular(&path, 16384).ok()??;
            let receipt: Receipt = serde_json::from_slice(&bytes).ok()?;
            (receipt.binding == binding && receipt.progress.valid()).then(|| serde_json::json!({
            "job_id":receipt.progress.job_id,"stage":receipt.progress.stage,
            "verified_chunks":receipt.progress.verified_chunks,"detail":receipt.progress.detail,
            "server_acknowledged":false
        }))
        })
        .take(32)
        .collect()
}

fn read_regular(path: &Path, maximum: u64) -> Result<Option<Vec<u8>>> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() > maximum
        || metadata.file_attributes()
            & windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT.0
            != 0
    {
        return Err("target_reparse_point".into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}
fn cache_root(path: &Path, binding: &str, manifest: &Manifest) -> PathBuf {
    path.with_extension("distribution")
        .join(digest(binding))
        .join(&manifest.id)
}
fn cached(root: &Path, manifest: &Manifest, index: usize) -> Result<bool> {
    Ok(
        read_regular(&root.join(format!("{index}.chunk")), CHUNK_SIZE as u64)?.is_some_and(
            |bytes| {
                Some(bytes.len()) == manifest.chunk_length(index)
                    && bytes_digest(&bytes) == manifest.hashes[index]
            },
        ),
    )
}
fn store_chunk(root: &Path, index: usize, bytes: &[u8]) -> Result<()> {
    let _guards = file_transaction::guard_directory(root, true)?;
    let target = root.join(format!("{index}.chunk"));
    if read_regular(&target, CHUNK_SIZE as u64)?.is_some() {
        fs::remove_file(&target)?;
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn verify_file(path: &Path, manifest: &Manifest) -> Result<bool> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() != manifest.size
        || metadata.file_attributes()
            & windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT.0
            != 0
    {
        return Ok(false);
    }
    for (index, expected) in manifest.hashes.iter().enumerate() {
        let mut bytes = vec![0; manifest.chunk_length(index).ok_or("manifest_invalid")?];
        file.read_exact(&mut bytes)?;
        if bytes_digest(&bytes) != *expected {
            return Ok(false);
        }
    }
    Ok(true)
}
fn publish(path: &Path, binding: &str, command: &Command, test_mode: bool) -> Result<()> {
    let parent = if test_mode {
        path.with_extension("distribution-output")
    } else {
        PathBuf::from(&command.directory)
    };
    let root = parent.join("WCCS-resources");
    let _guards = file_transaction::guard_directory(&root, true)?;
    file_transaction::owned_root(&root, &command.device_id)?;
    let version = root.join(&command.manifest.id);
    let _version_guards = file_transaction::guard_directory(&version, true)?;
    file_transaction::owned_root(&version, &command.device_id)?;
    let target = version.join(&command.manifest.name);
    if fs::symlink_metadata(&target).is_ok() {
        return if verify_file(&target, &command.manifest)? {
            Ok(())
        } else {
            Err("file_conflict".into())
        };
    }
    let temporary = version.join(format!(".{}.partial", command.job_id));
    if fs::symlink_metadata(&temporary).is_ok() {
        // Only this job's regular temporary file may be replaced after an interrupted publication.
        use std::os::windows::fs::MetadataExt;
        let metadata = fs::symlink_metadata(&temporary)?;
        if !metadata.is_file()
            || metadata.file_attributes()
                & windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT.0
                != 0
        {
            return Err("target_reparse_point".into());
        }
        fs::remove_file(&temporary)?;
    }
    let cache = cache_root(path, binding, &command.manifest);
    let _cache_guards = file_transaction::guard_directory(&cache, false)?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    for (index, expected) in command.manifest.hashes.iter().enumerate() {
        let bytes = read_regular(&cache.join(format!("{index}.chunk")), CHUNK_SIZE as u64)?
            .ok_or("cache_corrupt")?;
        if bytes_digest(&bytes) != *expected
            || Some(bytes.len()) != command.manifest.chunk_length(index)
        {
            return Err("cache_corrupt".into());
        }
        output.write_all(&bytes)?;
    }
    output.sync_all()?;
    drop(output);
    if !verify_file(&temporary, &command.manifest)? {
        return Err("cache_corrupt".into());
    }
    publish_verified(&temporary, &target, &command.manifest)
}

fn publish_verified(temporary: &Path, target: &Path, manifest: &Manifest) -> Result<()> {
    // Rename without replacement keeps both existing contestant files and old published versions.
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW},
        core::PCWSTR,
    };
    let wide_path = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let source = wide_path(temporary);
    let destination = wide_path(target);
    // SAFETY: both paths remain terminated and live; omission of REPLACE_EXISTING
    // makes publication fail atomically if another writer created the destination.
    if let Err(error) = unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    } {
        if fs::symlink_metadata(target).is_ok() {
            return if verify_file(target, manifest)? {
                fs::remove_file(temporary)?;
                Ok(())
            } else {
                Err("file_conflict".into())
            };
        }
        return Err(error.into());
    }
    Ok(())
}
fn report(
    client: &Client,
    server: &str,
    identity: &Identity,
    credential: &str,
    receipt: &Receipt,
) -> Result<()> {
    let result: serde_json::Value = client
        .post(format!("{server}/api/device/distribution/progress"))
        .bearer_auth(credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&receipt.progress)
        .send()?
        .error_for_status()?
        .json()?;
    if result["accepted"] != true || result["job_id"] != receipt.progress.job_id {
        return Err("distribution_ack_mismatch".into());
    }
    Ok(())
}
fn save_result(
    path: &Path,
    binding: &str,
    command: &Command,
    stage: &str,
    count: usize,
    detail: &str,
) -> Result<Receipt> {
    let receipt = Receipt {
        binding: binding.into(),
        progress: Progress {
            job_id: command.job_id.clone(),
            stage: stage.into(),
            verified_chunks: count,
            detail: detail.into(),
        },
    };
    persist(
        &path
            .with_extension("distribution")
            .join(format!("{}.json", command.job_id)),
        &serde_json::to_vec(&receipt)?,
    )
    .map_err(|_| "distribution_receipt_failed")?;
    Ok(receipt)
}
pub fn worker(path: &Path, test_mode: bool, stop: Arc<AtomicBool>) {
    let Ok(client) = Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    let mut cursors = std::collections::HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        match cycle(&client, path, test_mode, &stop, &mut cursors) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                crate::host::log_error(path, "distribution_sync_unavailable", error.as_ref())
            }
        }
        for _ in 0..10 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
pub fn simulate_once(path: &Path) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    cycle(
        &client,
        path,
        true,
        &AtomicBool::new(false),
        &mut std::collections::HashMap::new(),
    )?;
    Ok(())
}
fn cycle(
    client: &Client,
    path: &Path,
    test_mode: bool,
    stop: &AtomicBool,
    cursors: &mut std::collections::HashMap<String, usize>,
) -> Result<bool> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let credential = crate::identity::credential(path, &identity)
        .map_err(|_| "distribution_credential_unavailable")?;
    let server =
        connection::server(path, &identity).map_err(|_| "distribution_server_unavailable")?;
    let binding = format!("{server}:{}", connection::binding(&identity));
    let directory = path.with_extension("distribution");
    let _guards = file_transaction::guard_directory(&directory, true)?;
    let mut pending = std::collections::HashSet::new();
    for entry in fs::read_dir(&directory)? {
        let receipt_path = entry?.path();
        if receipt_path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = receipt_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let bytes = match read_regular(&receipt_path, 16384) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(_) => {
                pending.insert(id.to_owned());
                eprintln!("distribution_receipt_unreadable");
                continue;
            }
        };
        let receipt = serde_json::from_slice::<Receipt>(&bytes)
            .ok()
            .filter(|r| r.progress.valid() && r.progress.job_id == id);
        let quarantine = match receipt {
            Some(receipt) if receipt.binding == binding => {
                match report(client, &server, &identity, &credential, &receipt) {
                    Ok(()) => {
                        if fs::remove_file(&receipt_path).is_err() {
                            pending.insert(id.to_owned());
                        }
                        false
                    }
                    Err(error)
                        if error
                            .downcast_ref::<reqwest::Error>()
                            .and_then(|e| e.status())
                            .is_some_and(|s| {
                                matches!(s.as_u16(), 400 | 403 | 404 | 409 | 410 | 422)
                            }) =>
                    {
                        true
                    }
                    Err(_) => {
                        pending.insert(id.to_owned());
                        false
                    }
                }
            }
            _ => true,
        };
        if quarantine {
            let destination =
                receipt_path.with_extension(format!("{}.rejected", wccs_protocol::token()));
            if fs::rename(&receipt_path, destination).is_err() {
                pending.insert(id.to_owned());
            }
            eprintln!("distribution_receipt_rejected");
        }
    }
    let adapter = if test_mode { "simulation" } else { "service" };
    let reply: Reply = client
        .post(format!("{server}/api/device/distribution/poll"))
        .bearer_auth(&credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&serde_json::json!({"adapter":adapter}))
        .send()?
        .error_for_status()?
        .json()?;
    cursors.retain(|id, _| reply.jobs.iter().any(|job| job.job_id == *id));
    let mut downloading = false;
    for command in reply.jobs {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if command.device_id != identity.device_id
            || command.adapter != adapter
            || !wccs_protocol::valid_id(&command.job_id)
            || !command.manifest.valid()
            || !wccs_protocol::preparation::directory(&command.directory)
            || !(65536..=16777216).contains(&command.bytes_per_second)
        {
            return Err("distribution_command_invalid".into());
        }
        if pending.contains(&command.job_id) {
            continue;
        }
        let cache = cache_root(path, &binding, &command.manifest);
        let operation = (|| -> Result<(usize, bool)> {
            let _cache_guards = file_transaction::guard_directory(&cache, true)?;
            let mut verified = *cursors.get(&command.job_id).unwrap_or(&0);
            while verified < command.manifest.hashes.len()
                && cached(&cache, &command.manifest, verified)?
            {
                verified += 1;
            }
            if verified == command.manifest.hashes.len() {
                if command.publish {
                    publish(path, &binding, &command, test_mode)?;
                }
                return Ok((verified, command.publish));
            }
            Ok((verified, false))
        })();
        let (mut verified, published) = match operation {
            Ok(value) => value,
            Err(error) => {
                let detail = match error.to_string().as_str() {
                    "target_reparse_point" => "target_reparse_point",
                    "file_conflict" => "file_conflict",
                    "cache_corrupt" => "cache_corrupt",
                    _ => "transfer_failed",
                };
                save_result(path, &binding, &command, "failed", 0, detail)?;
                continue;
            }
        };
        if verified < command.manifest.hashes.len() {
            downloading = true;
            let _cache_guards = file_transaction::guard_directory(&cache, false)?;
            let index = verified;
            let mut response = client
                .get(format!(
                    "{server}/api/device/distribution/{}/{index}",
                    command.job_id
                ))
                .bearer_auth(&credential)
                .header("X-WCCS-Device-ID", &identity.device_id)
                .send()?
                .error_for_status()?;
            let mut bytes = Vec::new();
            response
                .by_ref()
                .take(CHUNK_SIZE as u64 + 1)
                .read_to_end(&mut bytes)?;
            if Some(bytes.len()) != command.manifest.chunk_length(index)
                || bytes_digest(&bytes) != command.manifest.hashes[index]
            {
                return Err("distribution_chunk_corrupt".into());
            }
            if store_chunk(&cache, index, &bytes).is_err() {
                save_result(
                    path,
                    &binding,
                    &command,
                    "failed",
                    verified,
                    "transfer_failed",
                )?;
                continue;
            }
            verified += 1;
        }
        cursors.insert(command.job_id.clone(), verified);
        let (stage, detail) = if published {
            ("published", "file_published")
        } else if verified == command.manifest.hashes.len() {
            ("ready", "awaiting_publication")
        } else {
            ("downloading", "chunks_verified")
        };
        let receipt = save_result(path, &binding, &command, stage, verified, detail)?;
        if report(client, &server, &identity, &credential, &receipt).is_err() {
            eprintln!("distribution_receipt_pending");
        }
    }
    Ok(downloading)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_receipts_redact_binding_and_reject_unknown_reason() {
        let root = std::env::temp_dir().join(format!(
            "wccs-distribution-diagnostic-{}",
            wccs_protocol::token()
        ));
        let path = root.join("identity.json");
        let identity:Identity=serde_json::from_value(serde_json::json!({"server":"https://example.test","device_id":"device","enrollment_token":"SECRET_ENROLLMENT","device_token":"SECRET_DEVICE"})).unwrap();
        let directory = path.with_extension("distribution");
        fs::create_dir_all(&directory).unwrap();
        let binding = format!("https://example.test:{}", connection::binding(&identity));
        let receipt = serde_json::json!({"binding":binding,"progress":{"job_id":"job","stage":"failed","verified_chunks":1,"detail":"transfer_failed"}});
        fs::write(directory.join("job.json"), receipt.to_string()).unwrap();
        let summary = serde_json::json!(diagnostic_summary(&path, &identity)).to_string();
        assert!(summary.contains("transfer_failed"));
        assert!(!summary.contains("SECRET"));
        assert!(!summary.contains("example.test"));
        assert!(!summary.contains(&binding));
        let mut malicious = receipt;
        malicious["progress"]["detail"] = serde_json::json!("SECRET_FROM_ERROR");
        fs::write(directory.join("job.json"), malicious.to_string()).unwrap();
        assert!(diagnostic_summary(&path, &identity).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publication_reuses_verified_chunks_and_preserves_changed_files() {
        let root =
            std::env::temp_dir().join(format!("wccs-distribution-{}", wccs_protocol::token()));
        fs::create_dir(&root).unwrap();
        let identity = root.canonicalize().unwrap().join("identity.json");
        let content = b"contest resource";
        let command = Command {
            job_id: "job".into(),
            device_id: "device".into(),
            generation: 1,
            manifest: Manifest {
                id: "artifact".into(),
                name: "资料.txt".into(),
                size: content.len() as u64,
                hashes: vec![bytes_digest(content)],
            },
            directory: "C:\\unused-native-target".into(),
            publish: true,
            bytes_per_second: 65536,
            adapter: "simulation".into(),
        };
        let cache = cache_root(&identity, "binding", &command.manifest);
        store_chunk(&cache, 0, content).unwrap();
        assert!(cached(&cache, &command.manifest, 0).unwrap());
        publish(&identity, "binding", &command, true).unwrap();
        publish(&identity, "binding", &command, true).unwrap();
        let target = identity
            .with_extension("distribution-output")
            .join("WCCS-resources/artifact/资料.txt");
        assert_eq!(fs::read(&target).unwrap(), content);
        fs::write(&target, b"contestant edit").unwrap();
        let partial = target.with_extension("partial");
        fs::write(&partial, content).unwrap();
        assert!(publish_verified(&partial, &target, &command.manifest).is_err());
        assert_eq!(fs::read(&partial).unwrap(), content);
        assert!(publish(&identity, "binding", &command, true).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"contestant edit");
        fs::remove_dir_all(root).unwrap();
    }
}
