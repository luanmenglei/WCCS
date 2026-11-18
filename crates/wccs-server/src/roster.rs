use crate::{ApiError, ApiResult, App, auth, management, seating};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use tokio_postgres::Transaction;
use wccs_protocol::{digest, now, valid_id};

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    id: String,
    code: String,
    name: String,
    school: String,
    seat_id: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create {
        name: String,
    },
    Import {
        text: String,
    },
    Edit {
        participant_id: String,
        code: String,
        name: String,
        school: String,
    },
    Auto {
        room_ids: Vec<String>,
    },
    Assign {
        changes: Vec<Assignment>,
    },
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    participant_id: String,
    seat_id: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    contest_id: String,
    base_version: i64,
    operation: Operation,
    #[serde(default)]
    preview_token: Option<String>,
}

pub(crate) async fn snapshot(tx: &Transaction<'_>) -> ApiResult<Value> {
    let version: i64 = tx
        .query_one("SELECT version FROM roster_head WHERE id=1", &[])
        .await?
        .get(0);
    let mut contests = Vec::new();
    for row in tx
        .query(
            "SELECT id,name,version,remote_contest_id FROM contests ORDER BY created_at,id",
            &[],
        )
        .await?
    {
        let id: String = row.get(0);
        contests.push(json!({"id":id,"name":row.get::<_,String>(1),"version":row.get::<_,i64>(2),"remote_contest_id":row.get::<_,Option<String>>(3),"participants":participants(tx,&id).await?}));
    }
    Ok(json!({"version":version,"contests":contests}))
}
async fn participants(tx: &Transaction<'_>, id: &str) -> ApiResult<Vec<Participant>> {
    Ok(tx.query("SELECT p.id,p.code,p.name,p.school,a.seat_id FROM participants p LEFT JOIN assignments a ON a.contest_id=p.contest_id AND a.participant_id=p.id WHERE p.contest_id=$1 ORDER BY p.code COLLATE \"C\"", &[&id]).await?.iter().map(|row| Participant {
        id: row.get(0), code: row.get(1), name: row.get(2), school: row.get(3), seat_id: row.get(4),
    }).collect())
}
fn fields(code: &str, name: &str, school: &str) -> Result<(), String> {
    for (label, value, max) in [
        ("参赛标识", code, 80),
        ("姓名", name, 100),
        ("学校", school, 200),
    ] {
        if value.is_empty()
            || value.chars().count() > max
            || value.chars().any(char::is_control)
            || value.contains('\u{fffd}')
        {
            return Err(format!(
                "{label} 必填，最多 {max} 字符，禁止控制字符及无效编码"
            ));
        }
    }
    Ok(())
}
struct ImportRow {
    line: usize,
    code: String,
    name: String,
    school: String,
}
fn parse(text: &str) -> (Vec<ImportRow>, Vec<Value>) {
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(|line| line.trim_end_matches('\r')) != Some("code\tname\tschool") {
        errors.push(json!({"line":1,"message":"表头必须为 code、name、school 三列，以制表符分隔"}));
    }
    let mut codes = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        if line.trim().is_empty() {
            continue;
        }
        let columns: Vec<_> = line.split('\t').map(str::trim).collect();
        if columns.len() != 3 {
            errors.push(json!({"line":number,"message":"必须为三列，不支持引号转义或多行字段"}));
            continue;
        }
        if let Err(message) = fields(columns[0], columns[1], columns[2]) {
            errors.push(json!({"line":number,"message":message}));
        }
        if let Some(previous) = codes.insert(columns[0], number) {
            errors.push(
                json!({"line":number,"message":format!("参赛标识重复，与第 {previous} 行冲突")}),
            );
        }
        rows.push(ImportRow {
            line: number,
            code: columns[0].into(),
            name: columns[1].into(),
            school: columns[2].into(),
        });
    }
    if rows.is_empty() || rows.len() > 500 {
        errors.push(json!({"line":1,"message":"每批需要 1～500 名选手"}));
    }
    (rows, errors)
}

