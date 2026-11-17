use crate::{
    connection,
    windows_agent::{Identity, Result, persist},
};
use futures_util::StreamExt;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wccs_protocol::{
    Heartbeat, HeartbeatReply, PROTOCOL_VERSION, Registration,
    diagnostics::{Check, Code, Outcome, Report, Step},
    now, token, valid_id,
};

fn read_identity(path: &Path) -> Result<Identity> {
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    if !valid_id(&identity.device_id) || identity.enrollment_token.len() != 64 {
        return Err("身份材料无效，请从管理端重新导出".into());
    }
    Ok(identity)
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

pub fn credential(path: &Path, identity: &Identity) -> Result<String> {
    use std::os::windows::fs::OpenOptionsExt;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("credential.lock"))?;
    let credential_path = path.with_extension("credential");
    let value = match fs::read_to_string(&credential_path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let value = identity.device_token.clone().unwrap_or_else(token);
            persist(&credential_path, value.as_bytes())?;
            value
        }
        Err(error) => return Err(error.into()),
    };
    if value.len() != 64 {
        return Err("设备凭据格式错误".into());
    }
    Ok(value)
}

fn status(path: &Path) -> Result<Value> {
    let identity = read_identity(path)?;
    let saved = connection::load(path, &identity)?;
    let service = path.parent().and_then(Path::parent)
        .and_then(|root| fs::read(root.join("installation.json")).ok())
        .filter(|bytes| bytes.len() <= 8192)
        .and_then(|bytes|serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|manifest|manifest["name"].as_str().filter(|name|valid_id(name)).map(str::to_owned))
        .map(|name|match crate::service::query(&name) {
            Ok((running,pid))=>json!({"name":name,"installed":true,"running":running,"process_id":pid,"source":"scm"}),
            Err(error) if error.code().0 as u32 & 0xffff == 1060=>json!({"name":name,"installed":false,"source":"scm"}),
            Err(_)=>json!({"name":name,"installed":null,"source":"scm","status":"permission_or_query_failed"}),
        });
    let latest = fs::read(path.with_extension("diagnostics.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Report>(&bytes).ok())
        .filter(|report| {
            report.valid()
                && report.device_id == identity.device_id
                && report.configuration_digest == connection::configuration_digest(&saved)
        });
    Ok(
        json!({"schema_version":1,"agent_version":env!("CARGO_PKG_VERSION"),
        "device_id":identity.device_id,"configuration_revision":saved.revision,
        "configuration":saved.configuration,"network":wccs_inventory::network(),
        "session_binary_present":std::env::current_exe()?.with_file_name("wccs-session.exe").is_file(),
        "installation_service":service,
        "recent_maintenance":crate::maintenance::diagnostic_summary(path,&identity),
        "pending_distribution":crate::distribution::diagnostic_summary(path,&identity),
        "latest_diagnostics":latest,"online_validation_pending":latest.is_none(),
        "presentation_cache":crate::presentation::cached_diagnostic(path).ok().flatten(),
        "credential_present":path.with_extension("credential").is_file()}),
    )
}

fn import(path: &Path, source: &str, base: u64) -> Result<()> {
    if source.len() > 4096 {
        return Err("配置最大 4096 字节".into());
    }
    let config = connection::validate(serde_json::from_str(source)?)?;
    let saved = connection::save(path, base, config)?;
    println!(
        "已离线保存配置 v{}，联网验证可在诊断菜单执行。",
        saved.revision
    );
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label} ");
    io::stdout().flush()?;
    let mut text = String::new();
    io::stdin().read_line(&mut text)?;
    Ok(text.trim().to_owned())
}

/// Console UI is usable from Explorer shortcuts or a terminal on the installed machine.
pub fn run(args: &[String]) -> Result<()> {
    match args {
        [operation, path] if operation == "--setup-status" => {
            println!("{}", serde_json::to_string_pretty(&status(Path::new(path))?)?);
        }
        [operation, path, source, revision] if operation == "--configure" => {
            let mut text = String::new();
            fs::File::open(source)?.take(4097).read_to_string(&mut text)?;
            import(Path::new(path), &text, revision.parse()?)?;
        }
        [operation, path, service, output] if operation == "--diagnose" || operation == "--diagnose-test" => {
            diagnose(Path::new(path), service, Path::new(output), operation == "--diagnose-test")?;
        }
        [operation, path] if operation == "--setup" => {
            let path = Path::new(path);
            loop {
                println!("\nWCCS 本机配置与诊断\n1 查看配置与网卡\n2 粘贴配置\n3 导入配置文件\n4 分步骤诊断并导出\n5 显示本机设备识别\nq 退出");
                let result = match prompt("选择")?.as_str() {
                    "1" => status(path).and_then(|value| { println!("{}", serde_json::to_string_pretty(&value)?); Ok(()) }),
                    "2" | "3" => {
                        let saved = connection::load(path, &read_identity(path)?)?;
                        let input = prompt("粘贴单行 JSON，或输入 @文件完整路径")?;
                        let text = if let Some(file) = input.strip_prefix('@') {
                            let mut text = String::new();
                            fs::File::open(file)?.take(4097).read_to_string(&mut text)?;
                            text
                        } else { input };
                        preview_import(path, &text, &saved)
                    }
                    "4" => {
                        let service = prompt("SCM 服务名（如 WCCS-Agent）")?;
                        let output = prompt("新诊断包文件完整路径（JSON）")?;
                        diagnose(path, &service, Path::new(&output), false)
                    }
                    "5" => {
                        let executable = std::env::current_exe()?.with_file_name("wccs-session.exe");
                        crate::maintenance::identify_local(path, &executable, 60)
                    }
                    "q" | "" => break,
                    _ => continue,
                };
                if result.is_err() { println!("操作未完成，请核对输入、文件访问权限及当前配置版本后重试。"); }
            }
        }
        _ => return Err("用法 --setup IDENTITY 或 --setup-status IDENTITY 或 --configure IDENTITY CONFIG BASE_REVISION 或 --diagnose IDENTITY SERVICE OUTPUT 或 --diagnose-test IDENTITY SESSION_EXE OUTPUT".into()),
    }
    Ok(())
}

fn preview_import(path: &Path, text: &str, saved: &connection::Saved) -> Result<()> {
    if text.len() > 4096 {
        return Err("配置最大 4096 字节".into());
    }
    let config = connection::validate(serde_json::from_str(text)?)?;
    println!(
        "当前版本 v{}\n{}\n拟保存\n{}",
        saved.revision,
        serde_json::to_string_pretty(&saved.configuration)?,
        serde_json::to_string_pretty(&config)?
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&wccs_inventory::network())?
    );
    if prompt("输入 SAVE 保存，其余输入取消")? == "SAVE" {
        import(path, text, saved.revision)?;
    }
    Ok(())
}

