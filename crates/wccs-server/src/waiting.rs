use crate::{ApiError, ApiResult, App, auth, management, seating};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio_postgres::{IsolationLevel, Transaction};
use wccs_protocol::{control::*, digest, now, token, valid_id};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    contest_id: String,
    seat_ids: Vec<String>,
    action: String,
    duration_seconds: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    confirm_session_end: bool,
    #[serde(default)]
    preview_token: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

async fn plan(tx: &Transaction<'_>, app: &App, input: &Write) -> ApiResult<Value> {
    if !valid_id(&input.request_id)
        || !valid_id(&input.contest_id)
        || input.seat_ids.is_empty()
        || input.seat_ids.len() > 500
        || input
            .seat_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.seat_ids.len()
        || !["enter", "leave", "prepare_login", "restore_teaching"].contains(&input.action.as_str())
        || !(30..=3600).contains(&input.duration_seconds)
    {
        return Err(ApiError::bad(
            "等待控制字段无效，选择 1～500 个座位，有效期 30～3600 秒",
        ));
    }
    let contest = tx
        .query_opt(
            "SELECT name,version FROM contests WHERE id=$1",
            &[&input.contest_id],
        )
        .await?
        .ok_or_else(|| ApiError::bad("请选择有效比赛"))?;
    let tool_version: i64 = tx
        .query_one("SELECT version FROM configuration_head WHERE id=1", &[])
        .await?
        .get(0);
    let tools = tx
        .query_opt(
            "SELECT snapshot FROM configurations WHERE version=$1",
            &[&tool_version],
        )
        .await?
        .map(|r| r.get::<_, Value>(0))
        .unwrap_or(json!({"tools":[]}));
    let tool_paths: Option<Vec<String>> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().map(String::from))
        .collect();
    let seating = seating::snapshot(tx).await?;
    let seats: HashMap<_, _> = seating["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|room| {
            room["seats"]
                .as_array()
                .unwrap()
                .iter()
                .map(move |seat| (seat["id"].as_str().unwrap(), (room, seat)))
        })
        .collect();
    let participants: HashMap<_, _> = tx
        .query(
            "SELECT p.id,p.name,p.school,a.seat_id FROM assignments a
             JOIN participants p ON p.id=a.participant_id AND p.contest_id=a.contest_id
             WHERE a.contest_id=$1 AND a.seat_id=ANY($2)",
            &[&input.contest_id, &input.seat_ids],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, String>(3), row))
        .collect();
    let mut device_ids = Vec::new();
    for seat_id in &input.seat_ids {
        let (_, seat) = seats
            .get(seat_id.as_str())
            .ok_or_else(|| ApiError::conflict("座位已变化，请重新选择"))?;
        if let Some(id) = seat["devices"]
            .as_array()
            .unwrap()
            .first()
            .and_then(Value::as_str)
        {
            device_ids.push(id);
        }
    }
    let devices: HashMap<_, _> = tx
        .query(
            "SELECT lifecycle,enrollment_generation,lifecycle_version,address_version,id,
             EXISTS(SELECT 1 FROM maintenance_jobs j WHERE j.device_id=devices.id AND j.stage IN ('queued','accepted','executing') AND j.expires_at>$2) AS maintenance
             FROM devices WHERE id=ANY($1)",
            &[&device_ids, &now()],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, String>(4), row))
        .collect();
    let previous_commands: HashMap<_, _> = tx
        .query(
            "SELECT DISTINCT ON(device_id) command,stage,sequence,device_id
             FROM waiting_commands WHERE device_id=ANY($1) ORDER BY device_id,sequence DESC",
            &[&device_ids],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, String>(3), row))
        .collect();
    let controls: HashMap<_, _> = tx
        .query(
            "SELECT * FROM control_devices WHERE device_id=ANY($1)",
            &[&device_ids],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, String>("device_id"), row))
        .collect();
    let mut targets = Vec::new();
    for seat_id in &input.seat_ids {
        let (room, seat) = seats[seat_id.as_str()];
        let device = seat["devices"].as_array().unwrap();
        let participant = participants.get(seat_id);
        let mut target = json!({"seat_id":seat_id,"room_id":room["id"],"room_name":room["name"],"number":seat["number"],"status":"unbound","device_id":null,"identity":null,"generation":null,"instance_id":null,"session_key":null});
        if let Some(p) = participant {
            target["identity"] = json!(WaitingIdentity {
                contest_id: input.contest_id.clone(),
                contest_name: contest.get(0),
                contest_version: contest.get(1),
                participant_id: p.get(0),
                name: p.get(1),
                school: p.get(2),
                seat_id: seat_id.clone(),
                seat_label: format!("{} / {}", room["name"].as_str().unwrap(), seat["number"]),
                room_id: room["id"].as_str().unwrap().into(),
                room_version: room["version"].as_i64().unwrap(),
            });
        }
        if seat["status"] == "conflict" {
            target["status"] = json!("conflict");
        } else if let Some(id) = device.first().and_then(Value::as_str) {
            target["device_id"] = json!(id);
            let d = devices
                .get(id)
                .ok_or_else(|| ApiError::conflict("设备已变化，请重新预览"))?;
            target["generation"] = json!(d.get::<_, i64>(1));
            target["lifecycle_version"] = json!(d.get::<_, i64>(2));
            target["address_version"] = json!(d.get::<_, i64>(3));
            let previous = previous_commands.get(id);
            target["head"] = json!(previous.as_ref().map(|r| r.get::<_, i64>(2)));
            if ["leave", "restore_teaching"].contains(&input.action.as_str())
                && let Some(old) = &previous
            {
                let command: Value = old.get(0);
                if command["identity"]["contest_id"] == input.contest_id {
                    target["identity"] = command["identity"].clone();
                }
            }
            target["status"] = json!("offline");
            if let Some(c) = controls.get(id) {
                target["instance_id"] = json!(c.get::<_, String>("instance_id"));
                target["session_key"] = json!(c.get::<_, Option<String>>("session_key"));
                target["connection_id"] = json!(c.get::<_, String>("connection_id"));
                let actual: Value = c.get("actual");
                target["account"] = actual["account"].clone();
                let online = c.get::<_, bool>("connected")
                    && c.get::<_, String>("server_epoch") == app.server_epoch
                    && now() - c.get::<_, i64>("last_seen") < LEASE_SECONDS as i64;
                if online || input.action == "leave" {
                    target["status"] = json!("ready");
                }
                if online && input.action == "enter" && target["session_key"].is_null() {
                    target["status"] = json!("no_session");
                }
                if online
                    && ["enter", "prepare_login"].contains(&input.action.as_str())
                    && actual["detail"] == "waiting_policy_recovery_failed"
                {
                    target["status"] = json!("recovery_required");
                }
            }
            if target["identity"].is_null() {
                target["status"] = json!("no_participant");
            }
            if d.get::<_, bool>("maintenance") && input.action != "leave" {
                target["status"] = json!("maintenance_active");
            }
            if d.get::<_, String>(0) != "enabled" {
                target["status"] = json!("disabled");
            }
            if let Some(old) = &previous {
                let command: Value = old.get(0);
                if command["identity"]["contest_id"] != input.contest_id
                    && ["enter", "prepare_login"]
                        .contains(&command["action"].as_str().unwrap_or(""))
                    && !["cancelled", "expired", "failed"]
                        .contains(&old.get::<_, String>(1).as_str())
                {
                    target["status"] = json!("other_contest");
                }
            }
        }
        if ["prepare_login", "restore_teaching"].contains(&input.action.as_str())
            && target["status"] == "ready"
        {
            let account = serde_json::from_value::<wccs_protocol::login::AccountState>(
                target["account"].clone(),
            )
            .ok();
            let reason = account.as_ref().map_or(Some("login_unavailable"), |a| {
                a.skip_reason(&input.action, input.confirm_session_end)
            });
            if let Some(reason) = reason {
                target["status"] = json!(reason);
            }
        }
        if input.action == "enter"
            && target["status"] == "ready"
            && ["simulation", "windows_test"]
                .contains(&target["account"]["adapter"].as_str().unwrap_or(""))
            && target["account"]["phase"] != "controlled"
        {
            target["status"] = json!("recovery_required");
        }
        if input.action == "prepare_login"
            && target["status"] == "ready"
            && target["account"]["adapter"] == "windows_test"
            && tool_paths.as_ref().is_none_or(Vec::is_empty)
        {
            target["status"] = json!("tools_unavailable");
        }
        targets.push(target);
    }
    let mut result = json!({"contest_id":input.contest_id,"contest_name":contest.get::<_,String>(0),"contest_version":contest.get::<_,i64>(1),"action":input.action,"duration_seconds":input.duration_seconds,"targets":targets});
    if ["prepare_login", "restore_teaching"].contains(&input.action.as_str()) {
        result["confirm_session_end"] = json!(input.confirm_session_end);
        result["tool_version"] = json!(tool_version);
        result["tool_paths"] = json!(tool_paths.unwrap_or_default());
    }
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
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Operate).await?;
    actor.seats(&tx, &input.seat_ids, &input.request_id).await?;
    let result = plan(&tx, &app, &input).await?;
    tx.commit().await?;
    Ok(Json(result))
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
    actor.seats(&tx, &input.seat_ids, &input.request_id).await?;
    let payload = serde_json::to_value(&input).unwrap();
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "waiting_commit", &payload).await?
    {
        // Historical scopes remain authoritative even if seats moved or disappeared.
        for row in tx
            .query(
                "SELECT room_ids FROM waiting_commands WHERE request_id=$1",
                &[&input.request_id],
            )
            .await?
        {
            actor.rooms(Some(&row.get::<_, Vec<String>>(0)))?;
        }
        return Ok(Json(previous));
    }
    let planned = plan(&tx, &app, &input).await?;
    if input.preview_token.as_deref() != planned["preview_token"].as_str() {
        return Err(ApiError::conflict("目标、身份或版本已变化，请重新预览"));
    }
    let targets: Vec<_> = planned["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["status"] == "ready")
        .collect();
    if targets.is_empty() {
        return Err(ApiError::conflict("没有可执行目标，请核对跳过原因"));
    }
    let devices: Vec<String> = targets
        .iter()
        .map(|target| target["device_id"].as_str().unwrap().to_owned())
        .collect();
    actor.devices(&tx, &devices, &input.request_id).await?;
    // Allocate and persist the batch in this transaction so control renewals do
    // not wait for several database round trips per selected seat.
    let sequences = tx.query(
        "SELECT nextval(pg_get_serial_sequence('waiting_commands','sequence')) FROM generate_series(1,$1::int)",
        &[&(targets.len() as i32)],
    ).await?;
    let issued_at = now();
    let mut commands = Vec::with_capacity(targets.len());
    for (target, sequence) in targets.into_iter().zip(sequences) {
        let device_id = target["device_id"].as_str().unwrap();
        let command = Command {
            protocol_version: CONTROL_VERSION,
            command_id: token(),
            request_id: input.request_id.clone(),
            sequence: sequence.get(0),
            device_id: device_id.into(),
            generation: target["generation"].as_i64().unwrap(),
            device_version: target["lifecycle_version"].as_i64().unwrap(),
            instance_id: target["instance_id"].as_str().unwrap().into(),
            session_key: target["session_key"].as_str().unwrap_or("none").into(),
            action: input.action.clone(),
            expires_at: issued_at + input.duration_seconds,
            login: if !target["account"].is_null() {
                Some(wccs_protocol::login::LoginPlan {
                    tool_version: planned["tool_version"].as_i64().unwrap_or(0),
                    tool_paths: serde_json::from_value(planned["tool_paths"].clone())
                        .unwrap_or_default(),
                    connection_id: target["connection_id"].as_str().unwrap().into(),
                    snapshot: serde_json::from_value(target["account"].clone())
                        .map_err(|_| ApiError::internal())?,
                    confirm_session_end: input.confirm_session_end,
                })
            } else {
                None
            },
            identity: serde_json::from_value(target["identity"].clone())
                .map_err(|_| ApiError::internal())?,
        };
        commands.push(command);
    }
    tx.execute("UPDATE waiting_commands SET stage='cancelled',updated_at=$2 WHERE device_id=ANY($1) AND stage NOT IN ('cancelled','expired','failed')", &[&devices,&issued_at]).await?;
    tx.execute("INSERT INTO waiting_commands(sequence,id,request_id,device_id,actor_id,actor_version,room_ids,contest_id,generation,instance_id,session_key,expires_at,command,updated_at)
        SELECT (c->>'sequence')::bigint,c->>'command_id',c->>'request_id',c->>'device_id',$2,$3,
            ARRAY[c#>>'{identity,room_id}'],c#>>'{identity,contest_id}',(c->>'generation')::bigint,
            c->>'instance_id',c->>'session_key',(c->>'expires_at')::bigint,c,$4
        FROM jsonb_array_elements($1::jsonb) AS c ORDER BY (c->>'sequence')::bigint",
        &[&json!(commands),&actor.id,&actor.version,&issued_at]).await?;
    let rooms: Vec<String> = planned["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["room_id"].as_str().unwrap().to_owned())
        .collect();
    auth::context(&tx, &input.request_id, Some(&rooms), &devices).await?;
    let result = json!({"request_id":input.request_id,"commands":commands,"preview":planned});
    management::record(&tx, &input.request_id, "waiting_commit", &payload, &result).await?;
    tx.commit().await?;
    Ok(Json(result))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    contest_id: Option<String>,
}

pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> ApiResult<Json<Value>> {
    if filter.contest_id.as_ref().is_some_and(|id| !valid_id(id)) {
        return Err(ApiError::bad("比赛 ID 无效"));
    }
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Read).await?;
    expire(&tx).await?;
    let rows = tx.query("SELECT w.*,c.actual,c.last_seen,c.connected,c.server_epoch,($1 OR COALESCE((cardinality(device_room_scope(w.device_id))>0
            AND device_room_scope(w.device_id)<@$2),false)) AS live_allowed
            FROM (SELECT DISTINCT ON (device_id,contest_id) * FROM waiting_commands
                WHERE ($1 OR (cardinality(room_ids)>0 AND room_ids<@$2))
                AND ($3::text IS NULL OR contest_id=$3)
                ORDER BY device_id,contest_id,sequence DESC) w
            LEFT JOIN control_devices c ON c.device_id=w.device_id
            ORDER BY w.sequence DESC LIMIT 500", &[&actor.all_rooms,&actor.room_ids,&filter.contest_id]).await?;
    let items: Vec<Value> = rows.iter().map(|r| {
        let online = r.get::<_,bool>("live_allowed") && r.get::<_,Option<bool>>("connected")==Some(true) && r.get::<_,Option<String>>("server_epoch").as_deref()==Some(&app.server_epoch) && r.get::<_,Option<i64>>("last_seen").is_some_and(|t| now()-t<LEASE_SECONDS as i64);
        json!({"command":r.get::<_,Value>("command"),"stage":r.get::<_,String>("stage"),"report":r.get::<_,Option<Value>>("report"),"online":online,"actual":if online {r.get::<_,Option<Value>>("actual")} else {None},"last_seen":if r.get::<_,bool>("live_allowed") {r.get::<_,Option<i64>>("last_seen")} else {None},"updated_at":r.get::<_,i64>("updated_at")})
    }).collect();
    tx.commit().await?;
    Ok(Json(json!({"items":items,"server_time":now()})))
}

