use crate::{ApiError, ApiResult, App, auth, management};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::Transaction;
use wccs_protocol::{
    control::WaitingIdentity,
    digest,
    maintenance::{Command, Poll, Reply, Result as TaskResult, Template},
    now, token, valid_id,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    template_version: u32,
    targets: Vec<String>,
    template: Template,
    #[serde(default)]
    preview_token: Option<String>,
}

async fn plan(tx: &Transaction<'_>, input: &Write) -> ApiResult<Value> {
    let unique: std::collections::HashSet<_> = input.targets.iter().collect();
    if !valid_id(&input.request_id)
        || input.template_version != 1
        || !input.template.valid()
        || input.targets.is_empty()
        || input.targets.len() > 500
        || unique.len() != input.targets.len()
        || input.targets.iter().any(|id| !valid_id(id))
    {
        return Err(ApiError::bad("维护模板版本、参数或目标无效"));
    }
    let rows = tx.query("SELECT d.id,d.name,d.lifecycle,d.lifecycle_version,d.enrollment_generation,d.address_version,d.observed_ip,
        m.instance_id,m.session_key,m.adapter,m.capabilities,m.last_seen,
        c.connected,c.last_seen AS control_seen,c.instance_id AS control_instance,c.actual,
        EXISTS(SELECT 1 FROM waiting_commands w WHERE w.device_id=d.id AND w.stage NOT IN ('failed','cancelled','expired') AND w.expires_at>$2 AND (w.command->>'action'='enter' OR (w.command->>'action'='prepare_login' AND w.stage<>'succeeded'))) AS waiting,
        EXISTS(SELECT 1 FROM maintenance_jobs j WHERE j.device_id=d.id AND j.stage IN ('queued','accepted','executing') AND j.expires_at>$2) AS busy
        FROM devices d LEFT JOIN maintenance_devices m ON m.device_id=d.id LEFT JOIN control_devices c ON c.device_id=d.id
        WHERE d.id=ANY($1)", &[&input.targets,&now()]).await?;
    if rows.len() != input.targets.len() {
        return Err(ApiError::bad("目标设备不存在"));
    }
    let preparation = match input.template.profile_version() {
        Some(version) => Some(crate::preparation::resolve(tx, version).await?),
        None => None,
    };
    let seating = crate::seating::snapshot(tx).await?;
    let mut targets = Vec::new();
    for id in &input.targets {
        let row = rows
            .iter()
            .find(|r| r.get::<_, String>("id") == *id)
            .unwrap();
        let instance: Option<String> = row.get("instance_id");
        let session: Option<String> = row.get("session_key");
        let capabilities: Option<Value> = row.get("capabilities");
        let recovery = if let Template::Recovery { options } = &input.template {
            crate::recovery::plan(
                tx,
                options,
                id,
                instance.as_deref().unwrap_or(""),
                row.get("enrollment_generation"),
            )
            .await?
        } else {
            None
        };
        let reason = if matches!(&input.template,Template::Recovery {options} if ["clean","reboot"].contains(&options.action.as_str()))
            && row.get::<_, Option<Value>>("actual").is_none_or(|a| {
                a["account"]["phase"] != "teaching" || a["account"]["policy"]["state"] != "ready"
            }) {
            Some("restore_teaching_required")
        } else if row.get::<_, String>("lifecycle") != "enabled" {
            Some("device_disabled")
        } else if !row
            .get::<_, Option<i64>>("last_seen")
            .is_some_and(|t| now() - t < 15)
            || row.get::<_, Option<bool>>("connected") != Some(true)
            || !row
                .get::<_, Option<i64>>("control_seen")
                .is_some_and(|t| now() - t < 15)
            || instance != row.get::<_, Option<String>>("control_instance")
        {
            Some("offline")
        } else if session.is_none() {
            Some("no_session")
        } else if capabilities.as_ref().is_none_or(|v| {
            !v.as_array().is_some_and(|caps| {
                caps.iter().any(|c| c == input.template.capability())
                    && (preparation
                        .as_ref()
                        .is_none_or(|p| p.profile.competition.is_none())
                        || caps.iter().any(|c| c == "competition-v1"))
            })
        }) {
            Some("unsupported_client")
        } else if preparation.is_some()
            && row
                .get::<_, Option<Value>>("actual")
                .as_ref()
                .is_none_or(|a| {
                    a["account"]["adapter"] != "windows_test"
                        || a["account"]["target_sid"].is_null()
                        || a["account"]["phase"] != "controlled"
                        || a["account"]["account"] != "owned"
                        || a["account"]["desktop_confirmed"] != true
                        || a["account"]["target_sid"] != a["account"]["observed_sid"]
                })
        {
            Some("ordinary_account_unavailable")
        } else if row.get::<_, bool>("waiting") {
            Some("waiting_active")
        } else if row.get::<_, bool>("busy") {
            Some("maintenance_active")
        } else {
            None
        };
        let mut identity = WaitingIdentity {
            contest_id: "identification".into(),
            contest_name: "工作人员设备识别".into(),
            contest_version: 0,
            participant_id: String::new(),
            name: id.clone(),
            school: row
                .get::<_, Option<String>>("observed_ip")
                .unwrap_or_else(|| "IP 未知".into()),
            seat_id: String::new(),
            seat_label: "未绑定".into(),
            room_id: String::new(),
            room_version: 0,
        };
        for room in seating["rooms"].as_array().into_iter().flatten() {
            for seat in room["seats"].as_array().into_iter().flatten() {
                if seat["devices"]
                    .as_array()
                    .is_some_and(|ds| ds.iter().any(|d| d == id))
                {
                    identity.seat_id = seat["id"].as_str().unwrap().into();
                    identity.room_id = room["id"].as_str().unwrap().into();
                    identity.room_version = room["version"].as_i64().unwrap();
                    identity.seat_label = format!(
                        "{} / {}{}",
                        room["name"].as_str().unwrap(),
                        seat["number"],
                        if seat["status"] == "conflict" {
                            " 待核对"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
        targets.push(json!({"recovery":recovery,"device_id":id,"name":row.get::<_,String>("name"),"reason":reason,
            "generation":row.get::<_,i64>("enrollment_generation"),"device_version":row.get::<_,i64>("lifecycle_version"),
            "address_version":row.get::<_,i64>("address_version"),"instance_id":instance,"session_key":session,
            "adapter":row.get::<_,Option<String>>("adapter"),"identity":identity}));
    }
    let mut result = json!({"template_version":1,"template":input.template,"preparation":preparation,"targets":targets});
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
    let identity = auth::authorize_snapshot(&tx, &session, auth::Permission::Operate).await?;
    identity
        .devices(&tx, &input.targets, &input.request_id)
        .await?;
    Ok(Json(plan(&tx, &input).await?))
}

pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Json(mut input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let preview = input.preview_token.take();
    let payload = serde_json::to_value(&input).map_err(|_| ApiError::internal())?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let identity = auth::authorize(&tx, &session, auth::Permission::Operate).await?;
    identity
        .devices(&tx, &input.targets, &input.request_id)
        .await?;
    if let Some(receipt) =
        management::previous(&tx, &input.request_id, "maintenance_commit", &payload).await?
    {
        return Ok(Json(receipt));
    }
    let plan = plan(&tx, &input).await?;
    if preview.as_deref() != plan["preview_token"].as_str() {
        return Err(ApiError::conflict("设备或维护配置已变化，请重新预览"));
    }
    let mut jobs = Vec::new();
    for target in plan["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["reason"].is_null())
    {
        let command = Command {
            recovery: serde_json::from_value(target["recovery"].clone())
                .map_err(|_| ApiError::internal())?,
            preparation: serde_json::from_value(plan["preparation"].clone())
                .map_err(|_| ApiError::internal())?,
            schema_version: 1,
            job_id: token(),
            request_id: input.request_id.clone(),
            device_id: target["device_id"].as_str().unwrap().into(),
            generation: target["generation"].as_i64().unwrap(),
            device_version: target["device_version"].as_i64().unwrap(),
            address_version: target["address_version"].as_i64().unwrap(),
            session_key: target["session_key"].as_str().unwrap().into(),
            instance_id: target["instance_id"].as_str().unwrap().into(),
            identity: serde_json::from_value(target["identity"].clone())
                .map_err(|_| ApiError::internal())?,
            expires_at: now()
                + if matches!(input.template, Template::Recovery { .. }) {
                    900
                } else {
                    180
                },
            template: input.template.clone(),
        };
        let value = serde_json::to_value(&command).unwrap();
        tx.execute("INSERT INTO maintenance_jobs(id,request_id,device_id,command,command_digest,actor_id,actor_version,room_ids,created_at,expires_at,updated_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,device_room_scope($3),$8,$9,$8)",
            &[&command.job_id,&command.request_id,&command.device_id,&value,&digest(&value.to_string()),&identity.id,&identity.version,&now(),&command.expires_at]).await?;
        if let Template::Recovery { options } = &command.template
            && options.action == "archive"
        {
            tx.execute(
                "INSERT INTO recovery_archives(job_id,snapshot,options) VALUES($1,$2,$3)",
                &[&command.job_id, &json!(command.recovery), &json!(options)],
            )
            .await?;
        }
        jobs.push(json!({"job_id":command.job_id,"device_id":command.device_id}));
    }
    let receipt = json!({"request_id":input.request_id,"jobs":jobs,"preview":plan});
    management::record(
        &tx,
        &input.request_id,
        "maintenance_commit",
        &payload,
        &receipt,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(receipt))
}

pub async fn list(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let identity = auth::authorize(&tx, &session, auth::Permission::Read).await?;
    let rows=tx.query("SELECT id,request_id,device_id,command,stage,result,updated_at,expires_at,cancel_requested FROM maintenance_jobs
        WHERE $1 OR (cardinality(room_ids)>0 AND room_ids<@$2) ORDER BY created_at DESC,id DESC LIMIT 500",&[&identity.all_rooms,&identity.room_ids]).await?;
    let jobs:Vec<_>=rows.iter().map(|r|json!({"job_id":r.get::<_,String>(0),"request_id":r.get::<_,String>(1),"device_id":r.get::<_,String>(2),"command":json!({"template":r.get::<_,Value>(3)["template"]}),
        "stage":r.get::<_,String>(4),"result":r.get::<_,Option<Value>>(5),"updated_at":r.get::<_,i64>(6),"expired":r.get::<_,i64>(7)<=now(),"cancel_requested":r.get::<_,bool>(8)})).collect();
    Ok(Json(json!({"jobs":jobs,"server_time":now()})))
}

pub async fn poll(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Poll>,
) -> ApiResult<Json<Reply>> {
    if input.schema_version != 1
        || !valid_id(&input.instance_id)
        || input.session_key.as_ref().is_some_and(|s| !valid_id(s))
        || !["service", "local_test"].contains(&input.adapter.as_str())
        || input.capabilities.is_empty()
        || input.capabilities.len() > 4
        || input.capabilities.iter().any(|c| {
            ![
                "identify-v1",
                "preparation-v1",
                "competition-v1",
                "recovery-v1",
            ]
            .contains(&c.as_str())
        })
    {
        return Err(ApiError::bad("维护能力或会话格式无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    tx.execute("INSERT INTO maintenance_devices VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(device_id) DO UPDATE SET
        instance_id=$2,session_key=$3,adapter=$4,capabilities=$5,last_seen=$6",
        &[&device,&input.instance_id,&input.session_key,&input.adapter,&json!(input.capabilities),&now()]).await?;
    // Recheck credentials, actor authority, connection and identity on every active lease.
    let row=tx.query_opt("SELECT j.command FROM maintenance_jobs j JOIN devices d ON d.id=j.device_id
        JOIN operators o ON o.id=j.actor_id JOIN control_devices c ON c.device_id=d.id
        WHERE d.id=$1 AND d.lifecycle='enabled' AND j.stage IN ('queued','accepted','executing') AND j.expires_at>$2 AND NOT j.cancel_requested
        AND o.enabled AND o.version=j.actor_version AND o.role<>'viewer'
        AND (o.all_rooms OR (cardinality(j.room_ids)>0 AND j.room_ids<@o.room_ids AND device_room_scope(d.id)<@o.room_ids))
        AND c.connected AND c.last_seen>$2-15 AND c.instance_id=$3 AND c.session_key=$4
        AND j.command->>'instance_id'=$3 AND j.command->>'session_key'=$4
        AND (j.command->>'generation')::bigint=d.enrollment_generation
        AND (j.command->>'device_version')::bigint=d.lifecycle_version
        AND (j.command->>'address_version')::bigint=d.address_version
        AND (j.command->'preparation' IS NULL OR (c.actual->'account'->>'adapter'='windows_test' AND c.actual->'account'->>'phase'='controlled' AND c.actual->'account'->>'account'='owned' AND c.actual->'account'->>'desktop_confirmed'='true' AND c.actual->'account'->>'target_sid'=c.actual->'account'->>'observed_sid'))
        AND (j.command->'preparation' IS NULL OR ((j.command->'preparation'->>'version')::bigint=(SELECT max(version) FROM preparation_profiles) AND (j.command->'preparation'->'profile'->>'tool_version')::bigint=(SELECT version FROM configuration_head WHERE id=1)))
        AND (j.command->'identity'->>'room_id'='' OR EXISTS(SELECT 1 FROM rooms r JOIN seats s ON s.room_id=r.id WHERE r.id=j.command->'identity'->>'room_id' AND r.version=(j.command->'identity'->>'room_version')::bigint AND s.id=j.command->'identity'->>'seat_id' AND s.expected_ip=d.observed_ip))
        ORDER BY j.created_at,j.id LIMIT 1",&[&device,&now(),&input.instance_id,&input.session_key]).await?;
    let command = row
        .map(|r| serde_json::from_value(r.get(0)))
        .transpose()
        .map_err(|_| ApiError::internal())?;
    tx.commit().await?;
    Ok(Json(Reply {
        schema_version: 1,
        server_time: now(),
        command,
    }))
}

pub async fn result(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<TaskResult>,
) -> ApiResult<Json<Value>> {
    if !input.valid() {
        return Err(ApiError::bad("维护结果格式无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    let row=tx.query_opt("SELECT j.result,j.command_digest,j.stage,j.request_id,j.room_ids,j.command FROM maintenance_jobs j JOIN devices d ON d.id=j.device_id WHERE j.id=$1 AND j.device_id=$2 AND (j.command->>'generation')::bigint=d.enrollment_generation",&[&input.job_id,&device]).await?
        .ok_or_else(||ApiError::forbidden("维护任务不属于此设备"))?;
    if row.get::<_, String>(1) != input.command_digest {
        return Err(ApiError::conflict("维护内容摘要不匹配"));
    }
    let command: Command = serde_json::from_value(row.get(5)).map_err(|_| ApiError::internal())?;
    if let Template::Recovery { options } = &command.template {
        if input.stage == "succeeded" {
            let manifest = input
                .recovery
                .as_ref()
                .ok_or_else(|| ApiError::bad("恢复结果缺少范围清单"))?;
            if manifest.target != options.target
                || (options.scope == "volume" && manifest.volume.system)
                || command
                    .recovery
                    .as_ref()
                    .is_some_and(|snapshot| &snapshot.manifest != manifest)
            {
                return Err(ApiError::conflict("恢复结果与批准范围不同"));
            }
            if options.action=="archive" && !tx.query_one("SELECT EXISTS(SELECT 1 FROM recovery_archives WHERE job_id=$1 AND verified)",&[&input.job_id]).await?.get::<_,bool>(0) {
                return Err(ApiError::conflict("归档尚未核验"));
            }
        }
    } else if input.recovery.is_some() {
        return Err(ApiError::bad("模板不接受恢复清单"));
    }
    let previous: Option<Value> = row.get(0);
    let stage: String = row.get(2);
    let value = json!(input);
    if previous.as_ref() != Some(&value) {
        if ["succeeded", "failed", "cancelled", "interrupted"].contains(&stage.as_str())
            || stage == input.stage
            || (stage == "executing" && input.stage == "accepted")
        {
            return Err(ApiError::conflict("维护任务已有后续或终态回执"));
        }
        tx.execute(
            "UPDATE maintenance_jobs SET result=$2,stage=$3,updated_at=$4 WHERE id=$1",
            &[&input.job_id, &value, &input.stage, &now()],
        )
        .await?;
        tx.execute("INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,room_ids,request_id,outcome,detail)
            VALUES('maintenance',$1,'task_result',$2,'maintenance_result',$3,$4,$5,$6,$7,$8)",
            &[&format!("{}-{}",input.job_id,input.stage),&now(),&input.job_id,&vec![device.clone()],&row.get::<_,Option<Vec<String>>>(4),&row.get::<_,String>(3),&input.stage,&value]).await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"accepted":true,"job_id":input.job_id,"stage":input.stage}),
    ))
}

#[derive(Deserialize, Serialize)]
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
        return Err(ApiError::bad("取消请求编号无效"));
    }
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Operate).await?;
    let row = tx
        .query_opt(
            "SELECT device_id,room_ids FROM maintenance_jobs WHERE id=$1",
            &[&input.job_id],
        )
        .await?
        .ok_or_else(|| ApiError::bad("任务不存在"))?;
    actor.rooms(row.get::<_, Option<Vec<String>>>(1).as_deref())?;
    actor.devices(&tx, &[row.get(0)], &input.request_id).await?;
    let payload = json!(input);
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "maintenance_cancel", &payload).await?
    {
        return Ok(Json(previous));
    }
    tx.execute("UPDATE maintenance_jobs SET cancel_requested=true,updated_at=$2 WHERE id=$1 AND stage IN ('queued','accepted','executing')",&[&input.job_id,&now()]).await?;
    let receipt = json!({"job_id":input.job_id,"request_id":input.request_id});
    management::record(
        &tx,
        &input.request_id,
        "maintenance_cancel",
        &payload,
        &receipt,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(receipt))
}
