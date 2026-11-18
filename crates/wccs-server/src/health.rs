use crate::{ApiError, ApiResult, App, auth};
use axum::{Json, extract::State, http::HeaderMap};
use serde_json::{Value, json};
use wccs_protocol::{health::Report, now};

pub async fn report(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Report>,
) -> ApiResult<Json<Value>> {
    if !input.valid() {
        return Err(ApiError::bad("健康数据格式无效"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let device = auth::device(&tx, &headers).await?;
    if device != input.device_id {
        return Err(ApiError::forbidden("健康报告设备不匹配"));
    }
    let row=tx.query_opt("SELECT d.enrollment_generation FROM devices d JOIN control_devices c ON c.device_id=d.id WHERE d.id=$1 AND c.instance_id=$2 AND c.connected AND c.last_seen>$3::bigint-15",&[&device,&input.instance_id,&now()]).await?.ok_or_else(||ApiError::conflict("健康报告宿主已失效"))?;
    let generation: i64 = row.get(0);
    let value = json!(input);
    let previous=tx.query_opt("SELECT instance_id,sequence,report FROM device_health WHERE device_id=$1 AND enrollment_generation=$2",&[&device,&generation]).await?;
    if let Some(previous) = &previous
        && previous.get::<_, String>(0) == input.instance_id
        && previous.get::<_, i64>(1) >= input.sequence
    {
        if previous.get::<_, Value>(2) != value {
            return Err(ApiError::conflict("健康采样序号已使用或过期"));
        }
    } else {
        let old = previous.as_ref().map(|r| r.get::<_, Value>(2));
        let mut changes = Vec::new();
        for field in ["keyboards", "mice", "displays"] {
            let current = &value["health"][field]["observation"];
            let before = old.as_ref().map(|v| &v["health"][field]["observation"]);
            if before.is_some_and(|b| b != current) || (before.is_none() && current["value"] == 0) {
                changes.push(json!({"kind":field,"before":before,"after":current}));
            }
        }
        if !changes.is_empty() {
            tx.execute("INSERT INTO events(source,source_id,category,occurred_at,operation,target,device_ids,room_ids,outcome,detail) VALUES('health',$1,'device_report',$2,'hardware_changed',$3,$4,device_room_scope($3),'warning',$5)",
                &[&format!("{}-{}-{}",device,input.instance_id,input.sequence),&now(),&device,&vec![device.clone()],&json!({"changes":changes})]).await?;
        }
        tx.execute("INSERT INTO device_health VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(device_id) DO UPDATE SET enrollment_generation=$2,instance_id=$3,sequence=$4,report=$5,received_at=$6",&[&device,&generation,&input.instance_id,&input.sequence,&value,&now()]).await?;
        if let Some(uptime) = input
            .health
            .uptime_seconds
            .observation
            .value()
            .and_then(|u| i64::try_from(*u).ok())
            && let Some(job) = headers
                .get("x-wccs-requested-reboot")
                .and_then(|v| v.to_str().ok())
                .filter(|job| wccs_protocol::valid_id(job))
        {
            let received = now();
            let boot = received.saturating_sub(uptime);
            tx.execute("UPDATE maintenance_jobs j SET reboot_observed_at=$2 FROM control_devices c
            WHERE j.id=$6 AND j.device_id=$1 AND c.device_id=j.device_id AND c.connected AND c.instance_id=$3
            AND j.reboot_observed_at IS NULL AND j.stage='succeeded' AND j.result->>'detail'='reboot_requested'
            AND j.command#>>'{template,options,action}'='reboot' AND j.command->>'instance_id'<>c.instance_id
            AND (j.command->>'generation')::bigint=$4 AND c.actual#>>'{account,phase}'='teaching'
            AND c.actual#>>'{account,policy,state}'='ready' AND $5::bigint BETWEEN j.reboot_requested_at-2 AND j.reboot_requested_at+120",
            &[&device,&received,&input.instance_id,&generation,&boot,&job]).await?;
        }
    }
    tx.commit().await?;
    Ok(Json(
        json!({"accepted":true,"device_id":device,"instance_id":input.instance_id,"sequence":input.sequence}),
    ))
}
