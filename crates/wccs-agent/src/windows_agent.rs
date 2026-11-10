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

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    server: String,
    device_id: String,
    enrollment_token: String,
    #[serde(default)]
    device_token: Option<String>,
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
    let url = reqwest::Url::parse(&identity.server)?;
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
    if !valid_id(&identity.device_id) {
        return Err("设备 ID 无效".into());
    }
    let credential_path = path.with_extension("credential");
    let credential = match &identity.device_token {
        Some(value) => value.clone(),
        None => match fs::read_to_string(&credential_path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let value = token();
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&credential_path)?;
                use std::io::Write;
                file.write_all(value.as_bytes())?;
                file.sync_all()?;
                value
            }
            Err(error) => return Err(error.into()),
        },
    };
    if credential.len() != 64 {
        return Err("设备令牌长度错误".into());
    }
    identity.device_token = Some(credential.clone());
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let boot_id = token();
    let mut delay = 5u64;
    loop {
        match cycle(&client, &identity, &boot_id, path) {
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

fn cycle(client: &Client, identity: &Identity, boot_id: &str, path: &Path) -> Result<()> {
    let server = identity.server.trim_end_matches('/');
    let credential = identity.device_token.as_ref().unwrap();
    let heartbeat = Heartbeat {
        protocol_version: PROTOCOL_VERSION,
        boot_id: boot_id.into(),
        agent_version: env!("CARGO_PKG_VERSION").into(),
    };
    let mut response = client
        .post(format!("{server}/api/device/heartbeat"))
        .bearer_auth(credential)
        .json(&heartbeat)
        .send()?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let registration = Registration {
            protocol_version: PROTOCOL_VERSION,
            device_id: identity.device_id.clone(),
            enrollment_token: identity.enrollment_token.clone(),
            device_token: credential.clone(),
        };
        client
            .post(format!("{server}/api/device/register"))
            .json(&registration)
            .send()?
            .error_for_status()?;
        response = client
            .post(format!("{server}/api/device/heartbeat"))
            .bearer_auth(credential)
            .json(&heartbeat)
            .send()?;
    }
    let reply: HeartbeatReply = response.error_for_status()?.json()?;
    if reply.protocol_version != PROTOCOL_VERSION {
        return Err("服务协议版本不兼容".into());
    }
    // Retry a persisted exact result before executing a newly assigned task.
    let pending = path.with_extension("pending.json");
    if pending.exists() {
        let result: CheckResult = serde_json::from_slice(&fs::read(&pending)?)?;
        let response = client
            .post(format!("{server}/api/device/result"))
            .bearer_auth(credential)
            .json(&result)
            .send()?;
        if !matches!(response.status().as_u16(), 200 | 410) {
            response.error_for_status()?;
        }
        fs::write(path.with_extension("completed"), &result.job_id)?;
        fs::remove_file(&pending)?;
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
        .bearer_auth(credential)
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
