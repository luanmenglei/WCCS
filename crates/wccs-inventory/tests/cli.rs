#![cfg(windows)]
#![forbid(unsafe_code)]

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use wccs_inventory::model::{EvidenceContext, Report};

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

#[test]
fn help_and_invalid_arguments_have_no_collection_side_effects() {
    let directory = TestDirectory::new();
    for (arguments, code) in [
        (vec!["--help"], 0),
        (vec!["--context", "invalid"], 2),
        (vec!["--output-dir"], 2),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
            .current_dir(&directory.0)
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(code));
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}

#[test]
fn real_cli_exports_json_and_summary_and_reads_tool_without_executing_it() {
    let directory = TestDirectory::new();
    let tools = directory.0.join("工具 文件");
    fs::create_dir(&tools).unwrap();
    let fake_tool = tools.join("g++.exe");
    let fake_bytes = b"synthetic tool file, deliberately not executable";
    fs::write(&fake_tool, fake_bytes).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-inventory"))
        .current_dir(&directory.0)
        .args(["--tool-dir", tools.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let runs: Vec<_> = fs::read_dir(directory.0.join("local/environment"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(runs.len(), 1);
    assert!(!runs[0].to_string_lossy().ends_with(".partial"));
    let report: Report =
        serde_json::from_slice(&fs::read(runs[0].join("report.json")).unwrap()).unwrap();
    assert_eq!(report.schema_version, 1);
    assert_eq!(report.evidence_context, EvidenceContext::Development);
    assert_eq!(report.collection_mode, "read_only");
    assert!(report.system.version.value().is_some());
    assert!(report.account.elevated.value().is_some());
    let gcc = report
        .tools
        .iter()
        .find(|tool| tool.executable == "g++.exe")
        .unwrap();
    assert!(
        gcc.detection
            .value()
            .unwrap()
            .iter()
            .any(|file| std::path::Path::new(&file.path) == fake_tool)
    );
    assert_eq!(fs::read(&fake_tool).unwrap(), fake_bytes);
    let summary = fs::read_to_string(runs[0].join("summary.txt")).unwrap();
    assert!(summary.contains("开发环境证据"));
    assert!(!summary.contains(tools.to_str().unwrap()));
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