pub async fn expire(tx: &Transaction<'_>) -> ApiResult<()> {
    expire_for(tx, None).await
}

async fn expire_for(tx: &Transaction<'_>, device: Option<&str>) -> ApiResult<()> {
    // Persist authority loss even when the device can no longer authenticate a socket.
    // A control poll checks its own device. Management queries still retire all
    // devices, including offline ones, without repeating a full scan per heartbeat.
    tx.execute(
        "UPDATE waiting_commands w SET stage='cancelled',updated_at=$1
            WHERE stage NOT IN ('failed','cancelled','expired')
            AND ($2::text IS NULL OR w.device_id=$2)
            AND NOT EXISTS(SELECT 1
            FROM devices d,operators o
            WHERE d.id=w.device_id
            AND d.lifecycle='enabled'
            AND d.enrollment_generation=w.generation
            AND d.lifecycle_version=(w.command->>'device_version')::bigint
            AND o.id=w.actor_id
            AND o.version=w.actor_version
            AND o.enabled
            AND o.role<>'viewer'
            AND (o.all_rooms OR w.room_ids<@o.room_ids))",
        &[&now(), &device],
    )
    .await?;

    tx.execute("UPDATE waiting_commands SET stage='expired',updated_at=$1 WHERE expires_at<=$1 AND stage NOT IN ('failed','cancelled','expired') AND ($2::text IS NULL OR device_id=$2)", &[&now(), &device]).await?;
    Ok(())
}

