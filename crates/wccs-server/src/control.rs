use crate::{ApiError, ApiResult, App, auth, seating, waiting};
use axum::{
    extract::{Request, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::derive_accept_key,
        protocol::{Role, WebSocketConfig},
    },
};
type WebSocket = WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>;
use serde_json::json;
use std::time::Duration;
use wccs_protocol::{control::*, now, token, valid_id};

pub async fn upgrade(
    State(app): State<App>,
    headers: HeaderMap,
    request: Request,
) -> ApiResult<Response> {
    if headers.contains_key("origin") {
        return Err(ApiError::forbidden("设备控制连接拒绝浏览器来源"));
    }
    if headers
        .get("sec-websocket-protocol")
        .and_then(|h| h.to_str().ok())
        != Some(CONTROL_PROTOCOL)
    {
        return Err(ApiError::bad("控制子协议不兼容"));
    }
    let id = {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        seating::lock(&tx).await?;
        let id = auth::device(&tx, &headers).await?;
        tx.commit().await?;
        id
    };
    let key = headers
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.len() == 24)
        .ok_or_else(|| ApiError::bad("WebSocket 握手无效"))?;
    if headers
        .get("sec-websocket-version")
        .and_then(|v| v.to_str().ok())
        != Some("13")
        || !headers
            .get("upgrade")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        || !headers
            .get("connection")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(',')
                    .any(|p| p.trim().eq_ignore_ascii_case("upgrade"))
            })
    {
        return Err(ApiError::bad("WebSocket 握手无效"));
    }
    let accept = derive_accept_key(key.as_bytes());
    tokio::spawn(async move {
        if let Ok(Ok(upgraded)) =
            tokio::time::timeout(Duration::from_secs(5), hyper::upgrade::on(request)).await
        {
            let config = WebSocketConfig::default()
                .max_message_size(Some(MAX_MESSAGE))
                .max_frame_size(Some(MAX_MESSAGE))
                .write_buffer_size(0)
                .max_write_buffer_size(MAX_MESSAGE * 2);
            let socket = WebSocketStream::from_raw_socket(
                hyper_util::rt::TokioIo::new(upgraded),
                Role::Server,
                Some(config),
            )
            .await;
            run(app, headers, id, socket).await;
        }
    });
    Ok((
        axum::http::StatusCode::SWITCHING_PROTOCOLS,
        [
            ("upgrade", "websocket"),
            ("connection", "Upgrade"),
            ("sec-websocket-protocol", CONTROL_PROTOCOL),
            ("sec-websocket-accept", accept.as_str()),
        ],
    )
        .into_response())
}

async fn run(app: App, headers: HeaderMap, id: String, mut socket: WebSocket) {
    let connection = token();
    let mut first = true;
    let mut instance = None;
    loop {
        let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(10), socket.next()).await
        else {
            break;
        };
        let Ok(poll) = serde_json::from_str::<ControlPoll>(&text) else {
            break;
        };
        if poll.protocol_version != CONTROL_VERSION
            || !valid_id(&poll.instance_id)
            || poll.session_key.as_ref().is_some_and(|s| !valid_id(s))
            || !poll.actual.valid()
            || !(poll.capabilities == ["waiting-v1"]
                || poll.capabilities == ["waiting-v1", "contest-login-simulation-v1"]
                || poll.capabilities == ["waiting-v1", "contest-login-windows-test-v1"])
            || (poll
                .actual
                .account
                .as_ref()
                .is_some_and(|s| s.adapter == "windows_test")
                && !poll
                    .capabilities
                    .iter()
                    .any(|c| c == "contest-login-windows-test-v1"))
            || (poll
                .actual
                .account
                .as_ref()
                .is_some_and(|s| s.adapter == "simulation")
                && !poll
                    .capabilities
                    .iter()
                    .any(|c| c == "contest-login-simulation-v1"))
            || instance.as_ref().is_some_and(|s| *s != poll.instance_id)
        {
            break;
        }
        instance = Some(poll.instance_id.clone());
        let result = exchange(&app, &headers, &id, &connection, first, &poll).await;
        let Ok(reply) = result else {
            break;
        };
        first = false;
        let response = Message::Text(serde_json::to_string(&reply).unwrap().into());
        if !matches!(
            tokio::time::timeout(Duration::from_secs(3), socket.send(response)).await,
            Ok(Ok(()))
        ) {
            break;
        }
        // One in-flight request bounds queue growth and limits authenticated flooding.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
    let Ok(mut db) = app.db.lock().await else {
        return;
    };
    if let Ok(tx) = db.transaction().await {
        let _ = seating::lock(&tx).await;
        if let Ok(count) = tx.execute("UPDATE control_devices SET connected=false WHERE device_id=$1 AND connection_id=$2", &[&id,&connection]).await
            && count == 1 { let _ = connection_event(&tx,&id,&connection,"disconnected").await; }
        let _ = tx.commit().await;
    }
}

