//! DOMjudge account changes are durable jobs with explicit previews and session revocation.
use crate::{ApiError, ApiResult, App, auth, management, seating};
use axum::{Json, extract::State, http::HeaderMap};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};
use tokio_postgres::Transaction;
use wccs_protocol::{digest, now, token, valid_id};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    url: String,
    username: String,
    password: String,
    ca_file: Option<String>,
}

pub(crate) struct Remote {
    connection: Connection,
    client: Client,
}

impl Remote {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let connection: Connection = serde_json::from_slice(&std::fs::read(path)?)?;
        let url = url::Url::parse(&connection.url)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || connection.username.is_empty()
            || connection.password.is_empty()
        {
            return Err("DOMjudge 需要 HTTPS 地址及独立 API 账号材料".into());
        }
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(path) = &connection.ca_file {
            builder = builder
                .add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(path)?)?);
        }
        Ok(Self {
            connection,
            client: builder.build()?,
        })
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        endpoint: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut request = self
            .client
            .request(
                method,
                format!(
                    "{}/api/v4/{endpoint}",
                    self.connection.url.trim_end_matches('/')
                ),
            )
            .basic_auth(&self.connection.username, Some(&self.connection.password));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                "remote_timeout"
            } else {
                "remote_connection_failed"
            }
            .to_owned()
        })?;
        if response.status().as_u16() == 404 {
            return Ok(Value::Null);
        }
        if !response.status().is_success() {
            return Err(format!("remote_http_{}", response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "remote_response_lost")? {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("remote_response_too_large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| "remote_response_invalid".into())
    }

    pub(crate) async fn get(&self, endpoint: &str) -> Result<Value, String> {
        self.request(Method::GET, endpoint, None).await
    }

    async fn user(
        &self,
        target: &Value,
        ip: Option<&str>,
        enabled: bool,
        create: bool,
    ) -> Result<(), String> {
        let username = string(target, "username")?;
        let body = json!({"id":username,"username":username,"name":target["name"],"email":null,
            "ip":ip,"password":format!("{}{}",token(),token()),"enabled":enabled,
            "team_id":username,"roles":["team"]});
        let (method, endpoint) = if create {
            (Method::POST, "users".to_owned())
        } else {
            (Method::PUT, format!("users/{username}"))
        };
        let result = self.request(method, &endpoint, Some(body)).await?;
        if result["username"] != username
            || result["ip"].as_str() != ip
            || result["enabled"] != enabled
        {
            return Err("remote_user_postcondition_failed".into());
        }
        Ok(())
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| "remote_or_job_data_invalid".into())
}

fn remote_user_snapshot(user: &Value) -> Value {
    if user.is_null() {
        return Value::Null;
    }
    json!({"id":user["id"],"username":user["username"],"name":user["name"],
        "team_id":user["team_id"],"roles":user["roles"],"ip":user["ip"],"enabled":user["enabled"]})
}

fn binding_digest(input: &Write, target: &Value) -> String {
    let mut target = target.clone();
    target.as_object_mut().unwrap().remove("remote_user");
    digest(
        &json!({"target":target,"contest":input.remote_contest_id,"group":input.group_id})
            .to_string(),
    )
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    contest_id: String,
    remote_contest_id: String,
    group_id: String,
    participant_ids: Vec<String>,
    #[serde(default)]
    preview_token: Option<String>,
}

async fn local_plan(tx: &Transaction<'_>, input: &Write) -> ApiResult<Value> {
    if !valid_id(&input.request_id)
        || !valid_id(&input.contest_id)
        || !wccs_protocol::preparation::domjudge_id(&input.remote_contest_id)
        || !wccs_protocol::preparation::domjudge_id(&input.group_id)
        || input.participant_ids.is_empty()
        || input.participant_ids.len() > 500
        || input.participant_ids.iter().any(|id| !valid_id(id))
        || input.participant_ids.iter().collect::<HashSet<_>>().len() != input.participant_ids.len()
    {
        return Err(ApiError::bad("DOMjudge 比赛、分组或选手范围无效"));
    }
    let rows = tx.query("SELECT p.id,p.code,p.name,p.school,a.seat_id,b.remote_contest_id FROM participants p LEFT JOIN domjudge_bindings b ON b.participant_id=p.id LEFT JOIN assignments a ON a.participant_id=p.id WHERE p.contest_id=$1 AND p.id=ANY($2) ORDER BY p.id", &[&input.contest_id,&input.participant_ids]).await?;
    if rows.len() != input.participant_ids.len() {
        return Err(ApiError::bad("选手不属于本场比赛"));
    }
    crate::contest_binding::require(tx, &input.contest_id, &input.remote_contest_id).await?;
    let seats = seating::snapshot(tx).await?;
    let devices = tx
        .query(
            "SELECT id,enrollment_generation,address_version,lifecycle FROM devices",
            &[],
        )
        .await?;
    let mut targets = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        let seat_id: Option<String> = row.get("seat_id");
        let seat = seats["rooms"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|room| {
                room["seats"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|seat| seat["id"].as_str() == seat_id.as_deref())
                    .map(|seat| (room, seat))
            });
        let username = format!(
            "wccs-{}",
            &digest(&format!("{}:{id}", input.contest_id))[..30]
        );
        let mut target = json!({"participant_id":id,"code":row.get::<_,String>("code"),"name":row.get::<_,String>("name"),
            "school":row.get::<_,String>("school"),"username":username,"seat_id":seat_id,"reason":"seat_unassigned"});
        if seat_id.is_none()
            && row.get::<_, Option<String>>("remote_contest_id").as_deref()
                == Some(&input.remote_contest_id)
        {
            target["revoke_only"] = json!(true);
            target["reason"] = Value::Null;
        }
        if let Some((room, seat)) = seat {
            target["room_id"] = room["id"].clone();
            target["room_version"] = room["version"].clone();
            target["seat_label"] = json!(format!(
                "{} / {}",
                room["name"].as_str().unwrap_or(""),
                seat["number"]
            ));
            target["ip"] = seat["expected_ip"].clone();
            target["device_id"] = seat["devices"][0].clone();
            target["reason"] = if seat["status"] == "online" {
                Value::Null
            } else {
                json!("seat_not_ready")
            };
            if let Some(device) = devices.iter().find(|device| {
                Some(device.get::<_, String>("id").as_str()) == target["device_id"].as_str()
            }) {
                target["generation"] = json!(device.get::<_, i64>("enrollment_generation"));
                target["address_version"] = json!(device.get::<_, i64>("address_version"));
                if device.get::<_, String>("lifecycle") != "enabled" {
                    target["reason"] = json!("device_disabled");
                }
            }
        }
        targets.push(target);
    }
    Ok(
        json!({"contest_id":input.contest_id,"remote_contest_id":input.remote_contest_id,"group_id":input.group_id,"targets":targets}),
    )
}

async fn preview_value(app: &App, session: &auth::Session, input: &Write) -> ApiResult<Value> {
    let mut plan = {
        let mut db = app.preview_db.lock().await?;
        let tx = db
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await?;
        auth::authorize_snapshot(&tx, session, auth::Permission::Admin)
            .await?
            .global()?;
        local_plan(&tx, input).await?
    };
    let remote = app
        .domjudge
        .as_ref()
        .ok_or_else(|| ApiError::bad("服务器尚未配置 DOMjudge API 连接材料"))?;
    let groups = remote
        .get(&format!("contests/{}/groups", input.remote_contest_id))
        .await
        .map_err(|reason| ApiError::new(502, &format!("DOMjudge 核对失败 {reason}")))?;
    if !groups
        .as_array()
        .is_some_and(|groups| groups.iter().any(|group| group["id"] == input.group_id))
    {
        return Err(ApiError::bad(
            "DOMjudge 比赛分组不存在，请核对比赛 ID 和分组 ID",
        ));
    }
    let users = remote
        .get("users")
        .await
        .map_err(|reason| ApiError::new(502, &format!("DOMjudge 账号读取失败 {reason}")))?;
    let users = users
        .as_array()
        .ok_or_else(|| ApiError::new(502, "DOMjudge 账号响应无效"))?;
    let selected: HashSet<_> = plan["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|target| target["username"].as_str().map(str::to_owned))
        .collect();
    for target in plan["targets"].as_array_mut().unwrap() {
        let user = users
            .iter()
            .find(|user| user["username"] == target["username"]);
        target["remote_user"] = remote_user_snapshot(user.unwrap_or(&Value::Null));
        if user.is_some_and(|user| {
            user["team_id"] != target["username"] || user["roles"] != json!(["team"])
        }) {
            target["reason"] = json!("remote_account_conflict");
        }
        if !target["ip"].is_null()
            && users.iter().any(|user| {
                user["enabled"] == true
                    && user["ip"] == target["ip"]
                    && !selected.contains(user["username"].as_str().unwrap_or(""))
            })
        {
            target["reason"] = json!("remote_ip_conflict");
        }
    }
    plan["preview_token"] = json!(digest(&plan.to_string()));
    Ok(plan)
}

pub async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    Ok(Json(preview_value(&app, &session, &input).await?))
}

pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let payload = json!(input);
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        auth::authorize(&tx, &session, auth::Permission::Admin)
            .await?
            .global()?;
        if let Some(previous) =
            management::previous(&tx, &input.request_id, "domjudge_sync", &payload).await?
        {
            return Ok(Json(previous));
        }
    }
    let plan = preview_value(&app, &session, &input).await?;
    if plan["preview_token"].as_str() != input.preview_token.as_deref() {
        return Err(ApiError::conflict(
            "名单、座位或 DOMjudge 绑定已变化，请重新核对",
        ));
    }
    if plan["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|target| !target["reason"].is_null())
    {
        return Err(ApiError::conflict("所选范围包含未核对项，请修复后重新预览"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    seating::lock(&tx).await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    actor.global()?;
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "domjudge_sync", &payload).await?
    {
        return Ok(Json(previous));
    }
    let current = local_plan(&tx, &input).await?;
    for (current, frozen) in current["targets"]
        .as_array()
        .unwrap()
        .iter()
        .zip(plan["targets"].as_array().unwrap())
    {
        let mut frozen = frozen.clone();
        frozen.as_object_mut().unwrap().remove("remote_user");
        if *current != frozen {
            return Err(ApiError::conflict("座位状态已变化，请重新核对"));
        }
    }
    if tx
        .query_opt(
            "SELECT id FROM domjudge_jobs WHERE stage IN ('queued','running')",
            &[],
        )
        .await?
        .is_some()
    {
        return Err(ApiError::conflict(
            "已有 DOMjudge 同步任务正在执行，请等待结果",
        ));
    }
    tx.execute("INSERT INTO domjudge_jobs(id,actor_id,actor_version,stage,payload,plan,created_at,updated_at) VALUES($1,$2,$3,'queued',$4,$5,$6,$6)",
        &[&input.request_id,&actor.id,&actor.version,&payload,&plan,&now()]).await?;
    let response =
        json!({"request_id":input.request_id,"job_id":input.request_id,"stage":"queued"});
    management::record(&tx, &input.request_id, "domjudge_sync", &payload, &response).await?;
    tx.commit().await?;
    Ok(Json(response))
}

pub async fn list(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize_snapshot(&tx, &session, auth::Permission::Read)
        .await?
        .global()?;
    let rows = tx.query("SELECT id,stage,plan,results,detail,updated_at FROM domjudge_jobs ORDER BY created_at DESC,id LIMIT 30", &[]).await?;
    let bindings = tx.query("SELECT participant_id,remote_contest_id,username,ip,updated_at FROM domjudge_bindings ORDER BY participant_id", &[]).await?;
    Ok(Json(
        json!({"configured":app.domjudge.is_some(),"url":app.domjudge.as_ref().map(|remote| &remote.connection.url),
        "bindings":bindings.iter().map(|row|json!({"participant_id":row.get::<_,String>(0),"remote_contest_id":row.get::<_,String>(1),"username":row.get::<_,String>(2),"ip":row.get::<_,String>(3),"updated_at":row.get::<_,i64>(4)})).collect::<Vec<_>>(),
        "jobs":rows.iter().map(|row|json!({"id":row.get::<_,String>(0),"stage":row.get::<_,String>(1),"plan":row.get::<_,Value>(2),
            "results":row.get::<_,Value>(3),"detail":row.get::<_,String>(4),"updated_at":row.get::<_,i64>(5)})).collect::<Vec<_>>()}),
    ))
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
                eprintln!("domjudge_worker_error status={}", error.status);
            }
        }
    });
}

