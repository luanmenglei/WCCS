use crate::{ApiError, ApiResult, App, auth};
use axum::{Json, extract::State, http::HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_postgres::Transaction;
use wccs_inventory::tools::{TOOL_CATALOG, ToolConfiguration};
use wccs_protocol::{
    ONLINE_SECONDS, PublishRequest, RecheckRequest, digest, now, token, valid_id, validate_tools,
};

pub async fn state(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await;
    let expiry = db.transaction().await?;
    crate::seating::lock(&expiry).await?;
    crate::lifecycle::expire(&expiry).await?;
    expiry.commit().await?;
    let tx = db
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let version: i64 = tx
        .query_one("SELECT version FROM configuration_head WHERE id=1", &[])
        .await?
        .get(0);
    let config = tx
        .query_opt(
            "SELECT snapshot, targets FROM configurations WHERE version=$1",
            &[&version],
        )
        .await?;
    let (configuration, targets) = config
        .map(|row| (row.get::<_, Value>(0), row.get::<_, Value>(1)))
        .unwrap_or((
            json!({"schema_version":1,"config_version":"0","tools":[]}),
            json!([]),
        ));
    let rows = tx
        .query(
            "SELECT d.id, d.name, d.last_seen, d.boot_id, d.agent_version,
                d.credential_hash IS NOT NULL AS registered, d.desired_job,
                d.lifecycle,d.lifecycle_version,d.application_id,d.application_at,d.enrollment_expires,
                d.enrollment_generation,d.rotation_id,d.rotation_expires,
                j.config_version, j.expires_at, r.result, r.received_at, r.id AS result_job
         FROM devices d
         LEFT JOIN jobs j ON j.id=d.desired_job
         LEFT JOIN LATERAL (
             SELECT result, received_at, id FROM jobs
             WHERE device_id=d.id AND result IS NOT NULL
             ORDER BY sequence DESC LIMIT 1
         ) r ON true
         ORDER BY d.id",
            &[],
        )
        .await?;
    let devices: Vec<Value> = rows.iter().map(|row| {
        let last_seen: Option<i64> = row.get("last_seen");
        let desired: Option<String> = row.get("desired_job");
        let result_job: Option<String> = row.get("result_job");
        let expires: Option<i64> = row.get("expires_at");
        json!({
            "id":row.get::<_,String>("id"), "name":row.get::<_,String>("name"),
            "lifecycle":row.get::<_,String>("lifecycle"), "lifecycle_version":row.get::<_,i64>("lifecycle_version"),
            "application_id":row.get::<_,Option<String>>("application_id"),"application_at":row.get::<_,Option<i64>>("application_at"),
            "enrollment_expires":row.get::<_,i64>("enrollment_expires"),"enrollment_generation":row.get::<_,i64>("enrollment_generation"),
            "rotation_id":row.get::<_,Option<String>>("rotation_id"),"rotation_expires":row.get::<_,Option<i64>>("rotation_expires"),
            "registered":row.get::<_,bool>("registered"), "last_seen":last_seen,
            "online":row.get::<_,bool>("registered") && last_seen.is_some_and(|seen| now()-seen < ONLINE_SECONDS),
            "boot_id":row.get::<_,Option<String>>("boot_id"), "agent_version":row.get::<_,Option<String>>("agent_version"),
            "desired_job":desired,"desired_version":row.get::<_,Option<i64>>("config_version"),
            "expired":expires.is_some_and(|time| time <= now()) && result_job != desired,
            "result":row.get::<_,Option<Value>>("result"), "received_at":row.get::<_,Option<i64>>("received_at"),
            "stale":result_job.is_some() && result_job != desired,
        })
    }).collect();
    let window = tx
        .query_one(
            "SELECT version,closes_at FROM registration_window WHERE id=1",
            &[],
        )
        .await?;
    let registration_window = json!({"version":window.get::<_,i64>(0),"closes_at":window.get::<_,i64>(1),"open":window.get::<_,i64>(1)>now()});
    let seating = crate::seating::snapshot(&tx).await?;
    let roster = crate::roster::snapshot(&tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"roster":roster,"registration_window":registration_window,"seating":seating,"protocol_version":1,"server_time":now(),"version":version,"configuration":configuration,"targets":targets,"catalog":serde_json::from_str::<Value>(TOOL_CATALOG).unwrap()["tools"],"devices":devices}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewDevice {
    id: String,
    name: String,
    enrollment_token: String,
}

pub async fn create_device(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<NewDevice>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    if !valid_id(&input.id)
        || input.name.trim().is_empty()
        || input.name.len() > 200
        || input.enrollment_token.len() != 64
    {
        return Err(ApiError::bad("设备 ID、名称或注册令牌格式错误"));
    }
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    tx.query_one(
        "SELECT version FROM configuration_head WHERE id=1 FOR UPDATE",
        &[],
    )
    .await?;
    let hash = digest(&input.enrollment_token);
    if let Some(row) = tx
        .query_opt(
            "SELECT name,enrollment_hash FROM devices WHERE id=$1",
            &[&input.id],
        )
        .await?
    {
        if row.get::<_, String>(0) != input.name || row.get::<_, String>(1) != hash {
            return Err(ApiError::conflict("设备 ID 已存在"));
        }
    } else {
        let count: i64 = tx
            .query_one("SELECT count(*) FROM devices", &[])
            .await?
            .get(0);
        if count >= 500 {
            return Err(ApiError::bad("本切片最多注册 500 台设备"));
        }
        tx.execute(
            "INSERT INTO devices (id,name,enrollment_hash,enrollment_expires) VALUES ($1,$2,$3,$4)",
            &[&input.id, &input.name, &hash, &(now() + 86400)],
        )
        .await?;
        audit(&tx, &app.config.admin_username, "create_device", &input.id).await?;
        crate::lifecycle::event(&tx, &input.id, "created", &input.id).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"device_id":input.id,"ok":true})))
}

