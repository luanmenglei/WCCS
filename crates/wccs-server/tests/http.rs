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
        axum::serve(listener, router(app)).await.unwrap();
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
