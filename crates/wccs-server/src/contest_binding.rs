use crate::{ApiError, ApiResult, App, auth, management};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

#[derive(Deserialize, Serialize)]
pub struct Binding {
    request_id: String,
    contest_id: String,
    previous: Option<String>,
    remote_contest_id: String,
}

pub async fn choices(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    {
        let mut db = app.preview_db.lock().await?;
        let tx = db.transaction().await?;
        auth::authorize_snapshot(&tx, &session, auth::Permission::Admin)
            .await?
            .global()?;
    }
    let contests = app
        .domjudge
        .as_ref()
        .ok_or_else(|| ApiError::conflict("未配置评测连接"))?
        .get("contests")
        .await
        .map_err(|reason| ApiError::new(502, &reason))?;
    let choices: Vec<_> = contests
        .as_array()
        .ok_or_else(|| ApiError::conflict("评测比赛列表无效"))?
        .iter()
        .filter_map(|contest| {
            Some(json!({"id":contest["id"].as_str()?,"name":contest["name"].as_str()?}))
        })
        .collect();
    Ok(Json(json!({"contests":choices})))
}

pub async fn bind(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Binding>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    if !wccs_protocol::valid_id(&input.request_id)
        || !wccs_protocol::preparation::domjudge_id(&input.remote_contest_id)
    {
        return Err(ApiError::bad("比赛编号无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin)
        .await?
        .global()?;
    let payload = json!(input);
    if let Some(result) =
        management::previous(&tx, &input.request_id, "contest_bind", &payload).await?
    {
        return Ok(Json(result));
    }
    let blocked: bool = tx.query_one(
        "SELECT EXISTS(SELECT 1 FROM contests WHERE remote_contest_id=$1 AND id<>$2)
         OR EXISTS(SELECT 1 FROM contest_control_jobs WHERE stage IN ('scheduled','running'))
         OR EXISTS(SELECT 1 FROM domjudge_jobs WHERE stage IN ('queued','running'))
         OR EXISTS(SELECT 1 FROM pause_seats WHERE remote_contest_id=$3)
         OR EXISTS(SELECT 1 FROM domjudge_bindings b JOIN participants p ON p.id=b.participant_id
           WHERE (p.contest_id=$2 AND b.remote_contest_id<>$1) OR (p.contest_id<>$2 AND b.remote_contest_id=$1))",
        &[&input.remote_contest_id,&input.contest_id,&input.previous],
    ).await?.get(0);
    if blocked {
        return Err(ApiError::conflict(
            "另一场已绑定，或存在待恢复座位、未迁移账号或进行中的任务，不能改绑",
        ));
    }
    let changed = tx.execute(
        "UPDATE contests SET remote_contest_id=$2 WHERE id=$1 AND remote_contest_id IS NOT DISTINCT FROM $3",
        &[&input.contest_id,&input.remote_contest_id,&input.previous],
    ).await?;
    if changed != 1 {
        return Err(ApiError::conflict("比赛绑定已变化，请刷新核对"));
    }
    let result = json!({"remote_contest_id":input.remote_contest_id});
    management::record(&tx, &input.request_id, "contest_bind", &payload, &result).await?;
    tx.commit().await?;
    Ok(Json(result))
}

pub async fn require(tx: &Transaction<'_>, contest: &str, remote: &str) -> ApiResult<()> {
    if tx
        .query_opt(
            "SELECT 1 FROM contests WHERE id=$1 AND remote_contest_id=$2",
            &[&contest, &remote],
        )
        .await?
        .is_none()
    {
        return Err(ApiError::conflict(
            "请先在 DOMjudge 账号页核对并绑定本场评测比赛",
        ));
    }
    Ok(())
}