fn check(report: &mut Report, previous: Option<&Report>, step: Step, outcome: Outcome, code: Code) {
    let checked_at = now();
    let last_success_at = if outcome == Outcome::Succeeded {
        Some(checked_at)
    } else {
        previous
            .filter(|p| {
                p.configuration_digest == report.configuration_digest
                    && p.device_id == report.device_id
            })
            .and_then(|p| p.checks.iter().find(|c| c.step == step))
            .and_then(|c| c.last_success_at)
    };
    println!("{step:?} {outcome:?} {code:?}");
    report.checks.push(Check {
        step,
        outcome,
        checked_at,
        last_success_at,
        code,
    });
}

fn diagnose(path: &Path, service: &str, output: &Path, test_mode: bool) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("diagnosis.lock"))?;
    let identity = read_identity(path)?;
    let saved = connection::load(path, &identity)?;
    let previous = fs::read(path.with_extension("diagnostics.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Report>(&bytes).ok());
    let mut report = Report {
        schema_version: 1,
        request_id: token(),
        device_id: identity.device_id.clone(),
        agent_version: env!("CARGO_PKG_VERSION").into(),
        evidence_context: if test_mode {
            wccs_protocol::diagnostics::EvidenceContext::LocalTest
        } else {
            wccs_protocol::diagnostics::EvidenceContext::Service
        },
        configuration_revision: saved.revision,
        configuration_digest: connection::configuration_digest(&saved),
        server_origin: saved.configuration.server.clone(),
        collected_at: now(),
        seat_claim: saved.configuration.seat_claim.clone(),
        checks: vec![],
    };
    let prior = previous.as_ref();
    check(
        &mut report,
        prior,
        Step::Configuration,
        Outcome::Succeeded,
        Code::SavedOffline,
    );
    let (service_outcome, service_code, service_pid) = if test_mode {
        (Outcome::Unknown, Code::NotConfigured, None)
    } else {
        match crate::service::query(service) {
            Ok((true, pid)) => (Outcome::Succeeded, Code::ServiceRunning, Some(pid)),
            Ok((false, _)) => (Outcome::Failed, Code::ServiceStopped, None),
            Err(error) if error.code().0 as u32 & 0xffff == 1060 => {
                (Outcome::Failed, Code::ServiceMissing, None)
            }
            Err(_) => (Outcome::Unknown, Code::PermissionDenied, None),
        }
    };
    check(
        &mut report,
        prior,
        Step::Installation,
        if service_pid.is_some() {
            Outcome::Succeeded
        } else {
            Outcome::Unknown
        },
        if service_pid.is_some() {
            Code::ReceiptVerified
        } else {
            Code::NotConfigured
        },
    );
    check(
        &mut report,
        prior,
        Step::Service,
        service_outcome,
        service_code,
    );
    let ipc = if test_mode {
        ipc_probe(Path::new(service), true, &report.request_id).map(|_| ())
    } else if let Some(pid) = service_pid {
        request_ipc(path, &report.request_id, pid)
    } else {
        Err("service_unavailable".into())
    };
    check(
        &mut report,
        prior,
        Step::SessionIpc,
        if ipc.is_ok() {
            Outcome::Succeeded
        } else {
            Outcome::Unknown
        },
        if ipc.is_ok() {
            Code::ReceiptVerified
        } else {
            Code::SessionUnavailable
        },
    );
    let credential = credential(path, &identity)?;
    let client = client()?;
    let context = if let Some(server) = &saved.configuration.server {
        network_checks(
            path,
            &client,
            &identity,
            &credential,
            server,
            &mut report,
            prior,
        )
    } else {
        for step in [
            Step::Server,
            Step::Authentication,
            Step::HttpHeartbeat,
            Step::Websocket,
        ] {
            check(
                &mut report,
                prior,
                step,
                Outcome::Pending,
                Code::NotConfigured,
            );
        }
        None
    };
    if !report.valid() {
        return Err("诊断报告不完整".into());
    }
    persist(
        &path.with_extension("diagnostics.json"),
        &serde_json::to_vec(&report)?,
    )?;
    let mut uploaded = false;
    if let Some(server) = &saved.configuration.server {
        let directory = path.with_extension("diagnostics-outbox");
        fs::create_dir_all(&directory)?;
        let envelope = Envelope {
            device_binding: connection::binding(&identity),
            server: server.clone(),
            report: report.clone(),
        };
        // A full offline queue must not prevent staff from exporting local evidence.
        if fs::read_dir(&directory)?.count() < 32 {
            persist(
                &directory.join(format!("{}.json", report.request_id)),
                &serde_json::to_vec(&envelope)?,
            )?;
            uploaded = flush(&client, path, &identity, &credential, server).is_ok()
                && !directory
                    .join(format!("{}.json", report.request_id))
                    .exists();
        }
    }
    // Only typed evidence is exported. Arbitrary logs, credentials, account names and paths are excluded.
    let package = json!({"schema_version":1,"mode":if test_mode {"local_test"} else {"service"},
        "configuration":saved.configuration,"report":report,"server_context":context,
        "network":wccs_inventory::network(),"server_acknowledged":uploaded,
        "associated_events":sanitized_events(path),"associated_tasks":crate::maintenance::diagnostic_summary(path,&identity),
        "pending_distribution":crate::distribution::diagnostic_summary(path,&identity)});
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(&mut file, &package)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    print_checks(&report);
    println!("诊断包已导出，服务端确认={uploaded}。失败项目请按管理页的恢复说明处理。");
    Ok(())
}

fn print_checks(report: &Report) {
    for check in &report.checks {
        let label = match check.step {
            Step::Configuration => "本机配置",
            Step::Installation => "安装状态",
            Step::Server => "服务器连接",
            Step::Authentication => "注册与认证",
            Step::HttpHeartbeat => "HTTP 心跳",
            Step::Websocket => "WSS 握手",
            Step::Service => "SCM 服务",
            Step::SessionIpc => "会话鉴权 IPC",
        };
        let status = match check.outcome {
            Outcome::Succeeded => "成功",
            Outcome::Failed => "失败",
            Outcome::Pending => "待联网验证",
            Outcome::Unknown => "未知",
        };
        let action = match check.code {
            Code::NotConfigured | Code::InvalidConfiguration => {
                "从本机菜单导入服务器配置，离线时可先保存"
            }
            Code::ConnectionUnavailable => {
                "教学离线可保持待机；需要联网时核对服务器地址、证书和服务状态后重试"
            }
            Code::AuthenticationRejected | Code::RegistrationRejected => {
                "在管理端核对设备身份和注册材料，不要复用其他设备凭据"
            }
            Code::RegistrationPending => "在管理端设备页面核对并批准注册",
            Code::ServiceStopped => "由工作人员启动该 SCM 服务后重试",
            Code::ServiceMissing | Code::FileMissing => "核对安装目录、配套会话程序和 SCM 服务名",
            Code::PermissionDenied => "以安装时的工作人员权限重新运行诊断",
            Code::SessionUnavailable | Code::IpcFailed => {
                "确认存在已登录的用户会话，并核对服务和会话程序版本"
            }
            Code::IncompatibleReply => "核对服务端与客户端的协议版本",
            _ => "无需恢复操作",
        };
        println!(
            "{label}: {status} · 最近成功时间={} · {action}",
            check
                .last_success_at
                .map(|t| t.to_string())
                .unwrap_or_else(|| "无".into())
        );
    }
    println!("部分成功表示只有对应步骤获得了实际回执；离线待机不影响已保存配置。");
}

fn sanitized_events(path: &Path) -> Vec<Value> {
    let mut contents = String::new();
    if fs::File::open(path.with_extension("host.log"))
        .and_then(|f| f.take(1_048_576).read_to_string(&mut contents))
        .is_err()
    {
        return vec![];
    }
    contents
        .lines()
        .rev()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let timestamp = fields.next()?.parse::<i64>().ok()?;
            let event = fields.next()?;
            if ![
                "host_started",
                "host_stopped",
                "control_connection_unavailable",
                "waiting_policy_recovery_failed",
                "waiting_policy_apply_failed",
                "session_launch_failed",
            ]
            .contains(&event)
            {
                return None;
            }
            Some(json!({"time":timestamp,"event":event}))
        })
        .take(64)
        .collect()
}

