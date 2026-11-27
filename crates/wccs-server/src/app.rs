use crate::{auth, contest_control, database, domjudge, migrations};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::{Mutex, Semaphore};

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
    pub(crate) readiness_db: database::Database,
    pub archive_db: database::Database,
    pub archive_upload_db: database::Database,
    pub preview_db: database::Database,
    pub sessions: Mutex<auth::Sessions>,
    pub password_slots: Arc<Semaphore>,
    pub download_slots: Semaphore,
    pub(crate) download_waiters: Semaphore,
    pub receipt_db: database::Database,
    pub receipt_slots: Semaphore,
    pub archive_slots: Arc<Semaphore>,
    pub download_limits: Mutex<HashMap<String, Instant>>,
    pub auth_limits: Mutex<auth::RateLimits>,
    pub(crate) domjudge: Option<domjudge::Remote>,
}

pub fn listen_address(config: &Config) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    let address: std::net::SocketAddr = config.bind.parse()?;
    if !address.ip().is_loopback() {
        return Err("明文后端仅允许监听回环地址，请通过本机 HTTPS 代理访问".into());
    }
    Ok(address)
}

pub(crate) fn validate_config(
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
    {
        let mut db = database.lock().await.map_err(|_| "数据库连接不可用")?;
        let tx = db.transaction().await?;
        migrations::apply(&tx, &config).await?;
        retire_previous_controller(&tx).await?;
        tx.commit().await?;
    }
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
        download_waiters: Semaphore::new(500),
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

// 同一事务完成迁移、撤销旧授权，再让新控制器的后台任务开始接单。
async fn retire_previous_controller(
    tx: &tokio_postgres::Transaction<'_>,
) -> Result<(), tokio_postgres::Error> {
    tx.execute(
        "UPDATE domjudge_jobs
         SET stage='interrupted',detail='server_restarted_review_required',updated_at=$1
         WHERE stage IN ('queued','running')",
        &[&wccs_protocol::now()],
    )
    .await?;
    tx.execute(
        "UPDATE maintenance_jobs
         SET cancel_requested=true,stage=CASE WHEN delivered THEN stage ELSE 'cancelled' END
         WHERE stage IN ('queued','accepted','executing')",
        &[],
    )
    .await?;
    tx.execute(
        "UPDATE contest_control_jobs
         SET stage='interrupted',detail='server_restarted_review_required',updated_at=$1
         WHERE stage IN ('scheduled','running')",
        &[&wccs_protocol::now()],
    )
    .await?;
    tx.execute(
        "UPDATE waiting_commands
         SET stage='cancelled',updated_at=$1
         WHERE stage NOT IN ('failed','cancelled','expired')",
        &[&wccs_protocol::now()],
    )
    .await?;
    tx.execute("UPDATE control_devices SET connected=false", &[])
        .await?;
    Ok(())
}
