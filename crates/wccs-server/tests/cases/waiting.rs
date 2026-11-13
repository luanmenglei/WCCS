use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use wccs_protocol::control::CONTROL_PROTOCOL;
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket(h: &Harness, credential: &str) -> Socket {
    socket_for(h, credential, "a").await
}
pub(super) async fn socket_for(h: &Harness, credential: &str, device: &str) -> Socket {
    let mut request = format!(
        "{}/api/device/control",
        h.url.replacen("http://", "ws://", 1)
    )
    .into_client_request()
    .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {credential}").parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("x-wccs-device-id", device.parse().unwrap());
    request
        .headers_mut()
        .insert("sec-websocket-protocol", CONTROL_PROTOCOL.parse().unwrap());
    connect_async(request).await.unwrap().0
}
pub(super) async fn poll(socket: &mut Socket, poll: &Value) -> Option<Value> {
    socket
        .send(Message::Text(poll.to_string().into()))
        .await
        .ok()?;
    match tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
    {
        Some(Ok(Message::Text(text))) => Some(serde_json::from_str(&text).unwrap()),
        _ => None,
    }
}
async fn preview(h: &Harness, input: &mut Value) -> Value {
    input["request_id"] = json!(token());
    let preview = expect_ok(h.post("waiting/preview", input).await).await;
    input["preview_token"] = preview["preview_token"].clone();
    preview
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn waiting_authorization_control_and_recovery() {
    let database = std::env::var("WCCS_TEST_DATABASE_URL").unwrap();
    let (db, connection) = tokio_postgres::connect(&database, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_handle = tokio::spawn(async move {
        connection.await.unwrap();
    });
    let schema = format!("test_waiting_{}", &token()[..24]);
    db.batch_execute(&format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema}"
    ))
    .await
    .unwrap();
    let salt = SaltString::encode_b64(b"synthetic-salt-16").unwrap();
    let config = Config {
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
    };
    let (mut admin, server) = start(config.clone()).await;
    let mut input = json!({"request_id":token(),"contest_id":"contest","seat_ids":["seat"],"action":"enter","duration_seconds":900});
    assert_eq!(admin.get("waiting").await.status(), 401);
    for path in ["waiting/preview", "waiting/commit"] {
        assert_eq!(admin.post(path, &input).await.status(), 401);
    }
    admin.login().await;
    for (id, ip) in [("A", "10.9.1.1"), ("B", "10.9.2.1")] {
        let mut room = room_input(id, 1, 1);
        room["seats"][0]["expected_ip"] = json!(ip);
        preview_room(&admin, &mut room).await;
        expect_ok(admin.post("rooms/save", &room).await).await;
    }
    let credential = token();
    expect_ok(
        admin
            .post(
                "devices",
                &json!({"id":"a","name":"Synthetic control","enrollment_token":token()}),
            )
            .await,
    )
    .await;
    db.execute("UPDATE devices SET lifecycle='enabled',credential_hash=$1,observed_ip='10.9.1.1',last_seen=$2 WHERE id='a'",&[&wccs_protocol::digest(&credential),&wccs_protocol::now()]).await.unwrap();
    let mut create = roster_input(
        &admin,
        "contest",
        json!({"kind":"create","name":"合成等待比赛"}),
    )
    .await;
    roster_commit(&admin, &mut create).await;
    let mut import = roster_input(
        &admin,
        "contest",
        json!({"kind":"import","text":"code\tname\tschool\n001\t林小禾\t合成学校"}),
    )
    .await;
    roster_commit(&admin, &mut import).await;
    let state = admin.state().await;
    let seat = state["seating"]["rooms"][0]["seats"][0]["id"].clone();
    let seat_b = state["seating"]["rooms"][1]["seats"][0]["id"].clone();
    let participant = state["roster"]["contests"][0]["participants"][0]["id"].clone();
    let mut assign = roster_input(
        &admin,
        "contest",
        json!({"kind":"assign","changes":[{"participant_id":participant,"seat_id":seat}]}),
    )
    .await;
    roster_commit(&admin, &mut assign).await;
    input["seat_ids"] = json!([seat]);
    let operator_input = operator_write(
        "control-operator",
        "control-operator",
        "operator",
        json!(["A"]),
    );
    let viewer_input = operator_write("control-viewer", "control-viewer", "viewer", json!(["A"]));
    expect_ok(admin.post("operators", &operator_input).await).await;
    expect_ok(admin.post("operators", &viewer_input).await).await;
    let operator = admin
        .identity("control-operator", "synthetic-operator-password")
        .await;
    let viewer = admin
        .identity("control-viewer", "synthetic-operator-password")
        .await;
    for path in ["waiting/preview", "waiting/commit"] {
        assert_eq!(viewer.post(path, &input).await.status(), 403);
        let mut cross = input.clone();
        cross["seat_ids"] = json!([seat, seat_b]);
        assert_eq!(operator.post(path, &cross).await.status(), 403);
    }
    assert_eq!(
        preview(&operator, &mut input).await["targets"][0]["status"],
        "offline"
    );
    let mut ws = socket(&admin, &credential).await;
    let mut observation = json!({"protocol_version":1,"instance_id":"host-one","session_key":"session-one","capabilities":["waiting-v1"],"actual":{"state":"desktop","command_id":null,"session_key":null,"monitors":0,"detail":"standby"},"report":null});
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    assert_eq!(
        preview(&operator, &mut input).await["targets"][0]["status"],
        "ready"
    );
    // A skipped seat still contributes to the management receipt's historical scope.
    let mut mixed = input.clone();
    mixed["seat_ids"] = json!([seat, seat_b]);
    preview(&admin, &mut mixed).await;
    expect_ok(admin.post("waiting/commit", &mixed).await).await;
    let visible_events = expect_ok(
        viewer
            .get(&format!(
                "events?request_id={}",
                mixed["request_id"].as_str().unwrap()
            ))
            .await,
    )
    .await;
    assert!(
        visible_events["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["operation"] != "waiting_commit")
    );
    // Roster changes after preview invalidate the exact identity snapshot.
    db.execute(
        "UPDATE contests SET version=version+1 WHERE id='contest'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(operator.post("waiting/commit", &input).await.status(), 409);
    preview(&operator, &mut input).await;
    let (a, b) = tokio::join!(
        operator.post("waiting/commit", &input),
        operator.post("waiting/commit", &input)
    );
    let receipt = expect_ok(a).await;
    assert_eq!(expect_ok(b).await, receipt);
    let command = receipt["commands"][0].clone();
    assert_eq!(command["identity"]["name"], "林小禾");
    let mut different = input.clone();
    different["action"] = json!("leave");
    assert_eq!(
        operator.post("waiting/commit", &different).await.status(),
        409
    );
    assert_eq!(admin.post("waiting/commit", &input).await.status(), 403);
    assert_eq!(
        poll(&mut ws, &observation).await.unwrap()["command"],
        command
    );
    for stage in ["accepted", "executing", "succeeded"] {
        if stage == "succeeded" {
            observation["actual"] = json!({"state":"waiting","command_id":command["command_id"],"session_key":"session-one","monitors":1,"detail":"ui_observed"});
        }
        observation["report"] = json!({"command_id":command["command_id"],"sequence":command["sequence"],"stage":stage,"actual":observation["actual"]});
        assert_eq!(
            poll(&mut ws, &observation).await.unwrap()["acknowledged"],
            command["command_id"]
        );
    }
    assert!(poll(&mut ws, &observation).await.is_some());
    let results = expect_ok(viewer.get("waiting").await).await;
    assert_eq!(results["items"][0]["stage"], "succeeded");
    let events = expect_ok(
        viewer
            .get(&format!(
                "events?request_id={}",
                input["request_id"].as_str().unwrap()
            ))
            .await,
    )
    .await;
    assert!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["operation"] == "waiting_succeeded")
    );
    assert!(!events.to_string().contains(&credential));
    // A replacement connection atomically fences old connection reports.
    let mut replacement = socket(&admin, &credential).await;
    assert!(poll(&mut replacement, &observation).await.is_some());
    assert!(poll(&mut ws, &observation).await.is_none());
    ws = replacement;
    observation["report"] = Value::Null;
    // Permission version cancellation remains after restoration.
    db.execute(
        "UPDATE operators SET version=version+1 WHERE id='control-operator'",
        &[],
    )
    .await
    .unwrap();
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    assert_eq!(operator.post("waiting/commit", &input).await.status(), 401);
    let operator = admin
        .identity("control-operator", "synthetic-operator-password")
        .await;
    // Leave is a separate explicit command with its own completion evidence.
    input["action"] = json!("leave");
    preview(&operator, &mut input).await;
    let leave =
        expect_ok(operator.post("waiting/commit", &input).await).await["commands"][0].clone();
    assert_eq!(poll(&mut ws, &observation).await.unwrap()["command"], leave);
    observation["actual"] = json!({"state":"desktop","command_id":null,"session_key":null,"monitors":0,"detail":"ended"});
    observation["report"] = json!({"command_id":leave["command_id"],"sequence":leave["sequence"],"stage":"succeeded","actual":observation["actual"]});
    assert!(poll(&mut ws, &observation).await.is_some());
    observation["report"] = Value::Null;
    input["action"] = json!("enter");
    // Rollback after partial command insertion also rolls back all event projections.
    preview(&operator, &mut input).await;
    db.batch_execute("CREATE FUNCTION fail_waiting() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.operation='waiting_commit' THEN RAISE EXCEPTION 'test fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_waiting BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION fail_waiting()").await.unwrap();
    assert_eq!(operator.post("waiting/commit", &input).await.status(), 500);
    assert_eq!(
        db.query_one(
            "SELECT count(*) FROM waiting_commands WHERE request_id=$1",
            &[&input["request_id"].as_str().unwrap()]
        )
        .await
        .unwrap()
        .get::<_, i64>(0),
        0
    );
    db.batch_execute("DROP TRIGGER fail_waiting ON audit_events; DROP FUNCTION fail_waiting()")
        .await
        .unwrap();
    let active =
        expect_ok(operator.post("waiting/commit", &input).await).await["commands"][0].clone();
    // Wrong completion evidence closes the channel without accepting success.
    observation["report"] = json!({"command_id":active["command_id"],"sequence":active["sequence"],"stage":"succeeded","actual":observation["actual"]});
    assert!(poll(&mut ws, &observation).await.is_none());
    observation["report"] = Value::Null;
    ws = socket(&admin, &credential).await;
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_object());
    // Session switch invalidates active enter. New host instances cannot replay it.
    observation["session_key"] = json!("session-two");
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    preview(&operator, &mut input).await;
    expect_ok(operator.post("waiting/commit", &input).await).await;
    observation["instance_id"] = json!("host-two");
    assert!(poll(&mut ws, &observation).await.is_none());
    ws = socket(&admin, &credential).await;
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    // Disable/enable between polls must not revive an old authorization.
    preview(&operator, &mut input).await;
    expect_ok(operator.post("waiting/commit", &input).await).await;
    expect_ok(
        admin
            .post(
                "devices/action",
                &device_action(&admin, "a", "disable").await,
            )
            .await,
    )
    .await;
    expect_ok(
        admin
            .post(
                "devices/action",
                &device_action(&admin, "a", "enable").await,
            )
            .await,
    )
    .await;
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    // Expiration uses server time and is durable.
    preview(&operator, &mut input).await;
    expect_ok(operator.post("waiting/commit", &input).await).await;
    db.execute(
        "UPDATE waiting_commands SET expires_at=0 WHERE request_id=$1",
        &[&input["request_id"].as_str().unwrap()],
    )
    .await
    .unwrap();
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    // A real two-socket batch records one success and one explicit failure independently.
    let credential_b = token();
    expect_ok(
        admin
            .post(
                "devices",
                &json!({"id":"b","name":"Synthetic B","enrollment_token":token()}),
            )
            .await,
    )
    .await;
    db.execute("UPDATE devices SET lifecycle='enabled',credential_hash=$1,observed_ip='10.9.2.1',last_seen=$2 WHERE id='b'",&[&wccs_protocol::digest(&credential_b),&wccs_protocol::now()]).await.unwrap();
    let mut imported = roster_input(
        &admin,
        "contest",
        json!({"kind":"import","text":"code\tname\tschool\n002\t陈小川\t合成学校"}),
    )
    .await;
    roster_commit(&admin, &mut imported).await;
    let state = admin.state().await;
    let person = state["roster"]["contests"][0]["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["code"] == "002")
        .unwrap()["id"]
        .clone();
    let mut assigned = roster_input(
        &admin,
        "contest",
        json!({"kind":"assign","changes":[{"participant_id":person,"seat_id":seat_b}]}),
    )
    .await;
    roster_commit(&admin, &mut assigned).await;
    let mut b = socket_for(&admin, &credential_b, "b").await;
    let mut bp = observation.clone();
    bp["instance_id"] = json!("host-b");
    assert!(poll(&mut b, &bp).await.is_some());
    let mut batch = input.clone();
    batch["seat_ids"] = json!([seat, seat_b]);
    preview(&admin, &mut batch).await;
    let mut concurrent = batch.clone();
    concurrent["request_id"] = json!(token());
    let (left, right) = tokio::join!(
        admin.post("waiting/commit", &batch),
        admin.post("waiting/commit", &concurrent)
    );
    let receipt = if left.status() == 200 {
        assert_eq!(right.status(), 409);
        expect_ok(left).await
    } else {
        assert_eq!(left.status(), 409);
        expect_ok(right).await
    };
    let ca = &receipt["commands"][0];
    let cb = &receipt["commands"][1];
    observation["actual"] = json!({"state":"waiting","command_id":ca["command_id"],"session_key":"session-two","monitors":1,"detail":"ui_observed"});
    observation["report"] = json!({"command_id":ca["command_id"],"sequence":ca["sequence"],"stage":"succeeded","actual":observation["actual"]});
    bp["actual"] = json!({"state":"failed","command_id":null,"session_key":null,"monitors":0,"detail":"ipc_rejected"});
    bp["report"] = json!({"command_id":cb["command_id"],"sequence":cb["sequence"],"stage":"failed","actual":bp["actual"]});
    assert!(poll(&mut ws, &observation).await.is_some());
    assert!(poll(&mut b, &bp).await.is_some());
    let rows = expect_ok(admin.get("waiting").await).await;
    assert_eq!(rows["items"][0]["stage"], "failed");
    assert_eq!(rows["items"][1]["stage"], "succeeded");
    let filtered = expect_ok(viewer.get("waiting").await).await;
    assert!(
        filtered["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["command"]["device_id"] == "a")
    );
    db.execute(
        "UPDATE devices SET observed_ip='10.9.2.1' WHERE id='a'",
        &[],
    )
    .await
    .unwrap();
    let moved = expect_ok(viewer.get("waiting").await).await;
    assert!(
        moved["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["actual"].is_null() && i["last_seen"].is_null())
    );
    db.execute(
        "UPDATE devices SET observed_ip='10.9.1.1' WHERE id='a'",
        &[],
    )
    .await
    .unwrap();
    let _ = b.close(None).await;
    observation["report"] = Value::Null;
    // Regressive stage reports are refused after completion.
    let mut wrong = observation.clone();
    wrong["report"] = json!({"command_id":ca["command_id"],"sequence":ca["sequence"],"stage":"accepted","actual":observation["actual"]});
    assert!(poll(&mut ws, &wrong).await.is_none());
    ws = socket(&admin, &credential).await;
    assert!(poll(&mut ws, &observation).await.is_some());
    // Credential rotation retires open sockets before they can write.
    let rotated = token();
    db.execute(
        "UPDATE devices SET credential_hash=$1 WHERE id='a'",
        &[&wccs_protocol::digest(&rotated)],
    )
    .await
    .unwrap();
    assert!(poll(&mut ws, &observation).await.is_none());
    ws = socket(&admin, &rotated).await;
    assert!(poll(&mut ws, &observation).await.is_some());
    // Identity generation and revocation are checked again on every exchange.
    preview(&operator, &mut input).await;
    expect_ok(operator.post("waiting/commit", &input).await).await;
    db.execute(
        "UPDATE devices SET enrollment_generation=enrollment_generation+1 WHERE id='a'",
        &[],
    )
    .await
    .unwrap();
    assert!(poll(&mut ws, &observation).await.is_none());
    ws = socket(&admin, &rotated).await;
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    // Oversized and malformed frames cannot reach the control mutation boundary.
    ws.send(Message::Text("x".repeat(65_537).into()))
        .await
        .unwrap();
    assert!(!matches!(ws.next().await, Some(Ok(Message::Text(_)))));
    ws = socket(&admin, &rotated).await;
    assert!(poll(&mut ws, &observation).await.is_some());
    preview(&operator, &mut input).await;
    let before = expect_ok(operator.post("waiting/commit", &input).await).await;
    let _ = ws.close(None).await;
    server.abort();
    let _ = server.await;
    let (mut restarted, server) = start(config).await;
    restarted.login().await;
    let operator = restarted
        .identity("control-operator", "synthetic-operator-password")
        .await;
    assert_eq!(
        expect_ok(operator.post("waiting/commit", &input).await).await,
        before
    );
    let mut ws = socket(&restarted, &rotated).await;
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    // More than 500 retries on one device cannot hide another device's latest status.
    db.batch_execute("WITH noise AS MATERIALIZED (SELECT nextval(pg_get_serial_sequence('waiting_commands','sequence')) AS seq,'noise-'||n::text AS id FROM generate_series(1,501) n), base AS (SELECT * FROM waiting_commands WHERE device_id='a' ORDER BY sequence DESC LIMIT 1)
        INSERT INTO waiting_commands(sequence,id,request_id,device_id,actor_id,actor_version,room_ids,contest_id,generation,instance_id,session_key,expires_at,command,stage,updated_at)
        SELECT noise.seq,noise.id,'capacity-synthetic',base.device_id,base.actor_id,base.actor_version,base.room_ids,base.contest_id,base.generation,base.instance_id,base.session_key,base.expires_at,
        base.command||jsonb_build_object('sequence',noise.seq,'command_id',noise.id,'request_id','capacity-synthetic'),'cancelled',base.updated_at FROM noise CROSS JOIN base").await.unwrap();
    let fleet = expect_ok(restarted.get("waiting?contest_id=contest").await).await;
    assert_eq!(fleet["items"].as_array().unwrap().len(), 2);
    assert!(
        fleet["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["command"]["device_id"] == "b")
    );
    let _ = ws.close(None).await;
    server.abort();
    let _ = server.await;
    db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    drop(db);
    connection_handle.abort();
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn login_preview_connection_fencing_and_completion() {
    let database = std::env::var("WCCS_TEST_DATABASE_URL").unwrap();
    let (db, connection) = tokio_postgres::connect(&database, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_handle = tokio::spawn(async move {
        connection.await.unwrap();
    });
    let schema = format!("test_login_{}", &token()[..24]);
    db.batch_execute(&format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema}"
    ))
    .await
    .unwrap();
    let salt = SaltString::encode_b64(b"synthetic-salt-16").unwrap();
    let config = Config {
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
    };
    let (mut admin, server) = start(config.clone()).await;
    let mut input = json!({"request_id":token(),"contest_id":"contest","seat_ids":["seat"],"action":"enter","duration_seconds":900});
    assert_eq!(admin.get("waiting").await.status(), 401);
    for path in ["waiting/preview", "waiting/commit"] {
        assert_eq!(admin.post(path, &input).await.status(), 401);
    }
    admin.login().await;
    for (id, ip) in [("A", "10.9.1.1"), ("B", "10.9.2.1")] {
        let mut room = room_input(id, 1, 1);
        room["seats"][0]["expected_ip"] = json!(ip);
        preview_room(&admin, &mut room).await;
        expect_ok(admin.post("rooms/save", &room).await).await;
    }
    let credential = token();
    expect_ok(
        admin
            .post(
                "devices",
                &json!({"id":"a","name":"Synthetic control","enrollment_token":token()}),
            )
            .await,
    )
    .await;
    db.execute("UPDATE devices SET lifecycle='enabled',credential_hash=$1,observed_ip='10.9.1.1',last_seen=$2 WHERE id='a'",&[&wccs_protocol::digest(&credential),&wccs_protocol::now()]).await.unwrap();
    let mut create = roster_input(
        &admin,
        "contest",
        json!({"kind":"create","name":"合成等待比赛"}),
    )
    .await;
    roster_commit(&admin, &mut create).await;
    let mut import = roster_input(
        &admin,
        "contest",
        json!({"kind":"import","text":"code\tname\tschool\n001\t林小禾\t合成学校"}),
    )
    .await;
    roster_commit(&admin, &mut import).await;
    let state = admin.state().await;
    let seat = state["seating"]["rooms"][0]["seats"][0]["id"].clone();
    let seat_b = state["seating"]["rooms"][1]["seats"][0]["id"].clone();
    let participant = state["roster"]["contests"][0]["participants"][0]["id"].clone();
    let mut assign = roster_input(
        &admin,
        "contest",
        json!({"kind":"assign","changes":[{"participant_id":participant,"seat_id":seat}]}),
    )
    .await;
    roster_commit(&admin, &mut assign).await;
    input["seat_ids"] = json!([seat]);
    let operator_input = operator_write(
        "control-operator",
        "control-operator",
        "operator",
        json!(["A"]),
    );
    let viewer_input = operator_write("control-viewer", "control-viewer", "viewer", json!(["A"]));
    expect_ok(admin.post("operators", &operator_input).await).await;
    expect_ok(admin.post("operators", &viewer_input).await).await;
    let operator = admin
        .identity("control-operator", "synthetic-operator-password")
        .await;
    let viewer = admin
        .identity("control-viewer", "synthetic-operator-password")
        .await;
    for path in ["waiting/preview", "waiting/commit"] {
        assert_eq!(viewer.post(path, &input).await.status(), 403);
        let mut cross = input.clone();
        cross["seat_ids"] = json!([seat, seat_b]);
        assert_eq!(operator.post(path, &cross).await.status(), 403);
    }
    assert_eq!(
        preview(&operator, &mut input).await["targets"][0]["status"],
        "offline"
    );

    let mut ws = socket(&admin, &credential).await;
    let account = json!({"adapter":"simulation","revision":0,"phase":"teaching","account":"absent","target_sid":null,"session":"none","observed_sid":null,"desktop_confirmed":false,"recovery_ready":true,"tools_ready":true,"teaching_enabled":true,"detail":"standby"});
    let mut observation = json!({"protocol_version":1,"instance_id":"host-one","session_key":null,"capabilities":["waiting-v1","contest-login-simulation-v1"],"actual":{"state":"desktop","command_id":null,"session_key":null,"monitors":0,"detail":"standby","account":account},"report":null});
    poll(&mut ws, &observation).await.unwrap();
    input["action"] = json!("prepare_login");
    observation["actual"]["state"] = json!("failed");
    observation["actual"]["detail"] = json!("waiting_policy_recovery_failed");
    for action in ["prepare_login", "enter"] {
        input["action"] = json!(action);
        poll(&mut ws, &observation).await.unwrap();
        assert_eq!(
            preview(&operator, &mut input).await["targets"][0]["status"],
            "recovery_required"
        );
    }
    observation["actual"]["state"] = json!("desktop");
    observation["actual"]["detail"] = json!("standby");
    input["action"] = json!("prepare_login");
    poll(&mut ws, &observation).await.unwrap();
    assert_eq!(
        preview(&operator, &mut input).await["targets"][0]["status"],
        "ready"
    );
    for (field, value, reason) in [
        ("tools_ready", json!(false), "tools_unavailable"),
        ("recovery_ready", json!(false), "recovery_unavailable"),
        (
            "session",
            json!("teaching"),
            "session_confirmation_required",
        ),
        ("session", json!("rdp"), "unsupported_session"),
        ("account", json!("conflict"), "account_conflict"),
        ("phase", json!("recovery_failed"), "recovery_required"),
    ] {
        observation["actual"]["account"][field] = value;
        poll(&mut ws, &observation).await.unwrap();
        assert_eq!(
            preview(&operator, &mut input).await["targets"][0]["status"],
            reason
        );
        observation["actual"]["account"] = account.clone();
    }
    poll(&mut ws, &observation).await.unwrap();
    preview(&operator, &mut input).await;
    assert_eq!(viewer.post("waiting/commit", &input).await.status(), 403);
    let result = expect_ok(operator.post("waiting/commit", &input).await).await;
    assert_eq!(
        expect_ok(operator.post("waiting/commit", &input).await).await,
        result
    );
    let mut changed = input.clone();
    changed["confirm_session_end"] = json!(true);
    assert_eq!(
        operator.post("waiting/commit", &changed).await.status(),
        409
    );
    let command = result["commands"][0].clone();
    assert_eq!(
        poll(&mut ws, &observation).await.unwrap()["command"],
        command
    );
    observation["actual"]["account"]["phase"] = json!("preparing");
    for stage in ["accepted", "executing"] {
        observation["report"] = json!({"command_id":command["command_id"],"sequence":command["sequence"],"stage":stage,"actual":observation["actual"]});
        poll(&mut ws, &observation).await.unwrap();
    }
    // Successful submission without SID/desktop proof is rejected and closes the socket.
    observation["report"]["stage"] = json!("succeeded");
    assert!(poll(&mut ws, &observation).await.is_none());
    observation["report"] = Value::Null;
    observation["actual"]["account"] = account.clone();
    let mut ws = socket(&admin, &credential).await;
    assert!(poll(&mut ws, &observation).await.unwrap()["command"].is_null());
    assert_eq!(
        expect_ok(operator.get("waiting").await).await["items"][0]["stage"],
        "cancelled"
    );
    preview(&operator, &mut input).await;
    // A new connection invalidates even a pre-commit preview.
    let mut replacement = socket(&admin, &credential).await;
    poll(&mut replacement, &observation).await.unwrap();
    assert_eq!(operator.post("waiting/commit", &input).await.status(), 409);
    assert!(poll(&mut ws, &observation).await.is_none());
    preview(&operator, &mut input).await;
    let command =
        expect_ok(operator.post("waiting/commit", &input).await).await["commands"][0].clone();
    observation["actual"]["account"] = json!({"adapter":"simulation","revision":8,"phase":"controlled","account":"owned","target_sid":"S-1-5-21-1001","session":"contest","observed_sid":"S-1-5-21-1001","desktop_confirmed":true,"recovery_ready":true,"tools_ready":true,"teaching_enabled":false,"detail":"desktop_observed"});
    observation["report"] = json!({"command_id":command["command_id"],"sequence":command["sequence"],"stage":"succeeded","actual":observation["actual"]});
    poll(&mut replacement, &observation).await.unwrap();
    poll(&mut replacement, &observation).await.unwrap();
    assert_eq!(
        expect_ok(operator.get("waiting").await).await["items"][0]["stage"],
        "succeeded"
    );
    let events = expect_ok(
        operator
            .get(&format!(
                "events?request_id={}",
                input["request_id"].as_str().unwrap()
            ))
            .await,
    )
    .await;
    assert!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["operation"] == "login_succeeded")
    );
    input["action"] = json!("restore_teaching");
    preview(&operator, &mut input).await;
    let restore =
        expect_ok(operator.post("waiting/commit", &input).await).await["commands"][0].clone();
    observation["actual"]["account"]["phase"] = json!("teaching");
    observation["actual"]["account"]["teaching_enabled"] = json!(true);
    observation["actual"]["account"]["detail"] = json!("restored");
    observation["report"] = json!({"command_id":restore["command_id"],"sequence":restore["sequence"],"stage":"succeeded","actual":observation["actual"]});
    poll(&mut replacement, &observation).await.unwrap();
    assert_eq!(
        expect_ok(operator.get("waiting").await).await["items"][0]["actual"]["account"]["desktop_confirmed"],
        true
    );
    // Protocol simulation of native capability verifies tool-version fencing, not Windows login.
    observation["report"] = Value::Null;
    observation["actual"]["account"]["adapter"] = json!("windows_test");
    observation["capabilities"] = json!(["waiting-v1", "contest-login-windows-test-v1"]);
    poll(&mut replacement, &observation).await.unwrap();
    input["action"] = json!("prepare_login");
    assert_eq!(
        preview(&operator, &mut input).await["targets"][0]["status"],
        "tools_unavailable"
    );
    let mut publish = json!({"request_id":token(),"base_version":0,"tools":[{"id":"compiler","name":"Synthetic compiler","kind":"executable","path":"C:/Synthetic/compiler.exe"}],"targets":["a"]});
    expect_ok(admin.post("configuration", &publish).await).await;
    let planned = preview(&operator, &mut input).await;
    assert_eq!(planned["tool_paths"], json!(["C:/Synthetic/compiler.exe"]));
    assert_eq!(planned["targets"][0]["status"], "ready");
    expect_ok(operator.post("waiting/commit", &input).await).await;
    assert!(poll(&mut replacement, &observation).await.unwrap()["command"].is_object());
    publish["request_id"] = json!(token());
    publish["base_version"] = json!(1);
    publish["tools"][0]["path"] = json!("C:/Synthetic/changed.exe");
    expect_ok(admin.post("configuration", &publish).await).await;
    assert!(poll(&mut replacement, &observation).await.unwrap()["command"].is_null());
    drop(ws);
    drop(replacement);
    server.abort();
    db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    connection_handle.abort();
}
