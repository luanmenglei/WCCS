use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, time::Duration};
use wccs_inventory::{
    model::ToolReport,
    tools::{check_configuration, inspect_file},
};
use wccs_protocol::{
    CheckResult, Heartbeat, HeartbeatReply, PROTOCOL_VERSION, Registration, token, valid_id,
};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    #[serde(default)]
    pub image_enrollment: bool,
    pub server: String,
    pub device_id: String,
    pub enrollment_token: String,
    #[serde(default)]
    pub device_token: Option<String>,
}

impl Identity {
    pub fn registration_body(
        &self,
        registration: Registration,
        path: &Path,
    ) -> Result<serde_json::Value> {
        if self.image_enrollment {
            let machine_name = wccs_inventory::host_name()?;
            if wccs_protocol::image_device_id(&machine_name).as_deref() != Some(&self.device_id) {
                return Err("image_machine_name_changed_requires_review".into());
            }
            Ok(serde_json::to_value(wccs_protocol::ImageRegistration {
                registration,
                machine_name,
                bootstrap_status: fs::read(path.with_extension("image-status.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .and_then(|value| value["code"].as_str().map(str::to_owned))
                    .unwrap_or_default(),
            })?)
        } else {
            Ok(serde_json::to_value(registration)?)
        }
    }
    pub fn registration_path(&self) -> &'static str {
        if self.image_enrollment {
            "image-register"
        } else {
            "register"
        }
    }
}

pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3
        || args.len() > 4
        || args[1] != "--identity"
        || (args.len() == 4 && args[3] != "--once")
    {
        return Err("用法 wccs-agent --identity local/m2/device.json [--once]".into());
    }
    let path = Path::new(&args[2]);
    // Windows denies other opens while this process owns the lock handle.
    // The OS releases it after a crash, so reconnect requires no manual cleanup.
    use std::os::windows::fs::OpenOptionsExt;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("lock"))?;
    run_locked(path, args.len() == 4)
}

fn run_locked(path: &Path, once: bool) -> Result<()> {
    let mut identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    if !valid_id(&identity.device_id) {
        return Err("设备 ID 无效".into());
    }
    let credential = crate::diagnostics::credential(path, &identity)?;
    identity.device_token = Some(credential.clone());
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let boot_id = token();
    let mut delay = 5u64;
    loop {
        match cycle(&client, &mut identity, &boot_id, path) {
            Ok(()) => {
                delay = 5;
                if once {
                    return Ok(());
                }
            }
            Err(error) => {
                eprintln!("device_sync_failed {error}");
                if once {
                    return Err(error);
                }
                delay = (delay * 2).min(60);
            }
        }
        std::thread::sleep(Duration::from_secs(delay + rand::random::<u64>() % 3));
    }
}