async fn work(app: &App) -> ApiResult<()> {
    let row = app.db.lock().await?.query_opt("UPDATE domjudge_jobs SET stage='running',updated_at=$1 WHERE id=(SELECT id FROM domjudge_jobs WHERE stage='queued' ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING id,payload,plan,actor_id,actor_version", &[&now()]).await?;
    let Some(row) = row else {
        return Ok(());
    };
    let id: String = row.get("id");
    let input: Write =
        serde_json::from_value(row.get("payload")).map_err(|_| ApiError::internal())?;
    let plan: Value = row.get("plan");
    let actor_id: String = row.get("actor_id");
    let actor_version: i64 = row.get("actor_version");
    let mut results = Vec::new();
    let result = execute(
        app,
        &id,
        &input,
        &plan,
        &actor_id,
        actor_version,
        &mut results,
    )
    .await;
    let (stage, detail) = match result {
        Ok(()) => ("succeeded", String::new()),
        Err(reason) => ("failed", reason),
    };
    loop {
        let saved: ApiResult<()> = async {
            app.db.lock().await?.execute("UPDATE domjudge_jobs SET stage=$2,results=$3,detail=$4,updated_at=$5 WHERE id=$1",
                &[&id,&stage,&json!(results),&detail,&now()]).await?;
            Ok(())
        }.await;
        if saved.is_ok() {
            return Ok(());
        }
        // Persist the final result after recovery without repeating any remote mutation.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn still_authorized(
    app: &App,
    input: &Write,
    plan: &Value,
    actor_id: &str,
    actor_version: i64,
) -> Result<(), String> {
    let mut db = app
        .db
        .lock()
        .await
        .map_err(|_| "local_database_unavailable")?;
    let tx = db
        .transaction()
        .await
        .map_err(|_| "local_database_unavailable")?;
    let actor = tx.query_opt("SELECT 1 FROM operators WHERE id=$1 AND version=$2 AND enabled AND all_rooms AND role='admin'", &[&actor_id,&actor_version])
        .await.map_err(|_| "local_database_unavailable")?;
    if actor.is_none() {
        return Err("operator_authorization_changed".into());
    }
    let current = local_plan(&tx, input)
        .await
        .map_err(|_| "local_target_changed")?;
    for (current, original) in current["targets"]
        .as_array()
        .unwrap()
        .iter()
        .zip(plan["targets"].as_array().unwrap())
    {
        let mut original = original.clone();
        original.as_object_mut().unwrap().remove("remote_user");
        if *current != original {
            return Err("local_target_changed".into());
        }
    }
    Ok(())
}

async fn save_progress(app: &App, job: &str, results: &[Value]) -> Result<(), String> {
    app.db
        .lock()
        .await
        .map_err(|_| "local_database_unavailable")?
        .execute(
            "UPDATE domjudge_jobs SET results=$2,updated_at=$3 WHERE id=$1",
            &[&job, &json!(results), &now()],
        )
        .await
        .map_err(|_| "local_database_unavailable")?;
    Ok(())
}

async fn execute(
    app: &App,
    job: &str,
    input: &Write,
    plan: &Value,
    actor_id: &str,
    actor_version: i64,
    results: &mut Vec<Value>,
) -> Result<(), String> {
    let remote = app.domjudge.as_ref().ok_or("remote_not_configured")?;
    let targets = plan["targets"].as_array().ok_or("job_data_invalid")?;
    // Revoke every selected old session before assigning any new IP, including seat swaps.
    for (index, target) in targets.iter().enumerate() {
        still_authorized(app, input, plan, actor_id, actor_version).await?;
        let username = string(target, "username")?;
        results.push(json!({"participant_id":target["participant_id"],"username":username,"stage":"checking","detail":"checking_remote_account"}));
        save_progress(app, job, results).await?;
        let current = remote.get(&format!("users/{username}")).await?;
        if remote_user_snapshot(&current) != target["remote_user"] {
            return Err("remote_account_changed".into());
        }
        let team = remote.get(&format!("teams/{username}")).await?;
        let marker = format!("WCCS {}", string(target, "participant_id")?);
        if !team.is_null() && team["public_description"] != marker {
            return Err("remote_team_conflict".into());
        }
        if target["revoke_only"] == true {
            if !current.is_null() && (current["enabled"] == true || !current["ip"].is_null()) {
                remote.user(target, None, false, false).await?;
            }
            results[index]["stage"] = json!("succeeded");
            results[index]["detail"] = json!("unassigned_account_revoked");
            save_progress(app, job, results).await?;
            continue;
        }
        let previous = app
            .db
            .lock()
            .await
            .map_err(|_| "local_database_unavailable")?
            .query_opt(
                "SELECT target_digest FROM domjudge_bindings WHERE participant_id=$1",
                &[&string(target, "participant_id")?],
            )
            .await
            .map_err(|_| "local_database_unavailable")?;
        if previous.is_some_and(|row| row.get::<_, String>(0) == binding_digest(input, target))
            && current["enabled"] == true
            && current["ip"] == target["ip"]
            && current["name"] == target["name"]
            && !team.is_null()
            && team["group_ids"] == json!([input.group_id])
            && team["name"]
                == format!(
                    "{} / {}",
                    string(target, "name")?,
                    string(target, "school")?
                )
        {
            results[index]["stage"] = json!("succeeded");
            results[index]["detail"] = json!("already_synced");
            save_progress(app, job, results).await?;
            continue;
        }
        results[index]["stage"] = json!("revoking");
        results[index]["detail"] = json!("remote_result_pending");
        save_progress(app, job, results).await?;
        remote.request(Method::POST, "teams", Some(json!({"id":username,"name":format!("{} / {}",string(target,"name")?,string(target,"school")?),
            "label":target["code"],"public_description":marker,"group_ids":[input.group_id]}))).await?;
        remote.user(target, None, false, current.is_null()).await?;
        results[index]["stage"] = json!("revoked");
        results[index]["detail"] = json!("old_session_revoked");
        save_progress(app, job, results).await?;
    }
    for (index, target) in targets.iter().enumerate() {
        if target["revoke_only"] == true || results[index]["detail"] == "already_synced" {
            continue;
        }
        still_authorized(app, input, plan, actor_id, actor_version).await?;
        let ip = string(target, "ip")?;
        let users = remote.get("users").await?;
        if users
            .as_array()
            .ok_or("remote_response_invalid")?
            .iter()
            .any(|user| user["enabled"] == true && user["ip"] == ip)
        {
            return Err("remote_ip_conflict".into());
        }
        results[index]["stage"] = json!("activating");
        results[index]["detail"] = json!("remote_result_pending");
        save_progress(app, job, results).await?;
        remote.user(target, Some(ip), true, false).await?;
        let validation = still_authorized(app, input, plan, actor_id, actor_version).await;
        if let Err(reason) = validation {
            let revoked = remote.user(target, None, false, false).await;
            return Err(if revoked.is_ok() {
                reason
            } else {
                "target_changed_remote_recovery_required".into()
            });
        }
        let visible = remote
            .get(&format!(
                "contests/{}/teams/{}",
                input.remote_contest_id,
                string(target, "username")?
            ))
            .await?;
        if visible.is_null() {
            remote.user(target, None, false, false).await?;
            return Err("remote_team_not_in_contest".into());
        }
        let users = remote.get("users").await?;
        if users
            .as_array()
            .ok_or("remote_response_invalid")?
            .iter()
            .filter(|user| user["enabled"] == true && user["ip"] == ip)
            .count()
            != 1
        {
            remote.user(target, None, false, false).await?;
            return Err("remote_ip_conflict_after_write".into());
        }
        let target_digest = binding_digest(input, target);
        app.db.lock().await.map_err(|_| "local_database_unavailable")?.execute(
            "INSERT INTO domjudge_bindings VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(participant_id) DO UPDATE SET remote_contest_id=$2,username=$3,ip=$4,target_digest=$5,updated_at=$6",
            &[&string(target,"participant_id")?,&input.remote_contest_id,&string(target,"username")?,&ip,&target_digest,&now()]).await.map_err(|_| "local_database_unavailable")?;
        results[index]["stage"] = json!("succeeded");
        results[index]["detail"] = json!("account_ip_verified");
        save_progress(app, job, results).await?;
    }
    let revoked: Vec<_> = targets
        .iter()
        .filter(|t| t["revoke_only"] == true)
        .filter_map(|t| t["participant_id"].as_str())
        .collect();
    if !revoked.is_empty() {
        app.db.lock().await.map_err(|_| "local_database_unavailable")?
            .execute("DELETE FROM domjudge_bindings WHERE participant_id=ANY($1) AND remote_contest_id=$2", &[&revoked,&input.remote_contest_id])
            .await.map_err(|_| "local_database_unavailable")?;
    }
    Ok(())
}
