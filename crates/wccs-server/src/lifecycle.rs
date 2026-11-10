use crate::{ApiError, ApiResult, App, auth, management, seating};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::Transaction;
use wccs_protocol::{digest, now, valid_id};

#[derive(Deserialize)]
pub struct HistoryQuery {
    before: Option<i64>,
    jobs_before: Option<i64>,
}

pub async fn history(
    State(app): State<App>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let identity = auth::authorize(&tx, &session, auth::Permission::Read).await?;
    identity.devices(&tx, std::slice::from_ref(&id), "").await?;
    let event_rows = tx
        .query(
            "SELECT sequence,operation,request_id,version,created_at FROM device_events
         WHERE device_id=$1 AND sequence<$2 AND ($3 OR (cardinality(room_ids)>0 AND room_ids<@$4)) ORDER BY sequence DESC LIMIT 100",
            &[&id, &query.before.unwrap_or(i64::MAX),&identity.all_rooms,&identity.room_ids],
        )
        .await?;
    let job_rows = tx.query(
        "SELECT sequence,id,config_version,expires_at,received_at,cancelled_at,cancellation_reason,result
         FROM jobs WHERE device_id=$1 AND sequence<$2 AND ($3 OR (cardinality(room_ids)>0 AND room_ids<@$4)) ORDER BY sequence DESC LIMIT 100",
        &[&id,&query.jobs_before.unwrap_or(i64::MAX),&identity.all_rooms,&identity.room_ids]).await?;
    let events: Vec<Value> = event_rows
        .iter()
        .map(|row| {
            json!({
                "sequence": row.get::<_,i64>(0),
                "operation": row.get::<_,String>(1),
                "request_id": row.get::<_,String>(2),
                "version": row.get::<_,i64>(3),
                "created_at": row.get::<_,i64>(4),
            })
        })
        .collect();
    let jobs: Vec<Value> = job_rows
        .iter()
        .map(|row| {
            json!({
                "sequence": row.get::<_,i64>(0),
                "id": row.get::<_,String>(1),
                "config_version": row.get::<_,i64>(2),
                "expires_at": row.get::<_,i64>(3),
                "received_at": row.get::<_,Option<i64>>(4),
                "cancelled_at": row.get::<_,Option<i64>>(5),
                "cancellation_reason": row.get::<_,Option<String>>(6),
                "result": row.get::<_,Option<Value>>(7),
            })
        })
        .collect();
    tx.commit().await?;
    Ok(Json(json!({"events":events,"jobs":jobs})))
}

pub(crate) async fn expire(tx: &Transaction<'_>) -> ApiResult<()> {
    tx.execute(
        "WITH expired AS (
        UPDATE devices SET lifecycle='expired',lifecycle_version=lifecycle_version+1
        WHERE lifecycle='pending' AND enrollment_expires<=$1 RETURNING id,lifecycle_version
        ) INSERT INTO device_events(device_id,operation,request_id,version,created_at)
        SELECT id,'expired','expiry',lifecycle_version,$1 FROM expired",
        &[&now()],
    )
    .await?;
    Ok(())
}

pub(crate) async fn event(
    tx: &Transaction<'_>,
    id: &str,
    operation: &str,
    request: &str,
) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO device_events(device_id,operation,request_id,version,created_at)
        SELECT id,$2,$3,lifecycle_version,$4 FROM devices WHERE id=$1",
        &[&id, &operation, &request, &now()],
    )
    .await?;
    Ok(())
}

