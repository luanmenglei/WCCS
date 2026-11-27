use crate::{ApiError, ApiResult, App, auth, management, seating};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::Transaction;
use wccs_protocol::{
    bytes_digest, digest,
    distribution::{Command, Manifest, Progress},
    now, token, valid_id,
};

pub async fn list(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;
    let artifacts = tx.query("SELECT manifest,ready,(SELECT count(*) FROM distribution_chunks c WHERE c.artifact_id=a.id) AS uploaded FROM distribution_artifacts a ORDER BY created_at DESC LIMIT 100", &[]).await?;
    let jobs = tx.query("SELECT j.*,d.name FROM distribution_jobs j JOIN devices d ON d.id=j.device_id WHERE $1 OR (cardinality(j.room_ids)>0 AND j.room_ids<@$2) ORDER BY j.updated_at DESC,j.id LIMIT 1000", &[&actor.all_rooms,&actor.room_ids]).await?;
    Ok(Json(
        json!({"artifacts":artifacts.iter().map(|row|json!({"manifest":row.get::<_,Value>(0),"ready":row.get::<_,bool>(1),"uploaded_chunks":row.get::<_,i64>(2)})).collect::<Vec<_>>(),
        "jobs":jobs.iter().map(|row|json!({"id":row.get::<_,String>("id"),"device_id":row.get::<_,String>("device_id"),"device_name":row.get::<_,String>("name"),"artifact_id":row.get::<_,String>("artifact_id"),"stage":if row.get::<_,bool>("publication_claimed") && row.get::<_,String>("stage")=="ready" {"publishing".to_string()} else {row.get::<_,String>("stage")},"verified_chunks":row.get::<_,i32>("verified_chunks"),"detail":row.get::<_,String>("detail"),"directory":row.get::<_,String>("directory"),"adapter":row.get::<_,String>("adapter"),"publish_at":row.get::<_,i64>("publish_at"),"updated_at":row.get::<_,i64>("updated_at")})).collect::<Vec<_>>() }),
    ))
}

pub async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Manifest>,
) -> ApiResult<Json<Value>> {
    if !input.valid() {
        return Err(ApiError::bad(
            "文件名、大小或分块摘要无效，单文件上限 512 MiB",
        ));
    }
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin)
        .await?
        .global()?;
    let manifest = json!(input);
    if let Some(row) = tx
        .query_opt(
            "SELECT manifest FROM distribution_artifacts WHERE id=$1",
            &[&input.id],
        )
        .await?
    {
        if row.get::<_, Value>(0) != manifest {
            return Err(ApiError::conflict("文件 ID 已关联另一内容"));
        }
    } else {
        tx.execute(
            "INSERT INTO distribution_artifacts(id,manifest,created_at) VALUES($1,$2,$3)",
            &[&input.id, &manifest, &now()],
        )
        .await?;
    }
    let indices: Vec<i32> = tx
        .query(
            "SELECT chunk_index FROM distribution_chunks WHERE artifact_id=$1 ORDER BY chunk_index",
            &[&input.id],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    tx.commit().await?;
    Ok(Json(json!({"id":input.id,"uploaded":indices})))
}