pub(crate) async fn previous(
    tx: &Transaction<'_>,
    id: &str,
    operation: &str,
    payload: &Value,
) -> ApiResult<Option<Value>> {
    if !valid_id(id) {
        return Err(ApiError::bad("请求编号无效"));
    }
    if let Some(row) = tx
        .query_opt(
            "SELECT operation,payload,response FROM requests WHERE id=$1",
            &[&id],
        )
        .await?
    {
        if row.get::<_, String>(0) != operation || row.get::<_, Value>(1) != *payload {
            return Err(ApiError::conflict("请求编号已用于不同内容"));
        }
        return Ok(Some(row.get(2)));
    }
    Ok(None)
}

async fn targets(tx: &Transaction<'_>, ids: &[String], enabled_only: bool) -> ApiResult<()> {
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    if ids.is_empty() || ids.len() > 500 || unique.len() != ids.len() {
        return Err(ApiError::bad("请选择 1～500 台不同的目标设备"));
    }
    for id in ids {
        if tx
            .query_opt(
                "SELECT id FROM devices WHERE id=$1 AND credential_hash IS NOT NULL AND (lifecycle='enabled' OR (NOT $2 AND lifecycle='disabled'))",
                &[id, &enabled_only],
            )
            .await?
            .is_none()
        {
            return Err(ApiError::bad("目标设备状态不允许本次操作，请核对注册与启用状态"));
        }
    }
    Ok(())
}

pub(crate) async fn issue(tx: &Transaction<'_>, ids: &[String], version: i64) -> ApiResult<()> {
    for id in ids {
        let job = token();
        tx.execute(
            "INSERT INTO jobs (id,device_id,config_version,expires_at) VALUES ($1,$2,$3,$4)",
            &[&job, id, &version, &(now() + 86400)],
        )
        .await?;
        tx.execute("UPDATE devices SET desired_job=$1 WHERE id=$2", &[&job, id])
            .await?;
    }
    Ok(())
}

pub(crate) async fn audit(
    tx: &Transaction<'_>,
    actor: &str,
    operation: &str,
    id: &str,
) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO audit_events (actor,operation,request_id,created_at) VALUES ($1,$2,$3,$4)",
        &[&actor, &operation, &id, &now()],
    )
    .await?;
    Ok(())
}

pub(crate) async fn remember(
    tx: &Transaction<'_>,
    app: &App,
    id: &str,
    operation: &str,
    payload: &Value,
    version: i64,
) -> ApiResult<Value> {
    let response = json!({"version":version,"request_id":id});
    tx.execute(
        "INSERT INTO requests (id,operation,payload,response,created_at) VALUES ($1,$2,$3,$4,$5)",
        &[&id, &operation, payload, &response, &now()],
    )
    .await?;
    audit(tx, &app.config.admin_username, operation, id).await?;
    Ok(response)
}

pub async fn publish(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<PublishRequest>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    validate_tools(&input.tools).map_err(|message| ApiError::bad(&message))?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let version: i64 = tx
        .query_one(
            "SELECT version FROM configuration_head WHERE id=1 FOR UPDATE",
            &[],
        )
        .await?
        .get(0);
    if let Some(response) = previous(&tx, &input.request_id, "publish", &payload).await? {
        return Ok(Json(response));
    }
    if version != input.base_version {
        return Err(ApiError::conflict(&format!(
            "配置已更新到 v{version}，请重新加载后编辑"
        )));
    }
    targets(&tx, &input.targets, false).await?;
    let next = version + 1;
    let snapshot = serde_json::to_value(ToolConfiguration {
        schema_version: 1,
        config_version: next.to_string(),
        tools: input.tools,
    })
    .unwrap();
    tx.execute(
        "INSERT INTO configurations VALUES ($1,$2,$3,$4)",
        &[
            &next,
            &snapshot,
            &serde_json::to_value(&input.targets).unwrap(),
            &now(),
        ],
    )
    .await?;
    tx.execute(
        "UPDATE configuration_head SET version=$1 WHERE id=1",
        &[&next],
    )
    .await?;
    tx.execute("UPDATE devices SET desired_job=NULL", &[])
        .await?;
    issue(&tx, &input.targets, next).await?;
    let response = remember(&tx, &app, &input.request_id, "publish", &payload, next).await?;
    tx.commit().await?;
    Ok(Json(response))
}

pub async fn recheck(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<RecheckRequest>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let version: i64 = tx
        .query_one(
            "SELECT version FROM configuration_head WHERE id=1 FOR UPDATE",
            &[],
        )
        .await?
        .get(0);
    if let Some(response) = previous(&tx, &input.request_id, "recheck", &payload).await? {
        return Ok(Json(response));
    }
    if version == 0 || version != input.base_version {
        return Err(ApiError::conflict("配置已变化，请重新加载后检查"));
    }
    targets(&tx, &input.targets, true).await?;
    let configured: Value = tx
        .query_one(
            "SELECT targets FROM configurations WHERE version=$1",
            &[&version],
        )
        .await?
        .get(0);
    let configured: Vec<String> =
        serde_json::from_value(configured).map_err(|_| ApiError::internal())?;
    if input.targets.iter().any(|id| !configured.contains(id)) {
        return Err(ApiError::bad("重新检查的设备必须属于当前配置目标"));
    }
    issue(&tx, &input.targets, version).await?;
    let response = remember(&tx, &app, &input.request_id, "recheck", &payload, version).await?;
    tx.commit().await?;
    Ok(Json(response))
}
