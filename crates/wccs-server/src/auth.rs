use crate::{ApiError, ApiResult, App};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, time::Instant};
use tokio_postgres::Transaction;
use wccs_protocol::{digest, now, token};

#[derive(Clone)]
pub struct Session {
    pub operator_id: String,
    pub version: i64,
    pub csrf: String,
    pub expires: i64,
}

pub type Sessions = HashMap<String, Session>;

#[derive(Clone, Serialize)]
pub struct Identity {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub all_rooms: bool,
    pub room_ids: Vec<String>,
    pub version: i64,
}
#[derive(Clone, Copy)]
pub enum Permission {
    Read,
    Operate,
    Admin,
}

pub fn login_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

// The configuration fence serializes authorization with permission edits and business writes.
// Cookie inspection releases the session mutex before entering this transaction.
pub async fn authorize(
    tx: &Transaction<'_>,
    session: &Session,
    permission: Permission,
) -> ApiResult<Identity> {
    crate::seating::lock(tx).await?;
    authorize_snapshot(tx, session, permission).await
}

// Read-only previews authorize against their consistent snapshot. Commit rechecks
// the current operator version and room scope under the configuration fence.
pub async fn authorize_snapshot(
    tx: &Transaction<'_>,
    session: &Session,
    permission: Permission,
) -> ApiResult<Identity> {
    let row = tx.query_opt("SELECT id,username,display_name,role,all_rooms,room_ids,version FROM operators WHERE id=$1 AND enabled AND version=$2", &[&session.operator_id,&session.version]).await?
        .filter(|_| session.expires > now())
        .ok_or_else(|| ApiError::unauthorized("账号或权限已变化，请重新登录并核对当前授权"))?;
    let identity = Identity {
        id: row.get(0),
        username: row.get(1),
        display_name: row.get(2),
        role: row.get(3),
        all_rooms: row.get(4),
        room_ids: row.get(5),
        version: row.get(6),
    };
    let allowed = match permission {
        Permission::Read => true,
        Permission::Operate => identity.role != "viewer",
        Permission::Admin => identity.role == "admin",
    };
    if !allowed {
        return Err(ApiError::forbidden("当前角色无权执行此操作"));
    }
    tx.query_one(
        "SELECT set_config('wccs.actor_id',$1,true),set_config('wccs.actor_name',$2,true)",
        &[&identity.id, &identity.username],
    )
    .await?;
    Ok(identity)
}

pub async fn context(
    tx: &Transaction<'_>,
    request: &str,
    rooms: Option<&[String]>,
    devices: &[String],
) -> ApiResult<()> {
    tx.query_one("SELECT set_config('wccs.request_id',$1,true),set_config('wccs.room_ids',COALESCE($2::text[]::text,''),true),set_config('wccs.device_ids',$3::text[]::text,true)", &[&request,&rooms,&devices]).await?;
    Ok(())
}

impl Identity {
    pub fn global(&self) -> ApiResult<()> {
        if !self.all_rooms {
            return Err(ApiError::forbidden(
                "此查询包含全场或未知范围历史，需要全场授权",
            ));
        }
        Ok(())
    }
    pub fn rooms(&self, rooms: Option<&[String]>) -> ApiResult<()> {
        if self.all_rooms
            || rooms.is_some_and(|ids| {
                !ids.is_empty() && ids.iter().all(|id| self.room_ids.contains(id))
            })
        {
            return Ok(());
        }
        Err(ApiError::forbidden(
            "目标不在授权机房范围内，或归属尚未核对",
        ))
    }
    pub async fn devices(
        &self,
        tx: &Transaction<'_>,
        ids: &[String],
        request: &str,
    ) -> ApiResult<()> {
        let mut rooms = Vec::new();
        let mut known = true;
        for id in ids {
            let scope: Option<Vec<String>> = tx
                .query_one("SELECT device_room_scope($1)", &[id])
                .await?
                .get(0);
            self.rooms(scope.as_deref())?;
            if let Some(scope) = scope {
                rooms.extend(scope);
            } else {
                known = false;
            }
        }
        rooms.sort();
        rooms.dedup();
        context(tx, request, if known { Some(&rooms) } else { None }, ids).await
    }
    pub async fn seats(
        &self,
        tx: &Transaction<'_>,
        ids: &[String],
        request: &str,
    ) -> ApiResult<()> {
        let rows = tx
            .query("SELECT id,room_id FROM seats WHERE id=ANY($1)", &[&ids])
            .await?;
        if !self.all_rooms
            && (rows.len() != ids.len()
                || rows
                    .iter()
                    .any(|r| !self.room_ids.contains(&r.get::<_, String>(1))))
        {
            return Err(ApiError::forbidden("选择的座位不在授权范围内"));
        }
        let rooms: Vec<String> = rows.iter().map(|r| r.get(1)).collect();
        context(tx, request, Some(&rooms), &[]).await
    }
}

pub async fn verify(app: &App, password: String, hash: String) -> ApiResult<bool> {
    let permit = app
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::internal())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        PasswordHash::new(&hash).ok().is_some_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
    })
    .await
    .map_err(|_| ApiError::internal())
}

