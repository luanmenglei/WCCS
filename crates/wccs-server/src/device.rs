use crate::{ApiError, ApiResult, App, auth};
use axum::{Json, extract::State, http::HeaderMap};
use serde_json::{Value, json};
use wccs_inventory::tools::ToolConfiguration;
use wccs_protocol::{
    CheckResult, Heartbeat, HeartbeatReply, Job, PROTOCOL_VERSION, Registration, digest, now,
    valid_id, validate_result,
};

pub async fn register(
    State(app): State<App>,
    Json(input): Json<Registration>,
) -> ApiResult<Json<Value>> {
    auth::rate_limit(&app, "registration", 600).await?;
    if input.protocol_version != PROTOCOL_VERSION
        || !valid_id(&input.device_id)
        || input.enrollment_token.len() != 64
        || input.device_token.len() != 64
    {
        return Err(ApiError::bad("设备注册格式或协议版本错误"));
    }
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let row = tx.query_opt("SELECT enrollment_hash,enrollment_expires,credential_hash FROM devices WHERE id=$1 FOR UPDATE", &[&input.device_id]).await?.ok_or_else(|| ApiError::unauthorized("注册材料无效"))?;
    let current: Option<String> = row.get(2);
    let credential = digest(&input.device_token);
    if let Some(current) = current {
        if current != credential {
            return Err(ApiError::conflict("设备已注册，不能使用旧注册材料接管身份"));
        }
    } else {
        if row.get::<_, String>(0) != digest(&input.enrollment_token)
            || row.get::<_, i64>(1) <= now()
        {
            return Err(ApiError::unauthorized("注册材料无效或已过期"));
        }
        tx.execute(
            "UPDATE devices SET credential_hash=$1 WHERE id=$2",
            &[&credential, &input.device_id],
        )
        .await
        .map_err(|error| {
            if error.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) {
                ApiError::conflict("设备凭据已绑定其他设备")
            } else {
                error.into()
            }
        })?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"device_id":input.device_id,"protocol_version":PROTOCOL_VERSION}),
    ))
}

pub async fn heartbeat(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Heartbeat>,
) -> ApiResult<Json<HeartbeatReply>> {
    let id = auth::device(&app, &headers).await?;
    if input.protocol_version != PROTOCOL_VERSION
        || !valid_id(&input.boot_id)
        || input.agent_version.len() > 80
    {
        return Err(ApiError::bad("心跳协议版本或字段错误"));
    }
    let db = app.db.lock().await;
    db.execute(
        "UPDATE devices SET last_seen=$1,boot_id=$2,agent_version=$3 WHERE id=$4",
        &[&now(), &input.boot_id, &input.agent_version, &id],
    )
    .await?;
    let row = db.query_opt("SELECT j.id,j.expires_at,c.snapshot FROM devices d JOIN jobs j ON d.desired_job=j.id JOIN configurations c ON c.version=j.config_version WHERE d.id=$1 AND j.result IS NULL AND j.expires_at>$2", &[&id,&now()]).await?;
    let job = match row {
        Some(row) => Some(Job {
            job_id: row.get(0),
            expires_at: row.get(1),
            configuration: serde_json::from_value(row.get(2)).map_err(|_| ApiError::internal())?,
        }),
        None => None,
    };
    Ok(Json(HeartbeatReply {
        protocol_version: PROTOCOL_VERSION,
        server_time: now(),
        job,
    }))
}

pub async fn result(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<CheckResult>,
) -> ApiResult<Json<Value>> {
    let id = auth::device(&app, &headers).await?;
    if input.device_id != id {
        return Err(ApiError::forbidden("结果设备身份不匹配"));
    }
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let row = tx.query_opt("SELECT j.result,j.expires_at,c.snapshot FROM jobs j JOIN configurations c ON c.version=j.config_version WHERE j.id=$1 AND j.device_id=$2 FOR UPDATE OF j", &[&input.job_id,&id]).await?.ok_or_else(|| ApiError::forbidden("任务未授权给此设备"))?;
    let config: ToolConfiguration =
        serde_json::from_value(row.get(2)).map_err(|_| ApiError::internal())?;
    validate_result(&input, &config).map_err(|message| ApiError::bad(&message))?;
    let value = serde_json::to_value(&input).unwrap();
    if let Some(previous) = row.get::<_, Option<Value>>(0) {
        if previous != value {
            return Err(ApiError::conflict("同一任务已回传不同结果"));
        }
    } else {
        if row.get::<_, i64>(1) <= now() {
            return Err(ApiError::new(410, "检查任务已过期，请管理员重新检查"));
        }
        tx.execute(
            "UPDATE jobs SET result=$1,received_at=$2 WHERE id=$3",
            &[&value, &now(), &input.job_id],
        )
        .await?;
    }
    let desired: Option<String> = tx
        .query_one("SELECT desired_job FROM devices WHERE id=$1", &[&id])
        .await?
        .get(0);
    let stale = desired.as_deref() != Some(&input.job_id);
    tx.commit().await?;
    Ok(Json(json!({"accepted":true,"stale":stale})))
}
