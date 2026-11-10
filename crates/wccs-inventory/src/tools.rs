#![forbid(unsafe_code)]

use crate::model::{Observation, Reason, Tool, ToolFile, ToolSearchError};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const EXECUTABLES: &[&str] = &[
    "g++.exe",
    "gcc.exe",
    "gdb.exe",
    "clang++.exe",
    "clang.exe",
    "cl.exe",
    "python.exe",
    "java.exe",
    "javac.exe",
    "rustc.exe",
    "cargo.exe",
    "code.exe",
    "devcpp.exe",
    "codeblocks.exe",
    "cmake.exe",
    "compilecpp.exe",
    "submit.exe",
];

pub fn search_directories(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path).filter(|path| path.is_absolute()));
    }
    for root in [
        "C:/MinGW/bin",
        "C:/msys64/mingw64/bin",
        "C:/msys64/ucrt64/bin",
    ] {
        directories.push(PathBuf::from(root));
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            for suffix in [
                "LLVM/bin",
                "Dev-Cpp",
                "Dev-Cpp/MinGW64/bin",
                "CodeBlocks",
                "CodeBlocks/MinGW/bin",
                "Microsoft VS Code",
            ] {
                directories.push(PathBuf::from(&root).join(suffix));
            }
        }
    }
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        directories.push(PathBuf::from(root).join("Programs/Microsoft VS Code"));
    }
    directories.extend_from_slice(extra);
    let mut unique = Vec::new();
    for directory in directories {
        if !unique.contains(&directory) {
            unique.push(directory);
        }
    }
    unique
}

// Injectable file lookup keeps denied/missing paths testable without changing machine ACLs.
pub fn probe_tool(
    executable: &str,
    directories: &[PathBuf],
    inspect: impl Fn(&Path) -> io::Result<bool>,
    version: impl Fn(&Path) -> Observation<String>,
) -> Tool {
    let mut files = Vec::new();
    let mut search_errors = Vec::new();
    for directory in directories {
        let path = directory.join(executable);
        match inspect(&path) {
            Ok(true) => files.push(ToolFile {
                path: path.to_string_lossy().into_owned(),
                file_version: version(&path),
            }),
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => search_errors.push(ToolSearchError {
                directory: directory.to_string_lossy().into_owned(),
                error: Observation::from_io(&error),
            }),
        }
    }
    let detection = if !files.is_empty() {
        Observation::available(files)
    } else if let Some(error) = search_errors.first() {
        match &error.error {
            Observation::Unavailable { reason, code } => Observation::Unavailable {
                reason: reason.clone(),
                code: *code,
            },
            Observation::Available { .. } => unreachable!(),
        }
    } else {
        Observation::unavailable(Reason::NotFoundInSearchScope)
    };
    Tool {
        executable: executable.to_owned(),
        detection,
        search_errors,
    }
}

pub fn inspect_file(path: &Path) -> io::Result<bool> {
    fs::metadata(path).map(|metadata| metadata.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_tool_is_explicit_and_version_is_not_called() {
        let tool = probe_tool(
            "g++.exe",
            &["synthetic".into()],
            |_| Err(io::ErrorKind::NotFound.into()),
            |_| panic!("absent tool"),
        );
        assert!(matches!(
            tool.detection,
            Observation::Unavailable {
                reason: Reason::NotFoundInSearchScope,
                ..
            }
        ));
    }

    #[test]
    fn denied_lookup_is_not_reported_as_absent() {
        let tool = probe_tool(
            "g++.exe",
            &["synthetic".into()],
            |_| Err(io::ErrorKind::PermissionDenied.into()),
            |_| panic!("denied tool"),
        );
        assert!(matches!(
            tool.detection,
            Observation::Unavailable {
                reason: Reason::PermissionDenied,
                ..
            }
        ));
    }

    #[test]
    fn found_tool_survives_missing_version_and_other_denied_directory() {
        let tool = probe_tool(
            "g++.exe",
            &["denied".into(), "中文 空格".into()],
            |path| {
                if path.starts_with("denied") {
                    Err(io::ErrorKind::PermissionDenied.into())
                } else {
                    Ok(true)
                }
            },
            |_| Observation::unavailable(Reason::Missing),
        );
        assert_eq!(tool.detection.value().unwrap().len(), 1);
        assert_eq!(tool.search_errors.len(), 1);
        assert!(
            tool.detection.value().unwrap()[0]
                .file_version
                .value()
                .is_none()
        );
    }
}
