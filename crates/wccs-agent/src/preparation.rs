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
    known_folder(&FOLDERID_Desktop, Some(token))
}
pub(crate) fn known_folder(id: &windows::core::GUID, token: Option<HANDLE>) -> Result<PathBuf> {
    // SAFETY: borrowed logon token is live. Shell allocates the returned string with CoTaskMemAlloc.
    unsafe {
        let text = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, token)?;
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
    audit_script_tree(path, token)
}

fn audit_script_tree(root: &Path, token: HANDLE) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let mut duplicate = HANDLE::default();
    // SAFETY: the logon token is borrowed for this call; Handle owns the duplicate.
    unsafe {
        DuplicateToken(token, SecurityImpersonation, &mut duplicate)?;
    }
    let token = login_windows::Handle(duplicate);
    for ancestor in root.ancestors().skip(1) {
        check_script_acl(ancestor, token.0, false)?;
    }
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err("target_reparse_point".into());
        }
        check_script_acl(&path, token.0, true)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

fn check_script_acl(path: &Path, token: HANDLE, inside: bool) -> Result<()> {
    let name = wccs_session::ipc::wide(path);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the terminated path, impersonation token and output buffers live through both calls.
    // LocalFree releases the security descriptor even when AccessCheck returns an error.
    unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
        .ok()?;
        let result = check_script_descriptor(descriptor, token, inside);
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        result
    }
}