async fn connection_event(
    tx: &tokio_postgres::Transaction<'_>,
    id: &str,
    connection: &str,
    state: &str,
) -> ApiResult<()> {
    tx.execute("INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,room_ids,outcome,detail)
            VALUES('control_connection',$1,'device_report',$2,$3,$4,ARRAY[$4],device_room_scope($4),$5,'{}')
            ON CONFLICT DO NOTHING", &[&format!("{connection}:{state}"),&now(),&format!("control_{state}"),&id,&state]).await?;
    Ok(())
}

async fn exchange(
    app: &App,
    headers: &HeaderMap,
    id: &str,
    connection: &str,
    first: bool,
    poll: &ControlPoll,
) -> ApiResult<ControlReply> {
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    seating::lock(&tx).await?;
    if auth::device(&tx, headers).await? != id {
        return Err(ApiError::forbidden("设备身份变化"));
    }
    let device = tx
        .query_one(
            "SELECT enrollment_generation,lifecycle FROM devices WHERE id=$1",
            &[&id],
        )
        .await?;
    let generation: i64 = device.get(0);
    if device.get::<_, String>(1) != "enabled" {
        return Err(ApiError::forbidden("设备控制已禁用"));
    }
    if first {
        tx.execute("INSERT INTO control_devices
            VALUES($1,$2,$3,$4,$5,$6,$7,true,$8)
            ON CONFLICT(device_id) DO UPDATE SET connection_id=$2,server_epoch=$3,instance_id=$4,session_key=$5,generation=$6,last_seen=$7,connected=true,actual=$8", &[&id,&connection,&app.server_epoch,&poll.instance_id,&poll.session_key,&generation,&now(),&json!(poll.actual)]).await?;
        connection_event(&tx, id, connection, "connected").await?;
    } else {
        let changed = tx
            .execute(
                "UPDATE control_devices SET last_seen=$3,session_key=$4,actual=$5
            WHERE device_id=$1
            AND connection_id=$2
            AND server_epoch=$6
            AND instance_id=$7
            AND generation=$8",
                &[
                    &id,
                    &connection,
                    &now(),
                    &poll.session_key,
                    &json!(poll.actual),
                    &app.server_epoch,
                    &poll.instance_id,
                    &generation,
                ],
            )
            .await?;
        if changed != 1 {
            return Err(ApiError::conflict("控制连接已被替换"));
        }
    }
    let command = waiting::current(&tx, id, &poll.instance_id, poll.session_key.as_deref()).await?;
    let mut acknowledged = None;
    if let Some(report) = &poll.report {
        if !report.actual.valid()
            || !["accepted", "executing", "succeeded", "failed"].contains(&report.stage.as_str())
        {
            return Err(ApiError::bad("执行回执格式无效"));
        }
        let row = tx.query_opt("SELECT command,stage,report FROM waiting_commands WHERE id=$1 AND device_id=$2 AND generation=$3 AND instance_id=$4", &[&report.command_id,&id,&generation,&poll.instance_id]).await?
            .ok_or_else(||ApiError::forbidden("回执身份代次不匹配"))?;
        let saved: Command =
            serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
        if saved.sequence != report.sequence {
            return Err(ApiError::conflict("回执版本不匹配"));
        }
        let stage: String = row.get(1);
        let terminal = ["cancelled", "expired", "failed", "succeeded"].contains(&stage.as_str());
        if terminal {
            if (stage == "succeeded" || stage == "failed")
                && row.get::<_, Option<serde_json::Value>>(2) != Some(json!(report))
            {
                return Err(ApiError::conflict("完成回执内容冲突"));
            }
        } else {
            if command.as_ref().map(|c| &c.command_id) != Some(&report.command_id) {
                return Err(ApiError::conflict("旧命令回执"));
            }
            let rank = |s: &str| match s {
                "pending" => 0,
                "accepted" => 1,
                "executing" => 2,
                _ => 3,
            };
            if rank(&report.stage) < rank(&stage) {
                return Err(ApiError::conflict("回执顺序错误"));
            }
            if report.stage == "succeeded" {
                let complete = if saved.action == "enter" {
                    report.actual.state == "waiting"
                        && report.actual.command_id.as_deref() == Some(&saved.command_id)
                        && report.actual.session_key.as_deref() == Some(&saved.session_key)
                        && report.actual.monitors > 0
                } else if saved.action == "prepare_login" {
                    saved.login.as_ref().is_some_and(|plan| {
                        report.actual.account.as_ref().is_some_and(|s| {
                            s.login_complete()
                                && s.adapter == plan.snapshot.adapter
                                && plan
                                    .snapshot
                                    .target_sid
                                    .as_ref()
                                    .is_none_or(|sid| s.target_sid.as_ref() == Some(sid))
                        })
                    })
                } else if saved.action == "restore_teaching" {
                    saved.login.as_ref().is_some_and(|plan| {
                        report.actual.account.as_ref().is_some_and(|s| {
                            s.adapter == plan.snapshot.adapter
                                && s.phase == "teaching"
                                && s.detail == "restored"
                        })
                    })
                } else {
                    report.actual.state == "desktop"
                };
                if !complete {
                    return Err(ApiError::bad("缺少实际完成证据"));
                }
            }
            tx.execute(
                "UPDATE waiting_commands SET stage=$2,report=$3,updated_at=$4 WHERE id=$1",
                &[&report.command_id, &report.stage, &json!(report), &now()],
            )
            .await?;
        }
        acknowledged = Some(report.command_id.clone());
    }
    tx.commit().await?;
    Ok(ControlReply {
        protocol_version: CONTROL_VERSION,
        connection_id: connection.into(),
        generation,
        server_time: now(),
        lease_seconds: LEASE_SECONDS,
        command,
        acknowledged,
    })
}