fn cycle(client: &Client, identity: &mut Identity, boot_id: &str, path: &Path) -> Result<()> {
    let server = crate::connection::server(path, identity)?;
    let credential = identity.device_token.as_ref().unwrap().clone();
    let heartbeat = Heartbeat {
        protocol_version: PROTOCOL_VERSION,
        boot_id: boot_id.into(),
        agent_version: env!("CARGO_PKG_VERSION").into(),
        reported_ips: reported_ips(&server),
    };
    let mut response = client
        .post(format!("{server}/api/device/heartbeat"))
        .bearer_auth(&credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&heartbeat)
        .send()?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        && path.with_extension("rotation.json").exists()
    {
        let candidate: Candidate =
            serde_json::from_slice(&fs::read(path.with_extension("rotation.json"))?)?;
        // A lost response can leave the server on the candidate key. Prove it before promotion.
        let probe = client
            .post(format!("{server}/api/device/heartbeat"))
            .bearer_auth(&candidate.new_token)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .json(&heartbeat)
            .send()?;
        if probe.status().is_success() {
            promote(identity, path, &candidate)?;
            return Ok(());
        }
        return Err("凭据切换未恢复，请管理员核对撤销或重新授权状态".into());
    }
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let registration = Registration {
            protocol_version: PROTOCOL_VERSION,
            device_id: identity.device_id.clone(),
            enrollment_token: identity.enrollment_token.clone(),
            device_token: credential.clone(),
        };
        let registered: serde_json::Value = client
            .post(format!(
                "{server}/api/device/{}",
                identity.registration_path()
            ))
            .json(&identity.registration_body(registration, path)?)
            .send()?
            .error_for_status()?
            .json()?;
        if registered["status"] == "pending" {
            println!(
                "device={} registration_pending application={}",
                identity.device_id, registered["application_id"]
            );
            return Ok(());
        }
        response = client
            .post(format!("{server}/api/device/heartbeat"))
            .bearer_auth(&credential)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .json(&heartbeat)
            .send()?;
    }
    let reply: HeartbeatReply = response.error_for_status()?.json()?;
    if reply.protocol_version != PROTOCOL_VERSION
        || !matches!(reply.lifecycle.as_str(), "enabled" | "disabled")
    {
        return Err("服务协议版本或设备状态不兼容".into());
    }
    if let Some(rotation_id) = &reply.rotation_id {
        rotate(client, identity, path, &server, rotation_id)?;
        return Ok(());
    }
    // Diagnostic retransmission shares HTTP ownership and cannot stall WSS renewals.
    let _ = crate::diagnostics::flush(client, path, identity, &credential, &server);
    // Retry a persisted exact result before executing a newly assigned task.
    let pending = path.with_extension("pending.json");
    if pending.exists() {
        let result: CheckResult = serde_json::from_slice(&fs::read(&pending)?)?;
        let response = client
            .post(format!("{server}/api/device/result"))
            .bearer_auth(&credential)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .json(&result)
            .send()?;
        if !matches!(response.status().as_u16(), 200 | 410) {
            response.error_for_status()?;
        }
        fs::write(path.with_extension("completed"), &result.job_id)?;
        fs::remove_file(&pending)?;
    }
    if reply.lifecycle == "disabled" {
        println!("device={} disabled_paused", identity.device_id);
        return Ok(());
    }
    let Some(job) = reply.job else {
        println!("device={} heartbeat_ok", identity.device_id);
        return Ok(());
    };
    if job.expires_at <= reply.server_time {
        return Err("配置任务已过期".into());
    }
    // A pending result may have completed this job after the heartbeat snapshot.
    let completed = path.with_extension("completed");
    if fs::read_to_string(&completed).ok().as_deref() == Some(&job.job_id) {
        return Ok(());
    }
    let (_, reports) = check_configuration(
        Some(&job.configuration),
        inspect_file,
        wccs_inventory::file_version,
    )?;
    let tools = reports
        .into_iter()
        .map(|report| match report {
            ToolReport::Configured(tool) => Ok(tool),
            _ => Err("工具报告版本错误"),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let result = CheckResult {
        protocol_version: PROTOCOL_VERSION,
        device_id: identity.device_id.clone(),
        job_id: job.job_id.clone(),
        config_version: job.configuration.config_version.clone(),
        boot_id: boot_id.into(),
        tools,
    };
    let temporary = path.with_extension("pending.tmp");
    let mut file = fs::File::create(&temporary)?;
    serde_json::to_writer(&mut file, &result)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, &pending)?;
    client
        .post(format!("{server}/api/device/result"))
        .bearer_auth(&credential)
        .header("X-WCCS-Device-ID", &identity.device_id)
        .json(&result)
        .send()?
        .error_for_status()?;
    fs::write(&completed, &job.job_id)?;
    fs::remove_file(&pending)?;
    println!(
        "device={} config={} job={} result_accepted",
        identity.device_id, result.config_version, result.job_id
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Candidate {
    request_id: String,
    new_token: String,
}

pub(crate) fn persist(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("durable.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

fn promote(identity: &mut Identity, path: &Path, candidate: &Candidate) -> Result<()> {
    persist(
        &path.with_extension("credential"),
        candidate.new_token.as_bytes(),
    )?;
    identity.device_token = Some(candidate.new_token.clone());
    fs::remove_file(path.with_extension("rotation.json"))?;
    println!(
        "device={} credential_rotation_persisted",
        identity.device_id
    );
    Ok(())
}

fn rotate(
    client: &Client,
    identity: &mut Identity,
    path: &Path,
    server: &str,
    rotation_id: &str,
) -> Result<()> {
    let candidate_path = path.with_extension("rotation.json");
    let previous = if candidate_path.exists() {
        Some(serde_json::from_slice::<Candidate>(&fs::read(
            &candidate_path,
        )?)?)
    } else {
        None
    };
    let candidate = match previous {
        Some(candidate) if candidate.request_id == rotation_id => candidate,
        _ => {
            let candidate = Candidate {
                request_id: rotation_id.into(),
                new_token: token(),
            };
            persist(&candidate_path, &serde_json::to_vec(&candidate)?)?;
            candidate
        }
    };
    client.post(format!("{server}/api/device/rotate"))
        .bearer_auth(identity.device_token.as_ref().unwrap())
        .json(&serde_json::json!({"device_id":identity.device_id,"request_id":candidate.request_id,"new_token":candidate.new_token}))
        .send()?.error_for_status()?;
    promote(identity, path, &candidate)
}

// The route-selected local address is diagnostic evidence only.
fn reported_ips(server: &str) -> Vec<String> {
    use std::net::{ToSocketAddrs, UdpSocket};
    let Ok(url) = reqwest::Url::parse(server) else {
        return vec![];
    };
    let Some(host) = url.host_str() else {
        return vec![];
    };
    let Ok(addresses) = (host, url.port_or_known_default().unwrap_or(443)).to_socket_addrs() else {
        return vec![];
    };
    for address in addresses {
        let bind = if address.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        if let Ok(socket) = UdpSocket::bind(bind)
            && socket.connect(address).is_ok()
            && let Ok(local) = socket.local_addr()
        {
            return vec![local.ip().to_string()];
        }
    }
    vec![]
}

pub fn validate_server(server: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(server)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("服务地址需要纯 origin".into());
    }
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("服务地址需要 HTTPS，本地回环支持 HTTP 验证".into());
    }
    Ok(url)
}

pub fn run_worker(path: &Path, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let initialize = || -> Result<(Identity, Client)> {
        let mut identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
        identity.device_token = Some(crate::diagnostics::credential(path, &identity)?);
        let client = Client::builder()
            .timeout(Duration::from_secs(12))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok((identity, client))
    };
    let Ok((mut identity, client)) = initialize() else {
        eprintln!("http_worker_initialization_failed");
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    };
    let boot = token();
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if cycle(&client, &mut identity, &boot, path).is_err() {
            eprintln!("device_http_sync_failed");
        }
        for _ in 0..25 {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    #[test]
    fn accepted_origins_normalize_case_and_keep_tls_requirements() {
        assert_eq!(
            validate_server("HTTPS://EXAMPLE.COM/").unwrap().as_str(),
            "https://example.com/"
        );
        assert_eq!(
            validate_server("HTTP://LOCALHOST:8788").unwrap().as_str(),
            "http://localhost:8788/"
        );
        assert!(validate_server("http://10.0.0.1").is_err());
        assert!(validate_server("https://user:password@example.com").is_err());
        assert!(validate_server("https://example.com/path").is_err());
        assert!(validate_server("https://example.com/?token=value").is_err());
    }
}
