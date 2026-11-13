use crate::{
    connection, diagnostics,
    windows_agent::{Identity, Result, persist},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use wccs_protocol::presentation::{Connection, Data, Display};

pub type Shared = Arc<Mutex<Option<Display>>>;
#[derive(Deserialize, Serialize)]
struct Cache {
    source: String,
    binding: String,
    display: Display,
}
fn timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn tick() -> u64 {
    // SAFETY: process-independent monotonic Windows boot tick has no arguments or ownership.
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}
fn connection(state: &str, source: &str, detail: &str) -> Connection {
    Connection {
        state: state.into(),
        source: source.into(),
        checked_at_ms: timestamp(),
        detail: detail.into(),
    }
}
fn oj(client: &reqwest::blocking::Client, data: &Data) -> Connection {
    let source = "client-domjudge-api";
    let (Some(base), Some(contest)) = (&data.domjudge_api, &data.domjudge_contest) else {
        return connection("unknown", source, "not_configured");
    };
    let response = client
        .get(format!("{}/contests/{contest}", base.trim_end_matches('/')))
        .send();
    let Ok(mut response) = response else {
        return connection("offline", source, "http_unavailable");
    };
    if matches!(response.status().as_u16(), 401 | 403) {
        return connection("offline", source, "authentication_required");
    }
    if !response.status().is_success() {
        return connection("offline", source, "http_unavailable");
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    if response
        .by_ref()
        .take(65537)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 65536
    {
        return connection("offline", source, "response_invalid");
    }
    if serde_json::from_slice::<Value>(&bytes)
        .ok()
        .is_some_and(|v| v["id"] == *contest && v["name"].is_string())
    {
        connection("online", source, "receipt_verified")
    } else {
        connection("offline", source, "response_invalid")
    }
}
pub fn worker(path: &Path, instance: &str, test_mode: bool, shared: Shared, stop: Arc<AtomicBool>) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    while !stop.load(Ordering::Relaxed) {
        if update(&client, path, instance, test_mode, &shared).is_err() {
            if let Ok(mut value) = shared.lock()
                && let Some(value) = value.as_mut()
            {
                value.wccs = connection("offline", "authenticated-http", "http_unavailable");
            }
            eprintln!("presentation_refresh_unavailable");
        }
        for _ in 0..50 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
fn update(
    client: &reqwest::blocking::Client,
    path: &Path,
    instance: &str,
    test_mode: bool,
    shared: &Shared,
) -> Result<()> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let server = connection::server(path, &identity)?;
    let credential = diagnostics::credential(path, &identity)?;
    let session = wccs_session::ipc::discover(test_mode).ok().map(|s| s.key);
    let started = Instant::now();
    let response: Value = client
        .post(format!("{server}/api/device/presentation"))
        .bearer_auth(&credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&json!({"instance_id":instance,"session_key":session}))
        .send()?
        .error_for_status()?
        .json()?;
    let received_tick = tick();
    let round_trip = started.elapsed().as_millis() as u64;
    if response["data"].is_null() {
        *shared.lock().map_err(|_| "presentation_lock")? = None;
        return Ok(());
    }
    let data: Data = serde_json::from_value(response["data"].clone())?;
    if data.schema_version != 1
        || data.device_id != identity.device_id
        || data.logo.as_ref().is_some_and(|l| l.pixels().is_none())
    {
        return Err("presentation_invalid".into());
    }
    let mut display = Display {
        clock_offset_ms: data.server_time_ms + round_trip as i64 / 2 - timestamp(),
        oj: connection("unknown", "client-domjudge-api", "not_configured"),
        data,
        received_tick_ms: received_tick,
        round_trip_ms: round_trip,
        wccs: connection("online", "authenticated-http", "receipt_verified"),
    };
    display.oj = oj(client, &display.data);
    let cache = Cache {
        source: server.clone(),
        binding: connection::binding(&identity),
        display: display.clone(),
    };
    persist(
        &path.with_extension("presentation.json"),
        &serde_json::to_vec(&cache)?,
    )?;
    *shared.lock().map_err(|_| "presentation_lock")? = Some(display.clone());
    let receipt: Value = client
        .post(format!("{server}/api/device/presentation/report"))
        .bearer_auth(&credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&display)
        .send()?
        .error_for_status()?
        .json()?;
    if receipt["accepted"] != true || receipt["command_id"] != display.data.command_id {
        return Err("presentation_ack_mismatch".into());
    }
    Ok(())
}

pub fn cached_diagnostic(path: &Path) -> Result<Option<Value>> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let cache_path = path.with_extension("presentation.json");
    if !cache_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(cache_path)?;
    if bytes.len() > 65536 {
        return Err("presentation_cache_size".into());
    }
    let cache: Cache = serde_json::from_slice(&bytes)?;
    if cache.binding != connection::binding(&identity)
        || Some(cache.source.as_str()) != connection::server(path, &identity).ok().as_deref()
        || cache.display.data.device_id != identity.device_id
    {
        return Err("presentation_cache_identity".into());
    }
    let mut value = json!(cache);
    if let Some(logo) = value["display"]["data"]["logo"].as_object_mut() {
        logo.remove("bmp_hex");
    }
    Ok(Some(value))
}