pub async fn current(
    tx: &Transaction<'_>,
    id: &str,
    instance: &str,
    session: Option<&str>,
) -> ApiResult<Option<Command>> {
    expire_for(tx, Some(id)).await?;
    let Some(row) = tx
        .query_opt(
            "SELECT * FROM waiting_commands WHERE device_id=$1 ORDER BY sequence DESC LIMIT 1",
            &[&id],
        )
        .await?
    else {
        return Ok(None);
    };
    if ["failed", "expired", "cancelled"].contains(&row.get::<_, String>("stage").as_str()) {
        return Ok(None);
    }
    let command: Command =
        serde_json::from_value(row.get("command")).map_err(|_| ApiError::internal())?;
    let valid: bool = tx
        .query_one(
            "SELECT EXISTS(SELECT 1
            FROM operators o,devices d
            WHERE o.id=$1
            AND o.version=$2
            AND o.enabled
            AND o.role<>'viewer'
            AND (o.all_rooms OR $3=ANY(o.room_ids))
            AND d.id=$4
            AND d.lifecycle='enabled'
            AND d.enrollment_generation=$5
            AND d.lifecycle_version=$6
            AND NOT d.needs_review
            AND NOT d.excluded
            AND EXISTS(SELECT 1
            FROM seats s
            WHERE s.id=$7
            AND s.expected_ip=d.observed_ip
            AND (SELECT count(*)
            FROM devices x
            WHERE x.observed_ip=s.expected_ip
            AND x.credential_hash IS NOT NULL
            AND NOT x.excluded)=1)
            AND device_room_scope(d.id)=ARRAY[$3])",
            &[
                &row.get::<_, String>("actor_id"),
                &row.get::<_, i64>("actor_version"),
                &command.identity.room_id,
                &id,
                &command.generation,
                &command.device_version,
                &command.identity.seat_id,
            ],
        )
        .await?
        .get(0);
    let mut valid = valid && command.instance_id == instance;
    if let Some(login) = &command.login {
        let connection: String = tx
            .query_one(
                "SELECT connection_id FROM control_devices WHERE device_id=$1",
                &[&id],
            )
            .await?
            .get(0);
        valid &= connection == login.connection_id;
        if command.action == "prepare_login" && login.snapshot.adapter == "windows_test" {
            let version: i64 = tx
                .query_one("SELECT version FROM configuration_head WHERE id=1", &[])
                .await?
                .get(0);
            valid &= version == login.tool_version;
        }
    }
    if ["enter", "prepare_login"].contains(&command.action.as_str()) {
        if command.action == "enter" {
            valid &= session == Some(command.session_key.as_str());
        }
        let unchanged: bool = tx
            .query_one(
                "SELECT EXISTS(SELECT 1
            FROM contests c,rooms r,assignments a
            WHERE c.id=$1
            AND c.version=$2
            AND r.id=$3
            AND r.version=$4
            AND a.contest_id=c.id
            AND a.participant_id=$5
            AND a.seat_id=$6)",
                &[
                    &command.identity.contest_id,
                    &command.identity.contest_version,
                    &command.identity.room_id,
                    &command.identity.room_version,
                    &command.identity.participant_id,
                    &command.identity.seat_id,
                ],
            )
            .await?
            .get(0);
        valid &= unchanged;
    }
    if !valid {
        tx.execute(
            "UPDATE waiting_commands SET stage='cancelled',updated_at=$2 WHERE id=$1",
            &[&command.command_id, &now()],
        )
        .await?;
        return Ok(None);
    }
    Ok(Some(command))
}