pub async fn upload(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, index)): Path<(String, i32)>,
    bytes: Bytes,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin)
        .await?
        .global()?;
    let row = tx
        .query_opt(
            "SELECT manifest FROM distribution_artifacts WHERE id=$1",
            &[&id],
        )
        .await?
        .ok_or_else(|| ApiError::bad("上传文件不存在"))?;
    let manifest: Manifest =
        serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    if index < 0
        || manifest.chunk_length(index as usize) != Some(bytes.len())
        || manifest.hashes[index as usize] != bytes_digest(&bytes)
    {
        return Err(ApiError::bad("分块长度或摘要不匹配"));
    }
    tx.execute(
        "INSERT INTO distribution_chunks VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
        &[&id, &index, &&bytes[..]],
    )
    .await?;
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM distribution_chunks WHERE artifact_id=$1",
            &[&id],
        )
        .await?
        .get(0);
    let ready = count as usize == manifest.hashes.len();
    tx.execute(
        "UPDATE distribution_artifacts SET ready=$2 WHERE id=$1",
        &[&id, &ready],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"index":index,"ready":ready})))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    artifact_id: String,
    profile_version: i64,
    device_ids: Vec<String>,
    publish_at: i64,
    bytes_per_second: i64,
    #[serde(default)]
    preview_token: Option<String>,
}
async fn plan(tx: &Transaction<'_>, input: &Write) -> ApiResult<Value> {
    if !valid_id(&input.request_id)
        || !valid_id(&input.artifact_id)
        || input.device_ids.is_empty()
        || input.device_ids.len() > 500
        || input.device_ids.iter().any(|id| !valid_id(id))
        || input
            .device_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.device_ids.len()
        || input.publish_at < 0
        || input.publish_at > now() + 30 * 86400
        || !(65536..=16777216).contains(&input.bytes_per_second)
    {
        return Err(ApiError::bad("分发范围、发布时间或限速无效"));
    }
    let artifact = tx
        .query_opt(
            "SELECT manifest FROM distribution_artifacts WHERE id=$1 AND ready",
            &[&input.artifact_id],
        )
        .await?
        .ok_or_else(|| ApiError::bad("文件尚未完整上传并校验"))?;
    let profile = crate::preparation::resolve(tx, input.profile_version).await?;
    let devices = tx.query("SELECT d.id,d.name,d.enrollment_generation,d.lifecycle,d.needs_review,d.excluded,m.adapter,m.generation,m.last_seen,c.actual,device_room_scope(d.id) AS rooms,EXISTS(SELECT 1 FROM distribution_jobs j WHERE j.device_id=d.id AND j.artifact_id=$2 AND j.stage IN ('queued','downloading','ready')) AS busy FROM devices d LEFT JOIN distribution_devices m ON m.device_id=d.id LEFT JOIN control_devices c ON c.device_id=d.id WHERE d.id=ANY($1)", &[&input.device_ids,&input.artifact_id]).await?;
    if devices.len() != input.device_ids.len() {
        return Err(ApiError::bad("设备不存在"));
    }
    let targets: Vec<Value> = input.device_ids.iter().map(|id| {
        let row = devices.iter().find(|row|row.get::<_,String>("id")==*id).unwrap();
        let adapter: Option<String> = row.get("adapter");
        let actual: Option<Value> = row.get("actual");
        let rooms: Option<Vec<String>> = row.get("rooms");
        let reason = if row.get::<_,String>("lifecycle")!="enabled" || row.get::<_,bool>("needs_review") || row.get::<_,bool>("excluded") || rooms.as_ref().is_none_or(Vec::is_empty) { Some("device_not_ready") }
        else if row.get::<_,Option<i64>>("generation")!=Some(row.get("enrollment_generation")) || row.get::<_,Option<i64>>("last_seen").is_none_or(|seen|now()-seen>=35) { Some("client_offline_or_unsupported") }
        else if row.get::<_,bool>("busy") { Some("transfer_active") }
        else if adapter.as_deref()==Some("service") && actual.as_ref().is_none_or(|a|a["account"]["account"]!="owned" || a["account"]["desktop_confirmed"]!=true || a["account"]["target_sid"]!=a["account"]["observed_sid"]) { Some("ordinary_account_unavailable") }
        else { None };
        json!({"device_id":id,"name":row.get::<_,String>("name"),"generation":row.get::<_,i64>("enrollment_generation"),"adapter":adapter,"room_ids":rooms,"reason":reason})
    }).collect();
    let mut result = json!({"artifact":artifact.get::<_,Value>(0),"directory":profile.profile.work_directory,"profile_version":input.profile_version,"publish_at":input.publish_at,"bytes_per_second":input.bytes_per_second,"targets":targets});
    result["preview_token"] = json!(digest(&result.to_string()));
    Ok(result)
}

