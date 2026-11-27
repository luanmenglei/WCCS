//! 设备注册材料兼容单机安装与统一镜像两种来源。
use crate::{AgentResult as Result, local_state::persist};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::Path,
    time::{Duration, Instant},
};
use wccs_protocol::{Registration, token};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    #[serde(default)]
    pub image_enrollment: bool,
    pub server: String,
    pub device_id: String,
    pub enrollment_token: String,
    #[serde(default)]
    pub device_token: Option<String>,
}

impl Identity {
    pub fn registration_body(
        &self,
        registration: Registration,
        path: &Path,
    ) -> Result<serde_json::Value> {
        if self.image_enrollment {
            let machine_name = wccs_inventory::host_name()?;
            if wccs_protocol::image_device_id(&machine_name).as_deref() != Some(&self.device_id) {
                return Err("image_machine_name_changed_requires_review".into());
            }
            Ok(serde_json::to_value(wccs_protocol::ImageRegistration {
                registration,
                machine_name,
                bootstrap_status: fs::read(path.with_extension("image-status.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .and_then(|value| value["code"].as_str().map(str::to_owned))
                    .unwrap_or_default(),
            })?)
        } else {
            Ok(serde_json::to_value(registration)?)
        }
    }
    pub fn registration_path(&self) -> &'static str {
        if self.image_enrollment {
            "image-register"
        } else {
            "register"
        }
    }
}

// 服务和诊断进程可能同时首次启动。锁覆盖读取、生成与落盘，确保它们取得同一份凭据。
pub fn credential(path: &Path, identity: &Identity) -> Result<String> {
    use std::os::windows::fs::OpenOptionsExt;
    let started = Instant::now();
    let _lock = loop {
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path.with_extension("credential.lock"))
        {
            Ok(file) => break file,
            Err(e)
                if matches!(e.raw_os_error(), Some(32 | 33))
                    && started.elapsed() < Duration::from_secs(3) =>
            {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(e) => return Err(e.into()),
        }
    };
    let credential_path = path.with_extension("credential");
    let value = match fs::read_to_string(&credential_path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let value = identity.device_token.clone().unwrap_or_else(token);
            persist(&credential_path, value.as_bytes())?;
            value
        }
        Err(error) => return Err(error.into()),
    };
    if value.len() != 64 {
        return Err("设备凭据格式错误".into());
    }
    Ok(value)
}