struct Plan {
    preview: Value,
    after: Vec<Participant>,
    name: String,
    create: bool,
}
async fn plan(tx: &Transaction<'_>, input: &Write) -> ApiResult<Plan> {
    if !valid_id(&input.request_id) || !valid_id(&input.contest_id) {
        return Err(ApiError::bad("请求或比赛 ID 无效"));
    }
    let row = tx
        .query_opt(
            "SELECT name,version FROM contests WHERE id=$1",
            &[&input.contest_id],
        )
        .await?;
    let create = matches!(input.operation, Operation::Create { .. });
    let (name, version) = if create {
        if row.is_some() {
            return Err(ApiError::conflict("比赛 ID 已存在"));
        }
        let count: i64 = tx
            .query_one("SELECT count(*) FROM contests", &[])
            .await?
            .get(0);
        if count >= 100 {
            return Err(ApiError::bad("本切片最多保存 100 场比赛"));
        }
        (
            String::new(),
            tx.query_one("SELECT version FROM roster_head WHERE id=1", &[])
                .await?
                .get::<_, i64>(0),
        )
    } else {
        let row = row.ok_or_else(|| ApiError::bad("比赛不存在，请显式选择比赛"))?;
        (row.get(0), row.get(1))
    };
    if version != input.base_version {
        return Err(ApiError::conflict(
            "比赛或比赛列表版本已变化，请重新加载并预览",
        ));
    }
    let before = participants(tx, &input.contest_id).await?;
    let mut after = before.clone();
    let mut name = name;
    let mut errors = Vec::new();
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    let state = seating::snapshot(tx).await?;
    let rooms = state["rooms"].as_array().unwrap();
    let seats: Vec<_> = rooms
        .iter()
        .flat_map(|room| room["seats"].as_array().unwrap())
        .collect();
    let devices = state["devices"].as_array().unwrap();
    let blocked = |seat: &Value| {
        seat["status"] == "conflict"
            || seat["devices"].as_array().unwrap().iter().any(|id| {
                devices
                    .iter()
                    .any(|device| device["id"] == *id && device["lifecycle"] == "disabled")
            })
    };
    match &input.operation {
        Operation::Create { name: title } => {
            name = title.trim().into();
            if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
                return Err(ApiError::bad("比赛名称需要 1～100 字符"));
            }
        }
        Operation::Import { text } => {
            let (rows, validation) = parse(text);
            errors = validation;
            for ImportRow {
                line,
                code,
                name,
                school,
            } in rows
            {
                let old = before.iter().find(|p| p.code == code);
                // Deterministic proposed ID is scoped to this contest. Existing IDs survive code edits.
                let candidate = Participant {
                    id: old.map_or_else(
                        || digest(&json!([input.contest_id, code, version]).to_string()),
                        |p| p.id.clone(),
                    ),
                    code,
                    name,
                    school,
                    seat_id: old.and_then(|p| p.seat_id.clone()),
                };
                let kind = if errors.iter().any(|error| error["line"] == line) {
                    "conflict"
                } else if old == Some(&candidate) {
                    "unchanged"
                } else if old.is_some() {
                    "modified"
                } else {
                    "added"
                };
                changes.push(json!({"line":line,"kind":kind,"before":old,"after":candidate}));
                if let Some(existing) = after.iter_mut().find(|p| p.id == candidate.id) {
                    *existing = candidate;
                } else {
                    after.push(candidate);
                }
            }
        }
        Operation::Edit {
            participant_id,
            code,
            name,
            school,
        } => {
            fields(code.trim(), name.trim(), school.trim())
                .map_err(|message| ApiError::bad(&message))?;
            let p = after
                .iter_mut()
                .find(|p| p.id == *participant_id)
                .ok_or_else(|| ApiError::bad("选手不属于本场比赛"))?;
            p.code = code.trim().into();
            p.name = name.trim().into();
            p.school = school.trim().into();
        }
        Operation::Auto { room_ids } => {
            let unique: HashSet<_> = room_ids.iter().collect();
            if unique.is_empty()
                || unique.len() != room_ids.len()
                || room_ids
                    .iter()
                    .any(|id| !rooms.iter().any(|room| room["id"] == *id))
            {
                return Err(ApiError::bad("请选择存在且不重复的机房"));
            }
            let occupied: HashSet<_> = before.iter().filter_map(|p| p.seat_id.as_ref()).collect();
            let mut available = Vec::new();
            // Stable room ID byte order, then numeric display number, independent of database collation.
            let mut selected: Vec<_> = rooms
                .iter()
                .filter(|room| room_ids.iter().any(|id| room["id"] == *id))
                .collect();
            selected.sort_by_key(|room| room["id"].as_str().unwrap());
            for room in selected {
                let mut ordered: Vec<_> = room["seats"].as_array().unwrap().iter().collect();
                ordered.sort_by_key(|seat| seat["number"].as_i64().unwrap());
                for seat in ordered {
                    let id = seat["id"].as_str().unwrap().to_string();
                    if occupied.contains(&id) || blocked(seat) {
                        skipped.push(json!({"seat_id":id,"reason":if occupied.contains(&id) {"occupied"} else {"blocked"}}));
                    } else {
                        available.push(id);
                    }
                }
            }
            after.sort_by(|a, b| a.code.cmp(&b.code));
            let needed = after.iter().filter(|p| p.seat_id.is_none()).count();
            if available.len() < needed {
                errors.push(json!({"line":0,"message":format!("容量不足，需要 {needed} 个座位，可用 {} 个",available.len())}));
            }
            for (p, id) in after
                .iter_mut()
                .filter(|p| p.seat_id.is_none())
                .zip(available)
            {
                p.seat_id = Some(id);
            }
        }
        Operation::Assign { changes: edits } => {
            let mut ids = HashSet::new();
            if edits.is_empty() || edits.len() > 500 {
                return Err(ApiError::bad("请选择 1～500 个调整目标"));
            }
            for edit in edits {
                if !ids.insert(&edit.participant_id) {
                    return Err(ApiError::bad("调整目标重复"));
                }
                let p = after
                    .iter_mut()
                    .find(|p| p.id == edit.participant_id)
                    .ok_or_else(|| ApiError::bad("选手不属于本场比赛"))?;
                p.seat_id = edit.seat_id.clone();
            }
        }
    }
    let mut codes = HashSet::new();
    let mut assigned = HashSet::new();
    for p in &after {
        if !codes.insert(&p.code) {
            errors.push(json!({"line":0,"message":"同场参赛标识重复"}));
        }
        if let Some(id) = &p.seat_id {
            if !assigned.insert(id) {
                errors.push(json!({"line":0,"message":"座位重复占用，请一起调整或交换原选手"}));
            }
            let seat = seats
                .iter()
                .find(|seat| seat["id"] == *id)
                .ok_or_else(|| ApiError::conflict("座位已移除，请重新选择"))?;
            let unchanged = before
                .iter()
                .any(|old| old.id == p.id && old.seat_id == p.seat_id);
            if !unchanged && blocked(seat) {
                errors.push(
                    json!({"line":0,"message":"目标座位存在地址冲突或禁用设备，请先处理设备状态"}),
                );
            }
        }
    }
    if after.len() > 500 {
        errors.push(json!({"line":0,"message":"同场最多 500 名选手"}));
    }
    if !matches!(input.operation, Operation::Import { .. }) {
        for p in &after {
            let old = before.iter().find(|old| old.id == p.id);
            changes.push(json!({"line":0,"kind":if old==Some(p) {"unchanged"} else {"modified"},"before":old,"after":p}));
        }
    }
    // Full room/address facts exclude heartbeat timestamps, but include online transitions.
    let mut payload = serde_json::to_value(input).unwrap();
    payload.as_object_mut().unwrap().remove("request_id");
    payload.as_object_mut().unwrap().remove("preview_token");
    let preview_token = digest(&json!([payload, before, after, state]).to_string());
    let preview = json!({"contest_id":input.contest_id,"name":name,"base_version":version,"preview_token":preview_token,"changes":changes,"errors":errors,"skipped":skipped,"rooms":rooms,"total":after.len()});
    Ok(Plan {
        preview,
        after,
        name,
        create,
    })
}

