#![forbid(unsafe_code)]

mod auth;
mod contest_binding;
mod contest_control;
mod control;
mod database;
pub mod deployment;
mod device;
mod diagnostics;
mod distribution;
mod domjudge;
mod events;
mod health;
mod image;
mod lifecycle;
mod maintenance;
mod management;
mod operators;
mod preparation;
mod presentation;
mod recovery;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domjudge_config: Option<String>,
    pub admin_username: String,
    pub admin_password_hash: String,
}

pub struct AppState {
    pub config: Config,
    pub server_epoch: String,
    pub db: database::Database,
    readiness_db: database::Database,
    pub archive_db: database::Database,
    pub archive_upload_db: database::Database,
    pub preview_db: database::Database,
    pub sessions: Mutex<auth::Sessions>,
    pub password_slots: Arc<Semaphore>,
    pub download_slots: Semaphore,
    pub receipt_db: database::Database,
    pub receipt_slots: Semaphore,
    pub archive_slots: Arc<Semaphore>,
    pub download_limits: Mutex<HashMap<String, Instant>>,
    pub auth_limits: Mutex<auth::RateLimits>,
    pub(crate) domjudge: Option<domjudge::Remote>,
}

#[derive(Debug)]
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
    pub fn unavailable() -> Self {
        Self::new(503, "数据库连接暂时不可用，可使用相同请求编号重试")
    }
}
impl From<tokio_postgres::Error> for ApiError {
    fn from(error: tokio_postgres::Error) -> Self {
        eprintln!("database_error code={:?}", error.code());
        if error.is_closed()
            || std::error::Error::source(&error).is_some_and(|source| source.is::<std::io::Error>())
            || error.code().is_some_and(|code| {
                code.code().starts_with("08") || matches!(code.code(), "57P01" | "57P02" | "57P03")
            })
        {
            return Self::unavailable();
        }
        Self::internal()
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

pub fn listen_address(config: &Config) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    let address: std::net::SocketAddr = config.bind.parse()?;
    if !address.ip().is_loopback() {
        return Err("明文后端仅允许监听回环地址，请通过本机 HTTPS 代理访问".into());
    }
    Ok(address)
}

fn validate_config(
    config: &Config,
) -> Result<Option<domjudge::Remote>, Box<dyn std::error::Error>> {
    let address: std::net::SocketAddr = listen_address(config)?;
    let origin = url::Url::parse(&config.origin)?;
    if origin.origin().ascii_serialization() != config.origin {
        return Err("origin 需与浏览器的协议、主机和端口完全一致".into());
    }
    let local_origin = config.origin == format!("http://127.0.0.1:{}", address.port())
        || config.origin == format!("http://localhost:{}", address.port());
    if !config.origin.starts_with("https://") && !(address.ip().is_loopback() && local_origin) {
        return Err("HTTP 仅允许本地回环验证，部署请配置 HTTPS 反向代理与 origin".into());
    }
    argon2::PasswordHash::new(&config.admin_password_hash).map_err(|_| "管理员密码摘要无效")?;
    config
        .domjudge_config
        .as_deref()
        .map(domjudge::Remote::load)
        .transpose()
}

pub async fn connect(config: Config) -> Result<App, Box<dyn std::error::Error>> {
    let domjudge = validate_config(&config)?;
    let database = database::Database::new(&config.database_url)?;
    let preview_db = database::Database::new(&config.database_url)?;
    let mut db = database.lock().await.map_err(|_| "数据库连接不可用")?;
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
    if versions.len() != 1
        || !(1..=deployment::SCHEMA_VERSION).contains(&versions[0].get::<_, i32>(0))
    {
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
    if versions[0].get::<_, i32>(0) <= 6 {
        tx.batch_execute(include_str!("../migrations/007_contest_login.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 7 {
        tx.batch_execute(include_str!("../migrations/008_diagnostics.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 8 {
        tx.batch_execute(include_str!("../migrations/009_maintenance.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 9 {
        tx.batch_execute(include_str!("../migrations/010_health.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 10 {
        tx.batch_execute(include_str!("../migrations/011_presentation.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 11 {
        tx.batch_execute(include_str!("../migrations/012_preparation.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 12 {
        tx.batch_execute(include_str!("../migrations/013_image_enrollment.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 13 {
        tx.batch_execute(include_str!("../migrations/014_domjudge.sql"))
            .await?;
    }
    tx.execute("UPDATE domjudge_jobs SET stage='interrupted',detail='server_restarted_review_required',updated_at=$1 WHERE stage IN ('queued','running')", &[&wccs_protocol::now()]).await?;
    if versions[0].get::<_, i32>(0) <= 14 {
        tx.batch_execute(include_str!("../migrations/015_distribution.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 15 {
        tx.batch_execute(include_str!("../migrations/016_contest_control.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 16 {
        tx.batch_execute(include_str!("../migrations/017_event_actions.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 17 {
        tx.batch_execute(include_str!("../migrations/018_recovery_archives.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 18 {
        tx.batch_execute(include_str!("../migrations/019_review.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 19 {
        tx.batch_execute(include_str!("../migrations/020_operator_workflows.sql"))
            .await?;
    }
    if versions[0].get::<_, i32>(0) <= 20 {
        tx.batch_execute(include_str!("../migrations/021_maintenance_grants.sql"))
            .await?;
    }
    tx.execute("UPDATE maintenance_jobs SET cancel_requested=true,stage=CASE WHEN delivered THEN stage ELSE 'cancelled' END WHERE stage IN ('queued','accepted','executing')", &[]).await?;
    tx.execute("UPDATE contest_control_jobs SET stage='interrupted',detail='server_restarted_review_required',updated_at=$1 WHERE stage IN ('scheduled','running')", &[&wccs_protocol::now()]).await?;
    tx.execute("UPDATE waiting_commands SET stage='cancelled',updated_at=$1 WHERE stage NOT IN ('failed','cancelled','expired')", &[&wccs_protocol::now()]).await?;
    tx.execute("UPDATE control_devices SET connected=false", &[])
        .await?;
    tx.commit().await?;
    drop(db);
    let app = Arc::new(AppState {
        receipt_db: database::Database::new(&config.database_url)?,
        readiness_db: database::Database::new(&config.database_url)?,
        archive_db: database::Database::new(&config.database_url)?,
        archive_upload_db: database::Database::new(&config.database_url)?,
        config,
        server_epoch: wccs_protocol::token(),
        db: database,
        preview_db,
        sessions: Mutex::new(HashMap::new()),
        password_slots: Arc::new(Semaphore::new(2)),
        download_slots: Semaphore::new(16),
        receipt_slots: Semaphore::new(2),
        archive_slots: Arc::new(Semaphore::new(2)),
        download_limits: Mutex::new(HashMap::new()),
        auth_limits: Mutex::new(HashMap::new()),
        domjudge,
    });
    domjudge::start(&app);
    contest_control::start(&app);
    Ok(app)
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

async fn authenticate_receipt(
    axum::extract::State(app): axum::extract::State<App>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        auth::device(&tx, request.headers()).await?;
        tx.commit().await?;
    }
    let _permit = app
        .receipt_slots
        .try_acquire()
        .map_err(|_| ApiError::new(429, "维护回执接收繁忙，请稍后重试"))?;
    Ok(next.run(request).await)
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/api/health", get(|| async { Json(json!({"service":"wccs","protocol_version":1,"server_time":wccs_protocol::now()})) }))
        .route("/api/ready", get(deployment::ready))
        .route("/api/waiting/preview", post(waiting::preview))
        .route("/api/waiting/commit", post(waiting::commit))
        .route("/api/waiting", get(waiting::list))
        .route("/api/device/control", get(control::upgrade))
        .route("/api/device/diagnostics/context", get(diagnostics::context))
        .route("/api/device/diagnostics/probe", get(diagnostics::probe))
        .route("/api/presentation", get(presentation::get).post(presentation::save))
        .route("/api/device/presentation", post(presentation::device))
        .route("/api/device/presentation/report", post(presentation::report))
        .route("/api/device/health", post(health::report))
        .route("/api/device/diagnostics", post(diagnostics::receive))
        .route("/api/maintenance/preview", post(maintenance::preview))
        .route("/api/maintenance/commit", post(maintenance::commit))
        .route("/api/preparation", get(preparation::get).post(preparation::save))
        .route("/api/contest-control", get(contest_control::list))
        .route("/api/contest-control/preview", post(contest_control::preview))
        .route("/api/contest-control/commit", post(contest_control::commit))
        .route("/api/contest-control/cancel", post(contest_control::cancel))
        .route("/api/domjudge", get(domjudge::list))
        .route("/api/domjudge/bind", post(contest_binding::bind))
        .route("/api/domjudge/choices", get(contest_binding::choices))
        .route("/api/domjudge/preview", post(domjudge::preview))
        .route("/api/domjudge/commit", post(domjudge::commit))
        .route("/api/distribution", get(distribution::list))
        .route("/api/distribution/artifacts", post(distribution::create))
        .route("/api/distribution/artifacts/{id}/{index}", post(distribution::upload))
        .route("/api/distribution/preview", post(distribution::preview))
        .route("/api/distribution/commit", post(distribution::commit))
        .route("/api/distribution/cancel", post(distribution::cancel))
        .route("/api/device/distribution/poll", post(distribution::poll))
        .route("/api/device/distribution/progress", post(distribution::progress))
        .route("/api/device/desktop-files/{job}/{artifact}/{index}", get(distribution::desktop_chunk))
        .route("/api/device/distribution/{job}/{index}", get(distribution::download))
        .route("/api/maintenance", get(maintenance::list))
        .route("/api/maintenance/cancel", post(maintenance::cancel))
        .route("/api/device/maintenance/poll", post(maintenance::poll))
        .route("/api/device/maintenance/authorize", post(maintenance::authorize))
        .route("/api/device/maintenance/result", post(maintenance::result).layer(DefaultBodyLimit::max(wccs_protocol::recovery::MAX_MANIFEST_BYTES)).layer(axum::middleware::from_fn_with_state(app.clone(), authenticate_receipt)))
        .route("/api/login", post(auth::login))
        .route("/api/session", get(auth::session))
        .route("/api/logout", post(auth::logout))
        .route("/api/operators", get(operators::list).post(operators::save))
        .route("/api/recovery", get(recovery::list))
        .route("/api/recovery/manifest/{job}", get(recovery::manifest))
        .route("/api/recovery/archives/{job}", get(recovery::download))
        .route("/api/device/recovery/{job}/{file}/{index}", post(recovery::upload))
        .route("/api/device/recovery/{job}/finish", post(recovery::finish))
        .route("/api/events", get(events::list))
        .route("/api/events/action", post(events::action))
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
        .route("/api/device/image-register", post(image::register))
        .route("/api/images/action", post(image::action))
        .route("/api/device/heartbeat", post(device::heartbeat))
        .route("/api/device/result", post(device::result))
        .fallback_service(ServeDir::new(&app.config.static_dir))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn(response_policy))
        .with_state(app)
}
