use super::*;

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn recovery_batch_is_not_truncated_by_newer_steps() {
    let fixture = Fixture::new().await;
    let mut socket = socket_for(&fixture.admin, &fixture.credential, "device-1").await;
    poll(&mut socket, &observation()).await.unwrap();
    fixture.db.batch_execute("INSERT INTO maintenance_jobs
        (id,request_id,device_id,command,command_digest,actor_id,actor_version,room_ids,created_at,expires_at,stage,updated_at)
        SELECT 'step-'||sequence, CASE WHEN sequence<=300 THEN 'inspected' ELSE 'archived' END,
        'device-'||(1+(sequence-1)%300),'{\"template\":{\"operation\":\"recovery\",\"options\":{\"contest_id\":\"contest\"}}}',
        'test',actor.id,1,ARRAY['room'],sequence,9999999999,'succeeded',sequence
        FROM generate_series(1,600) sequence CROSS JOIN (SELECT id FROM operators WHERE role='admin' LIMIT 1) actor").await.unwrap();
    let latest = expect_ok(fixture.admin.get("recovery?contest_id=contest").await).await;
    assert_eq!(latest["jobs"].as_array().unwrap().len(), 500);
    let batch = expect_ok(
        fixture
            .admin
            .get("recovery?contest_id=contest&request_id=inspected")
            .await,
    )
    .await;
    let rows = batch["jobs"].as_array().unwrap();
    assert_eq!(rows.len(), 300);
    assert!(rows.iter().all(|job| job["request_id"] == "inspected"));
    socket.close(None).await.unwrap();
    fixture.finish().await;
}

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
        if action != "inspect" {
            let reference = &receipt["preview"]["targets"][0]["recovery"];
            assert_eq!(reference["file_count"], 1);
            assert!(reference["job_id"].is_string());
            assert!(reference.get("manifest").is_none());
        }
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
        let batch = expect_ok(
            f.admin
                .get(&format!(
                    "recovery?contest_id=contest&request_id={}",
                    input["request_id"].as_str().unwrap()
                ))
                .await,
        )
        .await;
        assert_eq!(batch["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(batch["jobs"][0]["request_id"], input["request_id"]);
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

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn recovery_batch_approval_size_is_independent_of_file_lists() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let targets: Vec<_> = (1..=300).map(|n| format!("device-{n}")).collect();
    let mut options = json!({"action":"inspect","contest_id":"contest","scope":"workspace","target":"C:\\Contest","extensions":["cpp"],"skip_reason":"","previous_request":null,"confirm_session_end":false});
    let cmd = json!({"instance_id":"host-one","generation":1,"template":{"operation":"recovery","options":options}});
    let hash = wccs_protocol::bytes_digest(b"");
    let files: Vec<_> = (0..1000)
        .map(|n| json!({"path":format!("{n}.cpp"),"size":0,"hashes":[],"sha256":hash}))
        .collect();
    let result = json!({"stage":"succeeded","recovery":{"target":"C:\\Contest","volume":{"guid":"test","serial":"1","label":"","system":true},"directories":[],"files":files}});
    f.db.execute("INSERT INTO maintenance_jobs(id,request_id,device_id,command,command_digest,actor_id,actor_version,room_ids,created_at,expires_at,stage,updated_at,result)
      SELECT 'inspect-'||device,'previous',device,$2,'synthetic',o.id,1,ARRAY['room'],1,10,'succeeded',1,$3 FROM unnest($1::text[]) device CROSS JOIN operators o WHERE o.role='admin'", &[&targets,&cmd,&result]).await.unwrap();
    f.db.execute("INSERT INTO maintenance_devices SELECT device,'host-one','session-one','local_test',$2,$3 FROM unnest($1::text[]) device", &[&targets,&json!(["recovery-v1"]),&wccs_protocol::now()]).await.unwrap();
    f.db.execute(
        "UPDATE control_devices SET last_seen=$1",
        &[&wccs_protocol::now()],
    )
    .await
    .unwrap();
    f.db.execute("INSERT INTO events(source,source_id,category,occurred_at,operation,device_ids,room_ids,outcome,detail)
        SELECT 'maintenance','large-'||n,'task_result',100,'maintenance_result',ARRAY['device-1'],ARRAY['room'],'succeeded',$1 FROM generate_series(1,50)n", &[&result]).await.unwrap();
    let event_bytes = f
        .admin
        .get("events?operation=maintenance_result")
        .await
        .bytes()
        .await
        .unwrap();
    let events: Value = serde_json::from_slice(&event_bytes).unwrap();
    assert_eq!(events["items"].as_array().unwrap().len(), 50);
    assert!(event_bytes.len() < 100_000);
    options["action"] = json!("archive");
    options["previous_request"] = json!("previous");
    let mut request = json!({"request_id":token(),"template_version":1,"targets":targets,"template":{"operation":"recovery","options":options}});
    let started = Instant::now();
    let approval = async {
        let preview = expect_ok(f.admin.post("maintenance/preview", &request).await).await;
        request["preview_token"] = preview["preview_token"].clone();
        let commit = expect_ok(f.admin.post("maintenance/commit", &request).await).await;
        (preview, commit)
    };
    let heartbeat = async {
        let mut maximum = Duration::ZERO;
        for _ in 0..3 {
            let sample = Instant::now();
            poll(&mut ws, &observation()).await.unwrap();
            maximum = maximum.max(sample.elapsed());
        }
        maximum
    };
    let ((preview, commit), heartbeat_max) = tokio::join!(approval, heartbeat);
    println!("concurrent heartbeat max={heartbeat_max:?}");
    assert_eq!(commit["jobs"].as_array().unwrap().len(), 300);
    assert!(preview.to_string().len() < 500_000 && commit.to_string().len() < 550_000);
    println!(
        "300 devices x 1000 files: preview={} commit={} events={} bytes elapsed={:?}",
        preview.to_string().len(),
        commit.to_string().len(),
        event_bytes.len(),
        started.elapsed()
    );
    // Read a mixed archive through a bounded channel while fetching an independent preview.
    let block = vec![42u8; wccs_protocol::distribution::CHUNK_SIZE];
    let large = block.repeat(16);
    let mut manifest = result["recovery"].clone();
    manifest["files"].as_array_mut().unwrap().push(json!({"path":"large.cpp","size":large.len(),
        "sha256":wccs_protocol::bytes_digest(&large),"hashes":vec![wccs_protocol::bytes_digest(&block);16]}));
    let snapshot =
        json!({"inspection_job":"inspect-device-1","archive_job":null,"manifest":manifest});
    let archive = "inspect-device-1";
    f.db.execute(
        "INSERT INTO recovery_archives VALUES($1,$2,$3,true)",
        &[&archive, &snapshot, &options],
    )
    .await
    .unwrap();
    f.db.execute(
        "INSERT INTO recovery_chunks SELECT $1,1000,n,$2 FROM generate_series(0,15)n",
        &[&archive, &block],
    )
    .await
    .unwrap();
    let start = Instant::now();
    let mut response = f.admin.get(&format!("recovery/archives/{archive}")).await;
    assert_eq!(response.status(), 200);
    let mut bytes = response.chunk().await.unwrap().unwrap().to_vec();
    let first = start.elapsed();
    let preview_started = Instant::now();
    assert_eq!(
        f.admin.post("maintenance/preview", &request).await.status(),
        200
    );
    let preview_elapsed = preview_started.elapsed();
    while let Some(chunk) = response.chunk().await.unwrap() {
        bytes.extend_from_slice(&chunk);
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 1002);
    let mut restored = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("files/large.cpp").unwrap(), &mut restored)
        .unwrap();
    assert_eq!(restored, large);
    println!(
        "1000 empty files + 4 MiB archive: first byte={first:?} simultaneous preview={preview_elapsed:?}"
    );
    ws.close(None).await.unwrap();
    f.finish().await;
}
