//! One durable management operation coordinates DOMjudge and existing desktop commands.
use crate::{ApiError, ApiResult, App, auth, management, waiting};
use axum::{Json, extract::State, http::HeaderMap};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_postgres::Transaction;
use wccs_protocol::{digest, now, valid_id};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    contest_id: String,
    remote_contest_id: String,
    seat_ids: Vec<String>,
    action: String,
    execute_at: i64,
    duration_seconds: i64,
    #[serde(default)]
    preview_token: Option<String>,
}

impl Write {
    fn desktop(&self) -> ApiResult<waiting::Write> {
        serde_json::from_value(json!({"request_id":format!("cc-{}",self.request_id),
            "contest_id":self.contest_id,"seat_ids":self.seat_ids,
            "action":if self.opens() {"leave"} else {"enter"},
            "duration_seconds":self.duration_seconds}))
        .map_err(|_| ApiError::bad("比赛操作字段无效"))
    }

    fn opens(&self) -> bool {
        ["start", "resume"].contains(&self.action.as_str())
    }

    fn endpoint(&self) -> String {
        format!("contests/{}/wccs-control", self.remote_contest_id)
    }
}

async fn check_scope(tx: &Transaction<'_>, input: &Write) -> ApiResult<()> {
    crate::contest_binding::require(tx, &input.contest_id, &input.remote_contest_id).await?;
    if input.action != "pause" && tx.query_opt("SELECT 1 FROM pause_seats WHERE remote_contest_id=$1 AND NOT(seat_id=ANY($2)) LIMIT 1",&[&input.remote_contest_id,&input.seat_ids]).await?.is_some() {
        return Err(ApiError::conflict("尚有暂停座位未选中，请加入全部待恢复座位"));
    }
    Ok(())
}

async fn desktop_plan(tx: &Transaction<'_>, app: &App, input: &Write) -> ApiResult<Value> {
    check_scope(tx, input).await?;
    waiting::desktop_plan(tx, app, &input.desktop()?, true, input.action == "pause").await
}

fn remote_snapshot(value: &Value) -> ApiResult<Value> {
    if value["extension_version"] != 1
        || value["revision"].as_i64().is_none()
        || value["snapshot_hash"]
            .as_str()
            .is_none_or(|hash| hash.len() != 64)
        || value["phase"].as_str().is_none()
    {
        return Err(ApiError::conflict("DOMjudge 暂停扩展尚未就绪"));
    }
    Ok(
        json!({"revision":value["revision"],"snapshot_hash":value["snapshot_hash"],"phase":value["phase"]}),
    )
}

fn phase_allows(action: &str, phase: &str) -> bool {
    match action {
        "start" => ["scheduled", "running"].contains(&phase),
        "pause" => ["running", "paused"].contains(&phase),
        "resume" => ["paused", "running"].contains(&phase),
        "end" => ["running", "paused", "ended"].contains(&phase),
        _ => false,
    }
}

async fn plan(app: &App, session: &auth::Session, input: &Write) -> ApiResult<Value> {
    if !valid_id(&input.request_id)
        || input.request_id.len() > 76
        || !wccs_protocol::preparation::domjudge_id(&input.remote_contest_id)
        || !["start", "pause", "resume", "end"].contains(&input.action.as_str())
        || (input.execute_at != 0 && !(now() - 5..=now() + 7 * 86400).contains(&input.execute_at))
    {
        return Err(ApiError::bad("选择有效比赛操作，安排时间限未来七天"));
    }
    {
        let mut db = app.preview_db.lock().await?;
        let tx = db.transaction().await?;
        auth::authorize_snapshot(&tx, session, auth::Permission::Admin)
            .await?
            .global()?;
        check_scope(&tx, input).await?;
    }
    let remote = app
        .domjudge
        .as_ref()
        .ok_or_else(|| ApiError::conflict("尚未配置 DOMjudge"))?;
    let state = remote
        .get(&input.endpoint())
        .await
        .map_err(|reason| ApiError::new(502, &reason))?;
    let remote = remote_snapshot(&state)?;
    if !phase_allows(&input.action, remote["phase"].as_str().unwrap_or("")) {
        return Err(ApiError::conflict(
            "DOMjudge 当前阶段不支持所选操作，请重新核对",
        ));
    }
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize_snapshot(&tx, session, auth::Permission::Admin).await?;
    actor.global()?;
    actor.seats(&tx, &input.seat_ids, &input.request_id).await?;
    let desktop = desktop_plan(&tx, app, input).await?;
    let mut payload = serde_json::to_value(input).map_err(|_| ApiError::internal())?;
    payload.as_object_mut().unwrap().remove("preview_token");
    let mut result = json!({"input":payload,"desktop":desktop,"remote":remote});
    result["preview_token"] = json!(digest(&result.to_string()));
    Ok(result)
}

