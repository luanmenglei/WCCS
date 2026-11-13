use crate::{ApiError, ApiResult, App, auth};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use serde_json::{Value, json};
use wccs_inventory::tools::ToolConfiguration;
use wccs_protocol::{
    CheckResult, Heartbeat, HeartbeatReply, Job, PROTOCOL_VERSION, Registration, digest, now,
    valid_id, validate_result,
};

pub async fn register(
    State(app): State<App>,
    Json(input): Json<Registration>,
) -> ApiResult<Json<Value>> {
    auth::rate_limit(&app, "registration", 600).await?;
    if input.protocol_version != PROTOCOL_VERSION
        || !valid_id(&input.device_id)
        || input.enrollment_token.len() != 64
        || input.device_token.len() != 64
    {
        return Err(ApiError::bad("设备注册格式或协议版本错误"));
    }
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    crate::lifecycle::expire(&tx).await?;
    let row = tx.query_opt("SELECT enrollment_hash,enrollment_expires,credential_hash,lifecycle,applicant_hash,enrollment_generation,application_id FROM devices WHERE id=$1", &[&input.device_id]).await?.ok_or_else(|| ApiError::unauthorized("注册材料无效"))?;
    let credential = digest(&input.device_token);
    let state: String = row.get(3);
    if matches!(state.as_str(), "enabled" | "disabled")
        && row.get::<_, Option<String>>(2).as_deref() == Some(&credential)
    {
        return Ok(Json(
            json!({"device_id":input.device_id,"protocol_version":PROTOCOL_VERSION,"status":state}),
        ));
    }
    if state != "pending"
        || row.get::<_, String>(0) != digest(&input.enrollment_token)
        || row.get::<_, i64>(1) <= now()
    {
        return Err(ApiError::unauthorized(
            "注册材料无效、已使用或已过期，请管理员核对",
        ));
    }
    let application = digest(&format!("{}:{}", row.get::<_, String>(0), credential));
    if let Some(candidate) = row.get::<_, Option<String>>(4) {
        if candidate != credential {
            return Err(ApiError::conflict("此材料已关联另一申请，需管理员重新授权"));
        }
    } else {
        crate::lifecycle::reserve(&tx, &credential, &input.device_id, row.get(5)).await?;
        let open: bool = tx
            .query_one(
                "SELECT closes_at>$1 FROM registration_window WHERE id=1",
                &[&now()],
            )
            .await?
            .get(0);
        let auto = open && row.get::<_, i64>(5) == 1;
        tx.execute("UPDATE devices SET applicant_hash=$2,application_id=$3,application_at=$4,lifecycle_version=lifecycle_version+1,
            lifecycle=CASE WHEN $5 THEN 'enabled' ELSE 'pending' END,
            credential_hash=CASE WHEN $5 THEN $2 ELSE NULL END WHERE id=$1",
            &[&input.device_id,&credential,&application,&now(),&auto]).await?;
        crate::lifecycle::event(
            &tx,
            &input.device_id,
            if auto {
                "automatically_approved"
            } else {
                "registration_requested"
            },
            &application,
        )
        .await?;
    }
    let status: String = tx
        .query_one(
            "SELECT lifecycle FROM devices WHERE id=$1",
            &[&input.device_id],
        )
        .await?
        .get(0);
    tx.commit().await?;
    Ok(Json(
        json!({"device_id":input.device_id,"protocol_version":PROTOCOL_VERSION,"status":status,"application_id":application}),
    ))
}

pub async fn heartbeat(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<Heartbeat>,
) -> ApiResult<Json<HeartbeatReply>> {
    if input.protocol_version != PROTOCOL_VERSION
        || !valid_id(&input.boot_id)
        || input.agent_version.len() > 80
        || input.reported_ips.len() > 32
        || input
            .reported_ips
            .iter()
            .any(|ip| ip.parse::<std::net::IpAddr>().is_err())
    {
        return Err(ApiError::bad("心跳协议版本或字段错误"));
    }
    let (observed, source) = source_address(peer.ip(), &headers, &app.config.trusted_proxies);
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let id = auth::device(&tx, &headers).await?;
    tx.execute(
        "UPDATE devices SET last_seen=$1,boot_id=$2,agent_version=$3,peer_ip=$5,
        previous_ip=CASE WHEN observed_ip IS DISTINCT FROM $6 THEN observed_ip ELSE previous_ip END,
        needs_review=needs_review OR (address_version>0 AND observed_ip IS DISTINCT FROM $6),
        address_version=address_version + CASE WHEN address_version=0 OR observed_ip IS DISTINCT FROM $6 THEN 1 ELSE 0 END,
        observed_ip=$6,address_source=$7,reported_ips=$8 WHERE id=$4",
        &[&now(), &input.boot_id, &input.agent_version, &id, &peer.ip().to_canonical().to_string(), &observed, &source, &serde_json::to_value(&input.reported_ips).unwrap()],
    ).await?;
    let row = tx.query_opt("SELECT j.id,j.expires_at,c.snapshot FROM devices d JOIN jobs j ON d.desired_job=j.id JOIN configurations c ON c.version=j.config_version WHERE d.id=$1 AND d.lifecycle='enabled' AND j.cancelled_at IS NULL AND j.result IS NULL AND j.expires_at>$2", &[&id,&now()]).await?;
    let job = match row {
        Some(row) => Some(Job {
            job_id: row.get(0),
            expires_at: row.get(1),
            configuration: serde_json::from_value(row.get(2)).map_err(|_| ApiError::internal())?,
        }),
        None => None,
    };
    let status = tx
        .query_one(
            "SELECT lifecycle,rotation_id,rotation_expires FROM devices WHERE id=$1",
            &[&id],
        )
        .await?;
    let rotation_id = if status
        .get::<_, Option<i64>>(2)
        .is_some_and(|time| time > now())
    {
        status.get(1)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Json(HeartbeatReply {
        protocol_version: PROTOCOL_VERSION,
        server_time: now(),
        lifecycle: status.get(0),
        rotation_id,
        job,
    }))
}

