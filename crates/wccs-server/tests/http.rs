#![forbid(unsafe_code)]

use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use reqwest::{Client, Response};
use serde_json::{Value, json};
use wccs_protocol::token;
use wccs_server::{Config, connect, router};

struct Harness {
    url: String,
    client: Client,
    cookie: String,
    csrf: String,
}
impl Harness {
    async fn login(&mut self) {
        let response = self
            .client
            .post(format!("{}/api/login", self.url))
            .header("Origin", &self.url)
            .json(&json!({"username":"test-admin","password":"synthetic-test-password"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        self.cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        self.csrf = response.json::<Value>().await.unwrap()["csrf"]
            .as_str()
            .unwrap()
            .to_string();
    }
    async fn post(&self, path: &str, body: &Value) -> Response {
        self.client
            .post(format!("{}/api/{path}", self.url))
            .header("Origin", &self.url)
            .header("Cookie", &self.cookie)
            .header("X-CSRF-Token", &self.csrf)
            .json(body)
            .send()
            .await
            .unwrap()
    }
    async fn device(&self, path: &str, credential: &str, body: &Value) -> Response {
        self.client
            .post(format!("{}/api/device/{path}", self.url))
            .bearer_auth(credential)
            .json(body)
            .send()
            .await
            .unwrap()
    }
    async fn state(&self) -> Value {
        let response = self
            .client
            .get(format!("{}/api/state", self.url))
            .header("Cookie", &self.cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }
}

async fn start(config: Config) -> (Harness, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(&config.bind).await.unwrap();
    let mut config = config;
    config.origin = format!("http://{}", listener.local_addr().unwrap());
    config.bind = listener.local_addr().unwrap().to_string();
    let app = connect(config.clone()).await.unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (
        Harness {
            url: config.origin,
            client: Client::new(),
            cookie: String::new(),
            csrf: String::new(),
        },
        handle,
    )
}

fn result(job: &Value, device: &str) -> Value {
    json!({"protocol_version":1,"device_id":device,"job_id":job["job_id"],"config_version":job["configuration"]["config_version"],"boot_id":"boot-1","tools":job["configuration"]["tools"].as_array().unwrap().iter().map(|tool| {let mut item=tool.clone(); item["check"]=json!({"status":"unavailable","reason":if tool["path"].is_null(){"not_configured"}else{"missing"},"code":null}); item}).collect::<Vec<_>>()})
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn authenticated_configuration_lifecycle() {
    let database = std::env::var("WCCS_TEST_DATABASE_URL").expect("WCCS_TEST_DATABASE_URL");
    let (db, connection) = tokio_postgres::connect(&database, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_handle = tokio::spawn(async move {
        connection.await.unwrap();
    });
    let schema = format!("test_{}", &token()[..24]);
    db.batch_execute(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    let salt = SaltString::encode_b64(b"synthetic-salt-16").unwrap();
    let hash = Argon2::default()
        .hash_password(b"synthetic-test-password", &salt)
        .unwrap()
        .to_string();
    let config = Config {
        bind: "127.0.0.1:0".into(),
        origin: String::new(),
        database_url: format!(
            "{database} options='-c search_path={schema} -c statement_timeout=10000'"
        ),
        static_dir: "../../web/dist".into(),
        trusted_proxies: vec![],
        admin_username: "test-admin".into(),
        admin_password_hash: hash,
    };
    let (mut h, server) = start(config.clone()).await;
    assert_eq!(
        h.client
            .get(format!("{}/api/state", h.url))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        h.client
            .post(format!("{}/api/login", h.url))
            .header("Origin", "https://foreign.invalid")
            .json(&json!({"username":"test-admin","password":"synthetic-test-password"}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        h.post(
            "login",
            &json!({"username":"test-admin","password":"wrong"})
        )
        .await
        .status(),
        401
    );
    h.login().await;
    let csrf = h.csrf.clone();
    h.csrf = "wrong".into();
    assert_eq!(
        h.post(
            "configuration",
            &json!({"request_id":token(),"base_version":0,"tools":[],"targets":[]})
        )
        .await
        .status(),
        403
    );
    h.csrf = csrf;
    let enrollment = token();
    let credential = token();
    let device = json!({"id":"synthetic-1","name":"合成设备","enrollment_token":enrollment});
    assert_eq!(h.post("devices", &device).await.status(), 200);
    assert_eq!(h.post("devices", &device).await.status(), 200);
    let registration = json!({"protocol_version":1,"device_id":"synthetic-1","enrollment_token":enrollment,"device_token":credential});
    assert_eq!(h.post("device/register", &registration).await.status(), 200);
    assert_eq!(h.post("device/register", &registration).await.status(), 200);
    let second_enrollment = token();
    assert_eq!(
        h.post(
            "devices",
            &json!({"id":"synthetic-2","name":"第二合成设备","enrollment_token":second_enrollment})
        )
        .await
        .status(),
        200
    );
    assert_eq!(h.post("device/register", &json!({"protocol_version":1,"device_id":"synthetic-2","enrollment_token":second_enrollment,"device_token":credential})).await.status(),409);
    let mut takeover = registration.clone();
    takeover["device_token"] = json!(token());
    assert_eq!(h.post("device/register", &takeover).await.status(), 409);
    let heartbeat = json!({"protocol_version":1,"boot_id":"boot-1","agent_version":"test"});
    assert_eq!(
        h.device("heartbeat", &token(), &heartbeat).await.status(),
        401
    );
    assert_eq!(
        h.device("heartbeat", &credential, &heartbeat)
            .await
            .status(),
        200
    );
    let publish = json!({"request_id":token(),"base_version":0,"tools":[{"id":"submit","name":"脚本","kind":"script","path":null},{"id":"custom","name":"自定义","kind":"executable","path":"C:\\synthetic\\missing.exe"}],"targets":["synthetic-1"]});
    let mut invalid = publish.clone();
    invalid["tools"][1]["path"] = json!("relative.exe");
    assert_eq!(h.post("configuration", &invalid).await.status(), 400);
    assert_eq!(h.state().await["version"], 0);
    assert_eq!(h.post("configuration", &publish).await.status(), 200);
    assert_eq!(h.post("configuration", &publish).await.status(), 200);
    assert_eq!(h.state().await["version"], 1);
    let mut reused = publish.clone();
    reused["tools"][0]["name"] = json!("changed");
    assert_eq!(h.post("configuration", &reused).await.status(), 409);
    let old_job = h
        .device("heartbeat", &credential, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap()["job"]
        .clone();
    let old_result = result(&old_job, "synthetic-1");
    assert_eq!(
        h.device("result", &credential, &old_result).await.status(),
        200
    );
    assert_eq!(
        h.device("result", &credential, &old_result).await.status(),
        200
    );
    assert_eq!(h.state().await["devices"][0]["stale"], false);
    let mut mismatched = old_result.clone();
    mismatched["device_id"] = json!("other-device");
    assert_eq!(
        h.device("result", &credential, &mismatched).await.status(),
        403
    );
    mismatched = old_result.clone();
    mismatched["tools"][1]["path"] = json!("C:\\different.exe");
    assert_eq!(
        h.device("result", &credential, &mismatched).await.status(),
        400
    );
    mismatched = old_result.clone();
    mismatched["tools"][1]["check"]["reason"] = json!("permission_denied");
    assert_eq!(
        h.device("result", &credential, &mismatched).await.status(),
        409
    );
    let mut next = publish.clone();
    next["base_version"] = json!(1);
    next["request_id"] = json!(token());
    let mut concurrent = next.clone();
    concurrent["request_id"] = json!(token());
    let (a, b) = tokio::join!(
        h.post("configuration", &next),
        h.post("configuration", &concurrent)
    );
    let statuses = [a.status().as_u16(), b.status().as_u16()];
    assert!(statuses == [200, 409] || statuses == [409, 200]);
    assert_eq!(h.state().await["version"], 2);
    assert_eq!(h.state().await["devices"][0]["stale"], true);
    assert_eq!(
        h.device("result", &credential, &old_result)
            .await
            .json::<Value>()
            .await
            .unwrap()["stale"],
        true
    );
    let current_job = h
        .device("heartbeat", &credential, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap()["job"]
        .clone();
    let recheck = json!({"request_id":token(),"base_version":2,"targets":["synthetic-1"]});
    assert_eq!(h.post("recheck", &recheck).await.status(), 200);
    let desired = h.state().await["devices"][0]["desired_job"].clone();
    assert_eq!(h.post("recheck", &recheck).await.status(), 200);
    assert_eq!(h.state().await["devices"][0]["desired_job"], desired);
    let newest_job = h
        .device("heartbeat", &credential, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap()["job"]
        .clone();
    assert_eq!(
        h.device("result", &credential, &result(&newest_job, "synthetic-1"))
            .await
            .status(),
        200
    );
    // A delayed result from a superseded task must never replace newer evidence.
    assert_eq!(
        h.device("result", &credential, &result(&current_job, "synthetic-1"))
            .await
            .json::<Value>()
            .await
            .unwrap()["stale"],
        true
    );
    assert_eq!(h.state().await["devices"][0]["result"]["job_id"], desired);
    assert_eq!(h.state().await["devices"][0]["stale"], false);
    db.batch_execute(&format!("UPDATE {schema}.devices SET last_seen=0"))
        .await
        .unwrap();
    assert_eq!(h.state().await["devices"][0]["online"], false);
    h.device("heartbeat", &credential, &heartbeat).await;
    assert_eq!(h.state().await["devices"][0]["online"], true);
    db.batch_execute(&format!(
        "UPDATE {schema}.jobs SET expires_at=0 WHERE result IS NOT NULL"
    ))
    .await
    .unwrap();
    assert_eq!(h.state().await["devices"][0]["expired"], false);
    server.abort();
    let _ = server.await;
    let (mut restarted, server) = start(config).await;
    restarted.cookie = h.cookie.clone();
    assert_eq!(
        restarted
            .client
            .get(format!("{}/api/state", restarted.url))
            .header("Cookie", &restarted.cookie)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    restarted.login().await;
    assert_eq!(restarted.state().await["version"], 2);
    assert_eq!(
        restarted.state().await["devices"][0]["result"]["job_id"],
        desired
    );
    assert_eq!(
        restarted.post("configuration", &publish).await.status(),
        200
    );
    assert_eq!(restarted.state().await["version"], 2);
    assert_eq!(
        restarted
            .device("heartbeat", &credential, &heartbeat)
            .await
            .status(),
        200
    );
    let recheck = json!({"request_id":token(),"base_version":2,"targets":["synthetic-1"]});
    assert_eq!(restarted.post("recheck", &recheck).await.status(), 200);
    let expired_job = restarted
        .device("heartbeat", &credential, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap()["job"]
        .clone();
    db.batch_execute(&format!(
        "UPDATE {schema}.jobs SET expires_at=0 WHERE result IS NULL"
    ))
    .await
    .unwrap();
    assert_eq!(
        restarted
            .device("result", &credential, &result(&expired_job, "synthetic-1"))
            .await
            .status(),
        410
    );
    assert!(
        restarted
            .device("heartbeat", &credential, &heartbeat)
            .await
            .json::<Value>()
            .await
            .unwrap()["job"]
            .is_null()
    );
    assert_eq!(restarted.state().await["devices"][0]["expired"], true);
    assert_eq!(restarted.post("logout", &json!({})).await.status(), 200);
    assert_eq!(
        restarted
            .client
            .get(format!("{}/api/state", restarted.url))
            .header("Cookie", &restarted.cookie)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    server.abort();
    let _ = server.await;
    // The generated schema name consists solely of a constant prefix and random hex.
    db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    drop(db);
    connection_handle.await.unwrap();
}

fn room_input(id: &str, rows: i32, columns: i32) -> Value {
    let seats: Vec<Value> = (0..rows).flat_map(|row| (0..columns).map(move |column| json!({"row":row,"column":column,"number":row*columns+column+1,"expected_ip":null}))).collect();
    json!({"id":id,"request_id":token(),"base_version":0,"name":id,"rows":rows,"columns":columns,"start_number":1,"numbering":"row","origin":"top-left","seats":seats})
}

async fn preview_room(h: &Harness, input: &mut Value) -> Value {
    input["request_id"] = json!(token());
    let response = h.post("rooms/preview", input).await;
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let response: Value = response.json().await.unwrap();
    input["preview_token"] = response["preview_token"].clone();
    response
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn seating_migration_binding_and_concurrency() {
    let database = std::env::var("WCCS_TEST_DATABASE_URL").unwrap();
    let (db, connection) = tokio_postgres::connect(&database, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_handle = tokio::spawn(async move {
        connection.await.unwrap();
    });
    let schema = format!("test_seating_{}", &token()[..24]);
    db.batch_execute(&format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema}"
    ))
    .await
    .unwrap();
    // Seed an actual v1 database including identity, configuration and old evidence.
    db.batch_execute(include_str!("../migrations/001_tools.sql"))
        .await
        .unwrap();
    let credential = token();
    db.execute("INSERT INTO devices(id,name,enrollment_hash,enrollment_expires,credential_hash,desired_job) VALUES ('legacy','legacy','unused',0,$1,'old-job')",&[&wccs_protocol::digest(&credential)]).await.unwrap();
    db.execute(
        "INSERT INTO configurations VALUES (1,$1,$2,0)",
        &[
            &json!({"schema_version":1,"config_version":"1","tools":[]}),
            &json!(["legacy"]),
        ],
    )
    .await
    .unwrap();
    db.batch_execute("UPDATE configuration_head SET version=1; INSERT INTO jobs(id,device_id,config_version,expires_at,result,received_at) VALUES ('old-job','legacy',1,9999999999,'{\"historical\":true}',0)").await.unwrap();
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
    let (mut h, server) = start(config.clone()).await;
    h.login().await;
    let migrated = h.state().await;
    assert_eq!(migrated["version"], 1);
    assert_eq!(migrated["devices"][0]["result"], json!({"historical":true}));
    assert!(migrated["seating"]["rooms"].as_array().unwrap().is_empty());
    assert_eq!(
        migrated["seating"]["devices"][0]["observed_ip"],
        Value::Null
    );
    assert_eq!(
        db.query_one("SELECT version FROM schema_version", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        2
    );
    let mut room = room_input("A", 2, 3);
    room["seats"][0]["expected_ip"] = json!("127.0.0.1");
    assert_eq!(
        h.client
            .post(format!("{}/api/rooms/save", h.url))
            .json(&room)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        h.client
            .post(format!("{}/api/rooms/save", h.url))
            .header("Cookie", &h.cookie)
            .header("Origin", &h.url)
            .json(&room)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(h.post("rooms/save", &room).await.status(), 409);
    preview_room(&h, &mut room).await;
    assert_eq!(h.post("rooms/save", &room).await.status(), 200);
    assert_eq!(h.post("rooms/save", &room).await.status(), 200);
    let mut reused = room.clone();
    reused["name"] = json!("different");
    assert_eq!(h.post("rooms/save", &reused).await.status(), 409);
    let mut second = room_input("B", 3, 2);
    second["seats"][0]["expected_ip"] = json!("127.0.0.1");
    assert_eq!(h.post("rooms/preview", &second).await.status(), 409);
    second["seats"][0]["expected_ip"] = json!("10.0.1.1");
    preview_room(&h, &mut second).await;
    assert_eq!(h.post("rooms/save", &second).await.status(), 200);
    let old_seat = h.state().await["seating"]["rooms"][0]["seats"][0].clone();
    let heartbeat = json!({"protocol_version":1,"boot_id":"boot","agent_version":"test","reported_ips":["192.0.2.50"]});
    assert_eq!(h.device("heartbeat", "bad", &heartbeat).await.status(), 401);
    // Forged forwarding headers are ignored when the transport peer is untrusted.
    assert_eq!(
        h.client
            .post(format!("{}/api/device/heartbeat", h.url))
            .bearer_auth(&credential)
            .header("X-Forwarded-For", "10.0.1.1")
            .json(&heartbeat)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let bound = h.state().await;
    assert_eq!(bound["seating"]["rooms"][0]["seats"][0]["status"], "online");
    assert_eq!(bound["seating"]["devices"][0]["observed_ip"], "127.0.0.1");
    assert_eq!(
        bound["seating"]["devices"][0]["reported_ips"],
        json!(["192.0.2.50"])
    );
    // Renumber by physical position, preserving stable ID and IP.
    room["base_version"] = json!(1);
    room["numbering"] = json!("custom");
    room["seats"][0]["number"] = json!(6);
    room["seats"][5]["number"] = json!(1);
    preview_room(&h, &mut room).await;
    assert_eq!(h.post("rooms/save", &room).await.status(), 200);
    let seat = h.state().await["seating"]["rooms"][0]["seats"][0].clone();
    assert_eq!(seat["id"], old_seat["id"]);
    assert_eq!(seat["expected_ip"], old_seat["expected_ip"]);
    assert_eq!(seat["devices"], json!(["legacy"]));
    room["base_version"] = json!(2);
    let mut duplicate = room.clone();
    duplicate["seats"][1]["number"] = json!(6);
    assert_eq!(h.post("rooms/preview", &duplicate).await.status(), 400);
    duplicate = room.clone();
    duplicate["seats"][1]["expected_ip"] = json!("127.0.0.1");
    assert_eq!(h.post("rooms/preview", &duplicate).await.status(), 400);
    assert_eq!(h.state().await["seating"]["rooms"][0]["version"], 2);
    // Two administrators preview the same version. Exactly one update wins.
    room["name"] = json!("A edited");
    preview_room(&h, &mut room).await;
    let mut concurrent = room.clone();
    concurrent["request_id"] = json!(token());
    let (a, b) = tokio::join!(
        h.post("rooms/save", &room),
        h.post("rooms/save", &concurrent)
    );
    let mut statuses = vec![a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, vec![200, 409]);
    // A second authenticated device at the same address conflicts even when offline.
    let enroll = token();
    let second_credential = token();
    assert_eq!(
        h.post(
            "devices",
            &json!({"id":"other","name":"other","enrollment_token":enroll})
        )
        .await
        .status(),
        200
    );
    assert_eq!(h.device("register","",&json!({"protocol_version":1,"device_id":"other","enrollment_token":enroll,"device_token":second_credential})).await.status(),200);
    h.device("heartbeat", &second_credential, &heartbeat)
        .await
        .error_for_status()
        .unwrap();
    let conflict = h.state().await;
    assert_eq!(
        conflict["seating"]["rooms"][0]["seats"][0]["status"],
        "conflict"
    );
    let mut selection = json!({"request_id":token(),"base_version":1,"seat_ids":[seat["id"]]});
    let target = h
        .post("seating/check-preview", &selection)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(target["targets"][0]["status"], "conflict");
    assert_eq!(
        target["targets"][0]["device_ids"],
        json!(["legacy", "other"])
    );
    let other = conflict["seating"]["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["id"] == "other")
        .unwrap();
    let review = json!({"request_id":token(),"device_id":"other","base_version":other["address_version"],"exclude":true});
    assert_eq!(h.post("seating/review", &review).await.status(), 200);
    assert_eq!(h.post("seating/review", &review).await.status(), 200);
    let mut stale_review = review.clone();
    stale_review["request_id"] = json!(token());
    assert_eq!(h.post("seating/review", &stale_review).await.status(), 409);
    selection["preview_token"] = target["preview_token"].clone();
    assert_eq!(h.post("seating/check", &selection).await.status(), 409);
    let target = h
        .post("seating/check-preview", &selection)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(target["targets"][0]["status"], "ready");
    selection["preview_token"] = target["preview_token"].clone();
    db.batch_execute("INSERT INTO jobs(id,device_id,config_version,expires_at) VALUES ('unrelated-job','other',1,9999999999); UPDATE devices SET desired_job='unrelated-job' WHERE id='other'").await.unwrap();
    let issued = h
        .post("seating/check", &selection)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(issued["issued"], json!(["legacy"]));
    assert_eq!(
        h.state().await["devices"][1]["desired_job"],
        "unrelated-job"
    );
    let job = h.state().await["devices"][0]["desired_job"].clone();
    assert_eq!(
        h.post("seating/check", &selection)
            .await
            .json::<Value>()
            .await
            .unwrap(),
        issued
    );
    assert_eq!(h.state().await["devices"][0]["desired_job"], job);
    assert_eq!(h.state().await["targets"], json!(["legacy"]));
    db.batch_execute("UPDATE devices SET last_seen=0 WHERE id='legacy'")
        .await
        .unwrap();
    assert_eq!(
        h.post("seating/check-preview", &selection)
            .await
            .json::<Value>()
            .await
            .unwrap()["targets"][0]["status"],
        "offline"
    );
    h.device("heartbeat", &credential, &heartbeat)
        .await
        .error_for_status()
        .unwrap();
    // Resize preview explicitly carries removed position and its live association.
    let mut shrink = room_input("A", 1, 1);
    shrink["base_version"] = json!(3);
    shrink["seats"][0]["expected_ip"] = json!("127.0.0.1");
    let impact = preview_room(&h, &mut shrink).await;
    assert_eq!(
        impact["changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|change| change["kind"] == "removed")
            .count(),
        5
    );
    assert_eq!(h.post("rooms/save", &shrink).await.status(), 200);
    assert_eq!(h.state().await["devices"][0]["desired_job"], job);
    server.abort();
    let _ = server.await;
    let mut proxy_config = config;
    proxy_config.trusted_proxies = vec!["127.0.0.1".parse().unwrap()];
    let (mut restarted, server) = start(proxy_config).await;
    restarted.login().await;
    assert_eq!(
        restarted.state().await["seating"]["rooms"][0]["seats"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(restarted.post("rooms/save", &shrink).await.status(), 200);
    assert_eq!(
        restarted
            .post("seating/check", &selection)
            .await
            .json::<Value>()
            .await
            .unwrap(),
        issued
    );
    // Simulated explicit proxy causes an observed address change, requiring review.
    restarted
        .client
        .post(format!("{}/api/device/heartbeat", restarted.url))
        .bearer_auth(&credential)
        .header("X-Forwarded-For", "10.0.1.1")
        .json(&heartbeat)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let changed = restarted.state().await;
    assert_eq!(
        changed["seating"]["rooms"][0]["seats"][0]["status"],
        "unbound"
    );
    assert_eq!(
        changed["seating"]["rooms"][1]["seats"][0]["status"],
        "conflict"
    );
    assert_eq!(changed["seating"]["devices"][0]["needs_review"], true);
    assert_eq!(changed["seating"]["devices"][0]["previous_ip"], "127.0.0.1");
    let accept = json!({"request_id":token(),"device_id":"legacy","base_version":changed["seating"]["devices"][0]["address_version"],"exclude":false});
    assert_eq!(
        restarted.post("seating/review", &accept).await.status(),
        200
    );
    assert_eq!(
        restarted.state().await["seating"]["rooms"][1]["seats"][0]["status"],
        "online"
    );
    restarted
        .device("heartbeat", &credential, &heartbeat)
        .await
        .error_for_status()
        .unwrap();
    assert_eq!(
        restarted.state().await["seating"]["devices"][0]["address_source"],
        "invalid_proxy_header"
    );
    assert_eq!(
        restarted.state().await["seating"]["devices"][0]["observed_ip"],
        Value::Null
    );
    // Cross-room concurrent claims are serialized and roll back as a whole.
    let mut c = room_input("C", 1, 2);
    let mut d = room_input("D", 1, 2);
    c["seats"][1]["expected_ip"] = json!("192.0.2.111");
    d["seats"][1]["expected_ip"] = json!("192.0.2.111");
    preview_room(&restarted, &mut c).await;
    preview_room(&restarted, &mut d).await;
    let (first, second) = tokio::join!(
        restarted.post("rooms/save", &c),
        restarted.post("rooms/save", &d)
    );
    let mut codes = vec![first.status().as_u16(), second.status().as_u16()];
    codes.sort();
    assert_eq!(codes, vec![200, 409]);
    assert_eq!(
        restarted.state().await["seating"]["rooms"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    server.abort();
    let _ = server.await;
    db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    drop(db);
    connection_handle.await.unwrap();
}
