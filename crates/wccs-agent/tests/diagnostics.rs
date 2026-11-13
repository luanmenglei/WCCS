#![cfg(windows)]
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use std::{fs, process::Command};
use wccs_protocol::token;

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
    fs::remove_dir_all(root).unwrap();
}