pub(crate) async fn reserve(
    tx: &Transaction<'_>,
    hash: &str,
    id: &str,
    generation: i64,
) -> ApiResult<()> {
    let count = tx
        .execute(
            "INSERT INTO credential_history(hash,device_id,generation,created_at)
        VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING",
            &[&hash, &id, &generation, &now()],
        )
        .await?;
    if count != 1 {
        return Err(ApiError::conflict("凭据已使用，请使用独立身份目录与新凭据"));
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowRequest {
    request_id: String,
    base_version: i64,
    open: bool,
}

pub async fn window(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<WindowRequest>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    seating::lock(&tx).await?;
    if let Some(response) =
        management::previous(&tx, &input.request_id, "registration_window", &payload).await?
    {
        return Ok(Json(response));
    }
    let version: i64 = tx
        .query_one("SELECT version FROM registration_window WHERE id=1", &[])
        .await?
        .get(0);
    if version != input.base_version {
        return Err(ApiError::conflict("注册窗口版本已变化，请重新加载"));
    }
    let closes_at = if input.open { now() + 3600 } else { 0 };
    tx.execute(
        "UPDATE registration_window SET version=version+1,closes_at=$1 WHERE id=1",
        &[&closes_at],
    )
    .await?;
    let response = management::remember(
        &tx,
        &app,
        &input.request_id,
        "registration_window",
        &payload,
        version + 1,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    request_id: String,
    device_id: String,
    base_version: i64,
    action: String,
    #[serde(default)]
    enrollment_token: Option<String>,
}

pub async fn action(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    if !valid_id(&input.device_id) {
        return Err(ApiError::bad("设备编号无效"));
    }
    let mut payload = serde_json::to_value(&input).unwrap();
    // Request receipts retain only the digest of replacement material.
    payload["enrollment_token"] = json!(input.enrollment_token.as_ref().map(|value| digest(value)));
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let identity = auth::authorize(&tx, &session, auth::Permission::Operate).await?;
    if identity.role != "admin" && !["enable", "disable"].contains(&input.action.as_str()) {
        return Err(ApiError::forbidden(
            "注册审核、撤销和凭据管理需要管理员权限",
        ));
    }
    identity
        .devices(
            &tx,
            std::slice::from_ref(&input.device_id),
            &input.request_id,
        )
        .await?;
    seating::lock(&tx).await?;
    expire(&tx).await?;
    if let Some(response) =
        management::previous(&tx, &input.request_id, "device_action", &payload).await?
    {
        return Ok(Json(response));
    }
    let row = tx.query_opt("SELECT lifecycle,lifecycle_version,applicant_hash,enrollment_expires,rotation_id,rotation_expires,enrollment_hash
        FROM devices WHERE id=$1", &[&input.device_id]).await?.ok_or_else(||ApiError::new(404,"设备不存在"))?;
    let state: String = row.get(0);
    let version: i64 = row.get(1);
    if version != input.base_version {
        return Err(ApiError::conflict("设备状态已变化，请重新加载并核对"));
    }
    let next = match (input.action.as_str(), state.as_str()) {
        ("approve", "pending")
            if row.get::<_, Option<String>>(2).is_some() && row.get::<_, i64>(3) > now() =>
        {
            "enabled"
        }
        ("reject", "pending") => "rejected",
        ("enable", "disabled") => "enabled",
        ("disable", "enabled") => "disabled",
        ("revoke", "enabled" | "disabled") => "revoked",
        ("reauthorize", "revoked" | "rejected" | "expired") => "pending",
        ("rotate", "enabled" | "disabled") => state.as_str(),
        _ => return Err(ApiError::conflict("当前状态不允许此操作，请重新加载")),
    };
    if input.action == "approve" {
        tx.execute(
            "UPDATE devices SET credential_hash=applicant_hash WHERE id=$1",
            &[&input.device_id],
        )
        .await?;
    }
    if input.action == "revoke" {
        tx.execute(
            "UPDATE jobs SET cancelled_at=$2,cancellation_reason='device_revoked'
            WHERE device_id=$1 AND result IS NULL AND cancelled_at IS NULL",
            &[&input.device_id, &now()],
        )
        .await?;
        tx.execute("UPDATE devices SET credential_hash=NULL,desired_job=NULL,rotation_id=NULL,rotation_expires=NULL WHERE id=$1", &[&input.device_id]).await?;
        tx.execute(
            "DELETE FROM credential_rotations WHERE device_id=$1",
            &[&input.device_id],
        )
        .await?;
    }
    if input.action == "reauthorize" {
        let material = input
            .enrollment_token
            .as_ref()
            .filter(|value| value.len() == 64)
            .ok_or_else(|| ApiError::bad("需要新的逐设备注册材料"))?;
        let hash = digest(material);
        if hash == row.get::<_, String>(6) {
            return Err(ApiError::conflict("重新授权必须更换注册材料"));
        }
        tx.execute("UPDATE devices SET enrollment_hash=$2,enrollment_expires=$3,enrollment_generation=enrollment_generation+1,
            applicant_hash=NULL,application_id=NULL,application_at=NULL,credential_hash=NULL,last_seen=NULL,
            needs_review=true,address_version=address_version+1 WHERE id=$1",
            &[&input.device_id,&hash,&(now()+86400)]).await?;
    }
    if input.action == "rotate" {
        if row.get::<_, Option<String>>(4).is_some()
            && row
                .get::<_, Option<i64>>(5)
                .is_some_and(|time| time > now())
        {
            return Err(ApiError::conflict("已有凭据轮换等待客户端完成"));
        }
        tx.execute(
            "DELETE FROM credential_rotations WHERE device_id=$1",
            &[&input.device_id],
        )
        .await?;
        tx.execute(
            "UPDATE devices SET rotation_id=$2,rotation_expires=$3 WHERE id=$1",
            &[&input.device_id, &input.request_id, &(now() + 86400)],
        )
        .await?;
    }
    tx.execute(
        "UPDATE devices SET lifecycle=$2,lifecycle_version=lifecycle_version+1 WHERE id=$1",
        &[&input.device_id, &next],
    )
    .await?;
    event(&tx, &input.device_id, &input.action, &input.request_id).await?;
    let response = management::remember(
        &tx,
        &app,
        &input.request_id,
        "device_action",
        &payload,
        version + 1,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    device_id: String,
    request_id: String,
    new_token: String,
}

pub async fn rotate(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Rotation>,
) -> ApiResult<Json<Value>> {
    if !valid_id(&input.device_id) || !valid_id(&input.request_id) || input.new_token.len() != 64 {
        return Err(ApiError::bad("轮换请求格式错误"));
    }
    let old_hash = auth::credential_hash(&headers)?;
    let new_hash = digest(&input.new_token);
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    seating::lock(&tx).await?;
    let row = tx.query_opt("SELECT credential_hash,lifecycle,rotation_id,rotation_expires,enrollment_generation FROM devices WHERE id=$1", &[&input.device_id]).await?.ok_or_else(||ApiError::unauthorized("设备认证失败"))?;
    let current: Option<String> = row.get(0);
    let state: String = row.get(1);
    if !matches!(state.as_str(), "enabled" | "disabled") {
        return Err(ApiError::unauthorized("设备凭据已失效"));
    }
    if let Some(receipt) = tx.query_opt("SELECT old_hash,new_hash,expires_at FROM credential_rotations WHERE id=$1 AND device_id=$2", &[&input.request_id,&input.device_id]).await? {
        if current.as_deref()==Some(&new_hash) && receipt.get::<_,String>(1)==new_hash
            && (receipt.get::<_,String>(0)==old_hash || old_hash==new_hash) && receipt.get::<_,i64>(2)>now() {
            return Ok(Json(json!({"ok":true})));
        }
        return Err(ApiError::unauthorized("凭据切换重试不匹配或已过期"));
    }
    if current.as_deref() != Some(&old_hash) {
        return Err(ApiError::unauthorized("设备认证失败"));
    }
    if row.get::<_, Option<String>>(2).as_deref() != Some(&input.request_id)
        || !row
            .get::<_, Option<i64>>(3)
            .is_some_and(|time| time > now())
    {
        return Err(ApiError::conflict(
            "轮换请求已过期或未授权，请管理员重新发起",
        ));
    }
    reserve(&tx, &new_hash, &input.device_id, row.get(4)).await?;
    tx.execute(
        "INSERT INTO credential_rotations VALUES($1,$2,$3,$4,$5)",
        &[
            &input.request_id,
            &input.device_id,
            &old_hash,
            &new_hash,
            &row.get::<_, i64>(3),
        ],
    )
    .await?;
    tx.execute("UPDATE devices SET credential_hash=$2,rotation_id=NULL,rotation_expires=NULL,lifecycle_version=lifecycle_version+1 WHERE id=$1", &[&input.device_id,&new_hash]).await?;
    event(
        &tx,
        &input.device_id,
        "credential_rotated",
        &input.request_id,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
