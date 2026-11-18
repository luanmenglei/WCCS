use crate::{ApiError, ApiResult, App, auth, lifecycle, management, seating};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::Transaction;
use wccs_protocol::{
    ImageRegistration, ONLINE_SECONDS, PROTOCOL_VERSION, digest, image_device_id, now, valid_id,
};

pub async fn register(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<ImageRegistration>,
) -> ApiResult<Json<Value>> {
    auth::rate_limit(&app, "image_registration", 600).await?;
    let key =
        image_device_id(&input.machine_name).ok_or_else(|| ApiError::bad("系统机器名无效"))?;
    let registration = input.registration;
    if ![
        "",
        "ready",
        "bootstrap_failed",
        "support_packages_failed",
        "login_template_failed",
        "service_start_failed",
    ]
    .contains(&input.bootstrap_status.as_str())
    {
        return Err(ApiError::bad("镜像初始化状态无效"));
    }
    if registration.protocol_version != PROTOCOL_VERSION
        || registration.device_id != key
        || [&registration.enrollment_token, &registration.device_token]
            .iter()
            .any(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(ApiError::bad("镜像注册格式或机器名关联错误"));
    }
    let enrollment = digest(&registration.enrollment_token);
    let credential = digest(&registration.device_token);
    let application = digest(&format!("{key}:{enrollment}:{credential}"));
    let (observed, _) =
        crate::device::source_address(peer.ip(), &headers, &app.config.trusted_proxies);
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    seating::lock(&tx).await?;
    let previous = tx
        .query_opt(
            "SELECT enrollment_hash,credential_hash,lifecycle FROM devices WHERE id=$1",
            &[&key],
        )
        .await?;
    if let Some(row) = &previous
        && row.get::<_, String>(0) == enrollment
        && row.get::<_, Option<String>>(1).as_deref() == Some(&credential)
    {
        return Ok(Json(
            json!({"device_id":key,"status":row.get::<_,String>(2),"application_id":application}),
        ));
    }
    if let Some(row) = tx
        .query_opt(
            "SELECT status,expires_at FROM image_applications WHERE id=$1",
            &[&application],
        )
        .await?
    {
        let status: String = row.get(0);
        let status = if status == "pending" && row.get::<_, i64>(1) <= now() {
            "expired"
        } else {
            &status
        };
        tx.execute(
            "UPDATE image_applications SET bootstrap_status=$2,observed_ip=$3 WHERE id=$1",
            &[&application, &input.bootstrap_status, &observed],
        )
        .await?;
        tx.commit().await?;
        return Ok(Json(
            json!({"device_id":key,"status":status,"application_id":application}),
        ));
    }
    let open: bool = tx
        .query_one(
            "SELECT closes_at>$1 FROM registration_window WHERE id=1",
            &[&now()],
        )
        .await?
        .get(0);
    if !open {
        return Err(ApiError::conflict("注册窗口关闭，客户端将自动重试"));
    }
    if tx
        .query_opt(
            "SELECT hash FROM credential_history WHERE hash=$1",
            &[&credential],
        )
        .await?
        .is_some()
    {
        return Err(ApiError::conflict(
            "该凭据已经使用，请检查是否复制了逐机材料",
        ));
    }
    let count: i64 = tx.query_one("SELECT count(*) FROM image_applications WHERE device_id=$1 AND status='pending' AND expires_at>$2", &[&key,&now()]).await?.get(0);
    if count >= 8 {
        return Err(ApiError::conflict(
            "同名待审核申请过多，请管理员核对并拒绝重复申请",
        ));
    }
    let name = input.machine_name.to_ascii_uppercase();
    if previous.is_none() {
        // The legacy registration endpoint must never auto-approve an image application.
        tx.execute("INSERT INTO devices(id,name,enrollment_hash,enrollment_expires) VALUES($1,$2,'image-managed',$3)", &[&key,&name,&(now()+86400)]).await?;
    }
    tx.execute(
        "INSERT INTO image_applications VALUES($1,$2,$3,$4,$5,$6,$7,$8,'pending',$9)",
        &[
            &application,
            &key,
            &name,
            &enrollment,
            &credential,
            &observed,
            &now(),
            &(now() + 86400),
            &input.bootstrap_status,
        ],
    )
    .await?;
    lifecycle::event(&tx, &key, "image_registration_requested", &application).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"device_id":key,"status":"pending","application_id":application}),
    ))
}

