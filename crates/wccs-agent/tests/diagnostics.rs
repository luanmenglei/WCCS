#![cfg(windows)]
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use std::{fs, process::Command};
use wccs_protocol::token;

#[test]
fn native_recovery_requires_system_before_opening_any_state() {
    if wccs_session::ipc::is_system().unwrap() {
        return;
    }
    let root = std::env::temp_dir().join(format!("wccs-recovery-denied-{}", token()));
    fs::create_dir(&root).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
        .arg("--recover-login-native")
        .arg(root.join("device.json"))
        .arg(root.join("session.exe"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("native_login_requires_system"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir(root).unwrap();
}

#[test]
fn offline_setup_export_is_bounded_redacted_and_keeps_identity() {
    let root = std::env::temp_dir().join(format!("wccs-diagnostic-{}", token()));
    fs::create_dir(&root).unwrap();
    let identity = root.join("device.json");
    let secret = token();
    let bytes =
        serde_json::to_vec(&json!({"server":"","device_id":"offline","enrollment_token":secret}))
            .unwrap();
    fs::write(&identity, &bytes).unwrap();
    let tasks = identity.with_extension("maintenance");
    fs::create_dir(&tasks).unwrap();
    fs::write(tasks.join("job.json"),serde_json::to_vec(&json!({
        "server":"https://example.test","device_binding":wccs_protocol::digest(&format!("offline:{secret}")),
        "acknowledged":false,"command":{"schema_version":1,"job_id":"job","request_id":"request",
        "device_id":"offline","generation":1,"device_version":1,"address_version":1,"session_key":"session",
        "instance_id":"host","expires_at":1,"template":{"operation":"identify","duration_seconds":5},
        "identity":{"contest_id":"contest","contest_name":secret,"contest_version":1,"participant_id":"p",
        "name":secret,"school":secret,"seat_id":"seat","seat_label":"1","room_id":"room","room_version":1}},
        "result":{"schema_version":1,"job_id":"job","command_digest":"0".repeat(64),"stage":"failed","detail":"rollback_failed",
        "steps":[{"phase":"rollback","path":secret,"outcome":"failed","detail":secret}]}
    })).unwrap()).unwrap();
    fs::write(identity.with_extension("host.log"),format!("1 host_started Bearer {secret}\n2 {secret}\n3 control_connection_unavailable token={secret}\n")).unwrap();
    let report = root.join("report.json");
    let result = Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
        .arg("--diagnose-test")
        .arg(&identity)
        .arg(root.join("missing-session.exe"))
        .arg(&report)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = fs::read_to_string(&report).unwrap();
    assert!(!text.contains(&secret));
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["server_acknowledged"], false);
    assert_eq!(value["report"]["evidence_context"], "local_test");
    assert_eq!(value["associated_events"].as_array().unwrap().len(), 2);
    assert_eq!(value["associated_tasks"].as_array().unwrap().len(), 1);
    assert_eq!(value["associated_tasks"][0]["detail"], "rollback_failed");
    let checks = value["report"]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 8);
    assert_eq!(
        checks.iter().find(|c| c["step"] == "websocket").unwrap()["outcome"],
        "pending"
    );
    assert_eq!(fs::read(&identity).unwrap(), bytes);
    let status = Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
        .arg("--setup-status")
        .arg(&identity)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(!String::from_utf8_lossy(&status.stdout).contains(&secret));
    let outbox = identity.with_extension("diagnostics-outbox");
    fs::create_dir(&outbox).unwrap();
    for index in 0..32 {
        fs::write(outbox.join(format!("{index}.json")), b"{}").unwrap();
    }
    // A closed local port models an unreachable configured server without external traffic.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let configuration = root.join("connection-input.json");
    fs::write(
        &configuration,
        serde_json::to_vec(&json!({"schema_version":1,
        "server":format!("http://{address}"),"seat_claim":null}))
        .unwrap(),
    )
    .unwrap();
    assert!(
        Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
            .arg("--configure")
            .arg(&identity)
            .arg(&configuration)
            .arg("0")
            .output()
            .unwrap()
            .status
            .success()
    );
    let full_report = root.join("full-queue.json");
    let full = Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
        .arg("--diagnose-test")
        .arg(&identity)
        .arg(root.join("missing-session.exe"))
        .arg(&full_report)
        .output()
        .unwrap();
    assert!(
        full.status.success(),
        "{}",
        String::from_utf8_lossy(&full.stderr)
    );
    assert!(!fs::read_to_string(full_report).unwrap().contains(&secret));
    assert_eq!(fs::read_dir(outbox).unwrap().count(), 32);
    fs::remove_dir_all(root).unwrap();
}
