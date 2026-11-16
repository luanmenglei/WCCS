#![cfg(windows)]
#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use wccs_inventory::{
    model::{EvidenceContext, Observation, Reason, Report, ToolReport},
    tools::ToolConfiguration,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wccs-cli-{}-{unique}-中文 空格",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        // Exclusively owned directory created above, containing this test's reports.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_report(root: &Path) -> (Report, String) {
    let runs: Vec<_> = fs::read_dir(root.join("local/environment"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(runs.len(), 1);
    assert!(!runs[0].to_string_lossy().ends_with(".partial"));
    let json = fs::read(runs[0].join("report.json")).unwrap();
    assert!(!String::from_utf8_lossy(&json).contains("tool_search_directories"));
    (
        serde_json::from_slice(&json).unwrap(),
        fs::read_to_string(runs[0].join("summary.txt")).unwrap(),
    )
}

#[test]
fn help_catalog_and_invalid_arguments_have_no_collection_side_effects() {
    let directory = TestDirectory::new();
    for (arguments, code) in [
        (vec!["--help"], 0),
        (vec!["--list-tools"], 0),
        (vec!["--context", "invalid"], 2),
        (vec!["--output-dir"], 2),
        (vec!["--tool-dir", "anything"], 2),
        (vec!["--list-tools", "--tools-config", "anything"], 2),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
            .current_dir(&directory.0)
            .args(&arguments)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(code));
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
        if arguments == ["--list-tools"] {
            let catalog =
                ToolConfiguration::parse(std::str::from_utf8(&result.stdout).unwrap()).unwrap();
            assert!(catalog.tools.iter().all(|tool| tool.path.is_none()));
        }
    }
}

#[test]
fn unconfigured_cli_ignores_path_and_common_directory_hints() {
    let directory = TestDirectory::new();
    for name in ["g++.exe", "submit.py", "compilecpp.cmd"] {
        fs::write(directory.0.join(name), "synthetic tool").unwrap();
    }
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
        .current_dir(&directory.0)
        .env("PATH", &directory.0)
        .env("ProgramFiles", &directory.0)
        .env("LOCALAPPDATA", &directory.0)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (report, summary) = read_report(&directory.0);
    assert!(report.tools.is_empty());
    assert_eq!(
        report.tool_configuration,
        Some(Observation::unavailable(Reason::NotConfigured))
    );
    assert!(summary.contains("工具路径配置 · 未配置"));
}

#[test]
fn real_cli_checks_only_configured_files_and_never_falls_back_or_executes_scripts() {
    let directory = TestDirectory::new();
    let script = directory.0.join("自定义 入口.anything");
    let script_content = "'executed' | Set-Content executed.marker\n";
    fs::write(&script, script_content).unwrap();
    let executable = directory.0.join("重命名 编译器.exe");
    fs::write(&executable, "synthetic PE placeholder").unwrap();
    // A matching filename on PATH must not replace the missing configured file.
    fs::write(directory.0.join("submit.py"), "synthetic alternate").unwrap();
    let config_path = directory.0.join("工具 配置.json");
    let config = serde_json::json!({
        "schema_version": 1, "config_version": "room-a-v7", "tools": [
            {"id": "compilecpp", "name": "编译脚本", "kind": "script", "path": script},
            {"id": "custom-compiler", "name": "编译器", "kind": "executable", "path": executable},
            {"id": "submit", "name": "提交脚本", "kind": "script", "path": directory.0.join("missing/submit.py")},
            {"id": "python", "name": "Python", "kind": "executable", "path": null},
            {"id": "invalid-file", "name": "目录", "kind": "executable", "path": directory.0}
        ]
    });
    fs::write(&config_path, config.to_string()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
        .current_dir(&directory.0)
        .env("PATH", &directory.0)
        .arg("--tools-config")
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (report, summary) = read_report(&directory.0);
    assert_eq!(report.schema_version, 3);
    assert_eq!(report.evidence_context, EvidenceContext::Development);
    assert_eq!(report.collection_mode, "read_only");
    assert_eq!(
        report.tool_configuration,
        Some(Observation::available("room-a-v7".into()))
    );
    assert!(report.system.version.value().is_some());
    assert!(report.account.elevated.value().is_some());
    assert_eq!(report.tools.len(), 5);
    for (index, entry) in report.tools.iter().enumerate() {
        let ToolReport::Configured(tool) = entry else {
            panic!("current report required")
        };
        assert_eq!(tool.tool.id, config["tools"][index]["id"]);
        match index {
            0 => assert_eq!(
                tool.check.value().unwrap().file_version,
                Observation::unavailable(Reason::NotApplicable)
            ),
            1 => assert!(tool.check.value().is_some()),
            _ => {
                let expected = match index {
                    2 => Reason::Missing,
                    3 => Reason::NotConfigured,
                    _ => Reason::NotAFile,
                };
                assert!(
                    matches!(&tool.check, Observation::Unavailable { reason, .. } if *reason == expected)
                );
            }
        }
    }
    assert_eq!(fs::read_to_string(&script).unwrap(), script_content);
    assert!(!directory.0.join("executed.marker").exists());
    assert!(summary.contains("开发环境证据"));
    assert!(!summary.contains(directory.0.to_str().unwrap()));
}

#[test]
fn invalid_or_missing_configuration_fails_before_export() {
    let directory = TestDirectory::new();
    let config_path = directory.0.join("invalid.json");
    for content in [
        None,
        Some("{}"),
        Some(
            r#"{"schema_version":1,"config_version":"v1","tools":[{"id":"submit","name":"submit","kind":"script","path":"submit.py"}]}"#,
        ),
    ] {
        if let Some(content) = content {
            fs::write(&config_path, content).unwrap();
        }
        let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
            .current_dir(&directory.0)
            .arg("--tools-config")
            .arg(&config_path)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(!directory.0.join("local").exists());
    }
}

#[test]
fn system_file_version_matches_manifested_windows_tool() {
    let directory = TestDirectory::new();
    let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let expected = Command::new(&executable)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$v=(Get-Item -LiteralPath $env:WCCS_VERSION_TEST_FILE).VersionInfo; '{0}.{1}.{2}.{3}' -f $v.FileMajorPart,$v.FileMinorPart,$v.FileBuildPart,$v.FilePrivatePart",
        ])
        .env("WCCS_VERSION_TEST_FILE", &executable)
        .output()
        .unwrap();
    assert!(expected.status.success());
    let config = serde_json::json!({
        "schema_version": 1, "config_version": "native-version", "tools": [
            {"id": "powershell", "name": "PowerShell", "kind": "executable", "path": executable}
        ]
    });
    let path = directory.0.join("tools.json");
    fs::write(&path, config.to_string()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
        .current_dir(&directory.0)
        .arg("--tools-config")
        .arg(&path)
        .output()
        .unwrap();
    assert!(result.status.success());
    let (report, _) = read_report(&directory.0);
    let ToolReport::Configured(tool) = &report.tools[0] else {
        panic!("configured tool required");
    };
    assert_eq!(
        tool.check.value().unwrap().file_version.value().unwrap(),
        String::from_utf8(expected.stdout).unwrap().trim()
    );
}

#[test]
fn export_failure_returns_nonzero_without_overwriting_existing_file() {
    let directory = TestDirectory::new();
    let destination = directory.0.join("existing.txt");
    fs::write(&destination, "preserve me").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
        .arg("--output-dir")
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(fs::read_to_string(destination).unwrap(), "preserve me");
}