pub async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Operate).await?;
    actor
        .devices(&tx, &input.device_ids, &input.request_id)
        .await?;
    Ok(Json(plan(&tx, &input).await?))
}
pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Operate).await?;
    actor
        .devices(&tx, &input.device_ids, &input.request_id)
        .await?;
    let payload = json!(input);
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "distribution_commit", &payload).await?
    {
        return Ok(Json(previous));
    }
    let plan = plan(&tx, &input).await?;
    if plan["preview_token"].as_str() != input.preview_token.as_deref() {
        return Err(ApiError::conflict("分发配置或目标已变化，请重新预览"));
    }
    let mut ids = Vec::new();
    for target in plan["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|target| target["reason"].is_null())
    {
        let id = token();
        let rooms: Vec<String> =
            serde_json::from_value(target["room_ids"].clone()).map_err(|_| ApiError::internal())?;
        tx.execute("INSERT INTO distribution_jobs(id,request_id,actor_id,actor_version,device_id,generation,room_ids,artifact_id,directory,adapter,publish_at,bytes_per_second,stage,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'queued',$13)",
            &[&id,&input.request_id,&actor.id,&actor.version,&target["device_id"].as_str().unwrap(),&target["generation"].as_i64().unwrap(),&rooms,&input.artifact_id,&plan["directory"].as_str().unwrap(),&target["adapter"].as_str().unwrap(),&input.publish_at,&input.bytes_per_second,&now()]).await?;
        ids.push(id);
    }
    if ids.is_empty() {
        return Err(ApiError::conflict("没有可分发目标，请检查逐机原因"));
    }
    let response = json!({"request_id":input.request_id,"jobs":ids,"preview":plan});
    management::record(
        &tx,
        &input.request_id,
        "distribution_commit",
        &payload,
        &response,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    request_id: String,
    job_id: String,
}
pub async fn cancel(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Cancel>,
) -> ApiResult<Json<Value>> {
    if !valid_id(&input.request_id) || !valid_id(&input.job_id) {
        return Err(ApiError::bad("请求编号无效"));
    }
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Operate).await?;
    let row = tx
        .query_opt(
            "SELECT room_ids FROM distribution_jobs WHERE id=$1",
            &[&input.job_id],
        )
        .await?
        .ok_or_else(|| ApiError::bad("任务不存在"))?;
    actor.rooms(Some(&row.get::<_, Vec<String>>(0)))?;
    let payload = json!(input);
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "distribution_cancel", &payload).await?
    {
        return Ok(Json(previous));
    }
    if tx.execute("UPDATE distribution_jobs SET stage='cancelled',detail='cancelled_by_operator',updated_at=$2 WHERE id=$1 AND NOT publication_claimed AND stage IN ('queued','downloading','ready')", &[&input.job_id,&now()]).await? != 1 {
        return Err(ApiError::conflict("任务已进入发布或已经结束，请核对逐机结果"));
    }
    let response = json!({"job_id":input.job_id});
    management::record(
        &tx,
        &input.request_id,
        "distribution_cancel",
        &payload,
        &response,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

async fn expire(tx: &Transaction<'_>, device: &str) -> ApiResult<()> {
    tx.execute("UPDATE distribution_jobs j SET stage=CASE WHEN publication_claimed THEN 'failed' ELSE 'cancelled' END,detail=CASE WHEN publication_claimed THEN 'publication_unconfirmed' ELSE 'authorization_changed' END,updated_at=$2 WHERE j.device_id=$1 AND j.stage IN ('queued','downloading','ready') AND NOT EXISTS(SELECT 1 FROM devices d,operators o WHERE d.id=j.device_id AND d.enrollment_generation=j.generation AND d.lifecycle='enabled' AND NOT d.needs_review AND NOT d.excluded AND o.id=j.actor_id AND o.version=j.actor_version AND o.enabled AND o.role<>'viewer' AND (o.all_rooms OR j.room_ids<@o.room_ids) AND device_room_scope(d.id)=j.room_ids)", &[&device,&now()]).await?;
    Ok(())
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    pub adapter: String,
}
pub async fn poll(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Poll>,
) -> ApiResult<Json<Value>> {
    if !["service", "simulation"].contains(&input.adapter.as_str()) {
        return Err(ApiError::bad("分发适配器不兼容"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    tx.execute("INSERT INTO distribution_devices SELECT id,$2,enrollment_generation,$3 FROM devices WHERE id=$1 ON CONFLICT(device_id) DO UPDATE SET adapter=$2,generation=excluded.generation,last_seen=$3", &[&device,&input.adapter,&now()]).await?;
    expire(&tx, &device).await?;
    let rows=tx.query("SELECT j.*,a.manifest FROM distribution_jobs j JOIN distribution_artifacts a ON a.id=j.artifact_id WHERE j.device_id=$1 AND j.adapter=$2 AND j.stage IN ('queued','downloading','ready') ORDER BY j.updated_at,j.id LIMIT 4", &[&device,&input.adapter]).await?;
    let mut commands = Vec::new();
    for row in rows {
        let mut publish: bool = row.get("publication_claimed");
        if !publish
            && row.get::<_, String>("stage") == "ready"
            && row.get::<_, i64>("publish_at") <= now()
        {
            tx.execute(
                "UPDATE distribution_jobs SET publication_claimed=true WHERE id=$1",
                &[&row.get::<_, String>("id")],
            )
            .await?;
            publish = true;
        }
        commands.push(Command {
            job_id: row.get("id"),
            device_id: device.clone(),
            generation: row.get("generation"),
            manifest: serde_json::from_value(row.get("manifest"))
                .map_err(|_| ApiError::internal())?,
            directory: row.get("directory"),
            publish,
            bytes_per_second: row.get::<_, i64>("bytes_per_second") as u64,
            adapter: input.adapter.clone(),
        });
    }
    tx.commit().await?;
    Ok(Json(json!({"jobs":commands})))
}
pub async fn download(
    State(app): State<App>,
    headers: HeaderMap,
    Path((job, index)): Path<(String, i32)>,
) -> ApiResult<impl IntoResponse> {
    let _slot = download_slot(&app, &headers).await?;
    let bytes = {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        seating::lock(&tx).await?;
        let device = auth::device(&tx, &headers).await?;
        expire(&tx, &device).await?;
        let row=tx.query_opt("SELECT c.content,j.bytes_per_second FROM distribution_jobs j JOIN distribution_chunks c ON c.artifact_id=j.artifact_id AND c.chunk_index=$3 WHERE j.id=$1 AND j.device_id=$2 AND j.stage IN ('queued','downloading','ready')", &[&job,&device,&index]).await?;
        let row = row.ok_or_else(|| ApiError::conflict("分发授权已撤销或分块不存在"))?;
        let bytes: Vec<u8> = row.get(0);
        let limit: i64 = row.get(1);
        tx.commit().await?;
        (bytes, limit, device)
    };
    throttle(&app, bytes.2, bytes.0.len(), bytes.1).await?;
    Ok(([("Content-Type", "application/octet-stream")], bytes.0))
}

async fn download_slot<'a>(
    app: &'a App,
    headers: &HeaderMap,
) -> ApiResult<tokio::sync::SemaphorePermit<'a>> {
    if let Ok(slot) = app.download_slots.try_acquire() {
        return Ok(slot);
    }
    let busy = || ApiError::new(429, "下载并发繁忙，请续传重试");
    let _waiting = app.download_waiters.try_acquire().map_err(|_| busy())?;
    // The agent allows eight seconds per request. A 256 KiB block at the
    // minimum configured rate already uses four of those seconds.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        // Only authenticated devices join the FIFO. Release the database before
        // waiting. Both download routes recheck credentials and grants at read time.
        {
            let mut db = app.db.lock().await?;
            let tx = db.transaction().await?;
            auth::device(&tx, headers).await?;
            tx.commit().await?;
        }
        app.download_slots.acquire().await.map_err(|_| busy())
    })
    .await
    .map_err(|_| busy())?
}