fn session_value(session: &Session, identity: &Identity) -> serde_json::Value {
    let mut value = serde_json::to_value(identity).unwrap();
    value["csrf"] = json!(session.csrf);
    value
}

pub fn origin(app: &App, headers: &HeaderMap) -> ApiResult<()> {
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(app.config.origin.as_str())
    {
        return Err(ApiError::forbidden("请求来源不匹配"));
    }
    Ok(())
}

fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| part.trim().strip_prefix("wccs_session="))
}

pub async fn admin(app: &App, headers: &HeaderMap, write: bool) -> ApiResult<Session> {
    let key = digest(cookie(headers).unwrap_or_default());
    let mut sessions = app.sessions.lock().await;
    sessions.retain(|_, session| session.expires > now());
    let session = sessions
        .get(&key)
        .ok_or_else(|| ApiError::unauthorized("管理员会话已失效，请登录"))?;
    if let Some(binding) = headers.get("x-wccs-session")
        && binding.to_str().ok() != Some(session.csrf.as_str())
    {
        return Err(ApiError::unauthorized("浏览器身份已切换，请重新登录"));
    }
    if write {
        origin(app, headers)?;
        if headers.get("x-csrf-token").and_then(|v| v.to_str().ok()) != Some(session.csrf.as_str())
        {
            return Err(ApiError::forbidden("CSRF 校验失败"));
        }
    }
    Ok(session.clone())
}

pub async fn rate_limit(app: &App, scope: &'static str, maximum: u32) -> ApiResult<()> {
    let mut limits = app.auth_limits.lock().await;
    let window = limits.entry(scope).or_insert((Instant::now(), 0));
    if window.0.elapsed().as_secs() >= 60 {
        *window = (Instant::now(), 0);
    }
    window.1 += 1;
    if window.1 > maximum {
        return Err(ApiError::new(429, "认证请求过多，请稍后重试"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    username: String,
    password: String,
}

pub async fn login(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Login>,
) -> ApiResult<Response> {
    origin(&app, &headers)?;
    rate_limit(&app, "login", 30).await?;
    if input.password.len() > 256 {
        return Err(ApiError::unauthorized("用户名或密码错误"));
    }
    let row = app
        .db
        .lock()
        .await?
        .query_opt(
            "SELECT id,password_hash,version FROM operators WHERE login_key=$1",
            &[&login_key(&input.username)],
        )
        .await?;
    let hash = row
        .as_ref()
        .map(|r| r.get::<_, String>(1))
        .unwrap_or_else(|| app.config.admin_password_hash.clone());
    if !verify(&app, input.password, hash).await? || row.is_none() {
        return Err(ApiError::unauthorized("用户名或密码错误"));
    }
    let row = row.unwrap();
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let session = Session {
        csrf: token(),
        expires: now() + 8 * 3600,
        operator_id: row.get(0),
        version: row.get(2),
    };
    let identity = authorize(&tx, &session, Permission::Read).await?;
    let response = session_value(&session, &identity);
    let credential = token();
    let mut sessions = app.sessions.lock().await;
    sessions.retain(|_, session| session.expires > now());
    if let Some(previous) = cookie(&headers) {
        sessions.remove(&digest(previous));
    }
    if sessions.len() >= 128 {
        return Err(ApiError::new(429, "管理员会话数量达到上限"));
    }
    sessions.insert(digest(&credential), session);
    tx.commit().await?;
    let secure = if app.config.origin.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    Ok(([(header::SET_COOKIE, format!("wccs_session={credential}; HttpOnly; SameSite=Strict; Path=/api; Max-Age=28800{secure}"))], Json(response)).into_response())
}

pub async fn session(
    State(app): State<App>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    let session = admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let identity = authorize(&tx, &session, Permission::Read).await?;
    Ok(Json(session_value(&session, &identity)))
}

pub async fn logout(State(app): State<App>, headers: HeaderMap) -> ApiResult<Response> {
    admin(&app, &headers, true).await?;
    app.sessions
        .lock()
        .await
        .remove(&digest(cookie(&headers).unwrap_or_default()));
    Ok((
        [(
            header::SET_COOKIE,
            "wccs_session=; HttpOnly; SameSite=Strict; Path=/api; Max-Age=0",
        )],
        Json(json!({"ok": true})),
    )
        .into_response())
}

pub fn credential_hash(headers: &HeaderMap) -> ApiResult<String> {
    let credential = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if credential.len() != 64 {
        return Err(ApiError::unauthorized("设备认证失败"));
    }
    Ok(digest(credential))
}

// The caller holds the configuration fence through authentication and mutation.
pub async fn device(
    tx: &tokio_postgres::Transaction<'_>,
    headers: &HeaderMap,
) -> ApiResult<String> {
    let row = tx.query_opt("SELECT id FROM devices WHERE credential_hash=$1 AND lifecycle IN ('enabled','disabled')", &[&credential_hash(headers)?]).await?
        .ok_or_else(||ApiError::unauthorized("设备认证失败"))?;
    let id: String = row.get(0);
    if let Some(claim) = headers.get("x-wccs-device-id")
        && claim.to_str().ok() != Some(&id)
    {
        return Err(ApiError::forbidden("设备身份不匹配"));
    }
    Ok(id)
}
