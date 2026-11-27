//! 设备注册和心跳消息。旧客户端未上报的生命周期字段由反序列化默认值补齐。
use crate::{Job, digest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub protocol_version: u32,
    pub device_id: String,
    pub enrollment_token: String,
    pub device_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRegistration {
    pub registration: Registration,
    pub machine_name: String,
    #[serde(default)]
    pub bootstrap_status: String,
}

pub fn image_device_id(machine_name: &str) -> Option<String> {
    if machine_name.is_empty()
        || machine_name.len() > 63
        || !machine_name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return None;
    }
    Some(format!(
        "image-{}",
        &digest(&machine_name.to_ascii_uppercase())[..32]
    ))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub protocol_version: u32,
    pub boot_id: String,
    pub agent_version: String,
    #[serde(default)]
    pub reported_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HeartbeatReply {
    #[serde(default = "enabled_state")]
    pub lifecycle: String,
    #[serde(default)]
    pub rotation_id: Option<String>,
    pub protocol_version: u32,
    pub server_time: i64,
    pub job: Option<Job>,
}

fn enabled_state() -> String {
    "enabled".into()
}