pub async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    seating::lock(&tx).await?;
    Ok(Json(plan(&tx, &input).await?.preview))
}
pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    seating::lock(&tx).await?;
    if let Some(previous) =
        management::previous(&tx, &input.request_id, "roster_commit", &payload).await?
    {
        return Ok(Json(previous));
    }
    let plan = plan(&tx, &input).await?;
    if !plan.preview["errors"].as_array().unwrap().is_empty() {
        return Err(ApiError::bad("预览存在错误，整批未提交，请修正后重新预览"));
    }
    if input.preview_token.as_deref() != plan.preview["preview_token"].as_str() {
        return Err(ApiError::conflict(
            "预览已失效，名单、机房或设备状态发生变化，请重新预览",
        ));
    }
    let version = if plan.create {
        1
    } else {
        input.base_version + 1
    };
    if plan.create {
        tx.execute(
            "INSERT INTO contests VALUES ($1,$2,$3,$4)",
            &[&input.contest_id, &plan.name, &version, &now()],
        )
        .await?;
        tx.execute("UPDATE roster_head SET version=version+1 WHERE id=1", &[])
            .await?;
    } else {
        tx.execute(
            "UPDATE contests SET version=$2 WHERE id=$1",
            &[&input.contest_id, &version],
        )
        .await?;
    }
    for p in &plan.after {
        tx.execute("INSERT INTO participants (id,contest_id,code,name,school) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (id) DO UPDATE SET code=$3,name=$4,school=$5", &[&p.id,&input.contest_id,&p.code,&p.name,&p.school]).await?;
    }
    tx.execute(
        "DELETE FROM assignments WHERE contest_id=$1",
        &[&input.contest_id],
    )
    .await?;
    for p in &plan.after {
        if let Some(id) = &p.seat_id {
            tx.execute(
                "INSERT INTO assignments VALUES ($1,$2,$3)",
                &[&input.contest_id, &p.id, id],
            )
            .await?;
        }
    }
    let response = management::remember(
        &tx,
        &app,
        &input.request_id,
        "roster_commit",
        &payload,
        version,
    )
    .await?;
    tx.execute("INSERT INTO roster_events (contest_id,request_id,version,result,created_at) VALUES ($1,$2,$3,$4,$5)", &[&input.contest_id,&input.request_id,&version,&plan.preview,&now()]).await?;
    tx.commit().await?;
    Ok(Json(response))
}
#[derive(Deserialize)]
pub struct Cursor {
    #[serde(default)]
    before: Option<i64>,
}
pub async fn history(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Read)
        .await?
        .global()?;
    let rows=tx.query("SELECT sequence,request_id,version,result,created_at FROM roster_events WHERE contest_id=$1 AND sequence<$2 ORDER BY sequence DESC LIMIT 50", &[&id,&cursor.before.unwrap_or(i64::MAX)]).await?;
    Ok(Json(json!(rows.iter().map(|row| json!({"sequence":row.get::<_,i64>(0),"request_id":row.get::<_,String>(1),"version":row.get::<_,i64>(2),"result":row.get::<_,Value>(3),"created_at":row.get::<_,i64>(4)})).collect::<Vec<_>>())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tsv_validation_and_normalization() {
        let (rows, errors) =
            parse("\u{feff}code\tname\tschool\r\n 001 \t 合成 姓名 \t 合成学校 \r\n\n");
        assert!(errors.is_empty());
        assert_eq!(rows[0].code, "001");
        assert_eq!(rows[0].name, "合成 姓名");
        for text in [
            "code,name,school\na,b,c",
            "code\tname\tschool\nA\t甲\t校\n A \t乙\t校",
            "code\tname\tschool\nA\t\t校",
            "code\tname\tschool\nA\t甲\t坏\u{fffd}",
        ] {
            assert!(!parse(text).1.is_empty());
        }
    }
}
