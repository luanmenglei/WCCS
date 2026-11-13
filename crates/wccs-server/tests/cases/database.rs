use super::*;
use std::time::{Duration, Instant};
use waiting_tests::{poll, socket_for};

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

fn input(count: usize) -> Value {
    json!({"request_id":token(),"contest_id":"contest","seat_ids":(1..=count).map(|n|format!("seat-{n}")).collect::<Vec<_>>(),"action":"enter","duration_seconds":900})
}

async fn backend(db: &tokio_postgres::Client) -> i32 {
    db.query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0)
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
