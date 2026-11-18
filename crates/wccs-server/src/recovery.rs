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

pub(crate) async fn plan(
    tx: &Transaction<'_>,
    options: &Options,
    device: &str,
    instance: &str,
    generation: i64,
) -> ApiResult<Option<Snapshot>> {
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
    let Some(previous) = &options.previous_request else {
        return Ok(None);
    };
    let row=tx.query_opt("SELECT command,result FROM maintenance_jobs WHERE request_id=$1 AND device_id=$2 AND stage='succeeded'", &[previous,&device]).await?
        .ok_or_else(||ApiError::conflict("前一步尚未成功，请核对逐机结果"))?;
    let command: Command = serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    let wccs_protocol::maintenance::Template::Recovery { options: old } = &command.template else {
        return Err(ApiError::bad("引用任务并非赛后恢复任务"));
    };
    let expected = match options.action.as_str() {
        "archive" => "inspect",
        "clean" => "archive",
        "reboot" => "clean",
        _ => return Err(ApiError::bad("恢复步骤无效")),
    };
    if old.action != expected
        || !options.same_scope(old)
        || command.instance_id != instance
        || command.generation != generation
    {
        return Err(ApiError::conflict(
            "恢复范围、设备实例或前一步已变化，请重新检查",
        ));
    }
    let result: wccs_protocol::maintenance::Result =
        serde_json::from_value(row.get(1)).map_err(|_| ApiError::internal())?;
    let manifest = result
        .recovery
        .ok_or_else(|| ApiError::conflict("前一步缺少文件清单"))?;
    if !manifest.valid_for(options)
        || manifest.target != options.target
        || (options.scope == "volume" && manifest.volume.system)
    {
        return Err(ApiError::conflict("清理范围核验失败"));
    }
    let archive_job = if options.action == "clean" {
        Some(command.job_id.clone())
    } else {
        command
            .recovery
            .as_ref()
            .and_then(|s| s.archive_job.clone())
    };
    if let Some(id) = &archive_job
        && !tx
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM recovery_archives WHERE job_id=$1 AND verified)",
                &[id],
            )
            .await?
            .get::<_, bool>(0)
    {
        return Err(ApiError::conflict("归档或跳过回执尚未由服务器核验"));
    }
    Ok(Some(Snapshot {
        inspection_job: command
            .recovery
            .map(|s| s.inspection_job)
            .unwrap_or(command.job_id),
        archive_job,
        manifest,
    }))
}

async fn active(tx: &Transaction<'_>, headers: &HeaderMap, job: &str) -> ApiResult<()> {
    let device = auth::device(tx, headers).await?;
    tx.query_opt("SELECT 1 FROM recovery_archives a JOIN maintenance_jobs j ON j.id=a.job_id JOIN devices d ON d.id=j.device_id JOIN operators o ON o.id=j.actor_id JOIN control_devices c ON c.device_id=d.id WHERE a.job_id=$1 AND d.id=$2 AND d.lifecycle='enabled' AND (j.command->>'generation')::bigint=d.enrollment_generation AND (j.command->>'device_version')::bigint=d.lifecycle_version AND (j.command->>'address_version')::bigint=d.address_version AND j.stage IN ('accepted','executing') AND NOT j.cancel_requested AND j.expires_at>$3 AND o.enabled AND o.version=j.actor_version AND o.role<>'viewer' AND (o.all_rooms OR (cardinality(j.room_ids)>0 AND j.room_ids<@o.room_ids AND device_room_scope(d.id)<@o.room_ids)) AND c.connected AND c.last_seen>$3-15 AND c.instance_id=j.command->>'instance_id' AND c.session_key=j.command->>'session_key'", &[&job,&device,&now()]).await?
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
    let _slot = app
        .archive_slots
        .try_acquire()
        .map_err(|_| ApiError::new(429, "归档下载繁忙，请稍后重试"))?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;
    let row=tx.query_opt("SELECT a.snapshot,a.options,j.room_ids FROM recovery_archives a JOIN maintenance_jobs j ON j.id=a.job_id WHERE a.job_id=$1 AND a.verified",&[&job]).await?.ok_or_else(||ApiError::new(404,"归档尚未核验"))?;
    actor.rooms(row.get::<_, Option<Vec<String>>>(2).as_deref())?;
    let snapshot: Snapshot =
        serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    let options: Options = serde_json::from_value(row.get(1)).map_err(|_| ApiError::internal())?;
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let file_options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("wccs-manifest.json", file_options)
        .map_err(|_| ApiError::internal())?;
    zip.write_all(
        json!({"snapshot":snapshot,"policy":options})
            .to_string()
            .as_bytes(),
    )
    .map_err(|_| ApiError::internal())?;
    for (index, file) in snapshot
        .manifest
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| options.collects(&f.path))
    {
        zip.start_file(format!("files/{}", file.path), file_options)
            .map_err(|_| ApiError::internal())?;
        let rows=tx.query("SELECT content FROM recovery_chunks WHERE job_id=$1 AND file_index=$2 ORDER BY chunk_index",&[&job,&(index as i32)]).await?;
        for row in rows {
            zip.write_all(&row.get::<_, Vec<u8>>(0))
                .map_err(|_| ApiError::internal())?;
        }
    }
    let bytes = zip.finish().map_err(|_| ApiError::internal())?.into_inner();
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
