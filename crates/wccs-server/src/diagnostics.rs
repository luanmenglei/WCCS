use crate::{ApiError, ApiResult, App, auth};
use axum::{
    Json,
    extract::{Query, Request, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use wccs_protocol::{diagnostics::Report, now, valid_id};

/// A separate probe avoids replacing a live control connection or its authority.
pub async fn context(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let id = auth::device(&tx, &headers).await?;
    let row = tx.query_one("SELECT lifecycle,enrollment_generation,observed_ip,address_version,needs_review FROM devices WHERE id=$1", &[&id]).await?;
    let seating = crate::seating::snapshot(&tx).await?;
    let mut seats = Vec::new();
    for room in seating["rooms"].as_array().into_iter().flatten() {
        for seat in room["seats"].as_array().into_iter().flatten() {
            if seat["devices"]
                .as_array()
                .is_some_and(|devices| devices.iter().any(|device| device.as_str() == Some(&id)))
            {
                seats.push(json!({"room_id":room["id"],"room_name":room["name"],"seat":seat}));
            }
        }
    }
    Ok(Json(
        json!({"protocol_version":1,"device_id":id,"server_time":now(),
        "lifecycle":row.get::<_,String>(0),"enrollment_generation":row.get::<_,i64>(1),
        "observed_ip":row.get::<_,Option<String>>(2),"address_version":row.get::<_,i64>(3),
        "needs_review":row.get::<_,bool>(4),"seats":seats}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    nonce: String,
}

pub async fn probe(
    State(app): State<App>,
    headers: HeaderMap,
    Query(input): Query<Probe>,
    request: Request,
) -> ApiResult<Response> {
    if !valid_id(&input.nonce) {
        return Err(ApiError::bad("诊断关联号无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let id = auth::device(&tx, &headers).await?;
    let receipt = json!({"protocol_version":1,"device_id":id,"nonce":input.nonce,"server_time":now(),"server_epoch":app.server_epoch});
    let accept = crate::control::handshake(&headers)?;
    if headers.contains_key("origin") {
        return Err(ApiError::forbidden("设备诊断拒绝浏览器来源"));
    }
    drop(tx);
    drop(db);
    tokio::spawn(async move {
        use futures_util::SinkExt;
        use tokio_tungstenite::{
            WebSocketStream,
            tungstenite::{
                Message,
                protocol::{Role, WebSocketConfig},
            },
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let upgraded = hyper::upgrade::on(request).await?;
            let config = WebSocketConfig::default()
                .max_message_size(Some(4096))
                .max_frame_size(Some(4096));
            let mut socket = WebSocketStream::from_raw_socket(
                hyper_util::rt::TokioIo::new(upgraded),
                Role::Server,
                Some(config),
            )
            .await;
            socket
                .send(Message::Text(receipt.to_string().into()))
                .await?;
            socket.close(None).await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await;
    });
    Ok((
        axum::http::StatusCode::SWITCHING_PROTOCOLS,
        [
            ("upgrade", "websocket"),
            ("connection", "Upgrade"),
            ("sec-websocket-accept", accept.as_str()),
        ],
    )
        .into_response())
}

pub async fn receive(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Report>,
) -> ApiResult<Json<Value>> {
    if !input.valid() {
        return Err(ApiError::bad("诊断报告格式无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let id = auth::device(&tx, &headers).await?;
    if input.device_id != id {
        return Err(ApiError::forbidden("诊断设备身份不匹配"));
    }
    let generation: i64 = tx
        .query_one(
            "SELECT enrollment_generation FROM devices WHERE id=$1",
            &[&id],
        )
        .await?
        .get(0);
    let report = serde_json::to_value(&input).map_err(|_| ApiError::internal())?;
    if let Some(row) = tx.query_opt("SELECT device_id,enrollment_generation,report FROM device_diagnostics WHERE request_id=$1", &[&input.request_id]).await? {
        if row.get::<_,String>(0) != id || row.get::<_,i64>(1) != generation || row.get::<_,Value>(2) != report {
            return Err(ApiError::conflict("诊断请求编号已关联其他内容或身份"));
        }
    } else {
        tx.execute("INSERT INTO device_diagnostics VALUES($1,$2,$3,$4,$5)", &[&input.request_id,&id,&generation,&report,&now()]).await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"accepted":true,"request_id":input.request_id,"device_id":id}),
    ))
}