pub async fn snapshot(tx: &Transaction<'_>) -> ApiResult<Value> {
    let rows = tx.query("SELECT a.id,a.device_id,a.machine_name,a.observed_ip,a.created_at,a.expires_at,a.status,a.bootstrap_status,
        d.lifecycle_version,d.observed_ip AS previous_ip,d.last_seen,d.enrollment_generation,
        (SELECT count(*) FROM image_applications other WHERE other.device_id=a.device_id AND other.status='pending' AND other.expires_at>$1) AS competing,
        (SELECT array_agg(other.id ORDER BY other.id) FROM image_applications other WHERE other.device_id=a.device_id AND other.status='pending' AND other.expires_at>$1) AS candidates
        FROM image_applications a JOIN devices d ON d.id=a.device_id
        WHERE a.status='pending' ORDER BY a.created_at,a.id LIMIT 1000", &[&now()]).await?;
    Ok(json!(rows.iter().map(|row| json!({
        "id":row.get::<_,String>("id"),"device_id":row.get::<_,String>("device_id"),
        "review_token":digest(&json!([row.get::<_,String>("id"),row.get::<_,Option<String>>("observed_ip"),row.get::<_,Option<String>>("previous_ip"),row.get::<_,String>("bootstrap_status"),row.get::<_,Option<Vec<String>>>("candidates")]).to_string()),
        "bootstrap_status":row.get::<_,String>("bootstrap_status"),
        "machine_name":row.get::<_,String>("machine_name"),"observed_ip":row.get::<_,Option<String>>("observed_ip"),
        "previous_ip":row.get::<_,Option<String>>("previous_ip"),"created_at":row.get::<_,i64>("created_at"),
        "expires_at":row.get::<_,i64>("expires_at"),"base_version":row.get::<_,i64>("lifecycle_version"),
        "generation":row.get::<_,i64>("enrollment_generation"),"competing":row.get::<_,i64>("competing"),
        "online":row.get::<_,Option<i64>>("last_seen").is_some_and(|seen| now()-seen<ONLINE_SECONDS)
    })).collect::<Vec<_>>()))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    application_id: String,
    base_version: i64,
    review_token: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    request_id: String,
    approve: bool,
    targets: Vec<Selection>,
}

pub async fn action(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    if !valid_id(&input.request_id) || input.targets.is_empty() || input.targets.len() > 500 {
        return Err(ApiError::bad("请选择 1～500 个申请"));
    }
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    seating::lock(&tx).await?;
    if let Some(result) =
        management::previous(&tx, &input.request_id, "image_review", &payload).await?
    {
        return Ok(Json(result));
    }
    let review = snapshot(&tx).await?;
    let tokens: std::collections::HashMap<_, _> = review
        .as_array()
        .unwrap()
        .iter()
        .map(|application| {
            (
                application["id"].as_str().unwrap(),
                application["review_token"].as_str().unwrap(),
            )
        })
        .collect();
    let mut devices = std::collections::HashSet::new();
    for target in &input.targets {
        if tokens.get(target.application_id.as_str()).copied() != Some(target.review_token.as_str())
        {
            return Err(ApiError::conflict(
                "申请来源或同名申请已变化，请重新选择并核对",
            ));
        }
        let row = tx.query_opt("SELECT a.*,d.lifecycle_version,d.lifecycle,d.last_seen,d.enrollment_generation FROM image_applications a JOIN devices d ON d.id=a.device_id WHERE a.id=$1", &[&target.application_id]).await?.ok_or_else(||ApiError::bad("镜像申请不存在"))?;
        let device: String = row.get("device_id");
        if !devices.insert(device.clone()) {
            return Err(ApiError::bad("每台设备每次只能选择一个申请"));
        }
        if row.get::<_, String>("status") != "pending"
            || row.get::<_, i64>("lifecycle_version") != target.base_version
        {
            return Err(ApiError::conflict("申请已处理或设备版本变化，请刷新后核对"));
        }
        if input.approve {
            if row
                .get::<_, String>("bootstrap_status")
                .ends_with("_failed")
            {
                return Err(ApiError::conflict("客户端初始化失败，请先在现场修复并重试"));
            }
            if row
                .get::<_, Option<i64>>("last_seen")
                .is_some_and(|seen| now() - seen < ONLINE_SECONDS)
            {
                return Err(ApiError::conflict(
                    "原设备仍在线，请先核对同名机器，离线后再批准替换",
                ));
            }
            let credential: String = row.get("credential_hash");
            let enrollment: String = row.get("enrollment_hash");
            let generation = row.get::<_, i64>("enrollment_generation") + 1;
            lifecycle::reserve(&tx, &credential, &device, generation).await?;
            let state = if row.get::<_, String>("lifecycle") == "disabled" {
                "disabled"
            } else {
                "enabled"
            };
            tx.execute("UPDATE jobs SET cancelled_at=$2,cancellation_reason='image_reauthorized' WHERE device_id=$1 AND result IS NULL AND cancelled_at IS NULL", &[&device,&now()]).await?;
            tx.execute("UPDATE waiting_commands SET stage='cancelled',updated_at=$2 WHERE device_id=$1 AND stage NOT IN ('failed','cancelled','expired')", &[&device,&now()]).await?;
            tx.execute("UPDATE maintenance_jobs SET cancel_requested=true,updated_at=$2 WHERE device_id=$1 AND stage NOT IN ('succeeded','failed','cancelled','interrupted')", &[&device,&now()]).await?;
            tx.execute(
                "UPDATE control_devices SET connected=false WHERE device_id=$1",
                &[&device],
            )
            .await?;
            tx.execute(
                "DELETE FROM credential_rotations WHERE device_id=$1",
                &[&device],
            )
            .await?;
            tx.execute("UPDATE devices SET enrollment_hash=$2,credential_hash=$3,applicant_hash=$3,application_id=$4,application_at=$5,enrollment_expires=$6,
                enrollment_generation=$7,lifecycle=$8,lifecycle_version=lifecycle_version+1,last_seen=NULL,rotation_id=NULL,rotation_expires=NULL,desired_job=NULL,
                needs_review=true,address_version=address_version+1 WHERE id=$1",
                &[&device,&enrollment,&credential,&target.application_id,&now(),&(now()+86400),&generation,&state]).await?;
            tx.execute("UPDATE image_applications SET status='superseded' WHERE device_id=$1 AND status='pending' AND id<>$2", &[&device,&target.application_id]).await?;
        }
        tx.execute(
            "UPDATE image_applications SET status=$2 WHERE id=$1",
            &[
                &target.application_id,
                &if input.approve {
                    "approved"
                } else {
                    "rejected"
                },
            ],
        )
        .await?;
        lifecycle::event(
            &tx,
            &device,
            if input.approve {
                "image_approved"
            } else {
                "image_rejected"
            },
            &input.request_id,
        )
        .await?;
    }
    let response = management::remember(
        &tx,
        &app,
        &input.request_id,
        "image_review",
        &payload,
        now(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}
