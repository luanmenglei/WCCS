//! Sampling and upload are isolated from the control renewal thread.
use crate::{
    connection, diagnostics,
    windows_agent::{Identity, Result, persist},
};
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wccs_inventory::{
    HealthCollector,
    health::Sample,
    model::{Observation, Reason},
};
use wccs_protocol::health::Report;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    schema_version: u32,
    version: String,
}
fn deployment(path: &Path) -> Observation<String> {
    let bytes = match fs::read(path.with_extension("deployment.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Observation::unavailable(Reason::NotConfigured);
        }
        Err(error) => return Observation::from_io(&error),
    };
    if bytes.len() <= 1024
        && let Ok(value) = serde_json::from_slice::<Deployment>(&bytes)
        && value.schema_version == 1
        && !value.version.is_empty()
        && value.version.len() <= 128
        && !value.version.chars().any(char::is_control)
    {
        return Observation::available(value.version);
    }
    Observation::unavailable(Reason::SystemError)
}
pub fn sample(
    path: &Path,
    instance: &str,
    sequence: i64,
    collector: &mut HealthCollector,
) -> Result<Report> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let started = Instant::now();
    let health = collector.collect();
    Ok(Report {
        schema_version: 1,
        device_id: identity.device_id,
        instance_id: instance.into(),
        sequence,
        agent_version: Sample::new(Observation::available(env!("CARGO_PKG_VERSION").into())),
        deployment_version: Sample::new(deployment(path)),
        collection_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        health,
    })
}
pub fn worker(path: &Path, instance: &str, stop: Arc<AtomicBool>) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    let mut collector = HealthCollector::default();
    let mut sequence = 0;
    while !stop.load(Ordering::Relaxed) {
        sequence += 1;
        let outcome = (|| -> Result<()> {
            let report = sample(path, instance, sequence, &mut collector)?;
            persist(
                &path.with_extension("health.json"),
                &serde_json::to_vec(&report)?,
            )?;
            let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
            let server = connection::server(path, &identity)?;
            let credential = diagnostics::credential(path, &identity)?;
            let receipt: serde_json::Value = client
                .post(format!("{server}/api/device/health"))
                .bearer_auth(credential)
                .header("X-WCCS-Device-ID", &identity.device_id)
                .json(&report)
                .send()?
                .error_for_status()?
                .json()?;
            if receipt["accepted"] != true
                || receipt["instance_id"] != instance
                || receipt["sequence"] != sequence
            {
                return Err("health_ack_mismatch".into());
            }
            Ok(())
        })();
        if outcome.is_err() {
            eprintln!("health_update_unavailable");
        }
        // Slow collection stretches the interval, with no catch-up burst or shared control lock.
        for _ in 0..150 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