fn network_checks(
    path: &Path,
    client: &Client,
    identity: &Identity,
    credential: &str,
    server: &str,
    report: &mut Report,
    previous: Option<&Report>,
) -> Option<Value> {
    let health = client
        .get(format!("{server}/api/health"))
        .send()
        .ok()
        .and_then(|r| r.error_for_status().ok())
        .and_then(|r| r.json::<Value>().ok());
    let connected = health
        .as_ref()
        .is_some_and(|v| v["protocol_version"] == 1 && v["service"] == "wccs");
    check(
        report,
        previous,
        Step::Server,
        if connected {
            Outcome::Succeeded
        } else {
            Outcome::Failed
        },
        if connected {
            Code::ReceiptVerified
        } else {
            Code::ConnectionUnavailable
        },
    );
    let context = || {
        client
            .get(format!("{server}/api/device/diagnostics/context"))
            .bearer_auth(credential)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .send()
    };
    let mut response = context();
    let mut auth_code = Code::AuthenticationRejected;
    if response.as_ref().is_ok_and(|r| r.status().as_u16() == 401) {
        let registration = Registration {
            protocol_version: PROTOCOL_VERSION,
            device_id: identity.device_id.clone(),
            enrollment_token: identity.enrollment_token.clone(),
            device_token: credential.into(),
        };
        let registered = identity
            .registration_body(registration, path)
            .ok()
            .and_then(|body| {
                client
                    .post(format!(
                        "{server}/api/device/{}",
                        identity.registration_path()
                    ))
                    .json(&body)
                    .send()
                    .ok()
            })
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<Value>().ok());
        if registered
            .as_ref()
            .is_some_and(|v| v["status"] == "pending" && v["device_id"] == identity.device_id)
        {
            auth_code = Code::RegistrationPending;
        } else {
            response = context();
        }
    }
    let context = response
        .ok()
        .and_then(|r| r.error_for_status().ok())
        .and_then(|r| r.json::<Value>().ok())
        .filter(|v| v["protocol_version"] == 1 && v["device_id"] == identity.device_id);
    let authenticated = context.is_some();
    check(
        report,
        previous,
        Step::Authentication,
        if authenticated {
            Outcome::Succeeded
        } else {
            Outcome::Pending
        },
        if authenticated {
            Code::ReceiptVerified
        } else {
            auth_code
        },
    );
    let heartbeat = Heartbeat {
        protocol_version: PROTOCOL_VERSION,
        boot_id: report.request_id.clone(),
        agent_version: report.agent_version.clone(),
        reported_ips: vec![],
    };
    let heartbeat_ok = authenticated
        && client
            .post(format!("{server}/api/device/heartbeat"))
            .bearer_auth(credential)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .json(&heartbeat)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<HeartbeatReply>().ok())
            .is_some_and(|r| {
                r.protocol_version == 1 && matches!(r.lifecycle.as_str(), "enabled" | "disabled")
            });
    check(
        report,
        previous,
        Step::HttpHeartbeat,
        if heartbeat_ok {
            Outcome::Succeeded
        } else {
            Outcome::Pending
        },
        if heartbeat_ok {
            Code::ReceiptVerified
        } else {
            Code::ConnectionUnavailable
        },
    );
    let ws_ok = authenticated
        && websocket_probe(server, &identity.device_id, credential, &report.request_id).is_ok();
    check(
        report,
        previous,
        Step::Websocket,
        if ws_ok {
            Outcome::Succeeded
        } else {
            Outcome::Pending
        },
        if ws_ok {
            Code::ReceiptVerified
        } else {
            Code::ConnectionUnavailable
        },
    );
    context
}

