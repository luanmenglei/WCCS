#![forbid(unsafe_code)]

mod auth;
mod control;
mod device;
mod events;
mod lifecycle;
mod management;
mod operators;
mod roster;
mod seating;
mod waiting;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use tokio_postgres::{Client, NoTls};
use tower_http::services::ServeDir;

pub type ApiResult<T> = Result<T, ApiError>;
pub type App = Arc<AppState>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bind: String,
    pub origin: String,
    pub database_url: String,
    pub static_dir: String,
    #[serde(default)]
    pub trusted_proxies: Vec<std::net::IpAddr>,
    pub admin_username: String,
    pub admin_password_hash: String,
}

pub struct AppState {
    pub config: Config,
    pub server_epoch: String,
    pub db: Mutex<Client>,
    pub sessions: Mutex<auth::Sessions>,
    pub password_slots: Arc<Semaphore>,
    pub auth_limits: Mutex<HashMap<&'static str, (Instant, u32)>>,
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}
impl ApiError {
    pub fn new(status: u16, message: &str) -> Self {
        Self {
            status: StatusCode::from_u16(status).unwrap(),
            message: message.into(),
        }
    }
    pub fn bad(message: &str) -> Self {
        Self::new(400, message)
    }
    pub fn unauthorized(message: &str) -> Self {
        Self::new(401, message)
    }
    pub fn forbidden(message: &str) -> Self {
        Self::new(403, message)
    }
    pub fn conflict(message: &str) -> Self {
        Self::new(409, message)
    }
    pub fn internal() -> Self {
        Self::new(500, "服务器存储或内部操作失败，请重试并查看服务日志")
    }
}
impl From<tokio_postgres::Error> for ApiError {
    fn from(error: tokio_postgres::Error) -> Self {
        eprintln!("database_error code={:?}", error.code());
        Self::internal()
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

pub async fn connect(config: Config) -> Result<App, Box<dyn std::error::Error>> {
    let address: std::net::SocketAddr = config.bind.parse()?;
    let local_origin = config.origin == format!("http://127.0.0.1:{}", address.port())
        || config.origin == format!("http://localhost:{}", address.port());
    if !config.origin.starts_with("https://") && !(address.ip().is_loopback() && local_origin) {
        return Err("HTTP 仅允许本地回环验证，部署请配置 HTTPS 反向代理与 origin".into());
    }
    argon2::PasswordHash::new(&config.admin_password_hash).map_err(|_| "管理员密码摘要无效")?;
    let (mut db, connection) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("database_connection_closed code={:?}", error.code());
        }
    });
    let tx = db.transaction().await?;
    tx.batch_execute("SELECT pg_advisory_xact_lock(87127413)")
        .await?;
    let exists: bool = tx
        .query_one("SELECT to_regclass('schema_version') IS NOT NULL", &[])
        .await?
        .get(0);
    if !exists {
        tx.batch_execute(include_str!("../migrations/001_tools.sql"))
            .await?;
    }
    let versions = tx.query("SELECT version FROM schema_version", &[]).await?;
    if versions.len() != 1 || !matches!(versions[0].get::<_, i32>(0), 1..=6) {
        return Err("数据库版本不受支持".into());
    }
    if versions[0].get::<_, i32>(0) == 1 {
        tx.batch_execute(include_str!("../migrations/002_seating.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 2 {
        tx.batch_execute(include_str!("../migrations/003_lifecycle.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 3 {
        tx.batch_execute(include_str!("../migrations/004_roster.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 4 {
        tx.batch_execute(include_str!("../migrations/005_operators_events.sql"))
            .await?;
        tx.execute(
            "INSERT INTO operators VALUES($1,$2,$3,$2,'admin',true,true,'{}',1,$4,$5)",
            &[
                &wccs_protocol::token(),
                &config.admin_username,
                &auth::login_key(&config.admin_username),
                &config.admin_password_hash,
                &wccs_protocol::now(),
            ],
        )
        .await?;
    }
    if versions[0].get::<_, i32>(0) <= 5 {
        tx.batch_execute(include_str!("../migrations/006_waiting.sql"))
            .await?;
    }
    tx.execute("UPDATE waiting_commands SET stage='cancelled',updated_at=$1 WHERE stage NOT IN ('failed','cancelled','expired')", &[&wccs_protocol::now()]).await?;
    tx.execute("UPDATE control_devices SET connected=false", &[])
        .await?;
    tx.commit().await?;
    Ok(Arc::new(AppState {
        config,
        server_epoch: wccs_protocol::token(),
        db: Mutex::new(db),
        sessions: Mutex::new(HashMap::new()),
        password_slots: Arc::new(Semaphore::new(2)),
        auth_limits: Mutex::new(HashMap::new()),
    }))
}

async fn response_policy(request: Request, next: Next) -> Response {
    let mut response = match tokio::time::timeout(Duration::from_secs(15), next.run(request)).await
    {
        Ok(response) => response,
        Err(_) => ApiError::new(504, "请求超时，可使用相同请求编号重试").into_response(),
    };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "same-origin"),
        (
            "content-security-policy",
            "default-src 'self'; style-src 'self'; font-src 'self'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/api/waiting/preview", post(waiting::preview))
        .route("/api/waiting/commit", post(waiting::commit))
        .route("/api/waiting", get(waiting::list))
        .route("/api/device/control", get(control::upgrade))
        .route("/api/login", post(auth::login))
        .route("/api/session", get(auth::session))
        .route("/api/logout", post(auth::logout))
        .route("/api/operators", get(operators::list).post(operators::save))
        .route("/api/events", get(events::list))
        .route("/api/events/{id}", get(events::detail))
        .route("/api/state", get(management::state))
        .route("/api/roster/preview", post(roster::preview))
        .route("/api/roster/commit", post(roster::commit))
        .route("/api/roster/{id}/history", get(roster::history))
        .route("/api/devices", post(management::create_device))
        .route("/api/registration-window", post(lifecycle::window))
        .route("/api/devices/action", post(lifecycle::action))
        .route("/api/devices/{id}/history", get(lifecycle::history))
        .route("/api/device/rotate", post(lifecycle::rotate))
        .route("/api/configuration", post(management::publish))
        .route("/api/recheck", post(management::recheck))
        .route("/api/rooms/preview", post(seating::preview))
        .route("/api/rooms/save", post(seating::save))
        .route("/api/seating/check-preview", post(seating::check_preview))
        .route("/api/seating/check", post(seating::check))
        .route("/api/seating/review", post(seating::review))
        .route("/api/device/register", post(device::register))
        .route("/api/device/heartbeat", post(device::heartbeat))
        .route("/api/device/result", post(device::result))
        .fallback_service(ServeDir::new(&app.config.static_dir))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn(response_policy))
        .with_state(app)
}
