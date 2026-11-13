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
    if let Some(previous)=tx.query_opt("SELECT instance_id,sequence,report FROM device_health WHERE device_id=$1 AND enrollment_generation=$2",&[&device,&generation]).await?
        && previous.get::<_,String>(0)==input.instance_id
        && previous.get::<_,i64>(1)>=input.sequence {
        if previous.get::<_,Value>(2)!=value {return Err(ApiError::conflict("健康采样序号已使用或过期"));}
    } else {
        tx.execute("INSERT INTO device_health VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(device_id) DO UPDATE SET enrollment_generation=$2,instance_id=$3,sequence=$4,report=$5,received_at=$6",&[&device,&generation,&input.instance_id,&input.sequence,&value,&now()]).await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"accepted":true,"device_id":device,"instance_id":input.instance_id,"sequence":input.sequence}),
    ))
}