fn websocket_probe(server: &str, device: &str, credential: &str, nonce: &str) -> Result<()> {
    use tokio_tungstenite::{
        connect_async_with_config,
        tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
    };
    let mut url = crate::windows_agent::validate_server(server)?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme).map_err(|_| "websocket_scheme")?;
    url.set_path("/api/device/diagnostics/probe");
    url.set_query(Some(&format!("nonce={nonce}")));
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {credential}").parse()?);
    request
        .headers_mut()
        .insert("x-wccs-device-id", device.parse()?);
    tokio::runtime::Runtime::new()?.block_on(async {
        tokio::time::timeout(Duration::from_secs(8), async {
            let config = WebSocketConfig::default()
                .max_message_size(Some(4096))
                .max_frame_size(Some(4096));
            let (mut socket, _) = connect_async_with_config(request, Some(config), false).await?;
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                return Err("websocket_receipt_missing".into());
            };
            let receipt: Value = serde_json::from_str(&text)?;
            if receipt["protocol_version"] != 1
                || receipt["device_id"] != device
                || receipt["nonce"] != nonce
            {
                return Err("websocket_receipt_mismatch".into());
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await?
    })
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    device_binding: String,
    server: String,
    report: Report,
}

pub fn flush(
    client: &Client,
    path: &Path,
    identity: &Identity,
    credential: &str,
    server: &str,
) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("diagnostic-upload.lock"))?;
    let directory = path.with_extension("diagnostics-outbox");
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let envelope: Envelope = serde_json::from_slice(&fs::read(&path)?)?;
        if envelope.device_binding != connection::binding(identity) || envelope.server != server {
            continue;
        }
        let response: Value = client
            .post(format!("{server}/api/device/diagnostics"))
            .bearer_auth(credential)
            .header("X-WCCS-Device-ID", &identity.device_id)
            .json(&envelope.report)
            .send()?
            .error_for_status()?
            .json()?;
        if response["accepted"] != true
            || response["request_id"] != envelope.report.request_id
            || response["device_id"] != identity.device_id
        {
            return Err("diagnostic_ack_mismatch".into());
        }
        fs::remove_file(path)?;
        break;
    }
    Ok(())
}

