#![forbid(unsafe_code)]

pub mod control;

use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};
use wccs_inventory::{
    model::{ConfiguredTool, ToolPath},
    tools::ToolConfiguration,
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const ONLINE_SECONDS: i64 = 35;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

// Validate Windows paths independently of the server's host OS.
pub fn windows_absolute_file(path: &str) -> bool {
    let bytes = path.as_bytes();
    if path.len() > 1024 || path.chars().any(char::is_control) || path.ends_with(['\\', '/']) {
        return false;
    }
    if bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return !path[3..].contains(':');
    }
    if let Some(rest) = path.strip_prefix("\\\\") {
        let parts: Vec<_> = rest.split('\\').collect();
        return parts.len() >= 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && !matches!(*part, "." | ".." | "?"))
            && !rest.contains(':');
    }
    false
}

pub fn validate_tools(tools: &[ToolPath]) -> Result<(), String> {
    if tools.len() > 64 {
        return Err("最多配置 64 个工具".into());
    }
    let mut ids = HashSet::new();
    for tool in tools {
        if !valid_id(&tool.id)
            || !ids.insert(&tool.id)
            || tool.name.trim().is_empty()
            || tool.name.len() > 200
        {
            return Err("工具 ID 必须唯一且为字母、数字、横线或下划线，名称不能为空".into());
        }
        if tool
            .path
            .as_deref()
            .is_some_and(|path| !windows_absolute_file(path))
        {
            return Err(format!(
                "{} 需要 Windows 完整文件路径，未配置使用 null",
                tool.name
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishRequest {
    pub request_id: String,
    pub base_version: i64,
    pub tools: Vec<ToolPath>,
    pub targets: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecheckRequest {
    pub request_id: String,
    pub base_version: i64,
    pub targets: Vec<String>,
}

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
pub struct Heartbeat {
    pub protocol_version: u32,
    pub boot_id: String,
    pub agent_version: String,
    #[serde(default)]
    pub reported_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Job {
    pub job_id: String,
    pub expires_at: i64,
    pub configuration: ToolConfiguration,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub protocol_version: u32,
    pub device_id: String,
    pub job_id: String,
    pub config_version: String,
    pub boot_id: String,
    pub tools: Vec<ConfiguredTool>,
}

pub fn validate_result(result: &CheckResult, config: &ToolConfiguration) -> Result<(), String> {
    if result.protocol_version != PROTOCOL_VERSION
        || result.config_version != config.config_version
        || !valid_id(&result.boot_id)
        || result.tools.len() != config.tools.len()
    {
        return Err("检查版本、启动标识或结果项数不匹配".into());
    }
    for (actual, expected) in result.tools.iter().zip(&config.tools) {
        if serde_json::to_value(&actual.tool).unwrap() != serde_json::to_value(expected).unwrap() {
            return Err("检查结果与授权配置快照不匹配".into());
        }
        use wccs_inventory::model::{Observation, Reason};
        if expected.path.is_none()
            && !matches!(
                actual.check,
                Observation::Unavailable {
                    reason: Reason::NotConfigured,
                    ..
                }
            )
        {
            return Err("未配置路径必须报告 not_configured".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_result_fixture_roundtrips_without_shape_changes() {
        let value: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/result.json")).unwrap();
        let result: CheckResult = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(result).unwrap(), value);
    }

    #[test]
    fn windows_paths_are_host_independent() {
        for path in [
            r"C:\工具\g++.exe",
            "E:/tools/submit.py",
            r"\\host\share\file.cmd",
        ] {
            assert!(windows_absolute_file(path), "{path}");
        }
        for path in [
            "/usr/bin/g++",
            "C:foo.exe",
            "foo.exe",
            "",
            r"\\?\C:\foo",
            "C:/",
            "C:/x:stream",
            "C:/x\n",
        ] {
            assert!(!windows_absolute_file(path), "{path}");
        }
    }
    #[test]
    fn catalog_and_custom_ids_validate() {
        let mut catalog: ToolConfiguration =
            serde_json::from_str(wccs_inventory::tools::TOOL_CATALOG).unwrap();
        validate_tools(&catalog.tools).unwrap();
        catalog.tools.push(catalog.tools[0].clone());
        assert!(validate_tools(&catalog.tools).is_err());
    }
}
