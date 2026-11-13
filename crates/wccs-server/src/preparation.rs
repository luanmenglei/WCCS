use crate::{ApiError, ApiResult, App, auth, management};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wccs_protocol::{
    now,
    preparation::{Link, Profile, ResolvedTool, Snapshot},
};

pub async fn resolve(tx: &tokio_postgres::Transaction<'_>, version: i64) -> ApiResult<Snapshot> {
    let row=tx.query_opt("SELECT profile FROM preparation_profiles WHERE version=$1 AND version=(SELECT max(version) FROM preparation_profiles)",&[&version]).await?.ok_or_else(||ApiError::conflict("维护配置版本已变化或未发布"))?;
    let profile: Profile = serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    let tools = tools(tx, &profile).await?;
    Ok(Snapshot {
        version,
        profile,
        tools,
    })
}
async fn tools(
    tx: &tokio_postgres::Transaction<'_>,
    profile: &Profile,
) -> ApiResult<Vec<ResolvedTool>> {
    if !profile.valid() {
        return Err(ApiError::bad("维护路径、模板参数或脚本清单无效"));
    }
    let row=tx.query_opt("SELECT c.snapshot FROM configurations c JOIN configuration_head h ON h.version=c.version WHERE c.version=$1",&[&profile.tool_version]).await?.ok_or_else(||ApiError::conflict("工具配置版本已变化或未发布"))?;
    let config: wccs_inventory::tools::ToolConfiguration =
        serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())?;
    let tools: Vec<_> = config
        .tools
        .into_iter()
        .filter_map(|t| t.path.map(|path| ResolvedTool { id: t.id, path }))
        .collect();
    let has_tool = |id: &str| tools.iter().any(|t| t.id == id);
    for link in &profile.links {
        match link {
            Link::Tool { tool_id, .. } if !has_tool(tool_id) => {
                return Err(ApiError::bad("入口引用的工具未配置完整路径"));
            }
            Link::Resource { path, .. } if !wccs_protocol::preparation::directory(path) => {
                return Err(ApiError::bad("资料入口需要完整本地路径"));
            }
            Link::Website { url, .. } => {
                let parsed = url::Url::parse(url).map_err(|_| ApiError::bad("网站入口地址无效"))?;
                if !["http", "https"].contains(&parsed.scheme())
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || url.chars().any(char::is_control)
                    || url.len() > 2048
                {
                    return Err(ApiError::bad("网站入口需要无凭据 HTTP(S) 地址"));
                }
            }
            _ => {}
        }
    }
    if profile
        .package
        .as_ref()
        .is_some_and(|p| !has_tool(&p.runtime_tool_id))
    {
        return Err(ApiError::bad("脚本运行依赖未配置完整路径"));
    }
    Ok(tools)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    request_id: String,
    base_version: i64,
    profile: Profile,
}
pub async fn save(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Save>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    let actor = auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    actor.global()?;
    let payload = json!(input);
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "preparation_save", &payload).await?
    {
        return Ok(Json(previous));
    }
    let version: i64 = tx
        .query_one(
            "SELECT COALESCE(max(version),0) FROM preparation_profiles",
            &[],
        )
        .await?
        .get(0);
    if version != input.base_version {
        return Err(ApiError::conflict("维护配置已变化，请重新加载"));
    }
    tools(&tx, &input.profile).await?;
    let next = version + 1;
    tx.execute(
        "INSERT INTO preparation_profiles VALUES($1,$2,$3)",
        &[&next, &json!(input.profile), &now()],
    )
    .await?;
    let receipt = json!({"request_id":input.request_id,"version":next});
    management::record(
        &tx,
        &input.request_id,
        "preparation_save",
        &payload,
        &receipt,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(receipt))
}
pub async fn get(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.preview_db.lock().await?;
    let tx = db.transaction().await?;
    let _actor = auth::authorize_snapshot(&tx, &session, auth::Permission::Read).await?;

    let row = tx
        .query_opt(
            "SELECT version,profile FROM preparation_profiles ORDER BY version DESC LIMIT 1",
            &[],
        )
        .await?;
    Ok(Json(
        row.map(|r| json!({"version":r.get::<_,i64>(0),"profile":r.get::<_,Value>(1)}))
            .unwrap_or(json!({"version":0,"profile":null})),
    ))
}
