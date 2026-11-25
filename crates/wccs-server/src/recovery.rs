use crate::{ApiError, ApiResult, App, auth};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
};
use serde_json::{Value, json};
use tokio_postgres::Transaction;
use wccs_protocol::{
    bytes_digest,
    maintenance::Command,
    now,
    recovery::{Options, Snapshot},
};

pub(crate) struct Previous {
    instance: String,
    generation: i64,
    reference: Value,
}

impl Previous {
    pub(crate) fn reference(&self, instance: Option<&str>, generation: i64) -> ApiResult<&Value> {
        if Some(self.instance.as_str()) != instance || self.generation != generation {
            return Err(ApiError::conflict("设备代次或实例已变化，请重新检查"));
        }
        Ok(&self.reference)
    }
}

pub(crate) async fn references(
    tx: &Transaction<'_>,
    options: &Options,
    devices: &[String],
    actor: &auth::Identity,
) -> ApiResult<std::collections::HashMap<String, Previous>> {
    if !tx
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM contests WHERE id=$1)",
            &[&options.contest_id],
        )
        .await?
        .get::<_, bool>(0)
    {
        return Err(ApiError::bad("请选择已创建的比赛"));
    }
    let mut references = std::collections::HashMap::new();
    let Some(previous) = &options.previous_request else {
        return Ok(references);
    };
    let expected = match options.action.as_str() {
        "archive" => "inspect",
        "clean" => "archive",
        "reboot" => "clean",
        _ => return Err(ApiError::bad("恢复步骤无效")),
    };
    let rows = tx.query("SELECT j.device_id,j.id,j.recovery_template->'options' AS options,j.recovery_instance,j.recovery_generation,
        COALESCE(j.inspection_source,j.id) AS inspection_job,
        CASE WHEN $3='clean' THEN j.id ELSE j.archive_source END AS archive_job,
        j.recovery_files,j.recovery_directories,j.recovery_bytes::text,j.recovery_volume,j.recovery_collected,a.verified
        FROM maintenance_jobs j LEFT JOIN recovery_archives a ON a.job_id=CASE WHEN $3='clean' THEN j.id ELSE j.archive_source END
        WHERE j.request_id=$1 AND j.device_id=ANY($2) AND j.stage='succeeded' AND j.recovery_files IS NOT NULL AND ($4 OR (cardinality(j.room_ids)>0 AND j.room_ids<@$5))",
        &[previous,&devices,&options.action,&actor.all_rooms,&actor.room_ids]).await?;
    if rows.len() != devices.len() {
        return Err(ApiError::conflict(
            "前一步尚未成功或缺少清单，请核对逐机结果",
        ));
    }
    for row in rows {
        let old: Options =
            serde_json::from_value(row.get("options")).map_err(|_| ApiError::internal())?;
        if old.action != expected || !options.same_scope(&old) {
            return Err(ApiError::conflict("恢复范围或前一步已变化，请重新检查"));
        }
        let archive_job: Option<String> = row.get("archive_job");
        if archive_job.is_some() && row.get::<_, Option<bool>>("verified") != Some(true) {
            return Err(ApiError::conflict("归档或跳过回执尚未由服务器核验"));
        }
        let reference = json!({"job_id":row.get::<_,String>("id"),"inspection_job":row.get::<_,String>("inspection_job"),
            "archive_job":archive_job,"volume":row.try_get::<_,Value>("recovery_volume").map_err(|_|ApiError::conflict("前一步缺少卷身份"))?,"collected_count":row.get::<_,i32>("recovery_collected"),"file_count":row.get::<_,i32>("recovery_files"),"directory_count":row.get::<_,i32>("recovery_directories"),"total_bytes":row.get::<_,String>("recovery_bytes")});
        references.insert(
            row.get("device_id"),
            Previous {
                instance: row
                    .try_get("recovery_instance")
                    .map_err(|_| ApiError::conflict("前一步缺少设备实例"))?,
                generation: row
                    .try_get("recovery_generation")
                    .map_err(|_| ApiError::conflict("前一步缺少设备代次"))?,
                reference,
            },
        );
    }
    Ok(references)
}