fn ipc_probe(executable: &Path, test_mode: bool, nonce: &str) -> crate::host::Result<()> {
    use wccs_protocol::control::*;
    use wccs_session::ipc;
    let session = ipc::discover(test_mode)?;
    let mut child = ipc::Child::start(executable, &session, test_mode)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !child.ready()? {
        if Instant::now() >= deadline {
            return Err("ipc_timeout".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let command = Command {
        protocol_version: CONTROL_VERSION,
        command_id: nonce.into(),
        request_id: nonce.into(),
        sequence: 0,
        device_id: "diagnostic".into(),
        generation: 0,
        device_version: 0,
        instance_id: nonce.into(),
        session_key: session.key.clone(),
        action: "probe_ipc".into(),
        expires_at: 0,
        identity: WaitingIdentity {
            contest_id: String::new(),
            contest_name: String::new(),
            contest_version: 0,
            participant_id: String::new(),
            name: String::new(),
            school: String::new(),
            seat_id: String::new(),
            seat_label: String::new(),
            room_id: String::new(),
            room_version: 0,
        },
        login: None,
    };
    child.pipe.send(&IpcRequest {
        presentation: None,
        protocol_version: CONTROL_VERSION,
        command,
        lease_seconds: 5,
    })?;
    loop {
        if let Some(reply) = child.pipe.receive::<Actual>()? {
            if reply.command_id.as_deref() == Some(nonce)
                && reply.session_key.as_deref() == Some(&session.key)
                && reply.detail == "ui_observed"
            {
                child.stop()?;
                return Ok(());
            }
            return Err("ipc_receipt_mismatch".into());
        }
        if Instant::now() >= deadline {
            return Err("ipc_timeout".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn request_ipc(path: &Path, nonce: &str, pid: u32) -> crate::host::Result<()> {
    persist(
        &path.with_extension("diagnostic-request.json"),
        &serde_json::to_vec(&json!({"nonce":nonce}))?,
    )
    .map_err(|_| "diagnostic_request_failed")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let reply = fs::read(path.with_extension("diagnostic-reply.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        if let Some(reply) = reply
            .filter(|v| v["nonce"] == nonce && v["process_id"] == pid && v["test_mode"] == false)
        {
            return if reply["succeeded"] == true {
                Ok(())
            } else {
                Err("session_ipc_unavailable".into())
            };
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("diagnostic_service_timeout".into())
}

pub fn service_worker(path: &Path, executable: &Path, test_mode: bool, stop: Arc<AtomicBool>) {
    let mut last = String::new();
    while !stop.load(Ordering::Relaxed) {
        let request = fs::read(path.with_extension("diagnostic-request.json"))
            .ok()
            .filter(|b| b.len() <= 512)
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        if let Some(nonce) = request
            .as_ref()
            .and_then(|v| v["nonce"].as_str())
            .filter(|n| valid_id(n) && *n != last)
        {
            last = nonce.to_owned();
            let succeeded = ipc_probe(executable, test_mode, nonce).is_ok();
            let reply = json!({"nonce":nonce,"process_id":std::process::id(),"test_mode":test_mode,"succeeded":succeeded,"checked_at":now()});
            let _ = persist(
                &path.with_extension("diagnostic-reply.json"),
                reply.to_string().as_bytes(),
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
