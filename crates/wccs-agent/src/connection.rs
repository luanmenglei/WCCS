//! Site configuration is separate from per-device credentials and can be saved offline.
use crate::windows_agent::{Identity, Result, persist, validate_server};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use wccs_protocol::{diagnostics::SeatClaim, digest};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema_version: u32,
    pub server: Option<String>,
    pub seat_claim: Option<SeatClaim>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    pub device_binding: String,
    pub revision: u64,
    pub configuration: Configuration,
}

pub fn binding(identity: &Identity) -> String {
    digest(&format!(
        "{}:{}",
        identity.device_id, identity.enrollment_token
    ))
}

pub fn validate(mut config: Configuration) -> Result<Configuration> {
    if config.schema_version != 1 || config.seat_claim.as_ref().is_some_and(|s| !s.valid()) {
        return Err("配置版本或座位核对信息无效".into());
    }
    if let Some(server) = &config.server {
        config.server = Some(
            validate_server(server)?
                .as_str()
                .trim_end_matches('/')
                .to_owned(),
        );
    }
    Ok(config)
}

pub fn load(path: &Path, identity: &Identity) -> Result<Saved> {
    match fs::read(path.with_extension("connection.json")) {
        Ok(bytes) => {
            let saved: Saved = serde_json::from_slice(&bytes)?;
            if saved.device_binding != binding(identity) {
                return Err("本机配置属于其他设备身份，请重新导入".into());
            }
            validate(saved.configuration.clone())?;
            Ok(saved)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved {
            device_binding: binding(identity),
            revision: 0,
            configuration: validate(Configuration {
                schema_version: 1,
                server: (!identity.server.is_empty()).then(|| identity.server.clone()),
                seat_claim: None,
            })?,
        }),
        Err(error) => Err(error.into()),
    }
}

pub fn save(path: &Path, base_revision: u64, config: Configuration) -> Result<Saved> {
    use std::os::windows::fs::OpenOptionsExt;
    let config = validate(config)?;
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("configuration.lock"))?;
    let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
    let current = load(path, &identity)?;
    // Exact retries have no additional revision or side effects.
    if current.configuration == config {
        return Ok(current);
    }
    if current.revision != base_revision {
        return Err("配置版本已变化，请重新预览".into());
    }
    if let Some(claim) = &config.seat_claim {
        let adapters = wccs_inventory::network();
        if !adapters.value().is_some_and(|adapters| {
            adapters.iter().any(|a| {
                a.interface_index == claim.interface_index
                    && a.mac_address.eq_ignore_ascii_case(&claim.mac_address)
            })
        }) {
            return Err("目标网卡身份已变化，请刷新网卡并重新预览".into());
        }
    }
    let saved = Saved {
        device_binding: current.device_binding,
        revision: current.revision.checked_add(1).ok_or("配置版本溢出")?,
        configuration: config,
    };
    persist(
        &path.with_extension("connection.json"),
        &serde_json::to_vec_pretty(&saved)?,
    )?;
    Ok(saved)
}

pub fn server(path: &Path, identity: &Identity) -> Result<String> {
    load(path, identity)?
        .configuration
        .server
        .ok_or_else(|| "服务器地址待配置".into())
}

pub fn configuration_digest(saved: &Saved) -> String {
    digest(&serde_json::to_string(saved).expect("configuration is serializable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_versioned_bounded_and_credential_free() {
        let mut value = serde_json::json!({"schema_version":1,"server":null,"seat_claim":null});
        assert!(validate(serde_json::from_value(value.clone()).unwrap()).is_ok());
        value["server"] = "http://10.1.2.3".into();
        assert!(validate(serde_json::from_value(value.clone()).unwrap()).is_err());
        value["server"] = "https://example.test/?secret=abc".into();
        assert!(validate(serde_json::from_value(value.clone()).unwrap()).is_err());
        value["device_token"] = "secret".into();
        assert!(serde_json::from_value::<Configuration>(value).is_err());
    }
    #[test]
    fn save_conflict_retry_and_identity_change() {
        let root = std::env::temp_dir().join(format!("wccs-config-{}", wccs_protocol::token()));
        fs::create_dir(&root).unwrap();
        let path = root.join("device.json");
        let mut identity: Identity = serde_json::from_value(
            serde_json::json!({"server":"","device_id":"test","enrollment_token":"a".repeat(64)}),
        )
        .unwrap();
        fs::write(&path, serde_json::to_vec(&identity).unwrap()).unwrap();
        let config = Configuration {
            schema_version: 1,
            server: Some("https://example.test".into()),
            seat_claim: None,
        };
        assert_eq!(save(&path, 0, config.clone()).unwrap().revision, 1);
        assert_eq!(save(&path, 0, config).unwrap().revision, 1);
        let changed = Configuration {
            schema_version: 1,
            server: Some("https://other.test".into()),
            seat_claim: None,
        };
        assert!(save(&path, 0, changed).is_err());
        identity.device_id = "replacement".into();
        assert!(load(&path, &identity).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
