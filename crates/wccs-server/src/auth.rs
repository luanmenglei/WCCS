use crate::{ApiError, ApiResult, App};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, time::Instant};
use wccs_protocol::{digest, now, token};

pub struct Session {
    pub csrf: String,
    pub expires: i64,
}

pub type Sessions = HashMap<String, Session>;

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

pub async fn admin(app: &App, headers: &HeaderMap, write: bool) -> ApiResult<String> {
    let key = digest(cookie(headers).unwrap_or_default());
    let mut sessions = app.sessions.lock().await;
    sessions.retain(|_, session| session.expires > now());
    let session = sessions
        .get(&key)
        .ok_or_else(|| ApiError::unauthorized("管理员会话已失效，请登录"))?;
    if write {
        origin(app, headers)?;
        if headers.get("x-csrf-token").and_then(|v| v.to_str().ok()) != Some(session.csrf.as_str())
        {
            return Err(ApiError::forbidden("CSRF 校验失败"));
        }
    }
    Ok(session.csrf.clone())
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
    let permit = app
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::internal())?;
    let hash = app.config.admin_password_hash.clone();
    let valid = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        PasswordHash::new(&hash).ok().is_some_and(|hash| {
            Argon2::default()
                .verify_password(input.password.as_bytes(), &hash)
                .is_ok()
        })
    })
    .await
    .map_err(|_| ApiError::internal())?;
    if !valid || input.username != app.config.admin_username {
        return Err(ApiError::unauthorized("用户名或密码错误"));
    }
    let credential = token();
    let csrf = token();
    let mut sessions = app.sessions.lock().await;
    sessions.retain(|_, session| session.expires > now());
    if let Some(previous) = cookie(&headers) {
        sessions.remove(&digest(previous));
    }
    if sessions.len() >= 128 {
        return Err(ApiError::new(429, "管理员会话数量达到上限"));
    }
    sessions.insert(
        digest(&credential),
        Session {
            csrf: csrf.clone(),
            expires: now() + 8 * 3600,
        },
    );
    let secure = if app.config.origin.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    Ok(([(header::SET_COOKIE, format!("wccs_session={credential}; HttpOnly; SameSite=Strict; Path=/api; Max-Age=28800{secure}"))], Json(json!({"csrf": csrf, "username": app.config.admin_username}))).into_response())
}

pub async fn session(
    State(app): State<App>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    let csrf = admin(&app, &headers, false).await?;
    Ok(Json(
        json!({"csrf": csrf, "username": app.config.admin_username}),
    ))
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

pub async fn device(app: &App, headers: &HeaderMap) -> ApiResult<String> {
    let credential = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if credential.len() != 64 {
        return Err(ApiError::unauthorized("设备认证失败"));
    }
    let db = app.db.lock().await;
    let row = db
        .query_opt(
            "SELECT id FROM devices WHERE credential_hash=$1",
            &[&digest(credential)],
        )
        .await?;
    row.map(|row| row.get(0))
        .ok_or_else(|| ApiError::unauthorized("设备认证失败"))
}
