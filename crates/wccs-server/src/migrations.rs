use crate::{Config, auth};
use tokio_postgres::Transaction;

// 顺序对应已发布的数据库版本。新迁移追加到末尾，已有条目的位置决定旧库从哪一步继续。
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_tools.sql"),
    include_str!("../migrations/002_seating.sql"),
    include_str!("../migrations/003_lifecycle.sql"),
    include_str!("../migrations/004_roster.sql"),
    include_str!("../migrations/005_operators_events.sql"),
    include_str!("../migrations/006_waiting.sql"),
    include_str!("../migrations/007_contest_login.sql"),
    include_str!("../migrations/008_diagnostics.sql"),
    include_str!("../migrations/009_maintenance.sql"),
    include_str!("../migrations/010_health.sql"),
    include_str!("../migrations/011_presentation.sql"),
    include_str!("../migrations/012_preparation.sql"),
    include_str!("../migrations/013_image_enrollment.sql"),
    include_str!("../migrations/014_domjudge.sql"),
    include_str!("../migrations/015_distribution.sql"),
    include_str!("../migrations/016_contest_control.sql"),
    include_str!("../migrations/017_event_actions.sql"),
    include_str!("../migrations/018_recovery_archives.sql"),
    include_str!("../migrations/019_review.sql"),
    include_str!("../migrations/020_operator_workflows.sql"),
    include_str!("../migrations/021_maintenance_grants.sql"),
];
pub(crate) const SCHEMA_VERSION: i32 = MIGRATIONS.len() as i32;

pub(crate) async fn apply(
    tx: &Transaction<'_>,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    tx.batch_execute("SELECT pg_advisory_xact_lock(87127413)")
        .await?;
    let exists: bool = tx
        .query_one("SELECT to_regclass('schema_version') IS NOT NULL", &[])
        .await?
        .get(0);
    let installed = if exists {
        let rows = tx.query("SELECT version FROM schema_version", &[]).await?;
        let [row] = rows.as_slice() else {
            return Err("数据库版本不受支持".into());
        };
        let version: i32 = row.get(0);
        if !(1..=SCHEMA_VERSION).contains(&version) {
            return Err("数据库版本不受支持".into());
        }
        version as usize
    } else {
        0
    };

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(installed) {
        tx.batch_execute(sql).await?;
        if index + 1 == 5 {
            // v5 首次创建 operators 表。只在该步填入引导账户，重启时保留管理员后来修改的凭据。
            seed_admin(tx, config).await?;
        }
    }
    Ok(())
}

async fn seed_admin(tx: &Transaction<'_>, config: &Config) -> Result<(), tokio_postgres::Error> {
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
    Ok(())
}
