use crate::{ApiError, ApiResult, App, auth, management};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wccs_protocol::{
    digest, now,
    presentation::{Data, Display, Settings},
    valid_id,
};

#[derive(Deserialize)]
pub struct Contest {
    contest_id: String,
}
pub async fn get(
    State(app): State<App>,
    headers: HeaderMap,
    Query(input): Query<Contest>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;
    let row = tx
        .query_opt(
            "SELECT version,settings FROM presentation_settings WHERE contest_id=$1",
            &[&input.contest_id],
        )
        .await?;
    let (version, settings) = row
        .map(|r| (r.get::<_, i64>(0), r.get::<_, Value>(1)))
        .unwrap_or((0, json!(Settings::default())));
    let receipts=tx.query("SELECT p.device_id,p.report,p.received_at FROM presentation_receipts p JOIN devices d ON d.id=p.device_id WHERE p.report->'data'->>'generation'=d.enrollment_generation::text AND p.report->'data'->'identity'->>'contest_id'=$1 AND ($2 OR (cardinality(device_room_scope(d.id))>0 AND device_room_scope(d.id)<@$3))",&[&input.contest_id,&actor.all_rooms,&actor.room_ids]).await?.iter().map(|r|json!({"device_id":r.get::<_,String>(0),"report":r.get::<_,Value>(1),"received_at":r.get::<_,i64>(2)})).collect::<Vec<_>>();
    Ok(Json(
        json!({"contest_id":input.contest_id,"version":version,"settings":settings,"receipts":receipts}),
    ))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    request_id: String,
    contest_id: String,
    base_version: i64,
    settings: Settings,
}
pub async fn save(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Save>,
) -> ApiResult<Json<Value>> {
    if !valid_id(&input.contest_id) || !input.settings.valid() {
        return Err(ApiError::bad(
            "展示配置或校徽无效，校徽需 96×96 以内的 24 位无压缩 BMP",
        ));
    }
    if let Some(address) = &input.settings.domjudge_api {
        let url = url::Url::parse(address).map_err(|_| ApiError::bad("DOMjudge API 地址无效"))?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || address.len() > 2048
        {
            return Err(ApiError::bad(
                "DOMjudge API 地址必须为无凭据的 HTTP(S) 完整地址",
            ));
        }
    }
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    actor.global()?;
    let payload = json!(input);
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "presentation_save", &payload).await?
    {
        return Ok(Json(previous));
    }
    if tx
        .query_opt("SELECT id FROM contests WHERE id=$1", &[&input.contest_id])
        .await?
        .is_none()
    {
        return Err(ApiError::bad("比赛不存在"));
    }
    let version = tx
        .query_opt(
            "SELECT version FROM presentation_settings WHERE contest_id=$1",
            &[&input.contest_id],
        )
        .await?
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0);
    if version != input.base_version {
        return Err(ApiError::conflict("展示配置已变化，请重新加载"));
    }
    let next = version + 1;
    tx.execute("INSERT INTO presentation_settings VALUES($1,$2,$3,$4) ON CONFLICT(contest_id) DO UPDATE SET version=$2,settings=$3,updated_at=$4",&[&input.contest_id,&next,&json!(input.settings),&now()]).await?;
    let receipt = json!({"request_id":input.request_id,"version":next});
    management::record(
        &tx,
        &input.request_id,
        "presentation_save",
        &payload,
        &receipt,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(receipt))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub instance_id: String,
    pub session_key: Option<String>,
}
pub async fn device(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> ApiResult<Json<Value>> {
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    let row=tx.query_opt("SELECT w.command,w.stage,w.expires_at FROM waiting_commands w JOIN devices d ON d.id=w.device_id WHERE w.device_id=$1 AND w.generation=d.enrollment_generation AND w.sequence=(SELECT max(sequence) FROM waiting_commands WHERE device_id=$1)",&[&device]).await?;
    let command = row
        .and_then(|row| {
            if ["failed", "cancelled", "expired"].contains(&row.get::<_, String>(1).as_str())
                || row.get::<_, i64>(2) <= now()
            {
                return None;
            }
            serde_json::from_value::<wccs_protocol::control::Command>(row.get(0)).ok()
        })
        .filter(|command| {
            command.instance_id == input.instance_id
                && input.session_key.as_deref() == Some(&command.session_key)
        });
    let mut data = None;
    if let Some(command) = command.filter(|c| c.action == "enter") {
        let row = tx
            .query_opt(
                "SELECT version,settings FROM presentation_settings WHERE contest_id=$1",
                &[&command.identity.contest_id],
            )
            .await?;
        let (version, settings) = row
            .map(|r| (r.get::<_, i64>(0), r.get::<_, Value>(1)))
            .unwrap_or((0, json!(Settings::default())));
        let digest = digest(&settings.to_string());
        let settings: Settings =
            serde_json::from_value(settings).map_err(|_| ApiError::internal())?;
        data = Some(Data {
            schema_version: 1,
            command_id: command.command_id,
            device_id: device,
            generation: command.generation,
            logo: settings
                .logos
                .into_iter()
                .find(|l| l.school == command.identity.school),
            identity: command.identity,
            configuration_version: version,
            configuration_digest: digest,
            server_time_ms: timestamp(),
            starts_at_ms: settings.starts_at_ms,
            domjudge_api: settings.domjudge_api,
            domjudge_contest: settings.domjudge_contest,
        });
    }
    tx.commit().await?;
    Ok(Json(json!({"data":data})))
}
fn timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
pub async fn report(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Display>,
) -> ApiResult<Json<Value>> {
    if !["online", "offline", "unknown"].contains(&input.oj.state.as_str())
        || input.oj.source != "client-domjudge-api"
        || ![
            "receipt_verified",
            "not_configured",
            "http_unavailable",
            "authentication_required",
            "response_invalid",
        ]
        .contains(&input.oj.detail.as_str())
        || input.wccs.state != "online"
        || input.wccs.source != "authenticated-http"
        || input.wccs.detail != "receipt_verified"
        || input.round_trip_ms > 60_000
    {
        return Err(ApiError::bad("展示诊断回执无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    let row=tx.query_opt("SELECT w.command FROM waiting_commands w JOIN devices d ON d.id=w.device_id WHERE w.id=$1 AND d.id=$2 AND w.generation=d.enrollment_generation AND w.sequence=(SELECT max(sequence) FROM waiting_commands WHERE device_id=$2)",&[&input.data.command_id,&device]).await?.ok_or_else(||ApiError::forbidden("展示回执身份失效"))?;
    let command = serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    if !input.data.matches(&command) {
        return Err(ApiError::conflict("展示缓存身份不匹配"));
    }
    let settings = tx
        .query_opt(
            "SELECT version,settings FROM presentation_settings WHERE contest_id=$1",
            &[&input.data.identity.contest_id],
        )
        .await?;
    let (version, settings) = settings
        .map(|r| (r.get::<_, i64>(0), r.get::<_, Value>(1)))
        .unwrap_or((0, json!(Settings::default())));
    if version != input.data.configuration_version
        || digest(&settings.to_string()) != input.data.configuration_digest
    {
        return Err(ApiError::conflict("展示配置回执已过期"));
    }
    let configured: Settings =
        serde_json::from_value(settings).map_err(|_| ApiError::internal())?;
    if input.data.starts_at_ms != configured.starts_at_ms
        || input.data.domjudge_api != configured.domjudge_api
        || input.data.domjudge_contest != configured.domjudge_contest
        || input.data.logo.as_ref()
            != configured
                .logos
                .iter()
                .find(|logo| logo.school == input.data.identity.school)
    {
        return Err(ApiError::conflict("展示回执内容与配置不匹配"));
    }
    let mut value = json!(input); // Image bytes already reside in settings, avoiding repeated list payloads.
    if let Some(logo) = value["data"]["logo"].as_object_mut() {
        logo.remove("bmp_hex");
    }
    tx.execute("INSERT INTO presentation_receipts VALUES($1,$2,$3,$4) ON CONFLICT(device_id) DO UPDATE SET command_id=$2,report=$3,received_at=$4",&[&device,&input.data.command_id,&value,&now()]).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"accepted":true,"command_id":input.data.command_id}),
    ))
}