pub async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    Ok(Json(plan(&app, &session, &input).await?))
}

pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).map_err(|_| ApiError::internal())?;
    // Replay remains available even after the remote revision changes.
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        auth::authorize(&tx, &session, auth::Permission::Admin)
            .await?
            .global()?;
        if let Some(previous) =
            management::previous(&tx, &input.request_id, "contest_control", &payload).await?
        {
            return Ok(Json(previous));
        }
    }
    let planned = plan(&app, &session, &input).await?;
    if input.preview_token.as_deref() != planned["preview_token"].as_str()
        || planned["desktop"]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["status"] != "ready")
    {
        return Err(ApiError::conflict(
            "比赛或桌面状态已变化，需所有所选目标就绪后重新预览",
        ));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    actor.global()?;
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "contest_control", &payload).await?
    {
        return Ok(Json(previous));
    }
    if desktop_plan(&tx, &app, &input).await? != planned["desktop"] {
        return Err(ApiError::conflict("桌面状态已变化，请重新预览"));
    }
    if tx.query_one("SELECT EXISTS(SELECT 1 FROM contest_control_jobs WHERE stage IN ('scheduled','running'))",&[]).await?.get::<_,bool>(0) {
        return Err(ApiError::conflict("已有比赛联动任务，请完成或取消后继续"));
    }
    let execute_at = if input.execute_at == 0 {
        now()
    } else {
        input.execute_at
    };
    tx.execute("INSERT INTO contest_control_jobs(id,actor_id,actor_version,stage,execute_at,payload,plan,created_at,updated_at) VALUES($1,$2,$3,'scheduled',$4,$5,$6,$7,$7)",
        &[&input.request_id,&actor.id,&actor.version,&execute_at,&payload,&planned,&now()]).await?;
    let result = json!({"job_id":input.request_id,"stage":"scheduled"});
    management::record(&tx, &input.request_id, "contest_control", &payload, &result).await?;
    tx.commit().await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct ListFilter {
    contest_id: Option<String>,
}

pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    axum::extract::Query(filter): axum::extract::Query<ListFilter>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize_snapshot(&tx, &session, auth::Permission::Read)
        .await?
        .global()?;
    let pending: Vec<Value> = tx
        .query(
            "SELECT remote_contest_id,seat_id FROM pause_seats ORDER BY remote_contest_id,seat_id",
            &[],
        )
        .await?
        .iter()
        .map(|row| json!({"remote":row.get::<_,String>(0),"seat":row.get::<_,String>(1)}))
        .collect();
    let rows = tx.query("SELECT id,stage,payload,results,detail,execute_at,updated_at FROM contest_control_jobs WHERE $1::text IS NULL OR payload->>'contest_id'=$1 ORDER BY created_at DESC,id LIMIT 50",&[&filter.contest_id]).await?;
    Ok(Json(
        json!({"pending":pending,"jobs":rows.iter().map(|row|json!({"id":row.get::<_,String>(0),"stage":row.get::<_,String>(1),"payload":row.get::<_,Value>(2),"results":row.get::<_,Value>(3),"detail":row.get::<_,String>(4),"execute_at":row.get::<_,i64>(5),"updated_at":row.get::<_,i64>(6)})).collect::<Vec<_>>()}),
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
        return Err(ApiError::bad("任务编号无效"));
    }
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin)
        .await?
        .global()?;
    let payload = json!(input);
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "contest_control_cancel", &payload).await?
    {
        return Ok(Json(previous));
    }
    if tx.execute("UPDATE contest_control_jobs SET stage='cancelled',detail='operator_cancelled',updated_at=$2 WHERE id=$1 AND stage='scheduled'",&[&input.job_id,&now()]).await? != 1 {
        return Err(ApiError::conflict("任务已执行或结束，请核对实际状态并发起新的操作"));
    }
    let result = json!({"job_id":input.job_id,"stage":"cancelled"});
    management::record(
        &tx,
        &input.request_id,
        "contest_control_cancel",
        &payload,
        &result,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(result))
}