async fn throttle(app: &App, device: String, size: usize, limit: i64) -> ApiResult<()> {
    let duration = std::time::Duration::from_secs_f64(size as f64 / limit as f64);
    {
        let mut limits = app.download_limits.lock().await;
        let now = std::time::Instant::now();
        limits.retain(|_, expires| *expires > now);
        if limits.contains_key(&device) {
            return Err(ApiError::new(429, "此设备达到下载限速，请续传重试"));
        }
        limits.insert(device, now + duration);
    }
    tokio::time::sleep(duration).await;
    Ok(())
}
pub async fn desktop_chunk(
    State(app): State<App>,
    headers: HeaderMap,
    Path((job, artifact, index)): Path<(String, String, i32)>,
) -> ApiResult<impl IntoResponse> {
    let _slot = download_slot(&app, &headers).await?;
    let (device, bytes) = {
        let mut db = app.db.connection().await?;
        let (tx, queries) = db.waiting_transaction().await?;
        let device = auth::device(&tx, &headers).await?;
        let denied = || ApiError::forbidden("开赛桌面下载授权已结束");
        let control = tx.query_opt("SELECT c.instance_id,c.session_key FROM control_devices c JOIN waiting_commands w ON w.device_id=c.device_id WHERE c.device_id=$1 AND c.connected AND c.server_epoch=$2 AND c.last_seen>$3::bigint-15 AND w.id=$4 AND w.stage IN ('accepted','executing')", &[&device,&app.server_epoch,&now(),&job]).await?.ok_or_else(denied)?;
        let current =
            crate::waiting::current(&tx, queries, &device, control.get(0), control.get(1))
                .await?
                .ok_or_else(denied)?;
        if current.command_id != job
            || !current
                .desktop_files
                .is_some_and(|files| files.materials.iter().any(|f| f.id == artifact))
        {
            return Err(denied());
        }
        let bytes: Vec<u8> = tx
            .query_opt(
                "SELECT content FROM distribution_chunks WHERE artifact_id=$1 AND chunk_index=$2",
                &[&artifact, &index],
            )
            .await?
            .ok_or_else(|| ApiError::bad("资料分块不存在"))?
            .get(0);
        tx.commit().await?;
        (device, bytes)
    };
    throttle(&app, device, bytes.len(), 4 * 1024 * 1024).await?;
    Ok(([("Content-Type", "application/octet-stream")], bytes))
}

