use super::*;
use std::time::{Duration, Instant};
use waiting_tests::{poll, socket_for};

#[path = "capacity.rs"]
mod capacity;

#[path = "distribution.rs"]
mod distribution;

#[path = "recovery.rs"]
mod recovery;

#[path = "contest_control.rs"]
mod contest_control;

struct Fixture {
    admin: Harness,
    app: wccs_server::App,
    db: tokio_postgres::Client,
    driver: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    schema: String,
    credential: String,
}

impl Fixture {
    async fn new() -> Self {
        let database = std::env::var("WCCS_TEST_DATABASE_URL").unwrap();
        let (db, connection) = tokio_postgres::connect(&database, tokio_postgres::NoTls)
            .await
            .unwrap();
        let driver = tokio::spawn(async move { connection.await.unwrap() });
        let schema = format!("test_database_{}", &token()[..24]);
        db.batch_execute(&format!(
            "CREATE SCHEMA {schema}; SET search_path TO {schema}"
        ))
        .await
        .unwrap();
        let salt = SaltString::encode_b64(b"synthetic-salt-16").unwrap();
        let (mut admin, server, app) = start_with_app(Config {
            domjudge_config: None,
            bind: "127.0.0.1:0".into(),
            origin: String::new(),
            database_url: format!(
                "{database} options='-c search_path={schema} -c statement_timeout=10000'"
            ),
            static_dir: "../../web/dist".into(),
            trusted_proxies: vec![],
            admin_username: "test-admin".into(),
            admin_password_hash: Argon2::default()
                .hash_password(b"synthetic-test-password", &salt)
                .unwrap()
                .to_string(),
        })
        .await;
        admin.login().await;
        // Synthetic capacity data, isolated from every other test and deployment schema.
        db.batch_execute("INSERT INTO rooms VALUES('room','合成机房',20,25,1,'row','top-left',1);
            INSERT INTO seats SELECT 'seat-'||n,'room',(n-1)/25,(n-1)%25,n,
                '10.20.'||((n-1)/250)||'.'||((n-1)%250+1) FROM generate_series(1,500) n;
            INSERT INTO contests VALUES('contest','合成比赛',1,0);
            INSERT INTO participants SELECT 'participant-'||n,'contest',n::text,'选手'||n,'学校'||n
                FROM generate_series(1,500) n;
            INSERT INTO assignments SELECT 'contest','participant-'||n,'seat-'||n FROM generate_series(1,500) n;
            INSERT INTO devices(id,name,enrollment_hash,enrollment_expires,credential_hash,observed_ip,lifecycle,last_seen)
                SELECT 'device-'||number,'设备'||number,'unused',0,'synthetic-hash-'||number,expected_ip,'enabled',0 FROM seats;")
            .await.unwrap();
        let credential = token();
        db.execute(
            "UPDATE devices SET credential_hash=$1 WHERE id='device-1'",
            &[&wccs_protocol::digest(&credential)],
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO control_devices
            SELECT id,'connection-'||id,$1,'host-one','session-one',1,$2,true,$3 FROM devices",
            &[
                &app.server_epoch,
                &wccs_protocol::now(),
                &observation()["actual"],
            ],
        )
        .await
        .unwrap();
        Self {
            admin,
            app,
            db,
            driver,
            server,
            schema,
            credential,
        }
    }

    async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let connected: bool = self
                    .db
                    .query_one(
                        "SELECT connected FROM control_devices WHERE device_id='device-1'",
                        &[],
                    )
                    .await
                    .unwrap()
                    .get(0);
                if !connected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("WebSocket cleanup must finish before removing its schema");
        self.server.abort();
        let _ = self.server.await;
        drop(self.app);
        self.db
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .await
            .unwrap();
        self.driver.abort();
    }
}

