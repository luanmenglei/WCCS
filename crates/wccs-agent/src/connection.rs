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
    // While connected, configuration.server itself identifies the credential recipient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offline_binding: Option<OfflineBinding>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum OfflineBinding {
    Unassigned,
    Origin(String),
}

impl Saved {
    pub fn credential_origin(&self) -> Option<&str> {
        self.configuration
            .server
            .as_deref()
            .or(match &self.offline_binding {
                Some(OfflineBinding::Origin(origin)) => Some(origin.as_str()),
                _ => None,
            })
    }

    pub fn check_destination(&self, next: &Configuration) -> Result<()> {
        if let Some(next) = next.server.as_deref() {
            match self.credential_origin() {
                Some(current) if current == next => (),
                None if matches!(self.offline_binding, Some(OfflineBinding::Unassigned)) => (),
                _ => return Err("此身份不能改投其他服务入口。换主机请保留原 HTTPS 入口，换入口请使用新注册材料。".into()),
            }
        }
        Ok(())
    }
}

fn canonical_origin(server: &str) -> Result<String> {
    Ok(validate_server(server)?
        .as_str()
        .trim_end_matches('/')
        .to_owned())
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
        config.server = Some(canonical_origin(server)?);
    }
    Ok(config)
}

pub fn load(path: &Path, identity: &Identity) -> Result<Saved> {
    match fs::read(path.with_extension("connection.json")) {
        Ok(bytes) => {
            let mut saved: Saved = serde_json::from_slice(&bytes)?;
            if saved.device_binding != binding(identity) {
                return Err("本机配置属于其他设备身份，请重新导入".into());
            }
            saved.configuration = validate(saved.configuration)?;
            if saved.configuration.server.is_some() && saved.offline_binding.is_some() {
                return Err("连接配置同时声明了在线和离线入口".into());
            }
            if let Some(OfflineBinding::Origin(origin)) = &mut saved.offline_binding {
                *origin = canonical_origin(origin)?;
            } else if saved.configuration.server.is_none()
                && saved.offline_binding.is_none()
                && !identity.server.is_empty()
            {
                saved.offline_binding =
                    Some(OfflineBinding::Origin(canonical_origin(&identity.server)?));
            }
            Ok(saved)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved {
            device_binding: binding(identity),
            revision: 0,
            offline_binding: identity
                .server
                .is_empty()
                .then_some(OfflineBinding::Unassigned),
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
    current.check_destination(&config)?;
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
        offline_binding: if config.server.is_none() {
            current
                .configuration
                .server
                .map(OfflineBinding::Origin)
                .or(current.offline_binding)
        } else {
            None
        },
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
        let other = Configuration {
            schema_version: 1,
            server: Some("https://other.test".into()),
            seat_claim: None,
        };
        assert!(save(&path, 1, other.clone()).is_err());
        let mut offline = other.clone();
        offline.server = None;
        let detached = save(&path, 1, offline).unwrap();
        assert_eq!(detached.revision, 2);
        assert!(server(&path, &identity).is_err());
        assert_eq!(
            load(&path, &identity).unwrap().credential_origin(),
            Some("https://example.test")
        );
        assert!(save(&path, 2, other.clone()).is_err());
        let back = Configuration {
            server: Some("HTTPS://EXAMPLE.TEST:443/".into()),
            ..other
        };
        assert_eq!(save(&path, 2, back).unwrap().revision, 3);
        // Legacy detached records without any known destination require fresh enrollment.
        let legacy = serde_json::json!({"device_binding": binding(&identity), "revision": 4,
            "configuration":{"schema_version":1,"server":null,"seat_claim":null}});
        fs::write(
            path.with_extension("connection.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let saved = load(&path, &identity).unwrap();
        assert!(
            saved
                .check_destination(&Configuration {
                    server: Some("https://other.test".into()),
                    ..saved.configuration.clone()
                })
                .is_err()
        );
        identity.device_id = "replacement".into();
        assert!(load(&path, &identity).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