// Load one immutable predecessor off the heartbeat connection. Delivery rechecks the lease afterwards.
pub(crate) async fn prepare_delivery(app: &App, job: &str) -> ApiResult<()> {
    let mut db = app.archive_db.lock().await?;
    let tx = db.transaction().await?;
    let Some(row) = tx
        .query_opt(
            "SELECT j.command,j.recovery_reference,p.result->'recovery'
        FROM maintenance_jobs j JOIN maintenance_jobs p ON p.id=j.recovery_reference->>'job_id'
        WHERE j.id=$1 AND j.command_digest='' AND NOT j.cancel_requested AND p.stage='succeeded'
        FOR UPDATE OF j",
            &[&job],
        )
        .await?
    else {
        return Ok(());
    };
    let mut command: Value = row.get(0);
    let reference: Value = row.get(1);
    command["recovery"] = json!({"inspection_job":reference["inspection_job"],"archive_job":reference["archive_job"],"manifest":row.get::<_,Value>(2)});
    let typed: Command = serde_json::from_value(command).map_err(|_| ApiError::internal())?;
    let value = json!(typed);
    tx.execute(
        "UPDATE maintenance_jobs SET delivery_command=$2,command_digest=$3 WHERE id=$1",
        &[&job, &value, &wccs_protocol::digest(&value.to_string())],
    )
    .await?;
    if value["template"]["options"]["action"] == "archive" {
        tx.execute(
            "INSERT INTO recovery_archives(job_id,snapshot,options) VALUES($1,$2,$3)",
            &[&job, &value["recovery"], &value["template"]["options"]],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn active(tx: &Transaction<'_>, headers: &HeaderMap, job: &str) -> ApiResult<()> {
    let device = auth::device(tx, headers).await?;
    tx.query_opt("SELECT 1 FROM recovery_archives a JOIN maintenance_jobs j ON j.id=a.job_id JOIN devices d ON d.id=j.device_id JOIN operators o ON o.id=j.actor_id JOIN control_devices c ON c.device_id=d.id WHERE a.job_id=$1 AND d.id=$2 AND d.lifecycle='enabled' AND (j.command->>'generation')::bigint=d.enrollment_generation AND (j.command->>'device_version')::bigint=d.lifecycle_version AND (j.command->>'address_version')::bigint=d.address_version AND j.stage IN ('accepted','executing') AND NOT j.cancel_requested AND j.expires_at>$3 AND o.enabled AND o.version=j.actor_version AND o.role<>'viewer' AND (o.all_rooms OR (cardinality(j.room_ids)>0 AND j.room_ids<@o.room_ids AND device_room_scope(d.id)<@o.room_ids)) AND j.grant_connection=c.connection_id AND j.grant_epoch=c.server_epoch AND c.connected AND c.last_seen>$3-15 AND c.instance_id=j.command->>'instance_id' AND c.session_key=j.command->>'session_key'", &[&job,&device,&now()]).await?
        .ok_or_else(||ApiError::conflict("归档任务授权已结束，请重新检查并重试"))?;
    Ok(())
}
pub async fn upload(
    State(app): State<App>,
    headers: HeaderMap,
    Path((job, file, index)): Path<(String, usize, usize)>,
    bytes: Bytes,
) -> ApiResult<Json<Value>> {
    let mut db = app.archive_upload_db.lock().await?;
    let tx = db.transaction().await?;
    active(&tx, &headers, &job).await?;
    let file = i32::try_from(file).map_err(|_| ApiError::bad("归档文件索引无效"))?;
    let row = tx.query_one("SELECT (snapshot#>'{manifest,files}')->$2::integer,options FROM recovery_archives WHERE job_id=$1", &[&job,&file]).await?;
    let entry: wccs_protocol::recovery::File = serde_json::from_value(
        row.get::<_, Option<Value>>(0)
            .ok_or_else(|| ApiError::bad("归档文件索引无效"))?,
    )
    .map_err(|_| ApiError::internal())?;
    let options: Options = serde_json::from_value(row.get(1)).map_err(|_| ApiError::internal())?;
    if !options.collects(&entry.path)
        || entry.chunk_length(index) != Some(bytes.len())
        || entry.hashes[index] != bytes_digest(&bytes)
    {
        return Err(ApiError::bad("归档分块长度或摘要不匹配"));
    }
    let index = index as i32;
    tx.execute("INSERT INTO recovery_chunks VALUES($1,$2,$3,$4) ON CONFLICT(job_id,file_index,chunk_index) DO NOTHING", &[&job,&file,&index,&&bytes[..]]).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"accepted":true,"file_index":file,"chunk_index":index}),
    ))
}
pub async fn finish(
    State(app): State<App>,
    headers: HeaderMap,
    Path(job): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut db = app.archive_db.lock().await?;
    let tx = db.transaction().await?;
    active(&tx, &headers, &job).await?;
    if tx
        .query_one(
            "SELECT verified FROM recovery_archives WHERE job_id=$1",
            &[&job],
        )
        .await?
        .get::<_, bool>(0)
    {
        return Ok(Json(json!({"verified":true,"job_id":job})));
    }
    let row = tx
        .query_one(
            "SELECT snapshot,options FROM recovery_archives WHERE job_id=$1",
            &[&job],
        )
        .await?;
    let snapshot: Snapshot =
        serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    let options: Options = serde_json::from_value(row.get(1)).map_err(|_| ApiError::internal())?;
    use futures_util::TryStreamExt;
    use sha2::{Digest, Sha256};
    let stream = tx.query_raw("SELECT file_index,chunk_index,content FROM recovery_chunks WHERE job_id=$1 ORDER BY file_index,chunk_index", [&job]).await?;
    tokio::pin!(stream);
    for (file_index, file) in snapshot
        .manifest
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| options.collects(&f.path))
    {
        let mut digest = Sha256::new();
        for (index, hash) in file.hashes.iter().enumerate() {
            let row = stream
                .try_next()
                .await?
                .ok_or_else(|| ApiError::conflict("归档分块未齐全"))?;
            let bytes: &[u8] = row.get(2);
            if row.get::<_, i32>(0) != file_index as i32
                || row.get::<_, i32>(1) != index as i32
                || file.chunk_length(index) != Some(bytes.len())
                || bytes_digest(bytes) != *hash
            {
                return Err(ApiError::conflict("归档分块核验失败"));
            }
            digest.update(bytes);
        }
        if format!("{:x}", digest.finalize()) != file.sha256 {
            return Err(ApiError::conflict("归档文件摘要核验失败"));
        }
    }
    if stream.try_next().await?.is_some() {
        return Err(ApiError::conflict("归档含额外分块"));
    }
    tx.commit().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    active(&tx, &headers, &job).await?;
    tx.execute(
        "UPDATE recovery_archives SET verified=true WHERE job_id=$1",
        &[&job],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"verified":true,"job_id":job})))
}
pub async fn download(
    State(app): State<App>,
    headers: HeaderMap,
    Path(job): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let session = auth::admin(&app, &headers, false).await?;
    let slot = app
        .archive_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(429, "归档下载繁忙，请稍后重试"))?;
    let database = crate::database::Database::new(&app.config.database_url)?;
    {
        let mut db = database.lock().await?;
        let tx = db.transaction().await?;
        let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;
        let row=tx.query_opt("SELECT j.room_ids FROM recovery_archives a JOIN maintenance_jobs j ON j.id=a.job_id WHERE a.job_id=$1 AND a.verified",&[&job]).await?.ok_or_else(||ApiError::new(404,"归档尚未核验"))?;
        actor.rooms(row.get::<_, Option<Vec<String>>>(0).as_deref())?;
    }
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let archive_id = job.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = slot;
        if write_archive(database, &archive_id, sender.clone()).is_err() {
            let _ = sender.blocking_send(Err(std::io::Error::other("归档传输未完成")));
        }
    });
    let stream = futures_util::stream::unfold(receiver, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    });
    let bytes = axum::body::Body::from_stream(stream);
    Ok((
        [
            ("content-type", "application/zip".to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"wccs-archive-{job}.zip\""),
            ),
        ],
        bytes,
    ))
}