fn observation() -> Value {
    json!({"protocol_version":1,"instance_id":"host-one","session_key":"session-one","capabilities":["waiting-v1"],"actual":{"state":"desktop","command_id":null,"session_key":null,"monitors":0,"detail":"standby"},"report":null})
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn image_registration_window_approval_retry_and_conflict() {
    let f = Fixture::new().await;
    let key = wccs_protocol::image_device_id("503-41").unwrap();
    let input = json!({"machine_name":"503-41","registration":{"protocol_version":1,"device_id":key,
        "enrollment_token":token(),"device_token":token()}});
    assert_eq!(
        f.admin.device("image-register", "", &input).await.status(),
        409
    );
    f.db.execute(
        "UPDATE registration_window SET closes_at=$1",
        &[&(wccs_protocol::now() + 3600)],
    )
    .await
    .unwrap();
    let first = expect_ok(f.admin.device("image-register", "", &input).await).await;
    assert_eq!(first["status"], "pending");
    assert_eq!(
        f.admin
            .device("register", "", &input["registration"])
            .await
            .status(),
        401
    );
    f.db.execute("UPDATE registration_window SET closes_at=0", &[])
        .await
        .unwrap();
    assert_eq!(
        first,
        expect_ok(f.admin.device("image-register", "", &input).await).await
    );
    f.db.execute(
        "UPDATE image_applications SET expires_at=0,observed_ip='192.0.2.99'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        expect_ok(f.admin.device("image-register", "", &input).await).await["status"],
        "expired"
    );
    let state = f.admin.state().await;
    let application = &state["image_applications"][0];
    assert_eq!(application["machine_name"], "503-41");
    assert_eq!(application["observed_ip"], "127.0.0.1");
    let approve = json!({"request_id":token(),"approve":true,"targets":[{
        "application_id":first["application_id"],"base_version":application["base_version"],"review_token":application["review_token"]}]});
    f.db.execute(
        "UPDATE image_applications SET observed_ip='192.0.2.42' WHERE id=$1",
        &[&first["application_id"].as_str().unwrap()],
    )
    .await
    .unwrap();
    assert_eq!(f.admin.post("images/action", &approve).await.status(), 409);
    f.db.execute(
        "UPDATE image_applications SET observed_ip='127.0.0.1' WHERE id=$1",
        &[&first["application_id"].as_str().unwrap()],
    )
    .await
    .unwrap();
    let approved = expect_ok(f.admin.post("images/action", &approve).await).await;
    assert_eq!(
        approved,
        expect_ok(f.admin.post("images/action", &approve).await).await
    );
    assert_eq!(
        expect_ok(f.admin.device("image-register", "", &input).await).await["status"],
        "enabled"
    );
    let heartbeat = json!({"protocol_version":1,"boot_id":token(),"agent_version":"image-test","reported_ips":[]});
    expect_ok(
        f.admin
            .device(
                "heartbeat",
                input["registration"]["device_token"].as_str().unwrap(),
                &heartbeat,
            )
            .await,
    )
    .await;
    let mut restored = input.clone();
    restored["registration"]["enrollment_token"] = token().into();
    restored["registration"]["device_token"] = token().into();
    assert_eq!(
        f.admin
            .device("image-register", "", &restored)
            .await
            .status(),
        409
    );
    f.db.execute(
        "UPDATE registration_window SET closes_at=$1",
        &[&(wccs_protocol::now() + 3600)],
    )
    .await
    .unwrap();
    let second = expect_ok(f.admin.device("image-register", "", &restored).await).await;
    assert_eq!(first["device_id"], second["device_id"]);
    let state = f.admin.state().await;
    let application = &state["image_applications"][0];
    let mut replace = json!({"request_id":token(),"approve":true,"targets":[{
        "application_id":second["application_id"],"base_version":application["base_version"],"review_token":application["review_token"]}]});
    assert_eq!(f.admin.post("images/action", &replace).await.status(), 409);
    f.db.execute("UPDATE devices SET last_seen=0 WHERE id=$1", &[&key])
        .await
        .unwrap();
    let mut competing = restored.clone();
    competing["registration"]["enrollment_token"] = token().into();
    competing["registration"]["device_token"] = token().into();
    expect_ok(f.admin.device("image-register", "", &competing).await).await;
    assert_eq!(f.admin.post("images/action", &replace).await.status(), 409);
    let refreshed = f.admin.state().await;
    let refreshed = refreshed["image_applications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == second["application_id"])
        .unwrap();
    replace["targets"][0]["review_token"] = refreshed["review_token"].clone();
    expect_ok(f.admin.post("images/action", &replace).await).await;
    assert_eq!(
        f.admin
            .device(
                "heartbeat",
                input["registration"]["device_token"].as_str().unwrap(),
                &heartbeat
            )
            .await
            .status(),
        401
    );
    expect_ok(
        f.admin
            .device(
                "heartbeat",
                restored["registration"]["device_token"].as_str().unwrap(),
                &heartbeat,
            )
            .await,
    )
    .await;
    assert_eq!(
        f.db.query_one("SELECT count(*) FROM devices WHERE id=$1", &[&key])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    let mut invalid = restored.clone();
    invalid["machine_name"] = "503-42".into();
    assert_eq!(
        f.admin
            .device("image-register", "", &invalid)
            .await
            .status(),
        400
    );
    let serialized = f.admin.state().await.to_string();
    for value in [&input, &restored] {
        assert!(!serialized.contains(value["registration"]["device_token"].as_str().unwrap()));
        assert!(!serialized.contains(value["registration"]["enrollment_token"].as_str().unwrap()));
    }
    let mut batch = Vec::new();
    for (name, status) in [("503-42", "ready"), ("503-43", "bootstrap_failed")] {
        let registration = json!({"machine_name":name,"bootstrap_status":status,"registration":{
            "protocol_version":1,"device_id":wccs_protocol::image_device_id(name).unwrap(),
            "enrollment_token":token(),"device_token":token()}});
        let result = expect_ok(f.admin.device("image-register", "", &registration).await).await;
        let state = f.admin.state().await;
        let app = state["image_applications"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == result["application_id"])
            .unwrap();
        batch.push(json!({"application_id":result["application_id"],"base_version":1,"review_token":app["review_token"]}));
    }
    let failed_batch = json!({"request_id":token(),"approve":true,"targets":batch});
    assert_eq!(
        f.admin.post("images/action", &failed_batch).await.status(),
        409
    );
    let first_id = wccs_protocol::image_device_id("503-42").unwrap();
    assert!(
        f.db.query_one(
            "SELECT credential_hash FROM devices WHERE id=$1",
            &[&first_id]
        )
        .await
        .unwrap()
        .get::<_, Option<String>>(0)
        .is_none()
    );
    assert_eq!(
        f.admin
            .client
            .post(format!("{}/api/images/action", f.admin.url))
            .json(&failed_batch)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    f.db.execute("UPDATE control_devices SET connected=false", &[])
        .await
        .unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn diagnostics_receipts_replay_identity_and_control_isolation() {
    use futures_util::StreamExt;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };
    let f = Fixture::new().await;
    let mut control = socket_for(&f.admin, &f.credential, "device-1").await;
    let first = poll(&mut control, &observation()).await.unwrap();
    let mut request = input(1);
    let preview = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    request["preview_token"] = preview["preview_token"].clone();
    expect_ok(f.admin.post("waiting/commit", &request).await).await;
    let command = poll(&mut control, &observation()).await.unwrap()["command"].clone();
    let nonce = token();
    let mut ws_request = format!(
        "{}/api/device/diagnostics/probe?nonce={nonce}",
        f.admin.url.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    assert!(connect_async(ws_request.clone()).await.is_err());
    ws_request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", f.credential).parse().unwrap(),
    );
    ws_request
        .headers_mut()
        .insert("x-wccs-device-id", "device-1".parse().unwrap());
    let (mut ws, _) = connect_async(ws_request).await.unwrap();
    let Some(Ok(Message::Text(text))) = ws.next().await else {
        panic!("missing diagnostic receipt")
    };
    let receipt: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(receipt["device_id"], "device-1");
    assert_eq!(receipt["nonce"], nonce);
    let after = poll(&mut control, &observation()).await.unwrap();
    assert_eq!(after["connection_id"], first["connection_id"]);
    assert_eq!(after["command"], command);
    let context: Value = f
        .admin
        .client
        .get(format!("{}/api/device/diagnostics/context", f.admin.url))
        .bearer_auth(&f.credential)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(context["device_id"], "device-1");
    assert_eq!(context["seats"].as_array().unwrap().len(), 1);
    let mut report = json!({"schema_version":1,"request_id":token(),"device_id":"device-1","agent_version":"test",
        "evidence_context":"local_test","configuration_revision":1,"configuration_digest":"a".repeat(64),
        "collected_at":1,"seat_claim":null,"checks":(["configuration","installation","server","authentication","http_heartbeat","websocket","service","session_ipc"].map(|step|
        json!({"step":step,"outcome":"succeeded","checked_at":1,"last_success_at":1,"code":"receipt_verified"})))});
    assert_eq!(
        f.admin.device("diagnostics", "", &report).await.status(),
        401
    );
    let acknowledged = expect_ok(f.admin.device("diagnostics", &f.credential, &report).await).await;
    assert_eq!(acknowledged["request_id"], report["request_id"]);
    assert_eq!(
        expect_ok(f.admin.device("diagnostics", &f.credential, &report).await).await,
        acknowledged
    );
    report["collected_at"] = json!(2);
    assert_eq!(
        f.admin
            .device("diagnostics", &f.credential, &report)
            .await
            .status(),
        409
    );
    report["request_id"] = json!(token());
    report["device_id"] = json!("device-2");
    assert_eq!(
        f.admin
            .device("diagnostics", &f.credential, &report)
            .await
            .status(),
        403
    );
    let state = f.admin.state().await;
    let device = state["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "device-1")
        .unwrap();
    assert_eq!(
        device["diagnostics"]["request_id"],
        acknowledged["request_id"]
    );
    assert!(!device.to_string().contains(&f.credential));
    f.db.execute(
        "UPDATE devices SET enrollment_generation=enrollment_generation+1 WHERE id='device-1'",
        &[],
    )
    .await
    .unwrap();
    let state = f.admin.state().await;
    assert!(
        state["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "device-1")
            .unwrap()["diagnostics"]
            .is_null()
    );
    control.close(None).await.unwrap();
    f.finish().await;
}

fn input(count: usize) -> Value {
    json!({"request_id":token(),"contest_id":"contest","seat_ids":(1..=count).map(|n|format!("seat-{n}")).collect::<Vec<_>>(),"action":"enter","duration_seconds":900})
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn preparation_profiles_bind_tools_permissions_and_task_recovery_receipts() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    expect_ok(f.admin.post("configuration", &json!({"request_id":token(),"base_version":0,"tools":[{"id":"powershell","name":"PowerShell","kind":"executable","path":"C:\\Tools\\pwsh.exe"}],"targets":["device-1"]})).await).await;
    let profile = json!({"schema_version":1,"tool_version":1,"work_directory":"D:\\Contest","desktop_directory":"C:\\Users\\WccsContest\\Desktop","script_directory":"C:\\ProgramData\\WCCS\\Scripts","links":[{"kind":"tool","name":"PowerShell","tool_id":"powershell"}],"package":null});
    let mut save = json!({"request_id":token(),"base_version":0,"profile":profile});
    let receipt = expect_ok(f.admin.post("preparation", &save).await).await;
    assert_eq!(
        expect_ok(f.admin.post("preparation", &save).await).await,
        receipt
    );
    save["profile"]["work_directory"] = json!("D:\\changed");
    assert_eq!(f.admin.post("preparation", &save).await.status(), 409);
    save["request_id"] = json!(token());
    assert_eq!(f.admin.post("preparation", &save).await.status(), 409);
    save["base_version"] = json!(1);
    save["profile"]["links"][0]["tool_id"] = json!("missing");
    assert_eq!(f.admin.post("preparation", &save).await.status(), 400);
    let runtime = json!({"schema_version":1,"instance_id":"host-one","session_key":"session-one","adapter":"service","capabilities":["identify-v1","preparation-v1"]});
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    let mut request = json!({"request_id":token(),"template_version":1,"targets":["device-1"],"template":{"operation":"prepare_desktop","profile_version":1}});
    assert_eq!(
        expect_ok(f.admin.post("maintenance/preview", &request).await).await["targets"][0]["reason"],
        "ordinary_account_unavailable"
    );
    // This is a protocol fixture, not evidence of a real Windows account.
    f.db.execute("UPDATE control_devices SET actual=jsonb_set(actual,'{account}',$1) WHERE device_id='device-1'",&[&json!({"adapter":"windows_test","target_sid":"S-1-5-21-123","observed_sid":"S-1-5-21-123","phase":"controlled","account":"owned","desktop_confirmed":true})]).await.unwrap();
    let preview = expect_ok(f.admin.post("maintenance/preview", &request).await).await;
    assert!(preview["targets"][0]["reason"].is_null());
    assert_eq!(
        preview["preparation"]["tools"][0]["path"],
        "C:\\Tools\\pwsh.exe"
    );
    request["preview_token"] = preview["preview_token"].clone();
    expect_ok(f.admin.post("maintenance/commit", &request).await).await;
    let command = expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await["command"]
        .clone();
    assert_eq!(command["preparation"]["version"], 1);
    let outcome = json!({"schema_version":1,"job_id":command["job_id"],"command_digest":wccs_protocol::digest(&command.to_string()),"stage":"interrupted","detail":"rollback_complete","steps":[{"phase":"rollback","path":"D:\\Contest","outcome":"succeeded","detail":"recovered"}]});
    let ack = expect_ok(
        f.admin
            .device("maintenance/result", &f.credential, &outcome)
            .await,
    )
    .await;
    assert_eq!(
        expect_ok(
            f.admin
                .device("maintenance/result", &f.credential, &outcome)
                .await
        )
        .await,
        ack
    );
    let mut changed = outcome.clone();
    changed["stage"] = json!("succeeded");
    assert_eq!(
        f.admin
            .device("maintenance/result", &f.credential, &changed)
            .await
            .status(),
        409
    );
    request["request_id"] = json!(token());
    let preview = expect_ok(f.admin.post("maintenance/preview", &request).await).await;
    request["preview_token"] = preview["preview_token"].clone();
    expect_ok(
        f.admin
            .post(
                "preparation",
                &json!({"request_id":token(),"base_version":1,"profile":profile}),
            )
            .await,
    )
    .await;
    assert_eq!(
        f.admin.post("maintenance/commit", &request).await.status(),
        409
    );
    f.db.execute(
        "UPDATE operators SET role='viewer' WHERE username='test-admin'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(f.admin.post("preparation", &save).await.status(), 403);
    ws.close(None).await.unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn competition_configuration_checks_paths_and_client_capability() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    expect_ok(
        f.admin
            .post(
                "configuration",
                &json!({"request_id":token(),"base_version":0,"tools":[
        {"id":"powershell","name":"PowerShell","kind":"executable","path":"C:\\Tools\\pwsh.exe"},
        {"id":"gcc-cpp","name":"GNU C++","kind":"executable","path":"C:\\Tools\\g++.exe"}
    ],"targets":["device-1"]}),
            )
            .await,
    )
    .await;
    let content = "Write-Output synthetic";
    let files: Vec<_> = ["compilecpp.ps1", "submit.ps1", "contest-common.ps1"]
        .iter()
        .map(|path| json!({"path":path,"content":content,"sha256":wccs_protocol::digest(content)}))
        .collect();
    let mut save = json!({"request_id":token(),"base_version":0,"profile":{
        "schema_version":1,"tool_version":1,"work_directory":"C:\\Contest",
        "desktop_directory":"C:\\Users\\WccsContest\\Desktop","script_directory":"C:\\ProgramData\\WCCS\\Scripts",
        "links":[],"package":{"schema_version":1,"id":"contest","version":"v1","configuration_version":"v1",
            "runtime_tool_id":"powershell","runtime_file_version":"7.6.9","source":"synthetic","license":"test","files":files},
        "competition":{"compiler_tool_id":"missing","compiler_arguments":["-O2"],"judge_url":"https://judge.test",
            "contest_id":"demo","language_id":"cpp"}
    }});
    assert_eq!(f.admin.post("preparation", &save).await.status(), 400);
    save["profile"]["competition"]["compiler_tool_id"] = json!("gcc-cpp");
    save["profile"]["competition"]["judge_url"] = json!("https://user:secret@judge.test");
    assert_eq!(f.admin.post("preparation", &save).await.status(), 400);
    save["profile"]["competition"]["judge_url"] = json!("https://judge.test");
    expect_ok(f.admin.post("preparation", &save).await).await;
    let mut runtime = json!({"schema_version":1,"instance_id":"host-one","session_key":"session-one",
        "adapter":"service","capabilities":["identify-v1","preparation-v1"]});
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    let request = json!({"request_id":token(),"template_version":1,"targets":["device-1"],
        "template":{"operation":"deploy_scripts","profile_version":1}});
    assert_eq!(
        expect_ok(f.admin.post("maintenance/preview", &request).await).await["targets"][0]["reason"],
        "unsupported_client"
    );
    runtime["capabilities"] = json!(["identify-v1", "preparation-v1", "competition-v1"]);
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    assert_eq!(
        expect_ok(f.admin.post("maintenance/preview", &request).await).await["targets"][0]["reason"],
        "ordinary_account_unavailable"
    );
    ws.close(None).await.unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn domjudge_jobs_require_global_authority_and_explicit_restart_review() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let listed = expect_ok(f.admin.get("domjudge").await).await;
    assert_eq!(listed["configured"], false);
    assert_eq!(listed["jobs"], json!([]));
    let input = json!({"request_id":token(),"contest_id":"contest","remote_contest_id":"demo",
        "group_id":"participants","participant_ids":["participant-1"]});
    assert_eq!(f.admin.post("domjudge/preview", &input).await.status(), 409);
    let binding = json!({"request_id":token(),"contest_id":"contest","remote_contest_id":"demo","previous":null});
    let bound = expect_ok(f.admin.post("domjudge/bind", &binding).await).await;
    assert_eq!(
        bound,
        expect_ok(f.admin.post("domjudge/bind", &binding).await).await
    );
    assert_eq!(f.admin.post("domjudge/preview", &input).await.status(), 400);
    let mut wrong = input.clone();
    wrong["remote_contest_id"] = json!("wrong");
    assert_eq!(f.admin.post("domjudge/preview", &wrong).await.status(), 409);
    f.db.execute(
        "UPDATE operators SET all_rooms=false,role='viewer' WHERE username='test-admin'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(f.admin.get("domjudge").await.status(), 403);
    f.db.execute(
        "UPDATE operators SET all_rooms=true,role='viewer' WHERE username='test-admin'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(f.admin.post("domjudge/preview", &input).await.status(), 403);
    f.db.execute(
        "UPDATE operators SET role='admin' WHERE username='test-admin'",
        &[],
    )
    .await
    .unwrap();
    f.db.execute("INSERT INTO domjudge_jobs(id,actor_id,actor_version,stage,payload,plan,created_at,updated_at) SELECT 'interrupted-job',id,version,'running','{}','{}',0,0 FROM operators WHERE username='test-admin'", &[]).await.unwrap();
    let restarted = wccs_server::connect(f.app.config.clone()).await.unwrap();
    let row =
        f.db.query_one(
            "SELECT stage,detail FROM domjudge_jobs WHERE id='interrupted-job'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "interrupted");
    assert_eq!(row.get::<_, String>(1), "server_restarted_review_required");
    drop(restarted);
    ws.close(None).await.unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn maintenance_preview_cancel_replay_and_identity_fences() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let runtime = json!({"schema_version":1,"instance_id":"host-one","session_key":"session-one","adapter":"local_test","capabilities":["identify-v1"]});
    expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await;
    let mut request = json!({"request_id":token(),"template_version":1,"targets":["device-1"],"template":{"operation":"identify","duration_seconds":30}});
    let preview = expect_ok(f.admin.post("maintenance/preview", &request).await).await;
    assert!(preview["targets"][0]["reason"].is_null());
    request["preview_token"] = preview["preview_token"].clone();
    let receipt = expect_ok(f.admin.post("maintenance/commit", &request).await).await;
    assert_eq!(
        expect_ok(f.admin.post("maintenance/commit", &request).await).await,
        receipt
    );
    let job = &receipt["jobs"][0]["job_id"];
    request["template"]["duration_seconds"] = json!(31);
    assert_eq!(
        f.admin.post("maintenance/commit", &request).await.status(),
        409
    );
    let waiting = expect_ok(f.admin.post("waiting/preview", &input(1)).await).await;
    assert_eq!(waiting["targets"][0]["status"], "maintenance_active");
    let command = expect_ok(
        f.admin
            .device("maintenance/poll", &f.credential, &runtime)
            .await,
    )
    .await["command"]
        .clone();
    assert_eq!(command["job_id"], *job);
    let mut result = json!({"schema_version":1,"job_id":job,"command_digest":wccs_protocol::digest(&command.to_string()),"stage":"accepted","detail":"persisted"});
    let ack = expect_ok(
        f.admin
            .device("maintenance/result", &f.credential, &result)
            .await,
    )
    .await;
    assert_eq!(
        expect_ok(
            f.admin
                .device("maintenance/result", &f.credential, &result)
                .await
        )
        .await,
        ack
    );
    result["detail"] = json!("display_observed");
    assert_eq!(
        f.admin
            .device("maintenance/result", &f.credential, &result)
            .await
            .status(),
        409
    );
    result["stage"] = json!("executing");
    expect_ok(
        f.admin
            .device("maintenance/result", &f.credential, &result)
            .await,
    )
    .await;
    let cancel = json!({"request_id":token(),"job_id":job});
    let cancelled = expect_ok(f.admin.post("maintenance/cancel", &cancel).await).await;
    assert_eq!(
        expect_ok(f.admin.post("maintenance/cancel", &cancel).await).await,
        cancelled
    );
    assert!(
        expect_ok(
            f.admin
                .device("maintenance/poll", &f.credential, &runtime)
                .await
        )
        .await["command"]
            .is_null()
    );
    result["stage"] = json!("cancelled");
    result["detail"] = json!("connection_lost");
    expect_ok(
        f.admin
            .device("maintenance/result", &f.credential, &result)
            .await,
    )
    .await;
    let listed = expect_ok(f.admin.get("maintenance").await).await;
    assert_eq!(listed["jobs"][0]["stage"], "cancelled");
    f.db.execute(
        "UPDATE devices SET enrollment_generation=enrollment_generation+1 WHERE id='device-1'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        f.admin
            .device("maintenance/result", &f.credential, &result)
            .await
            .status(),
        403
    );
    ws.close(None).await.unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn health_samples_reject_old_hosts_replays_and_invalid_values() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let unknown = json!({"sampled_at":1,"observation":{"status":"unavailable","reason":"unsupported","code":null}});
    let mut report = json!({"schema_version":1,"device_id":"device-1","instance_id":"host-one","sequence":1,"collection_ms":20,
        "agent_version":{"sampled_at":1,"observation":{"status":"available","value":"0.3.0"}},"deployment_version":unknown,
        "health":{"uptime_seconds":unknown,"cpu_percent":unknown,"memory":unknown,"displays":unknown,"keyboards":unknown,"mice":unknown}});
    let ack = expect_ok(f.admin.device("health", &f.credential, &report).await).await;
    assert_eq!(
        expect_ok(f.admin.device("health", &f.credential, &report).await).await,
        ack
    );
    report["collection_ms"] = json!(30);
    assert_eq!(
        f.admin
            .device("health", &f.credential, &report)
            .await
            .status(),
        409
    );
    report["sequence"] = json!(2);
    report["health"]["cpu_percent"]["observation"] = json!({"status":"available","value":101});
    assert_eq!(
        f.admin
            .device("health", &f.credential, &report)
            .await
            .status(),
        400
    );
    report["health"]["cpu_percent"] = unknown;
    report["instance_id"] = json!("old-host");
    assert_eq!(
        f.admin
            .device("health", &f.credential, &report)
            .await
            .status(),
        409
    );
    report["instance_id"] = json!("host-one");
    expect_ok(f.admin.device("health", &f.credential, &report).await).await;
    // Failed or late health uploads leave the independent control connection valid.
    let response = poll(&mut ws, &observation()).await.unwrap();
    assert!(response["command"].is_null());
    let state = f.admin.state().await;
    let device = state["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "device-1")
        .unwrap();
    assert_eq!(device["health"]["sequence"], 2);
    assert_eq!(
        device["health"]["health"]["cpu_percent"]["observation"]["status"],
        "unavailable"
    );
    ws.close(None).await.unwrap();
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn presentation_versions_identity_and_reads_cannot_cancel_control() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let mut request = input(1);
    let preview = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    request["preview_token"] = preview["preview_token"].clone();
    expect_ok(f.admin.post("waiting/commit", &request).await).await;
    let command = poll(&mut ws, &observation()).await.unwrap()["command"].clone();
    let mut save = json!({"request_id":token(),"contest_id":"contest","base_version":0,"settings":{"starts_at_ms":1234567,"domjudge_api":null,"domjudge_contest":null,"logos":[]}});
    let receipt = expect_ok(f.admin.post("presentation", &save).await).await;
    assert_eq!(
        expect_ok(f.admin.post("presentation", &save).await).await,
        receipt
    );
    save["request_id"] = json!(token());
    assert_eq!(f.admin.post("presentation", &save).await.status(), 409);
    let query = json!({"instance_id":"host-one","session_key":"session-one"});
    let data = expect_ok(f.admin.device("presentation", &f.credential, &query).await).await["data"]
        .clone();
    assert_eq!(data["configuration_version"], 1);
    assert_eq!(data["command_id"], command["command_id"]);
    let wrong = json!({"instance_id":"wrong-host","session_key":"wrong-session"});
    assert!(
        expect_ok(f.admin.device("presentation", &f.credential, &wrong).await).await["data"]
            .is_null()
    );
    assert_eq!(
        poll(&mut ws, &observation()).await.unwrap()["command"],
        command
    );
    let mut report = json!({"data":data,"received_tick_ms":100,"round_trip_ms":5,"clock_offset_ms":-1000,
        "wccs":{"state":"online","source":"authenticated-http","checked_at_ms":1,"detail":"receipt_verified"},
        "oj":{"state":"unknown","source":"client-domjudge-api","checked_at_ms":1,"detail":"not_configured"}});
    expect_ok(
        f.admin
            .device("presentation/report", &f.credential, &report)
            .await,
    )
    .await;
    report["data"]["identity"]["school"] = json!("changed");
    assert_eq!(
        f.admin
            .device("presentation/report", &f.credential, &report)
            .await
            .status(),
        409
    );
    let list = expect_ok(f.admin.get("presentation?contest_id=contest").await).await;
    assert_eq!(list["receipts"].as_array().unwrap().len(), 1);
    ws.close(None).await.unwrap();
    f.finish().await;
}

async fn backend(db: &tokio_postgres::Client) -> i32 {
    db.query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn standby_check_and_readiness_do_not_take_control() {
    use wccs_server::deployment::{ControllerLease, check_standby};
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let mut request = input(1);
    let preview = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    request["preview_token"] = preview["preview_token"].clone();
    expect_ok(f.admin.post("waiting/commit", &request).await).await;
    let owner = ControllerLease::acquire(&f.app.config)
        .await
        .unwrap()
        .unwrap();
    assert!(
        ControllerLease::acquire(&f.app.config)
            .await
            .unwrap()
            .is_none()
    );
    let web = std::env::temp_dir().join(format!("wccs-standby-{}", token()));
    std::fs::create_dir(&web).unwrap();
    let mut candidate = f.app.config.clone();
    candidate.static_dir = web.to_str().unwrap().into();
    assert!(check_standby(&candidate).await.is_err());
    std::fs::write(web.join("index.html"), "<title>WCCS</title>").unwrap();
    let report = check_standby(&candidate).await.unwrap();
    assert_eq!(report["migration_required"], false);
    assert_eq!(report["database_read_only"], false);
    assert_eq!(f.admin.get("ready").await.status(), 200);
    assert!(poll(&mut ws, &observation()).await.unwrap()["command"].is_object());
    f.db.batch_execute("UPDATE schema_version SET version=19")
        .await
        .unwrap();
    assert_eq!(
        check_standby(&candidate).await.unwrap()["migration_required"],
        true
    );
    assert_eq!(f.admin.get("health").await.status(), 200);
    assert_eq!(f.admin.get("ready").await.status(), 503);
    f.db.batch_execute("UPDATE schema_version SET version=20")
        .await
        .unwrap();
    assert_eq!(f.admin.get("ready").await.status(), 200);
    std::fs::remove_file(web.join("index.html")).unwrap();
    std::fs::remove_dir(web).unwrap();
    drop(owner);
    tokio::time::timeout(Duration::from_secs(4), async {
        while ControllerLease::acquire(&f.app.config)
            .await
            .unwrap()
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    ws.close(None).await.unwrap();
    f.finish().await;
}

async fn wait_for_recovery(admin: &Harness) {
    let start = Instant::now();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = admin.get("session").await.status();
            if status == 200 {
                break;
            }
            // A request can race the driver's close notification and fail once.
            assert_eq!(status, 503);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("same server and session must recover after connection loss");
    println!(
        "database_recovered elapsed_ms={}",
        start.elapsed().as_millis()
    );
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn database_reconnection_preserves_session_and_request_replay() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    assert!(poll(&mut ws, &observation()).await.unwrap()["command"].is_null());
    let mut request = input(1);
    let preview = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    request["preview_token"] = preview["preview_token"].clone();
    let receipt = expect_ok(f.admin.post("waiting/commit", &request).await).await;
    let epoch = f.app.server_epoch.clone();
    let writer_pid = backend(&f.app.db.lock().await.unwrap()).await;
    let preview_pid = backend(&f.app.preview_db.lock().await.unwrap()).await;
    for pid in [writer_pid, preview_pid] {
        assert!(
            f.db.query_one("SELECT pg_terminate_backend($1)", &[&pid])
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    }
    wait_for_recovery(&f.admin).await;
    // Concurrent callers share one replacement writer and retain the logged-in session.
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let admin = f.admin.clone();
        requests.spawn(async move { admin.get("session").await.status() });
    }
    while let Some(result) = requests.join_next().await {
        assert_eq!(result.unwrap(), 200);
    }
    assert_ne!(backend(&f.app.db.lock().await.unwrap()).await, writer_pid);
    assert_eq!(f.app.server_epoch, epoch);
    assert_eq!(
        expect_ok(f.admin.post("waiting/commit", &request).await).await,
        receipt
    );
    assert_eq!(
        f.db.query_one("SELECT count(*) FROM waiting_commands", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        f.db.query_one("SELECT stage FROM waiting_commands", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "pending"
    );
    expect_ok(f.admin.post("waiting/preview", &input(1)).await).await;
    assert_ne!(
        backend(&f.app.preview_db.lock().await.unwrap()).await,
        preview_pid
    );
    assert!(!poll(&mut ws, &observation()).await.unwrap()["command"].is_null());

    // A lost connection rolls back an in-flight transaction without replaying it.
    let mut writer = f.app.db.lock().await.unwrap();
    let tx = writer.transaction().await.unwrap();
    let pid: i32 = tx
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    tx.execute("UPDATE contests SET version=999 WHERE id='contest'", &[])
        .await
        .unwrap();
    f.db.query_one("SELECT pg_terminate_backend($1)", &[&pid])
        .await
        .unwrap();
    assert!(tx.commit().await.is_err());
    drop(writer);
    wait_for_recovery(&f.admin).await;
    assert_eq!(f.admin.get("state").await.status(), 200);
    assert_eq!(
        f.db.query_one("SELECT version FROM contests WHERE id='contest'", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        expect_ok(f.admin.post("waiting/commit", &request).await).await,
        receipt
    );

    // A changed client instance still invalidates the old control command after DB recovery.
    let mut replacement = socket_for(&f.admin, &f.credential, "device-1").await;
    let mut changed = observation();
    changed["instance_id"] = json!("host-two");
    assert!(poll(&mut replacement, &changed).await.unwrap()["command"].is_null());
    assert_eq!(
        f.db.query_one("SELECT stage FROM waiting_commands", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "cancelled"
    );
    drop(ws);
    drop(replacement);
    f.finish().await;
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn blocked_500_seat_preview_keeps_control_heartbeats_running() {
    let mut f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    assert!(poll(&mut ws, &observation()).await.unwrap()["command"].is_null());
    let request = input(500);
    let baseline = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    let preview_pid = backend(&f.app.preview_db.lock().await.unwrap()).await;
    let blocker = f.db.transaction().await.unwrap();
    blocker
        .batch_execute("LOCK TABLE participants IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let admin = f.admin.clone();
    let body = request.clone();
    let pending = tokio::spawn(async move { admin.post("waiting/preview", &body).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = blocker
                .query_one(
                    "SELECT wait_event_type='Lock' FROM pg_stat_activity WHERE pid=$1",
                    &[&preview_pid],
                )
                .await
                .unwrap()
                .get::<_, Option<bool>>(0)
                .unwrap_or(false);
            if waiting {
                break;
            }
            // pg_stat_activity can otherwise reuse a transaction-local statistics snapshot.
            blocker
                .batch_execute("SELECT pg_stat_clear_snapshot()")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("preview must reach the deliberately blocked SELECT");
    let mut latencies = Vec::new();
    for _ in 0..8 {
        let start = Instant::now();
        let reply = tokio::time::timeout(Duration::from_secs(2), poll(&mut ws, &observation()))
            .await
            .expect("preview must not block the control lease")
            .unwrap();
        assert!(reply["command"].is_null());
        latencies.push(start.elapsed().as_millis());
    }
    assert!(!pending.is_finished());
    assert_eq!(f.admin.get("session").await.status(), 200);
    blocker
        .execute("UPDATE contests SET version=2 WHERE id='contest'", &[])
        .await
        .unwrap();
    blocker.commit().await.unwrap();
    let result = expect_ok(pending.await.unwrap()).await;
    // Every read belongs to the snapshot established before the concurrent edit.
    assert_eq!(result, baseline);
    let targets = result["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 500);
    for (index, target) in targets.iter().enumerate() {
        let number = index + 1;
        assert_eq!(target["status"], "ready");
        assert_eq!(target["device_id"], format!("device-{number}"));
        assert_eq!(
            target["identity"]["participant_id"],
            format!("participant-{number}")
        );
        assert_eq!(target["identity"]["school"], format!("学校{number}"));
    }
    let mut stale = request.clone();
    stale["preview_token"] = result["preview_token"].clone();
    assert_eq!(f.admin.post("waiting/commit", &stale).await.status(), 409);
    let fresh = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    assert_eq!(fresh["contest_version"], 2);
    stale["preview_token"] = fresh["preview_token"].clone();
    f.db.execute(
        "UPDATE operators SET version=version+1 WHERE username='test-admin'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(f.admin.post("waiting/commit", &stale).await.status(), 401);
    assert_eq!(
        f.admin.post("waiting/preview", &request).await.status(),
        401
    );
    latencies.sort();
    println!(
        "blocked_preview seats=500 heartbeat_samples={} p50_ms={} max_ms={}",
        latencies.len(),
        latencies[latencies.len() / 2],
        latencies.last().unwrap()
    );
    drop(ws);
    f.finish().await;
}
