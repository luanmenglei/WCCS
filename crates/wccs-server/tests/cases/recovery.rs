use super::*;

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn recovery_v2_negotiates_capacity_and_compact_authorization() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let mut runtime = json!({"schema_version":1,"instance_id":"host-one","session_key":"session-one","adapter":"service","capabilities":["recovery-v1"]});
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    let mut input = json!({"request_id":token(),"template_version":1,"targets":["device-1"],"template":{"operation":"recovery","options":{"action":"inspect","contest_id":"contest","target":"C:\\Contest","scope":"workspace","extensions":["cpp"],"skip_reason":"","previous_request":null,"confirm_session_end":false,"limits":{"max_entries":10000,"archive_bytes":536870912,"timeout_seconds":3600}}}});
    let preview = expect_ok(f.admin.post("maintenance/preview", &input).await).await;
    assert_eq!(preview["targets"][0]["reason"], "unsupported_client");
    runtime["capabilities"] = json!(["recovery-v1", "recovery-v2"]);
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    let preview = expect_ok(f.admin.post("maintenance/preview", &input).await).await;
    assert!(preview["targets"][0]["reason"].is_null());
    input["preview_token"] = preview["preview_token"].clone();
    expect_ok(f.admin.post("maintenance/commit", &input).await).await;
    let command = expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await["command"]
        .clone();
    let digest = wccs_protocol::digest(&command.to_string());
    let lease = expect_ok(
        f.admin
            .device("maintenance/authorize", &f.credential, &runtime)
            .await,
    )
    .await;
    assert_eq!(lease["command_digest"], digest);
    assert!(lease.get("command").is_none());
    let report = json!({"schema_version":1,"job_id":command["job_id"],"command_digest":digest,"stage":"accepted","detail":"persisted"});
    expect_ok(
        f.admin
            .device("maintenance/result", &f.credential, &report)
            .await,
    )
    .await;
    expect_ok(
        f.admin
            .post(
                "maintenance/cancel",
                &json!({"request_id":token(),"job_id":command["job_id"]}),
            )
            .await,
    )
    .await;
    let lease = expect_ok(
        f.admin
            .device("maintenance/authorize", &f.credential, &runtime)
            .await,
    )
    .await;
    assert!(lease["command_digest"].is_null());
    ws.close(None).await.unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn recovery_requires_verified_archive_confirmation_and_frozen_target() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let runtime = json!({"schema_version":1,"instance_id":"host-one","session_key":"session-one","adapter":"service","capabilities":["recovery-v1"]});
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    let bytes = b"int main(){}";
    let hash = wccs_protocol::bytes_digest(bytes);
    let manifest = json!({"target":"C:\\Contest","volume":{"guid":"synthetic-volume","serial":"1234","label":"test","system":true},"files":[{"path":"main.cpp","size":bytes.len(),"sha256":hash,"hashes":[hash]}],"directories":[]});
    let mut options = json!({"action":"inspect","contest_id":"contest","target":"C:\\Contest","scope":"workspace","extensions":["cpp"],"skip_reason":"","previous_request":null,"confirm_session_end":false});
    let mut archive_id = String::new();
    for action in ["inspect", "archive", "clean", "reboot"] {
        options["action"] = json!(action);
        let mut input = json!({"request_id":token(),"template_version":1,"targets":["device-1"],"template":{"operation":"recovery","options":options}});
        if ["clean", "reboot"].contains(&action) {
            assert_eq!(
                f.admin.post("maintenance/preview", &input).await.status(),
                400
            );
            options["confirm_session_end"] = json!(true);
            input["template"]["options"] = options.clone();
            let p = expect_ok(f.admin.post("maintenance/preview", &input).await).await;
            assert_eq!(p["targets"][0]["reason"], "restore_teaching_required");
            f.db.execute("UPDATE control_devices SET actual=jsonb_set(actual,'{account}',$1) WHERE device_id='device-1'",&[&json!({"phase":"teaching","policy":{"state":"ready"}})]).await.unwrap();
        }
        let preview = expect_ok(f.admin.post("maintenance/preview", &input).await).await;
        assert!(preview["targets"][0]["reason"].is_null(), "{preview}");
        input["preview_token"] = preview["preview_token"].clone();
        let receipt = expect_ok(f.admin.post("maintenance/commit", &input).await).await;
        assert_eq!(
            expect_ok(f.admin.post("maintenance/commit", &input).await).await,
            receipt
        );
        let command = expect_ok(
            f.admin
                .device("maintenance/poll", &f.credential, &runtime)
                .await,
        )
        .await["command"]
            .clone();
        let id = command["job_id"].as_str().unwrap();
        let digest = wccs_protocol::digest(&command.to_string());
        let mut report = json!({"schema_version":1,"job_id":id,"command_digest":digest,"stage":"accepted","detail":"persisted"});
        expect_ok(
            f.admin
                .device("maintenance/result", &f.credential, &report)
                .await,
        )
        .await;
        if action == "archive" {
            archive_id = id.into();
            assert_eq!(
                f.admin
                    .device(&format!("recovery/{id}/finish"), &f.credential, &json!({}))
                    .await
                    .status(),
                409
            );
            let send = |data: Vec<u8>| {
                f.admin
                    .client
                    .post(format!("{}/api/device/recovery/{id}/0/0", f.admin.url))
                    .bearer_auth(&f.credential)
                    .body(data)
                    .send()
            };
            assert_eq!(send(b"bad".to_vec()).await.unwrap().status(), 400);
            assert_eq!(send(bytes.to_vec()).await.unwrap().status(), 200);
            assert_eq!(send(bytes.to_vec()).await.unwrap().status(), 200);
            expect_ok(
                f.admin
                    .device(&format!("recovery/{id}/finish"), &f.credential, &json!({}))
                    .await,
            )
            .await;
            let response = f.admin.get(&format!("recovery/archives/{id}")).await;
            assert_eq!(response.status(), 200);
            let body = response.bytes().await.unwrap();
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(body)).unwrap();
            use std::io::Read;
            let mut restored = Vec::new();
            zip.by_name("files/main.cpp")
                .unwrap()
                .read_to_end(&mut restored)
                .unwrap();
            assert_eq!(restored, bytes);
        }
        report["stage"] = json!("succeeded");
        report["detail"] = json!(match action {
            "inspect" => "recovery_inspected",
            "archive" => "archive_verified",
            "clean" => "cleanup_complete",
            _ => "reboot_requested",
        });
        report["recovery"] = manifest.clone();
        let mut changed = report.clone();
        changed["recovery"]["target"] = json!("D:\\Other");
        assert_eq!(
            f.admin
                .device("maintenance/result", &f.credential, &changed)
                .await
                .status(),
            409
        );
        expect_ok(
            f.admin
                .device("maintenance/result", &f.credential, &report)
                .await,
        )
        .await;
        expect_ok(
            f.admin
                .device("maintenance/result", &f.credential, &report)
                .await,
        )
        .await;
        options["previous_request"] = input["request_id"].clone();
        options["confirm_session_end"] = json!(false);
        if action == "clean" {
            f.db.execute(
                "UPDATE control_devices SET actual=$1 WHERE device_id='device-1'",
                &[&observation()["actual"]],
            )
            .await
            .unwrap();
        }
    }
    let listed = expect_ok(f.admin.get("recovery?contest_id=contest").await).await;
    assert_eq!(listed["jobs"].as_array().unwrap().len(), 4);
    assert!(listed["jobs"][0]["result"].get("recovery").is_none());
    assert_eq!(
        expect_ok(f.admin.get("recovery?contest_id=other").await).await["jobs"],
        json!([])
    );
    let page = expect_ok(
        f.admin
            .get(&format!(
                "recovery/manifest/{archive_id}?contest_id=contest"
            ))
            .await,
    )
    .await;
    assert_eq!(page["files"][0]["path"], "main.cpp");
    assert!(
        listed["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| j["reboot_observed"] == false)
    );
    assert_eq!(
        f.admin
            .device(
                &format!("recovery/{archive_id}/finish"),
                &f.credential,
                &json!({})
            )
            .await
            .status(),
        409
    );
    let reboot = listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["template"]["options"]["action"] == "reboot")
        .unwrap()["job_id"]
        .as_str()
        .unwrap();
    ws.close(None).await.unwrap();
    let mut next = socket_for(&f.admin, &f.credential, "device-1").await;
    let mut obs = observation();
    obs["instance_id"] = json!("new-host");
    poll(&mut next, &obs).await.unwrap();
    f.db.execute(
        "UPDATE control_devices SET actual=$1 WHERE device_id='device-1'",
        &[&json!({"state":"desktop","account":{"phase":"teaching","policy":{"state":"ready"}}})],
    )
    .await
    .unwrap();
    let unknown = json!({"sampled_at":1,"observation":{"status":"unavailable","reason":"unsupported","code":null}});
    let mut health = json!({"schema_version":1,"device_id":"device-1","instance_id":"new-host","sequence":1,"collection_ms":20,
        "agent_version":{"sampled_at":1,"observation":{"status":"available","value":"test"}},"deployment_version":unknown,
        "health":{"uptime_seconds":{"sampled_at":1,"observation":{"status":"available","value":1}},"cpu_percent":unknown,"memory":unknown,"displays":unknown,"keyboards":unknown,"mice":unknown}});
    let send = |marker: &str, h: &Value| {
        f.admin
            .client
            .post(format!("{}/api/device/health", f.admin.url))
            .bearer_auth(&f.credential)
            .header("x-wccs-requested-reboot", marker)
            .json(h)
            .send()
    };
    assert_eq!(send("unrelated", &health).await.unwrap().status(), 200);
    assert!(
        f.db.query_one(
            "SELECT reboot_observed_at FROM maintenance_jobs WHERE id=$1",
            &[&reboot]
        )
        .await
        .unwrap()
        .get::<_, Option<i64>>(0)
        .is_none()
    );
    health["sequence"] = json!(2);
    assert_eq!(send(reboot, &health).await.unwrap().status(), 200);
    next.close(None).await.unwrap();
    assert!(
        f.db.query_one(
            "SELECT reboot_observed_at FROM maintenance_jobs WHERE id=$1",
            &[&reboot]
        )
        .await
        .unwrap()
        .get::<_, Option<i64>>(0)
        .is_some()
    );
    f.finish().await;
}