fn check_script_descriptor(
    descriptor: PSECURITY_DESCRIPTOR,
    token: HANDLE,
    inside: bool,
) -> Result<()> {
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ.0,
        GenericWrite: FILE_GENERIC_WRITE.0,
        GenericExecute: FILE_GENERIC_EXECUTE.0,
        GenericAll: FILE_ALL_ACCESS.0,
    };
    let mut privileges = PRIVILEGE_SET::default();
    let mut length = size_of::<PRIVILEGE_SET>() as u32;
    let mut granted = 0;
    let mut allowed = windows::core::BOOL::default();
    const MAXIMUM_ALLOWED: u32 = 0x02000000;
    // SAFETY: the caller owns the live descriptor and impersonation token.
    // All result buffers are initialized, and an undersized privilege buffer fails closed.
    unsafe {
        AccessCheck(
            descriptor,
            token,
            MAXIMUM_ALLOWED,
            &mapping,
            Some(&mut privileges),
            &mut length,
            &mut granted,
            &mut allowed,
        )?;
    }
    let mut unsafe_rights =
        DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | FILE_WRITE_ATTRIBUTES;
    if inside {
        unsafe_rights |= FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA;
    }
    if allowed.as_bool() && granted & unsafe_rights.0 != 0 {
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
    shortcut_with_arguments(target, "", temporary)
}
fn shortcut_with_arguments(target: &str, arguments: &str, temporary: &Path) -> Result<Vec<u8>> {
    let target = wccs_session::ipc::wide(target);
    let arguments = wccs_session::ipc::wide(arguments);
    let file = wccs_session::ipc::wide(temporary);
    // SAFETY: COM objects stay on this initialized thread. ShellLink copies the strings.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _com = Com;
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target.as_ptr()))?;
        link.SetArguments(PCWSTR(arguments.as_ptr()))?;
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
            for (name, action) in [("恢复后关机", "shutdown"), ("恢复后重启", "restart")]
            {
                updates.push((
                    root.join(format!("{name}.lnk")),
                    shortcut_with_arguments(
                        std::env::current_exe()?
                            .to_str()
                            .ok_or("power_path_invalid")?,
                        &format!("--power {action}"),
                        &directory.join(format!("{}-{action}.lnk", command.job_id)),
                    )?,
                ));
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
            let package_directory = root
                .join("packages")
                .join(&package.id)
                .join(&package.version);
            let version = if profile.competition.is_some() {
                package_directory.join(format!("configuration-{}", snapshot.version))
            } else {
                package_directory
            };
            for file in &package.files {
                let target = version.join(&file.path);
                if target.exists() && fs::read(&target)? != file.content.as_bytes() {
                    return Err("immutable_package_version_conflict".into());
                }
                updates.push((target, file.content.as_bytes().to_vec()));
            }
            let manifest = serde_json::to_vec(package)?;
            updates.push((version.join("manifest.json"), manifest.clone()));
            let mut active = serde_json::json!({"schema_version":1,"id":package.id,"version":package.version,"configuration_version":package.configuration_version,"manifest_sha256":files::bytes_digest(&manifest),"runtime_path":runtime.path,"runtime_file_version":package.runtime_file_version,"directory":version});
            if let Some(competition) = &profile.competition {
                let compiler = snapshot
                    .tools
                    .iter()
                    .find(|tool| tool.id == competition.compiler_tool_id)
                    .ok_or("competition_compiler_missing")?;
                let configuration = serde_json::to_vec(&serde_json::json!({
                    "schema_version": 1, "version": snapshot.version.to_string(),
                    "compiler_path": compiler.path,
                    "compiler_arguments": competition.compiler_arguments,
                    "judge_url": competition.judge_url,
                    "contest_id": competition.contest_id,
                    "language_id": competition.language_id
                }))?;
                let configuration_path = version.join("contest.json");
                if configuration_path.exists() && fs::read(&configuration_path)? != configuration {
                    return Err("immutable_competition_configuration_conflict".into());
                }
                active["configuration_sha256"] =
                    serde_json::json!(files::bytes_digest(&configuration));
                updates.push((configuration_path, configuration));
                updates.push((
                    root.join("contest-launch.ps1"),
                    include_bytes!("../../../scripts/contest-launch.ps1").to_vec(),
                ));
                // Batch only forwards user arguments into an ordinary-account PowerShell process.
                let runtime_path = runtime.path.replace('%', "%%");
                for name in ["compilecpp", "submit"] {
                    let wrapper = format!(
                        "@echo off\r\nsetlocal DisableDelayedExpansion\r\n\"{runtime_path}\" -NoLogo -NoProfile -File \"%~dp0contest-launch.ps1\" {name} %*\r\nexit /b %errorlevel%\r\n"
                    );
                    updates.push((root.join(format!("{name}.cmd")), wrapper.into_bytes()));
                }
                let terminal = format!(
                    "@echo off\r\nsetlocal DisableDelayedExpansion\r\nset \"PATH=%~dp0;%PATH%\"\r\ncd /d \"{}\"\r\n\"{runtime_path}\" -NoLogo -NoProfile -NoExit\r\n",
                    profile.work_directory.replace('%', "%%")
                );
                let terminal_path = root.join("contest-terminal.cmd");
                updates.push((terminal_path.clone(), terminal.into_bytes()));
                let actual_desktop = desktop(account.0)?;
                let _desktop_guards = files::guard_directory(&actual_desktop, false)?;
                if actual_desktop.canonicalize()?
                    != Path::new(&profile.desktop_directory).canonicalize()?
                {
                    return Err("desktop_account_path_mismatch".into());
                }
                let desktop_root = actual_desktop.join("WCCS");
                updates.push(files::owned_root(&desktop_root, &command.device_id)?);
                updates.push((
                    desktop_root.join("比赛终端.lnk"),
                    shortcut(
                        &terminal_path.to_string_lossy(),
                        &directory.join(format!("{}-terminal.lnk", command.job_id)),
                    )?,
                ));
            }
            // The activation pointer is last, after all immutable version files have been verified.
            updates.push((root.join("active.json"), serde_json::to_vec(&active)?));
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
            if matches!(command.template, Template::DeployScripts { .. }) {
                audit_script_tree(Path::new(&profile.script_directory), account.0)?;
            }
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

#[cfg(test)]
mod acl_tests {
    use super::*;
    #[test]
    fn read_only_root_does_not_imply_read_only_children() -> Result<()> {
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        // SAFETY: process pseudo-handle is borrowed; the two returned handles are uniquely owned.
        let account = unsafe {
            let mut process_token = HANDLE::default();
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut process_token,
            )?;
            let process_token = login_windows::Handle(process_token);
            let mut account = HANDLE::default();
            DuplicateToken(process_token.0, SecurityImpersonation, &mut account)?;
            login_windows::Handle(account)
        };
        for (access, inside, expected) in [
            ("FR", true, true),
            ("0x12008b", true, false),
            ("0x1200c9", false, false),
            ("0x160089", true, false),
        ] {
            let sddl =
                wccs_session::ipc::wide(format!("O:S-1-5-21-1-2-3-456G:SYD:(A;;{access};;;WD)"));
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // SAFETY: SDDL is terminated. LocalFree owns the allocated descriptor after success.
            let result = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )?;
                let result = check_script_descriptor(descriptor, account.0, inside);
                let _ = LocalFree(Some(HLOCAL(descriptor.0)));
                result
            };
            assert_eq!(result.is_ok(), expected, "{access}: {result:?}");
        }
        Ok(())
    }
}
