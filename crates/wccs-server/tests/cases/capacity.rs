use super::*;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
use tokio::{sync::mpsc, task::JoinSet};

#[path = "capacity_transfer.rs"]
mod transfer;

enum Event {
    Ready,
    Received(Instant),
    Reconnected,
}

async fn receive_events(receiver: &mut mpsc::UnboundedReceiver<Event>, count: usize) -> Vec<Event> {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            events.push(
                receiver
                    .recv()
                    .await
                    .expect("all clients must remain alive"),
            );
        }
        events
    })
    .await
    .expect("clients did not finish the current phase within 60 seconds")
}

fn percentile(values: &[u128], percent: usize) -> u128 {
    values[(values.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn concurrent_control_preview_commit_and_reconnect() {
    let count: usize = std::env::var("WCCS_LOAD_CLIENTS")
        .map(|value| value.parse().expect("integer client count"))
        .unwrap_or(300);
    let seconds: u64 = std::env::var("WCCS_LOAD_SECONDS")
        .map(|value| value.parse().expect("integer duration"))
        .unwrap_or(15);
    assert!((1..=500).contains(&count));
    assert!((5..=3600).contains(&seconds));
    let downloaders = std::env::var("WCCS_LOAD_DOWNLOADERS")
        .map(|value| value.parse::<usize>().expect("integer downloader count"))
        .unwrap_or(0);
    assert!(downloaders <= count);
    #[cfg(target_os = "linux")]
    {
        // This process owns both ends of every test connection, plus management
        // and database sockets. Fail before starting clients if the shell is capped.
        let limits = std::fs::read_to_string("/proc/self/limits").unwrap();
        let soft_limit = limits
            .lines()
            .find(|line| line.starts_with("Max open files"))
            .and_then(|line| line.split_whitespace().nth(3))
            .and_then(|value| value.parse::<usize>().ok());
        assert!(
            soft_limit.is_none_or(|limit| limit >= (count + downloaders) * 2 + 128),
            "load test owns client and server sockets; run with ulimit -n 4096"
        );
    }
    let f = Fixture::new().await;
    let mut credentials = Vec::with_capacity(count);
    for number in 1..=count {
        let credential = token();
        f.db.execute(
            "UPDATE devices SET credential_hash=$1 WHERE id=$2",
            &[
                &wccs_protocol::digest(&credential),
                &format!("device-{number}"),
            ],
        )
        .await
        .unwrap();
        credentials.push(credential);
    }
    let phase = Arc::new(AtomicU8::new(0));
    let download_credentials = credentials[..downloaders].to_vec();
    // Keep one counter per observed millisecond across the fleet so the load
    // generator's sample storage doesn't grow with every poll in an hour-long run.
    let poll_times = Arc::new(Mutex::new(BTreeMap::<u128, usize>::new()));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut clients = JoinSet::new();
    let started = Instant::now();
    for (index, credential) in credentials.into_iter().enumerate() {
        let admin = f.admin.clone();
        let phase = phase.clone();
        let poll_times = poll_times.clone();
        let sender = sender.clone();
        clients.spawn(async move {
            let device = format!("device-{}", index + 1);
            let mut socket = socket_for(&admin, &credential, &device).await;
            let mut state = observation();
            let mut last_command = Value::Null;
            let mut connected = false;
            let mut reconnected = false;
            loop {
                let current_phase = phase.load(Ordering::Relaxed);
                if current_phase == 3 {
                    socket.close(None).await.unwrap();
                    return;
                }
                if current_phase == 2 && !reconnected {
                    socket.close(None).await.unwrap();
                    socket = socket_for(&admin, &credential, &device).await;
                    state = observation();
                    state["instance_id"] = json!("host-restarted");
                    let reply = poll(&mut socket, &state).await.unwrap();
                    assert!(reply["command"].is_null(), "old authorization survived restart");
                    reconnected = true;
                    sender.send(Event::Reconnected).unwrap();
                }
                let begin = Instant::now();
                let reply = poll(&mut socket, &state).await.expect("control channel closed under load");
                let elapsed = begin.elapsed().as_millis();
                *poll_times.lock().unwrap().entry(elapsed).or_default() += 1;
                if !connected {
                    connected = true;
                    sender.send(Event::Ready).unwrap();
                }
                let command = &reply["command"];
                if command.is_object() && command["command_id"] != last_command {
                    assert!(!reconnected, "old command replayed after reconnect");
                    last_command = command["command_id"].clone();
                    sender.send(Event::Received(Instant::now())).unwrap();
                    state["actual"] = json!({"state":"waiting","command_id":last_command,
                        "session_key":"session-one","monitors":1,"detail":"ui_observed"});
                    state["report"] = json!({"command_id":last_command,"sequence":command["sequence"],
                        "stage":"succeeded","actual":state["actual"]});
                } else if command.is_null() && state["actual"]["state"] == "waiting" {
                    state["actual"] = observation()["actual"].take();
                    state["report"] = Value::Null;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
    drop(sender);
    assert!(
        receive_events(&mut receiver, count)
            .await
            .iter()
            .all(|event| matches!(event, Event::Ready))
    );
    let startup_ms = started.elapsed().as_millis();
    let transfers = transfer::start(&f, download_credentials, phase.clone()).await;
    let mut request = input(count);
    // In a long run exercise expiry before the reconnect. In a short run
    // retain a live grant so reconnect itself must cancel the command.
    let test_expiry = seconds >= 900;
    request["duration_seconds"] = json!(if test_expiry {
        seconds - 30
    } else {
        900.max(seconds + 60)
    });
    let preview_start = Instant::now();
    let preview = expect_ok(f.admin.post("waiting/preview", &request).await).await;
    let preview_ms = preview_start.elapsed().as_millis();
    assert!(
        preview["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|target| target["status"] == "ready")
    );
    request["preview_token"] = preview["preview_token"].clone();
    let committed = Instant::now();
    let receipt = expect_ok(f.admin.post("waiting/commit", &request).await).await;
    assert_eq!(receipt["commands"].as_array().unwrap().len(), count);
    assert_eq!(
        expect_ok(f.admin.post("waiting/commit", &request).await).await,
        receipt
    );
    let mut received_ms: Vec<_> = receive_events(&mut receiver, count)
        .await
        .into_iter()
        .map(|event| {
            let Event::Received(at) = event else {
                panic!("unexpected load test phase")
            };
            at.duration_since(committed).as_millis()
        })
        .collect();

    // Exercise the same read endpoints as the management page while all clients renew.
    let soak_started = Instant::now();
    let mut reads = 0;
    while soak_started.elapsed() < Duration::from_secs(seconds) {
        assert_eq!(f.admin.get("waiting").await.status(), 200);
        assert_eq!(f.admin.get("state").await.status(), 200);
        expect_ok(f.admin.post("waiting/preview", &input(count)).await).await;
        reads += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let rows = expect_ok(f.admin.get("waiting").await).await;
    let expected_stage = if test_expiry { "expired" } else { "succeeded" };
    assert_eq!(
        rows["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["stage"] == expected_stage
                && (!test_expiry || row["actual"]["state"] == "desktop"))
            .count(),
        count
    );
    for operation in ["waiting_pending", "waiting_succeeded"] {
        let events: i64 =
            f.db.query_one(
                "SELECT count(*) FROM events WHERE request_id=$1 AND operation=$2",
                &[&request["request_id"].as_str().unwrap(), &operation],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            events as usize, count,
            "duplicate or missing per-device audit event"
        );
    }
    phase.store(2, Ordering::Relaxed);
    if let Some(transfers) = transfers {
        transfers.finish().await;
    }
    assert!(
        receive_events(&mut receiver, count)
            .await
            .iter()
            .all(|event| matches!(event, Event::Reconnected))
    );
    phase.store(3, Ordering::Relaxed);
    while let Some(result) = clients.join_next().await {
        result.expect("client task failed");
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let remaining: i64 = f.db.query_one(
                "SELECT count(*) FROM control_devices WHERE connected AND instance_id='host-restarted'", &[])
                .await.unwrap().get(0);
            if remaining == 0 { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("load client cleanup did not finish");
    let rows = expect_ok(f.admin.get("waiting").await).await;
    assert!(
        rows["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["stage"] == if test_expiry { "expired" } else { "cancelled" })
    );
    f.finish().await;
    let poll_times = poll_times.lock().unwrap();
    let polls: usize = poll_times.values().sum();
    let mut cumulative = 0;
    let (&poll_p95, _) = poll_times
        .iter()
        .find(|(_, n)| {
            cumulative += **n;
            cumulative >= (polls * 95).div_ceil(100)
        })
        .unwrap();
    received_ms.sort_unstable();
    println!(
        "control_load clients={count} seconds={seconds} management_cycles={reads} startup_ms={startup_ms} preview_ms={preview_ms} polls={} poll_p95_ms={} poll_max_ms={} receive_p95_ms={} receive_max_ms={}",
        polls,
        poll_p95,
        poll_times.last_key_value().unwrap().0,
        percentile(&received_ms, 95),
        received_ms.last().unwrap()
    );
    assert!(
        startup_ms <= 60_000,
        "concentrated startup exceeded 60 seconds"
    );
    if count <= 300 {
        assert!(
            percentile(&received_ms, 95) <= 2_000,
            "command reception p95 exceeded 2 seconds"
        );
    }
}