pub async fn result(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<CheckResult>,
) -> ApiResult<Json<Value>> {
    let mut db = app.db.lock().await?;
    let tx = db.transaction().await?;
    crate::seating::lock(&tx).await?;
    let id = auth::device(&tx, &headers).await?;
    if input.device_id != id {
        return Err(ApiError::forbidden("结果设备身份不匹配"));
    }

    let row = tx.query_opt("SELECT j.result,j.expires_at,c.snapshot,j.cancelled_at FROM jobs j JOIN configurations c ON c.version=j.config_version WHERE j.id=$1 AND j.device_id=$2 FOR UPDATE OF j", &[&input.job_id,&id]).await?.ok_or_else(|| ApiError::forbidden("任务未授权给此设备"))?;
    if row.get::<_, Option<i64>>(3).is_some() {
        return Err(ApiError::new(410, "任务已因撤销取消，保留历史记录"));
    }
    let config: ToolConfiguration =
        serde_json::from_value(row.get(2)).map_err(|_| ApiError::internal())?;
    validate_result(&input, &config).map_err(|message| ApiError::bad(&message))?;
    let value = serde_json::to_value(&input).unwrap();
    if let Some(previous) = row.get::<_, Option<Value>>(0) {
        if previous != value {
            return Err(ApiError::conflict("同一任务已回传不同结果"));
        }
    } else {
        if row.get::<_, i64>(1) <= now() {
            return Err(ApiError::new(410, "检查任务已过期，请管理员重新检查"));
        }
        tx.execute(
            "UPDATE jobs SET result=$1,received_at=$2 WHERE id=$3",
            &[&value, &now(), &input.job_id],
        )
        .await?;
    }
    let desired: Option<String> = tx
        .query_one("SELECT desired_job FROM devices WHERE id=$1", &[&id])
        .await?
        .get(0);
    let stale = desired.as_deref() != Some(&input.job_id);
    tx.commit().await?;
    Ok(Json(json!({"accepted":true,"stale":stale})))
}

// Walk X-Forwarded-For from the transport peer toward the client, stopping
// at the first untrusted hop. A trusted proxy must append/replace this header.
fn source_address(
    peer: std::net::IpAddr,
    headers: &HeaderMap,
    trusted: &[std::net::IpAddr],
) -> (Option<String>, &'static str) {
    let peer = peer.to_canonical();
    if !trusted.contains(&peer) {
        return (Some(peer.to_string()), "transport_peer");
    }
    let values: Vec<_> = headers.get_all("x-forwarded-for").iter().collect();
    if values.len() != 1 {
        return (None, "invalid_proxy_header");
    }
    let Ok(value) = values[0].to_str() else {
        return (None, "invalid_proxy_header");
    };
    let addresses: Result<Vec<std::net::IpAddr>, _> =
        value.split(',').map(|ip| ip.trim().parse()).collect();
    let Ok(addresses) = addresses else {
        return (None, "invalid_proxy_header");
    };
    if addresses.is_empty() || addresses.len() > 16 {
        return (None, "invalid_proxy_header");
    }
    let mut current = peer;
    for address in addresses.iter().rev() {
        if !trusted.contains(&current) {
            break;
        }
        current = address.to_canonical();
    }
    (Some(current.to_string()), "trusted_proxy")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_trust_stops_at_first_untrusted_hop() {
        let peer = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 10.1.0.9".parse().unwrap());
        assert_eq!(
            source_address(peer, &headers, &[]).0.as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            source_address(peer, &headers, &[peer]).0.as_deref(),
            Some("10.1.0.9")
        );
        headers.insert("x-forwarded-for", "invalid".parse().unwrap());
        assert_eq!(source_address(peer, &headers, &[peer]).0, None);
    }
}
