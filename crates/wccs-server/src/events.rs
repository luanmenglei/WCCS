use crate::{ApiError, ApiResult, App, auth};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wccs_protocol::digest;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    from: Option<i64>,
    to: Option<i64>,
    actor_id: Option<String>,
    operation: Option<String>,
    category: Option<String>,
    device_id: Option<String>,
    room_id: Option<String>,
    contest_id: Option<String>,
    request_id: Option<String>,
    outcome: Option<String>,
    limit: Option<i64>,
    #[serde(skip_serializing)]
    cursor: Option<String>,
}

async fn query(
    app: &App,
    headers: &HeaderMap,
    filter: Filter,
    id: Option<i64>,
) -> ApiResult<Value> {
    let session = auth::admin(app, headers, false).await?;
    let limit = filter.limit.unwrap_or(50);
    if !(1..=100).contains(&limit)
        || filter.from.zip(filter.to).is_some_and(|(a, b)| a > b)
        || [
            &filter.actor_id,
            &filter.operation,
            &filter.category,
            &filter.device_id,
            &filter.room_id,
            &filter.contest_id,
            &filter.request_id,
            &filter.outcome,
        ]
        .iter()
        .any(|v| v.as_ref().is_some_and(|s| s.len() > 200))
    {
        return Err(ApiError::bad("事件筛选或每页数量无效"));
    }
    let fingerprint = digest(&serde_json::to_string(&filter).unwrap());
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let identity = auth::authorize(&tx, &session, auth::Permission::Read).await?;
    let maximum: i64 = tx
        .query_one("SELECT COALESCE(max(id),0) FROM events WHERE ($1 OR (cardinality(room_ids)>0 AND room_ids<@$2)) AND ($3 OR operation<>'operator_save')", &[&identity.all_rooms,&identity.room_ids,&(identity.role=="admin")])
        .await?
        .get(0);
    let (ceiling, before_time, before_id) = if let Some(cursor) = &filter.cursor {
        let parts: Vec<_> = cursor.split('.').collect();
        if parts.len() != 4 || parts[3] != fingerprint {
            return Err(ApiError::bad("游标与筛选条件不匹配，请重新查询"));
        }
        let numbers = parts[..3]
            .iter()
            .map(|s| s.parse::<i64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ApiError::bad("事件游标无效"))?;
        if numbers[0] > maximum || numbers.iter().any(|n| *n < 0) {
            return Err(ApiError::bad("事件游标无效"));
        }
        (numbers[0], numbers[1], numbers[2])
    } else {
        (maximum, i64::MAX, i64::MAX)
    };
    // Apply authorization before filters and pagination. No unfiltered counts or facets escape.
    let rows = tx
        .query(
            "SELECT * FROM events WHERE id<=$1 AND (occurred_at,id)<($2,$3)
        AND ($4 OR (cardinality(room_ids)>0 AND room_ids<@$5))
        AND ($6 OR operation<>'operator_save')
        AND ($7::bigint IS NULL OR occurred_at>=$7) AND ($8::bigint IS NULL OR occurred_at<=$8)
        AND ($9::text IS NULL OR actor_id=$9) AND ($10::text IS NULL OR operation=$10)
        AND ($11::text IS NULL OR category=$11) AND ($12::text IS NULL OR $12=ANY(device_ids))
        AND ($13::text IS NULL OR $13=ANY(room_ids)) AND ($14::text IS NULL OR contest_id=$14)
        AND ($15::text IS NULL OR request_id=$15) AND ($16::text IS NULL OR outcome=$16)
        AND ($17::bigint IS NULL OR id=$17) ORDER BY occurred_at DESC,id DESC LIMIT $18",
            &[
                &ceiling,
                &before_time,
                &before_id,
                &identity.all_rooms,
                &identity.room_ids,
                &(identity.role == "admin"),
                &filter.from,
                &filter.to,
                &filter.actor_id,
                &filter.operation,
                &filter.category,
                &filter.device_id,
                &filter.room_id,
                &filter.contest_id,
                &filter.request_id,
                &filter.outcome,
                &id,
                &(limit + 1),
            ],
        )
        .await?;
    let more = rows.len() > limit as usize;
    let items: Vec<Value>=rows.iter().take(limit as usize).map(|row|json!({
        "id":row.get::<_,i64>("id").to_string(),"source":row.get::<_,String>("source"),"source_id":row.get::<_,String>("source_id"),
        "category":row.get::<_,String>("category"),"occurred_at":row.get::<_,i64>("occurred_at"),
        "actor_id":row.get::<_,Option<String>>("actor_id"),"actor_name":row.get::<_,Option<String>>("actor_name"),
        "operation":row.get::<_,String>("operation"),"target":row.get::<_,Option<String>>("target"),
        "device_ids":row.get::<_,Vec<String>>("device_ids"),"room_ids":row.get::<_,Option<Vec<String>>>("room_ids"),
        "contest_id":row.get::<_,Option<String>>("contest_id"),"request_id":row.get::<_,Option<String>>("request_id"),
        "version":row.get::<_,Option<i64>>("version"),"outcome":row.get::<_,String>("outcome"),
        "scope_kind":scope_kind(row),"detail":row.get::<_,Value>("detail"),"legacy":row.get::<_,bool>("legacy")
    })).collect();
    if id.is_some() {
        return items
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::new(404, "事件不存在或不在授权范围内"));
    }
    let cursor = if more {
        let last = &rows[limit as usize - 1];
        Some(format!(
            "{ceiling}.{}.{}.{fingerprint}",
            last.get::<_, i64>("occurred_at"),
            last.get::<_, i64>("id")
        ))
    } else {
        None
    };
    Ok(json!({"items":items,"next_cursor":cursor}))
}

pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> ApiResult<Json<Value>> {
    Ok(Json(query(&app, &headers, filter, None).await?))
}
pub async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        query(&app, &headers, Filter::default(), Some(id)).await?,
    ))
}

fn scope_kind(row: &tokio_postgres::Row) -> &'static str {
    if row.get::<_, Option<Vec<String>>>("room_ids").is_some() {
        return "rooms";
    }
    if !row.get::<_, bool>("legacy")
        && [
            "operator_save",
            "publish",
            "registration_window",
            "roster_commit",
            "roster_result",
        ]
        .contains(&row.get::<_, String>("operation").as_str())
    {
        return "global";
    }
    "unknown"
}