type ArchiveSender = tokio::sync::mpsc::Sender<std::io::Result<Bytes>>;

struct ArchiveOutput(ArchiveSender);
impl std::io::Write for ArchiveOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let size = bytes.len().min(64 * 1024);
        self.0
            .blocking_send(Ok(Bytes::copy_from_slice(&bytes[..size])))
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        Ok(size)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn write_archive(
    database: crate::database::Database,
    job: &str,
    sender: ArchiveSender,
) -> ApiResult<()> {
    use std::io::{BufWriter, Write};
    let runtime = tokio::runtime::Handle::current();
    let mut db = runtime.block_on(database.lock())?;
    let tx = runtime.block_on(db.transaction())?;
    let row = runtime.block_on(tx.query_one(
        "SELECT snapshot,options FROM recovery_archives WHERE job_id=$1 AND verified",
        &[&job],
    ))?;
    let snapshot: Snapshot =
        serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    let policy: Options = serde_json::from_value(row.get(1)).map_err(|_| ApiError::internal())?;
    let file_options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut sink = BufWriter::with_capacity(64 * 1024, ArchiveOutput(sender));
    let mut zip = zip::ZipWriter::new_stream(&mut sink);
    zip.start_file("wccs-manifest.json", file_options)
        .map_err(|_| ApiError::internal())?;
    #[derive(serde::Serialize)]
    struct ArchiveManifest<'a> {
        snapshot: &'a Snapshot,
        policy: &'a Options,
    }
    serde_json::to_writer(
        &mut zip,
        &ArchiveManifest {
            snapshot: &snapshot,
            policy: &policy,
        },
    )
    .map_err(|_| ApiError::internal())?;
    let statement = runtime.block_on(tx.prepare("SELECT file_index,chunk_index,content FROM recovery_chunks WHERE job_id=$1 ORDER BY file_index,chunk_index"))?;
    let portal = runtime.block_on(tx.bind(&statement, &[&job]))?;
    let mut rows = Vec::new().into_iter();
    for (index, file) in snapshot
        .manifest
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| policy.collects(&f.path))
    {
        zip.start_file(format!("files/{}", file.path), file_options)
            .map_err(|_| ApiError::internal())?;
        for chunk in 0..file.hashes.len() {
            if rows.len() == 0 {
                rows = runtime.block_on(tx.query_portal(&portal, 4))?.into_iter();
            }
            let row = rows.next().ok_or_else(ApiError::internal)?;
            let bytes: &[u8] = row.get(2);
            if row.get::<_, i32>(0) != index as i32
                || row.get::<_, i32>(1) != chunk as i32
                || file.chunk_length(chunk) != Some(bytes.len())
            {
                return Err(ApiError::internal());
            }
            zip.write_all(bytes).map_err(|_| ApiError::internal())?;
        }
    }
    zip.finish().map_err(|_| ApiError::internal())?;
    sink.flush().map_err(|_| ApiError::internal())?;
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct Filter {
    contest_id: String,
    #[serde(default)]
    offset: i32,
    request_id: Option<String>,
}

pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;
    let rows = tx.query("SELECT j.id,j.request_id,j.device_id,d.name,j.recovery_template AS template,j.stage,
        j.recovery_summary AS result,j.expires_at,j.reboot_requested_at,j.reboot_observed_at,a.verified,
        j.recovery_files AS file_count, j.recovery_bytes::text AS recovery_bytes,
        j.recovery_directories AS directory_count
        FROM maintenance_jobs j JOIN devices d ON d.id=j.device_id LEFT JOIN recovery_archives a ON a.job_id=j.id
        WHERE j.recovery_contest=$3 AND ($4::text IS NULL OR j.request_id=$4)
        AND ($1 OR (cardinality(j.room_ids)>0 AND j.room_ids<@$2)) ORDER BY j.created_at DESC,j.id LIMIT CASE WHEN $4::text IS NULL THEN 500 ELSE NULL END",
&[&actor.all_rooms,&actor.room_ids,&filter.contest_id,&filter.request_id]).await?;
    let jobs:Vec<_> = rows.iter().map(|r| json!({"job_id":r.get::<_,String>("id"),"request_id":r.get::<_,String>("request_id"),
        "device_id":r.get::<_,String>("device_id"),"device_name":r.get::<_,String>("name"),"stage":r.get::<_,String>("stage"),
        "template":r.get::<_,Value>("template"),"result":r.get::<_,Option<Value>>("result"),"expired":r.get::<_,i64>("expires_at")<=now(),
        "archive_verified":r.get::<_,Option<bool>>("verified").unwrap_or(false),"reboot_observed":r.get::<_,Option<i64>>("reboot_observed_at").is_some(),
        "reboot_requested_at":r.get::<_,Option<i64>>("reboot_requested_at"),"reboot_observed_at":r.get::<_,Option<i64>>("reboot_observed_at"),
        "total_bytes":r.get::<_,Option<String>>("recovery_bytes"),"file_count":r.get::<_,Option<i32>>("file_count"),"directory_count":r.get::<_,Option<i32>>("directory_count")})).collect();
    Ok(Json(json!({"jobs":jobs})))
}

pub async fn manifest(
    State(app): State<App>,
    headers: HeaderMap,
    Path(job): Path<String>,
    Query(filter): Query<Filter>,
) -> ApiResult<Json<Value>> {
    if filter.offset < 0 {
        return Err(ApiError::bad("分页位置无效"));
    }
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;
    let row = tx.query_opt("SELECT j.result#>'{recovery,volume}', j.result#>>'{recovery,target}',
        COALESCE((SELECT jsonb_agg(jsonb_build_object('path',f->>'path','size',f->'size')) FROM
            (SELECT f FROM jsonb_array_elements(j.result#>'{recovery,files}') f LIMIT 100 OFFSET $4::integer) page),'[]'::jsonb),
        COALESCE((SELECT jsonb_agg(d) FROM (SELECT d FROM jsonb_array_elements(j.result#>'{recovery,directories}') d LIMIT 100 OFFSET $4::integer) page),'[]'::jsonb)
        FROM maintenance_jobs j WHERE j.id=$1 AND j.command#>>'{template,options,contest_id}'=$2 AND j.result ? 'recovery'
        AND ($3 OR (cardinality(j.room_ids)>0 AND j.room_ids<@$5))",
        &[&job,&filter.contest_id,&actor.all_rooms,&filter.offset,&actor.room_ids]).await?
        .ok_or_else(||ApiError::new(404,"清单不存在"))?;
    Ok(Json(
        json!({"volume":row.get::<_,Value>(0),"target":row.get::<_,String>(1),"files":row.get::<_,Value>(2),"directories":row.get::<_,Value>(3)}),
    ))
}
