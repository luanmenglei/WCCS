//! Inspect a restored deployment without running startup migrations or contest workers.
use crate::{App, Config, database::Database, validate_config};
use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

pub(crate) const SCHEMA_VERSION: i32 = 20;

/// Database-wide ownership for the lifetime of the serving process. Never reconnect
/// this session: losing it requires a fresh startup and retirement of the old epoch.
pub struct ControllerLease {
    _client: tokio_postgres::Client,
    driver: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}

impl ControllerLease {
    pub async fn acquire(config: &Config) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let connection = Database::connection_config(&config.database_url)?;
        let (client, driver) = tokio::time::timeout(
            Duration::from_secs(4),
            connection.connect(tokio_postgres::NoTls),
        )
        .await??;
        let driver = tokio::spawn(driver);
        let acquired: bool = tokio::time::timeout(
            Duration::from_secs(4),
            client.query_one("SELECT pg_try_advisory_lock(87127414::bigint)", &[]),
        )
        .await??
        .get(0);
        Ok(acquired.then_some(Self {
            _client: client,
            driver,
        }))
    }

    pub async fn lost(&mut self) {
        let _ = (&mut self.driver).await;
    }
}

async fn inspect_database(db: &Database) -> Result<(i32, bool), &'static str> {
    let mut client = db.lock().await.map_err(|_| "数据库连接不可用")?;
    let tx = client
        .transaction()
        .await
        .map_err(|_| "无法查询数据库状态")?;
    tx.batch_execute("SET LOCAL statement_timeout='2s'")
        .await
        .map_err(|_| "数据库检查初始化失败")?;
    let row = tx.query_one(
        "SELECT version, pg_is_in_recovery() OR current_setting('transaction_read_only') = 'on' FROM schema_version",
        &[],
    ).await.map_err(|_| "无法读取唯一的数据库版本，请检查目标数据库及 schema")?;
    let version = row.try_get(0).map_err(|_| "数据库版本字段无效")?;
    let read_only = row.try_get(1).map_err(|_| "数据库状态无效")?;
    tx.commit().await.map_err(|_| "数据库检查未完成")?;
    Ok((version, read_only))
}

async fn database_status(db: &Database) -> Result<(i32, bool), &'static str> {
    tokio::time::timeout(Duration::from_secs(4), inspect_database(db))
        .await
        .map_err(|_| "数据库就绪检查超时")?
}

pub async fn check_standby(config: &Config) -> Result<Value, Box<dyn std::error::Error>> {
    validate_config(config)?;
    let index =
        std::fs::File::open(Path::new(&config.static_dir).join("index.html"))?.metadata()?;
    if !index.is_file() || index.len() == 0 {
        return Err("管理台入口 index.html 不可用".into());
    }
    let db = Database::new(&config.database_url)?;
    let (schema, read_only) = database_status(&db).await?;
    if !(1..=SCHEMA_VERSION).contains(&schema) {
        return Err("备用数据库版本不在当前服务支持的迁移范围内".into());
    }
    Ok(
        json!({"origin":config.origin, "database_schema":schema, "target_schema":SCHEMA_VERSION,
        "migration_required":schema != SCHEMA_VERSION, "database_read_only":read_only}),
    )
}

pub(crate) async fn ready(State(app): State<App>) -> (StatusCode, Json<Value>) {
    let ready = matches!(
        database_status(&app.readiness_db).await,
        Ok((SCHEMA_VERSION, false))
    );
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(json!({"service":"wccs","ready":ready})),
    )
}