pub fn start(app: &App) {
    if app.domjudge.is_none() {
        return;
    }
    let weak = std::sync::Arc::downgrade(app);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some(app) = weak.upgrade() else {
                break;
            };
            if let Err(error) = work(&app).await {
                eprintln!("contest_control_worker status={}", error.status);
            }
        }
    });
}

fn target_identity(value: &Value) -> Value {
    json!({"device_id":value["device_id"],"identity":value["identity"],"generation":value["generation"],
        "instance_id":value["instance_id"],"session_key":value["session_key"],"connection_id":value["connection_id"],
        "lifecycle_version":value["lifecycle_version"],"address_version":value["address_version"],"status":value["status"]})
}

async fn checked_desktop(
    tx: &Transaction<'_>,
    app: &App,
    input: &Write,
    planned: &Value,
    session: &auth::Session,
    issued: bool,
) -> ApiResult<(auth::Identity, Value)> {
    let actor = auth::authorize(tx, session, auth::Permission::Admin).await?;
    actor.global()?;
    let current = desktop_plan(tx, app, input).await?;
    let changed = if issued {
        current["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(target_identity)
            .collect::<Vec<_>>()
            != planned["desktop"]["targets"]
                .as_array()
                .unwrap()
                .iter()
                .map(target_identity)
                .collect::<Vec<_>>()
    } else {
        current != planned["desktop"]
    };
    if changed {
        return Err(ApiError::conflict("target_changed_review_required"));
    }
    Ok((actor, current))
}

async fn remote_action(
    app: &App,
    input: &Write,
    planned: &Value,
    results: &mut Value,
) -> ApiResult<()> {
    let remote = app.domjudge.as_ref().ok_or_else(ApiError::internal)?;
    let body = json!({"request_id":input.request_id,"action":input.action,
        "revision":planned["remote"]["revision"],"snapshot_hash":planned["remote"]["snapshot_hash"]});
    results["remote"] = json!("pending");
    let response = remote
        .request(Method::POST, &input.endpoint(), Some(body))
        .await
        .map_err(|reason| ApiError::new(502, &reason))?;
    remote_snapshot(&response)?;
    results["remote"] = response;
    Ok(())
}

async fn execute(
    app: &App,
    input: &Write,
    planned: &Value,
    session: &auth::Session,
    results: &mut Value,
) -> ApiResult<()> {
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        checked_desktop(&tx, app, input, planned, session, false).await?;
        if input.action == "pause" {
            tx.execute(
                "INSERT INTO pause_seats SELECT $1,unnest($2::text[]) ON CONFLICT DO NOTHING",
                &[&input.remote_contest_id, &input.seat_ids],
            )
            .await?;
        }
        tx.commit().await?;
    }
    if input.opens() {
        let state = app
            .domjudge
            .as_ref()
            .ok_or_else(ApiError::internal)?
            .get(&input.endpoint())
            .await
            .map_err(|reason| ApiError::new(502, &reason))?;
        if remote_snapshot(&state)? != planned["remote"]
            || !phase_allows(&input.action, state["phase"].as_str().unwrap_or(""))
        {
            return Err(ApiError::conflict("remote_changed_review_required"));
        }
    }
    // Close submissions before entering waiting. Open submissions only after all
    // selected clients acknowledge leaving waiting. No implicit compensation resumes a contest.
    if !input.opens() {
        remote_action(app, input, planned, results).await?;
    }
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        let (actor, current) = checked_desktop(&tx, app, input, planned, session, false).await?;
        results["desktop"] = waiting::issue(&tx, &actor, &input.desktop()?, current).await?;
        tx.execute(
            "UPDATE contest_control_jobs SET results=$2,updated_at=$3 WHERE id=$1",
            &[&input.request_id, &*results, &now()],
        )
        .await?;
        tx.commit().await?;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        checked_desktop(&tx, app, input, planned, session, true).await?;
        let rows = tx
            .query(
                "SELECT device_id,stage,report FROM waiting_commands WHERE request_id=$1",
                &[&format!("cc-{}", input.request_id)],
            )
            .await?;
        results["reports"] = json!(rows.iter().map(|row|json!({"device_id":row.get::<_,String>(0),"stage":row.get::<_,String>(1),"report":row.get::<_,Option<Value>>(2)})).collect::<Vec<_>>());
        if rows.iter().any(|row| {
            ["failed", "cancelled", "expired"].contains(&row.get::<_, String>(1).as_str())
        }) {
            return Err(ApiError::conflict("desktop_failed_review_required"));
        }
        if rows.len() == input.seat_ids.len()
            && rows
                .iter()
                .all(|row| row.get::<_, String>(1) == "succeeded")
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ApiError::conflict("desktop_timeout_review_required"));
        }
    }
    if input.opens() {
        remote_action(app, input, planned, results).await?;
    }
    Ok(())
}

