use crate::{ApiError, ApiResult, App, auth, management};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use tokio_postgres::Transaction;
use wccs_protocol::{ONLINE_SECONDS, digest, now, token, valid_id};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeatInput {
    row: i32,
    column: i32,
    number: i32,
    expected_ip: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomWrite {
    request_id: String,
    base_version: i64,
    id: String,
    name: String,
    rows: i32,
    columns: i32,
    start_number: i32,
    numbering: String,
    origin: String,
    seats: Vec<SeatInput>,
    #[serde(default)]
    preview_token: Option<String>,
}

fn validate(input: &RoomWrite) -> ApiResult<()> {
    if !valid_id(&input.id)
        || !valid_id(&input.request_id)
        || input.name.trim().is_empty()
        || input.name.len() > 200
        || !(1..=50).contains(&input.rows)
        || !(1..=50).contains(&input.columns)
        || input.rows * input.columns > 500
        || !(0..=1_000_000).contains(&input.start_number)
        || !["row", "column", "snake", "custom"].contains(&input.numbering.as_str())
        || !["top-left", "top-right", "bottom-left", "bottom-right"]
            .contains(&input.origin.as_str())
    {
        return Err(ApiError::bad(
            "机房字段无效，每边 1～50，最多 500 个座位，起始号 0～1000000",
        ));
    }
    if input.seats.len() != (input.rows * input.columns) as usize {
        return Err(ApiError::bad("座位数量与矩形尺寸不一致"));
    }
    let mut positions = HashSet::new();
    let mut numbers = HashSet::new();
    let mut ips = HashSet::new();
    for seat in &input.seats {
        let location = format!(
            "第 {} 行第 {} 列",
            i64::from(seat.row) + 1,
            i64::from(seat.column) + 1
        );
        if !(0..input.rows).contains(&seat.row)
            || !(0..input.columns).contains(&seat.column)
            || !positions.insert((seat.row, seat.column))
        {
            return Err(ApiError::bad(&format!("{location} 位置无效或重复")));
        }
        if !(input.start_number..input.start_number + input.rows * input.columns)
            .contains(&seat.number)
            || !numbers.insert(seat.number)
        {
            return Err(ApiError::bad(&format!(
                "{location} 编号 {} 重复或不在连续范围内",
                seat.number
            )));
        }
        if input.numbering != "custom" {
            let row = if input.origin.starts_with("bottom") {
                input.rows - 1 - seat.row
            } else {
                seat.row
            };
            let column = if input.origin.ends_with("right") {
                input.columns - 1 - seat.column
            } else {
                seat.column
            };
            let offset = match input.numbering.as_str() {
                "column" => column * input.rows + row,
                "snake" if row % 2 == 1 => row * input.columns + input.columns - 1 - column,
                _ => row * input.columns + column,
            };
            if seat.number != input.start_number + offset {
                return Err(ApiError::bad(&format!(
                    "{location} 编号与规则不符，请应用编号规则或选择自定义排列"
                )));
            }
        }
        if let Some(ip) = &seat.expected_ip {
            if ip.parse::<std::net::Ipv4Addr>().is_err() {
                return Err(ApiError::bad(&format!("{location} IPv4 格式错误")));
            }
            if !ips.insert(ip) {
                return Err(ApiError::bad(&format!("{location} IP {ip} 重复")));
            }
        }
    }
    Ok(())
}

// All administrative mutations and observations lock this row in the same order.
// This also serializes requests across multiple server processes.
pub(crate) async fn lock(tx: &Transaction<'_>) -> ApiResult<i64> {
    Ok(tx
        .query_one(
            "SELECT version FROM configuration_head WHERE id=1 FOR UPDATE",
            &[],
        )
        .await?
        .get(0))
}

pub(crate) async fn snapshot(tx: &Transaction<'_>) -> ApiResult<Value> {
    let device_rows = tx
        .query(
            "SELECT id, peer_ip, observed_ip, address_source, reported_ips, previous_ip,
                address_version, needs_review, excluded, last_seen, lifecycle,
                credential_hash IS NOT NULL AS registered
         FROM devices ORDER BY id",
            &[],
        )
        .await?;
    let devices: Vec<Value> = device_rows
        .iter()
        .map(|row| {
            let seen: Option<i64> = row.get("last_seen");
            json!({
                "id": row.get::<_,String>("id"),
                "peer_ip": row.get::<_,Option<String>>("peer_ip"),
                "observed_ip": row.get::<_,Option<String>>("observed_ip"),
                "address_source": row.get::<_,Option<String>>("address_source"),
                "reported_ips": row.get::<_,Value>("reported_ips"),
                "previous_ip": row.get::<_,Option<String>>("previous_ip"),
                "address_version": row.get::<_,i64>("address_version"),
                "needs_review": row.get::<_,bool>("needs_review"),
                "excluded": row.get::<_,bool>("excluded"),
                "lifecycle": row.get::<_,String>("lifecycle"),
                "registered": row.get::<_,bool>("registered"),
                "online": row.get::<_,bool>("registered") && seen.is_some_and(|time| now()-time < ONLINE_SECONDS),
            })
        })
        .collect();
    let seat_rows = tx
        .query(
            "SELECT id, room_id, row_index, column_index, number, expected_ip
         FROM seats ORDER BY room_id, row_index, column_index",
            &[],
        )
        .await?;
    let seats: Vec<Value> = seat_rows
        .iter()
        .map(|row| {
            let ip: Option<String> = row.get("expected_ip");
            let matches: Vec<&Value> = devices
                .iter()
                .filter(|device| {
                    ip.is_some()
                        && device["observed_ip"].as_str() == ip.as_deref()
                        && device["registered"] == true
                        && device["excluded"] == false
                })
                .collect();
            let status = if matches.len() > 1
                || matches.iter().any(|device| device["needs_review"] == true)
            {
                "conflict"
            } else if let Some(device) = matches.first() {
                if device["online"] == true {
                    "online"
                } else {
                    "offline"
                }
            } else {
                "unbound"
            };
            json!({
                "id": row.get::<_,String>("id"),
                "room_id": row.get::<_,String>("room_id"),
                "row": row.get::<_,i32>("row_index"),
                "column": row.get::<_,i32>("column_index"),
                "number": row.get::<_,i32>("number"),
                "expected_ip": ip,
                "status": status,
                "devices": matches.iter().map(|device| device["id"].clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let room_rows = tx
        .query("SELECT * FROM rooms ORDER BY name,id", &[])
        .await?;
    let rooms: Vec<Value> = room_rows
        .iter()
        .map(|row| {
            let id: String = row.get("id");
            json!({
                "id": id,
                "name": row.get::<_,String>("name"),
                "rows": row.get::<_,i32>("rows"),
                "columns": row.get::<_,i32>("columns"),
                "start_number": row.get::<_,i32>("start_number"),
                "numbering": row.get::<_,String>("numbering"),
                "origin": row.get::<_,String>("origin"),
                "version": row.get::<_,i64>("version"),
                "seats": seats.iter().filter(|seat| seat["room_id"] == id).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({"rooms":rooms,"devices":devices}))
}

async fn room_preview(tx: &Transaction<'_>, input: &RoomWrite) -> ApiResult<Value> {
    validate(input)?;
    let current = snapshot(tx).await?;
    let rooms = current["rooms"].as_array().unwrap();
    let old = rooms.iter().find(|room| room["id"] == input.id);
    let version = old.map_or(0, |room| room["version"].as_i64().unwrap());
    if version != input.base_version {
        return Err(ApiError::conflict(&format!(
            "机房已更新到 v{version}，请重新加载机房"
        )));
    }
    if old.is_none() && rooms.len() >= 100 {
        return Err(ApiError::bad("最多 100 间机房"));
    }
    for room in rooms.iter().filter(|room| room["id"] != input.id) {
        for seat in room["seats"].as_array().unwrap() {
            if let Some(ip) = seat["expected_ip"].as_str()
                && let Some(candidate) = input
                    .seats
                    .iter()
                    .find(|candidate| candidate.expected_ip.as_deref() == Some(ip))
            {
                return Err(ApiError::conflict(&format!(
                    "第 {} 行第 {} 列 IP {ip} 已用于机房 {} 座位 {}",
                    candidate.row + 1,
                    candidate.column + 1,
                    room["name"],
                    seat["number"]
                )));
            }
        }
    }
    let old_seats = old
        .map(|room| room["seats"].as_array().unwrap().as_slice())
        .unwrap_or(&[]);
    let removed_ids: Vec<String> = old_seats
        .iter()
        .filter(|seat| {
            seat["row"].as_i64().unwrap() >= i64::from(input.rows)
                || seat["column"].as_i64().unwrap() >= i64::from(input.columns)
        })
        .map(|seat| seat["id"].as_str().unwrap().to_string())
        .collect();
    if let Some(row) = tx.query_opt("SELECT c.name,p.code FROM assignments a JOIN contests c ON c.id=a.contest_id JOIN participants p ON p.id=a.participant_id WHERE a.seat_id=ANY($1) LIMIT 1", &[&removed_ids]).await? {
        return Err(ApiError::conflict(&format!("移除座位仍分配给比赛 {} 的选手 {}，请在对应比赛先调整或解除分配",row.get::<_,String>(0),row.get::<_,String>(1))));
    }
    let mut changes = Vec::new();
    for seat in &input.seats {
        let previous = old_seats
            .iter()
            .find(|old| old["row"] == seat.row && old["column"] == seat.column);
        let incoming: Vec<&Value> = current["devices"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|device| {
                seat.expected_ip.is_some()
                    && device["observed_ip"].as_str() == seat.expected_ip.as_deref()
                    && device["excluded"] == false
            })
            .collect();
        changes.push(json!({
            "row": seat.row,
            "column": seat.column,
            "number": seat.number,
            "expected_ip": seat.expected_ip,
            "kind": if previous.is_some() {"retained"} else {"added"},
            "previous": previous,
            "incoming_devices": incoming,
        }));
    }
    for seat in old_seats.iter().filter(|seat| {
        seat["row"].as_i64().unwrap() >= input.rows as i64
            || seat["column"].as_i64().unwrap() >= input.columns as i64
    }) {
        changes.push(json!({
            "row": seat["row"],
            "column": seat["column"],
            "number": seat["number"],
            "expected_ip": null,
            "kind": "removed",
            "previous": seat,
            "incoming_devices": [],
        }));
    }
    let mut payload = serde_json::to_value(input).unwrap();
    payload.as_object_mut().unwrap().remove("preview_token");
    payload.as_object_mut().unwrap().remove("request_id");
    let preview_token = digest(&json!([payload, changes]).to_string());
    Ok(json!({"changes":changes,"preview_token":preview_token}))
}

pub async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<RoomWrite>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    lock(&tx).await?;
    Ok(Json(room_preview(&tx, &input).await?))
}

pub async fn save(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<RoomWrite>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    lock(&tx).await?;
    if let Some(response) =
        management::previous(&tx, &input.request_id, "room_save", &payload).await?
    {
        return Ok(Json(response));
    }
    let preview = room_preview(&tx, &input).await?;
    if input.preview_token.as_deref() != preview["preview_token"].as_str() {
        return Err(ApiError::conflict("布局或关联影响已变化，请重新预览后保存"));
    }
    let version = input.base_version + 1;
    tx.execute("INSERT INTO rooms (id,name,rows,columns,start_number,numbering,origin,version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (id) DO UPDATE SET name=$2,rows=$3,columns=$4,start_number=$5,numbering=$6,origin=$7,version=$8", &[&input.id,&input.name,&input.rows,&input.columns,&input.start_number,&input.numbering,&input.origin,&version]).await?;
    tx.execute(
        "DELETE FROM seats WHERE room_id=$1 AND (row_index >= $2 OR column_index >= $3)",
        &[&input.id, &input.rows, &input.columns],
    )
    .await?;
    for seat in &input.seats {
        tx.execute("INSERT INTO seats (id,room_id,row_index,column_index,number,expected_ip) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (room_id,row_index,column_index) DO UPDATE SET number=$5,expected_ip=$6", &[&token(),&input.id,&seat.row,&seat.column,&seat.number,&seat.expected_ip]).await?;
    }
    let response =
        management::remember(&tx, &app, &input.request_id, "room_save", &payload, version).await?;
    tx.commit().await?;
    Ok(Json(response))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    request_id: String,
    base_version: i64,
    seat_ids: Vec<String>,
    #[serde(default)]
    preview_token: Option<String>,
}

async fn selected_targets(
    tx: &Transaction<'_>,
    input: &Selection,
    version: i64,
) -> ApiResult<Value> {
    if version == 0 || version != input.base_version {
        return Err(ApiError::conflict(
            "请先发布工具配置，或重新加载当前配置版本",
        ));
    }
    let unique: HashSet<_> = input.seat_ids.iter().collect();
    if unique.is_empty() || unique.len() != input.seat_ids.len() || unique.len() > 500 {
        return Err(ApiError::bad("请选择 1～500 个不同座位"));
    }
    let state = snapshot(tx).await?;
    let configured: Value = tx
        .query_one(
            "SELECT targets FROM configurations WHERE version=$1",
            &[&version],
        )
        .await?
        .get(0);
    let mut targets = Vec::new();
    for id in &input.seat_ids {
        let seat = state["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|room| room["seats"].as_array().unwrap())
            .find(|seat| seat["id"] == *id)
            .ok_or_else(|| ApiError::conflict("选择的座位已移除，请重新选择"))?;
        let device = seat["devices"].as_array().unwrap().first();
        let status = match seat["status"].as_str().unwrap() {
            "unbound" => "unregistered",
            "conflict" => "conflict",
            "offline" => "offline",
            _ if state["devices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| Some(&d["id"]) == device && d["lifecycle"] != "enabled") =>
            {
                "disabled"
            }
            _ if !configured.as_array().unwrap().contains(device.unwrap()) => {
                "outside_configuration"
            }
            _ => "ready",
        };
        targets.push(json!({
            "seat_id": id,
            "number": seat["number"],
            "device_id": device,
            "device_ids": seat["devices"],
            "status": status,
        }));
    }
    let preview_token = digest(&json!([version, targets]).to_string());
    Ok(json!({"targets":targets,"preview_token":preview_token}))
}

pub async fn check_preview(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Selection>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let version = lock(&tx).await?;
    Ok(Json(selected_targets(&tx, &input, version).await?))
}

pub async fn check(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Selection>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    let version = lock(&tx).await?;
    if let Some(response) =
        management::previous(&tx, &input.request_id, "seat_check", &payload).await?
    {
        return Ok(Json(response));
    }
    let preview = selected_targets(&tx, &input, version).await?;
    if input.preview_token.as_deref() != preview["preview_token"].as_str() {
        return Err(ApiError::conflict("实际设备目标或状态已变化，请重新预览"));
    }
    let ids: Vec<String> = preview["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|target| target["status"] == "ready")
        .map(|target| target["device_id"].as_str().unwrap().to_owned())
        .collect();
    management::issue(&tx, &ids, version).await?;
    management::remember(
        &tx,
        &app,
        &input.request_id,
        "seat_check",
        &payload,
        version,
    )
    .await?;
    let response = json!({"version":version,"request_id":input.request_id,"targets":preview["targets"],"issued":ids});
    tx.execute(
        "UPDATE requests SET response=$1 WHERE id=$2",
        &[&response, &input.request_id],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    request_id: String,
    device_id: String,
    base_version: i64,
    exclude: bool,
}

pub async fn review(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Review>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers, true).await?;
    let payload = serde_json::to_value(&input).unwrap();
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    lock(&tx).await?;
    if let Some(response) =
        management::previous(&tx, &input.request_id, "address_review", &payload).await?
    {
        return Ok(Json(response));
    }
    let count = tx.execute("UPDATE devices SET excluded=$1,needs_review=false,address_version=address_version+1 WHERE id=$2 AND address_version=$3", &[&input.exclude,&input.device_id,&input.base_version]).await?;
    if count == 0 {
        return Err(ApiError::conflict("设备地址已变化，请重新加载后核对"));
    }
    let response = management::remember(
        &tx,
        &app,
        &input.request_id,
        "address_review",
        &payload,
        input.base_version + 1,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_contiguous_numbers_positions_and_ips() {
        let mut room: RoomWrite = serde_json::from_value(json!({"request_id":"r","base_version":0,"id":"a","name":"A","rows":1,"columns":2,"start_number":10,"numbering":"custom","origin":"top-left","seats":[{"row":0,"column":0,"number":11,"expected_ip":"10.0.0.1"},{"row":0,"column":1,"number":10,"expected_ip":null}]})).unwrap();
        assert!(validate(&room).is_ok());
        room.seats[1].number = 11;
        assert!(validate(&room).is_err());
        room.seats[1].number = 10;
        room.seats[1].expected_ip = Some("10.0.0.1".into());
        assert!(validate(&room).is_err());
        room.seats[1].expected_ip = Some("010.0.0.2".into());
        assert!(validate(&room).is_err());
        room.seats[0].row = i32::MAX;
        assert!(validate(&room).is_err());
    }
}
