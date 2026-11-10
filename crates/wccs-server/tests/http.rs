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

async fn device_action(h: &Harness, id: &str, action: &str) -> Value {
    let state = h.state().await;
    let device = state["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    json!({"request_id":token(),"base_version":device["lifecycle_version"],"device_id":id,"action":action})
}

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn enrollment_lifecycle_rotation_and_restart() {
    let database = std::env::var("WCCS_TEST_DATABASE_URL").unwrap();
    let (db, connection) = tokio_postgres::connect(&database, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_handle = tokio::spawn(async move {
        connection.await.unwrap();
    });
    let schema = format!("test_lifecycle_{}", &token()[..24]);
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
    let (mut h, server) = start(config.clone()).await;
    let heartbeat = json!({"protocol_version":1,"boot_id":"boot","agent_version":"test"});
    let enrollment = token();
    let credential = token();
    let registration = json!({"protocol_version":1,"device_id":"review","enrollment_token":enrollment,"device_token":credential});
    for path in ["registration-window", "devices/action"] {
        assert_eq!(h.post(path,&json!({"request_id":token(),"base_version":1,"open":true,"device_id":"review","action":"approve"})).await.status(),422);
    }
    assert_eq!(
        h.post(
            "registration-window",
            &json!({"request_id":token(),"base_version":1,"open":true})
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        h.post(
            "devices/action",
            &json!({"request_id":token(),"base_version":1,"device_id":"review","action":"approve"})
        )
        .await
        .status(),
        401
    );
    h.login().await;
    assert_eq!(h.state().await["registration_window"]["open"], false);
    assert_eq!(
        h.post(
            "devices",
            &json!({"id":"review","name":"synthetic review","enrollment_token":enrollment})
        )
        .await
        .status(),
        200
    );
    let first: Value = h
        .device("register", "", &registration)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(first["status"], "pending");
    assert_eq!(
        h.device("register", "", &registration)
            .await
            .json::<Value>()
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        h.device("heartbeat", &credential, &heartbeat)
            .await
            .status(),
        401
    );
    let mut takeover = registration.clone();
    takeover["device_token"] = json!(token());
    assert_eq!(h.device("register", "", &takeover).await.status(), 409);
    let approve = device_action(&h, "review", "approve").await;
    let mut other = approve.clone();
    other["request_id"] = json!(token());
    let (a, b) = tokio::join!(
        h.post("devices/action", &approve),
        h.post("devices/action", &other)
    );
    let winner = if a.status() == 200 {
        approve.clone()
    } else {
        other
    };
    let mut statuses = vec![a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, vec![200, 409]);
    assert_eq!(h.post("devices/action", &winner).await.status(), 200);
    let mut reused = winner.clone();
    reused["action"] = json!("revoke");
    assert_eq!(h.post("devices/action", &reused).await.status(), 409);
    assert_eq!(
        h.device("heartbeat", &credential, &heartbeat)
            .await
            .status(),
        200
    );
    assert_eq!(
        h.client
            .post(format!("{}/api/device/heartbeat", h.url))
            .bearer_auth(&credential)
            .header("X-WCCS-Device-ID", "different")
            .json(&heartbeat)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let publish = json!({"request_id":token(),"base_version":0,"tools":[{"id":"submit","name":"submit","kind":"script","path":null}],"targets":["review"]});
    assert_eq!(h.post("configuration", &publish).await.status(), 200);
    let job = h
        .device("heartbeat", &credential, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap()["job"]
        .clone();
    let mut room = room_input("lifecycle-room", 1, 1);
    room["seats"][0]["expected_ip"] = json!("127.0.0.1");
    preview_room(&h, &mut room).await;
    assert_eq!(h.post("rooms/save", &room).await.status(), 200);
    let state = h.state().await;
    let seat_id = state["seating"]["rooms"][0]["seats"][0]["id"].clone();
    let selection =
        json!({"request_id":token(),"base_version":1,"seat_ids":[seat_id],"preview_token":""});
    let disable = device_action(&h, "review", "disable").await;
    assert_eq!(h.post("devices/action", &disable).await.status(), 200);
    let paused = h
        .device("heartbeat", &credential, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(paused["lifecycle"], "disabled");
    assert_eq!(
        h.state().await["seating"]["rooms"][0]["seats"][0]["devices"],
        json!(["review"])
    );
    assert_eq!(
        h.post("seating/check-preview", &selection)
            .await
            .json::<Value>()
            .await
            .unwrap()["targets"][0]["status"],
        "disabled"
    );
    assert!(paused["job"].is_null());
    assert_eq!(
        h.device("result", &credential, &result(&job, "review"))
            .await
            .status(),
        200
    );
    assert_eq!(
        h.post(
            "recheck",
            &json!({"request_id":token(),"base_version":1,"targets":["review"]})
        )
        .await
        .status(),
        400
    );
    let invalid = device_action(&h, "review", "approve").await;
    assert_eq!(h.post("devices/action", &invalid).await.status(), 409);
    let enable = device_action(&h, "review", "enable").await;
    assert_eq!(h.post("devices/action", &enable).await.status(), 200);
    assert_eq!(
        h.client
            .post(format!("{}/api/devices/action", h.url))
            .header("Cookie", &h.cookie)
            .header("Origin", &h.url)
            .json(&enable)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let rotation = device_action(&h, "review", "rotate").await;
    assert_eq!(h.post("devices/action", &rotation).await.status(), 200);
    let next = token();
    let switch = json!({"device_id":"review","request_id":rotation["request_id"],"new_token":next});
    assert_eq!(h.device("rotate", &credential, &switch).await.status(), 200);
    assert_eq!(h.device("rotate", &credential, &switch).await.status(), 200);
    assert_eq!(
        h.device("heartbeat", &credential, &heartbeat)
            .await
            .status(),
        401
    );
    assert_eq!(h.device("register", "", &registration).await.status(), 401);
    assert_eq!(h.device("heartbeat", &next, &heartbeat).await.status(), 200);
    let mut changed = switch.clone();
    changed["new_token"] = json!(token());
    assert_eq!(
        h.device("rotate", &credential, &changed).await.status(),
        401
    );
    let window = json!({"request_id":token(),"base_version":1,"open":true});
    assert_eq!(h.post("registration-window", &window).await.status(), 200);
    server.abort();
    let (mut h, server) = start(config.clone()).await;
    h.login().await;
    assert_eq!(h.state().await["registration_window"]["open"], true);
    assert_eq!(h.post("registration-window", &window).await.status(), 200);
    assert_eq!(h.device("rotate", &credential, &switch).await.status(), 200);
    assert_eq!(h.post("devices/action", &winner).await.status(), 200);
    db.execute("UPDATE credential_rotations SET expires_at=0", &[])
        .await
        .unwrap();
    assert_eq!(h.device("rotate", &credential, &switch).await.status(), 401);
    let enroll2 = token();
    assert_eq!(
        h.post(
            "devices",
            &json!({"id":"second","name":"second","enrollment_token":enroll2})
        )
        .await
        .status(),
        200
    );
    let register2 = json!({"protocol_version":1,"device_id":"second","enrollment_token":enroll2,"device_token":credential});
    assert_eq!(h.device("register", "", &register2).await.status(), 409);
    let mut register2 = register2;
    register2["device_token"] = json!(token());
    assert_eq!(
        h.device("register", "", &register2)
            .await
            .json::<Value>()
            .await
            .unwrap()["status"],
        "enabled"
    );
    assert_eq!(
        h.post(
            "recheck",
            &json!({"request_id":token(),"base_version":1,"targets":["review"]})
        )
        .await
        .status(),
        200
    );
    let cancelled = h
        .device("heartbeat", &next, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap()["job"]
        .clone();
    let revoke = device_action(&h, "review", "revoke").await;
    assert_eq!(h.post("devices/action", &revoke).await.status(), 200);
    assert_eq!(
        h.state().await["seating"]["rooms"][0]["seats"][0]["devices"],
        json!([])
    );
    assert_eq!(h.device("heartbeat", &next, &heartbeat).await.status(), 401);
    assert_eq!(
        h.device("result", &next, &result(&cancelled, "review"))
            .await
            .status(),
        401
    );
    assert_eq!(h.device("rotate", &credential, &switch).await.status(), 401);
    assert_eq!(h.state().await["targets"], json!(["review"]));
    assert_eq!(
        h.device(
            "heartbeat",
            register2["device_token"].as_str().unwrap(),
            &heartbeat
        )
        .await
        .status(),
        200
    );
    let mut reauthorize = device_action(&h, "review", "reauthorize").await;
    let material = token();
    reauthorize["enrollment_token"] = json!(material);
    assert_eq!(h.post("devices/action", &reauthorize).await.status(), 200);
    assert_eq!(h.post("devices/action", &reauthorize).await.status(), 200);
    assert_eq!(h.device("register", "", &registration).await.status(), 401);
    assert_eq!(h.device("register","",&json!({"protocol_version":1,"device_id":"review","enrollment_token":material,"device_token":next})).await.status(),409);
    let recovered = token();
    let recovery = json!({"protocol_version":1,"device_id":"review","enrollment_token":material,"device_token":recovered});
    assert_eq!(
        h.device("register", "", &recovery)
            .await
            .json::<Value>()
            .await
            .unwrap()["status"],
        "pending"
    );
    // Pending review, candidate ownership and deduplication survive a process restart.
    server.abort();
    let (mut h, server) = start(config).await;
    h.login().await;
    assert_eq!(
        h.device("register", "", &recovery)
            .await
            .json::<Value>()
            .await
            .unwrap()["status"],
        "pending"
    );
    let approve = device_action(&h, "review", "approve").await;
    assert_eq!(h.post("devices/action", &approve).await.status(), 200);
    let restored = h
        .device("heartbeat", &recovered, &heartbeat)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert!(restored["job"].is_null());
    assert_eq!(
        h.device("result", &recovered, &result(&cancelled, "review"))
            .await
            .status(),
        410
    );
    let history = h
        .client
        .get(format!("{}/api/devices/review/history", h.url))
        .header("Cookie", &h.cookie)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(history["jobs"][0]["cancellation_reason"], "device_revoked");
    let serialized = history.to_string();
    assert!(!serialized.contains(&credential));
    assert!(!serialized.contains(&material));
    assert_eq!(
        h.client
            .get(format!("{}/api/devices/review/history", h.url))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let close = json!({"request_id":token(),"base_version":2,"open":false});
    assert_eq!(h.post("registration-window", &close).await.status(), 200);
    for id in ["reject", "expired"] {
        let material = token();
        assert_eq!(
            h.post(
                "devices",
                &json!({"id":id,"name":id,"enrollment_token":material})
            )
            .await
            .status(),
            200
        );
        let request = json!({"protocol_version":1,"device_id":id,"enrollment_token":material,"device_token":token()});
        assert_eq!(
            h.device("register", "", &request)
                .await
                .json::<Value>()
                .await
                .unwrap()["status"],
            "pending"
        );
        if id == "reject" {
            let action = device_action(&h, id, "reject").await;
            assert_eq!(h.post("devices/action", &action).await.status(), 200);
        } else {
            db.execute(
                "UPDATE devices SET enrollment_expires=0 WHERE id=$1",
                &[&id],
            )
            .await
            .unwrap();
        }
        assert_eq!(h.device("register", "", &request).await.status(), 401);
    }
    assert_eq!(
        h.state().await["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "expired")
            .unwrap()["lifecycle"],
        "expired"
    );
    // A global publish retains a disabled target without allowing it to execute.
    let disable = device_action(&h, "review", "disable").await;
    assert_eq!(h.post("devices/action", &disable).await.status(), 200);
    assert_eq!(
        h.post(
            "configuration",
            &json!({"request_id":token(),"base_version":1,"tools":[],"targets":["review","second"]})
        )
        .await
        .status(),
        200
    );
    assert!(
        h.device("heartbeat", &recovered, &heartbeat)
            .await
            .json::<Value>()
            .await
            .unwrap()["job"]
            .is_null()
    );
    let enable = device_action(&h, "review", "enable").await;
    assert_eq!(h.post("devices/action", &enable).await.status(), 200);
    assert_eq!(
        h.device("heartbeat", &recovered, &heartbeat)
            .await
            .json::<Value>()
            .await
            .unwrap()["job"]["configuration"]["config_version"],
        "2"
    );
    let empty_history = h
        .client
        .get(format!(
            "{}/api/devices/review/history?before=0&jobs_before=0",
            h.url
        ))
        .header("Cookie", &h.cookie)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(empty_history["events"], json!([]));
    assert_eq!(empty_history["jobs"], json!([]));
    server.abort();
    db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    connection_handle.abort();
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
    assert_eq!(
        h.post(
            "registration-window",
            &json!({"request_id":token(),"base_version":1,"open":true})
        )
        .await
        .status(),
        200
    );
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
    assert_eq!(h.post("device/register", &takeover).await.status(), 401);
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
    assert_eq!(
        h.post(
            "registration-window",
            &json!({"request_id":token(),"base_version":1,"open":true})
        )
        .await
        .status(),
        200
    );
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
        3
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