async fn work(app: &App) -> ApiResult<()> {
    let row = app.db.lock().await?.query_opt("UPDATE contest_control_jobs SET stage='running',updated_at=$1 WHERE id=(SELECT id FROM contest_control_jobs WHERE stage='scheduled' AND (execute_at<=$1 OR (payload->>'action'='start' AND NOT results ? 'armed')) ORDER BY execute_at LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING id,payload,plan,actor_id,actor_version,results,execute_at",&[&now()]).await?;
    let Some(row) = row else {
        return Ok(());
    };
    let id: String = row.get("id");
    let input: Write =
        serde_json::from_value(row.get("payload")).map_err(|_| ApiError::internal())?;
    let session = auth::Session {
        operator_id: row.get("actor_id"),
        version: row.get("actor_version"),
        csrf: String::new(),
        expires: i64::MAX,
    };
    let mut planned: Value = row.get("plan");
    let mut results: Value = row.get("results");
    let arming = input.action == "start" && input.execute_at > now() && results["armed"] != true;
    let result = if arming {
        arm_start(app, &input, &mut planned, &session, &mut results).await
    } else {
        execute(app, &input, &planned, &session, &mut results).await
    };
    let (stage, detail) = match result {
        Ok(()) if arming => ("scheduled", String::new()),
        Ok(()) => ("succeeded", String::new()),
        Err(error) => ("failed", error.message),
    };
    loop {
        let saved: ApiResult<()> = async {
            let mut db = app.db.lock().await?;
            let tx = db.transaction().await?;
            if stage=="succeeded" && input.action != "pause" {
                tx.execute("DELETE FROM pause_seats WHERE remote_contest_id=$1 AND seat_id=ANY($2)",&[&input.remote_contest_id,&input.seat_ids]).await?;
            }
            tx.execute("UPDATE contest_control_jobs SET stage=$2,results=$3,detail=$4,updated_at=$5,plan=$6 WHERE id=$1",&[&id,&stage,&results,&detail,&now(),&planned]).await?;
            tx.commit().await?;
            Ok(())
        }.await;
        if saved.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn arm_start(
    app: &App,
    input: &Write,
    planned: &mut Value,
    session: &auth::Session,
    results: &mut Value,
) -> ApiResult<()> {
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        checked_desktop(&tx, app, input, planned, session, false).await?;
    }
    let mut hold = input.clone();
    hold.request_id = format!("{}-arm", input.request_id);
    hold.action = "hold_start".into();
    remote_action(app, &hold, planned, results).await?;
    planned["remote"] = remote_snapshot(&results["remote"])?;
    results["armed"] = json!(true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_fence_preserves_connection_identity_but_ignores_command_head() {
        let original = json!({"device_id":"device","identity":{"seat_id":"seat"},
            "generation":1,"instance_id":"boot","connection_id":"socket-one",
            "session_key":"session","lifecycle_version":1,"address_version":1,"status":"ready","head":1});
        let mut current = original.clone();
        current["head"] = json!(2);
        assert_eq!(target_identity(&original), target_identity(&current));
        for field in [
            "connection_id",
            "generation",
            "session_key",
            "address_version",
            "status",
            "identity",
        ] {
            let mut changed = current.clone();
            changed[field] = json!("changed");
            assert_ne!(target_identity(&original), target_identity(&changed));
        }
    }

    #[test]
    fn remote_revision_requires_the_installed_extension() {
        assert!(remote_snapshot(&Value::Null).is_err());
        let value = json!({"extension_version":1,"revision":3,"snapshot_hash":"a".repeat(64),"phase":"paused","server_time":123});
        assert_eq!(
            remote_snapshot(&value).unwrap(),
            json!({"revision":3,"snapshot_hash":"a".repeat(64),"phase":"paused"})
        );
    }
}
