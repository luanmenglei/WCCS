use crate::{file_transaction as files, host::Result, login_native, login_windows};
use std::{
    fs,
    path::{Path, PathBuf},
};
use wccs_protocol::{
    maintenance::{Command, Template},
    preparation::{Link, Step},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::Com::*,
        UI::Shell::*,
    },
    core::{Interface, PCWSTR},
};

fn step(phase: &str, path: &Path, detail: &str) -> Step {
    Step {
        phase: phase.into(),
        path: path.to_string_lossy().into(),
        outcome: "succeeded".into(),
        detail: detail.into(),
    }
}
pub fn journal_path(identity: &Path, job: &str) -> PathBuf {
    identity
        .with_extension("preparation")
        .join(format!("{job}.json"))
}
fn desktop(token: HANDLE) -> Result<PathBuf> {
    // SAFETY: borrowed logon token is live. Shell allocates the returned string with CoTaskMemAlloc.
    unsafe {
        let text = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, Some(token))?;
        let result = text.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(text.0.cast()));
        Ok(result?)
    }
}
fn secure_directory(path: &Path, token: HANDLE) -> Result<()> {
    let _parents = files::guard_directory(path.parent().ok_or("scripts_parent_missing")?, true)?;
    if !path.exists() {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let sddl =
            wccs_session::ipc::wide("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)");
        let target = wccs_session::ipc::wide(path);
        // SAFETY: SDDL and target are terminated; LocalFree releases the sole allocated descriptor.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let result = CreateDirectoryW(PCWSTR(target.as_ptr()), Some(&attributes));
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            result?;
        }
    }
    let _guards = files::guard_directory(path, false)?;
    let writable = login_windows::as_user(token, || {
        use std::os::windows::fs::OpenOptionsExt;
        Ok(fs::OpenOptions::new()
            .access_mode(FILE_ADD_FILE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
            .is_ok())
    })?;
    if writable {
        return Err("script_directory_writable_by_contest_account".into());
    }
    Ok(())
}
struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: paired with this thread's successful CoInitializeEx call.
        unsafe {
            CoUninitialize();
        }
    }
}
fn shortcut(target: &str, temporary: &Path) -> Result<Vec<u8>> {
    let target = wccs_session::ipc::wide(target);
    let file = wccs_session::ipc::wide(temporary);
    // SAFETY: COM objects stay on this initialized thread. ShellLink copies the strings.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _com = Com;
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target.as_ptr()))?;
        let persist: IPersistFile = link.cast()?;
        persist.Save(PCWSTR(file.as_ptr()), true)?;
    }
    let bytes = fs::read(temporary)?;
    fs::remove_file(temporary)?;
    Ok(bytes)
}
pub fn execute(
    identity: &Path,
    command: &Command,
    mut authorization: impl FnMut() -> bool,
) -> Result<Vec<Step>> {
    let started = std::time::Instant::now();
    if !wccs_session::ipc::is_system()? {
        return Err("maintenance_requires_system".into());
    }
    let snapshot = command
        .preparation
        .as_ref()
        .ok_or("preparation_snapshot_missing")?;
    if !snapshot.profile.valid() || command.template.profile_version() != Some(snapshot.version) {
        return Err("preparation_version_mismatch".into());
    }
    let (work, account) = login_native::maintenance_account(identity, &command.device_id)?;
    let sid = login_windows::token_sid(account.0)?;
    let mut authorized = || {
        started.elapsed() < std::time::Duration::from_secs(60)
            && authorization()
            && login_native::maintenance_active(
                identity,
                &command.device_id,
                &command.session_key,
                &sid,
            )
    };
    if !authorized() {
        return Err("maintenance_identity_changed".into());
    }
    let profile = &snapshot.profile;
    let _work_guards = files::guard_directory(&work, false)?;
    if work.canonicalize()? != Path::new(&profile.work_directory).canonicalize()? {
        return Err("work_directory_mismatch".into());
    }
    let tool_paths: Vec<_> = snapshot.tools.iter().map(|t| t.path.clone()).collect();
    login_windows::check_access(account.0, &tool_paths, &work)?;
    let mut steps = vec![step(
        "preflight",
        &work,
        "ordinary_account_read_write_verified",
    )];
    if matches!(command.template, Template::CheckWorkspace { .. }) {
        if !authorized() {
            return Err("preparation_authorization_ended".into());
        }
        return Ok(steps);
    }
    let directory = identity.with_extension("preparation");
    fs::create_dir_all(&directory)?;
    let mut updates = Vec::new();
    let completion = match &command.template {
        Template::PrepareDesktop { .. } => {
            let actual_desktop = desktop(account.0)?;
            let _desktop_guards = files::guard_directory(&actual_desktop, false)?;
            if actual_desktop.canonicalize()?
                != Path::new(&profile.desktop_directory).canonicalize()?
            {
                return Err("desktop_account_path_mismatch".into());
            }
            login_windows::check_access(account.0, &tool_paths, &actual_desktop)?;
            let root = actual_desktop.join("WCCS");
            updates.push(files::owned_root(&root, &command.device_id)?);
            for (index, link) in profile.links.iter().enumerate() {
                if !authorized() {
                    return Err("preparation_authorization_ended".into());
                }
                let (extension, bytes) = match link {
                    Link::Website { url, .. } => (
                        "url",
                        format!("[InternetShortcut]\r\nURL={url}\r\n").into_bytes(),
                    ),
                    Link::Tool { tool_id, .. } => {
                        let tool = snapshot
                            .tools
                            .iter()
                            .find(|t| &t.id == tool_id)
                            .ok_or("tool_not_configured")?;
                        (
                            "lnk",
                            shortcut(
                                &tool.path,
                                &directory.join(format!("{}-{index}.lnk", command.job_id)),
                            )?,
                        )
                    }
                    Link::Resource { path, .. } => {
                        login_windows::as_user(account.0, || {
                            fs::metadata(path)?;
                            Ok(())
                        })?;
                        (
                            "lnk",
                            shortcut(
                                path,
                                &directory.join(format!("{}-{index}.lnk", command.job_id)),
                            )?,
                        )
                    }
                };
                updates.push((root.join(format!("{}.{}", link.name(), extension)), bytes));
            }
            "desktop_updated"
        }
        Template::DeployScripts { .. } => {
            let package = profile.package.as_ref().ok_or("script_package_missing")?;
            let runtime = snapshot
                .tools
                .iter()
                .find(|t| t.id == package.runtime_tool_id)
                .ok_or("script_runtime_missing")?;
            if wccs_inventory::file_version(Path::new(&runtime.path)).value()
                != Some(&package.runtime_file_version)
            {
                return Err("script_runtime_version_mismatch".into());
            }
            let root = Path::new(&profile.script_directory);
            secure_directory(root, account.0)?;
            updates.push(files::owned_root(root, &command.device_id)?);
            let version = root
                .join("packages")
                .join(&package.id)
                .join(&package.version);
            for file in &package.files {
                let target = version.join(&file.path);
                if target.exists() && fs::read(&target)? != file.content.as_bytes() {
                    return Err("immutable_package_version_conflict".into());
                }
                updates.push((target, file.content.as_bytes().to_vec()));
            }
            let manifest = serde_json::to_vec(package)?;
            updates.push((version.join("manifest.json"), manifest.clone()));
            // The activation pointer is last, after all immutable version files have been verified.
            updates.push((root.join("active.json"),serde_json::to_vec(&serde_json::json!({"schema_version":1,"id":package.id,"version":package.version,"configuration_version":package.configuration_version,"manifest_sha256":files::bytes_digest(&manifest),"runtime_path":runtime.path,"runtime_file_version":package.runtime_file_version,"directory":version}))?));
            "scripts_activated"
        }
        _ => return Err("preparation_template_required".into()),
    };
    let journal = files::apply(
        &journal_path(identity, &command.job_id),
        &command.job_id,
        completion,
        updates,
        authorized,
        |updates| {
            login_windows::as_user(account.0, || {
                for update in updates {
                    if fs::read(&update.path)? != update.content {
                        return Err("ordinary_account_content_unreadable".into());
                    }
                }
                Ok(())
            })
        },
    )?;
    for file in journal.files {
        steps.push(step("verification", &file.path, "content_verified"));
    }
    Ok(steps)
}