pub async fn progress(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Progress>,
) -> ApiResult<Json<Value>> {
    if !input.valid() {
        return Err(ApiError::bad("分发结果无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    expire(&tx, &device).await?;
    let row=tx.query_opt("SELECT j.*,a.manifest FROM distribution_jobs j JOIN distribution_artifacts a ON a.id=j.artifact_id WHERE j.id=$1 AND j.device_id=$2", &[&input.job_id,&device]).await?.ok_or_else(||ApiError::forbidden("分发任务不属于本设备"))?;
    let manifest: Manifest =
        serde_json::from_value(row.get("manifest")).map_err(|_| ApiError::internal())?;
    let stage: String = row.get("stage");
    let late_publication = row.get::<_, String>("detail") == "publication_unconfirmed"
        && input.stage == "published"
        && tx
            .query_one(
                "SELECT enrollment_generation=$2 FROM devices WHERE id=$1",
                &[&device, &row.get::<_, i64>("generation")],
            )
            .await?
            .get::<_, bool>(0);
    if late_publication || !["published", "failed", "cancelled"].contains(&stage.as_str()) {
        if input.verified_chunks > manifest.hashes.len()
            || (["ready", "published"].contains(&input.stage.as_str())
                && input.verified_chunks != manifest.hashes.len())
            || (input.stage == "published" && !row.get::<_, bool>("publication_claimed"))
        {
            return Err(ApiError::bad("分发进度或发布时间不匹配"));
        }
        tx.execute("UPDATE distribution_jobs SET stage=$2,verified_chunks=$3,detail=$4,updated_at=$5 WHERE id=$1", &[&input.job_id,&input.stage,&(input.verified_chunks as i32),&input.detail,&now()]).await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"job_id":input.job_id,"accepted":true,"cancelled":stage=="cancelled"}),
    ))
}
