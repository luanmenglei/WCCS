use crate::valid_id;
use serde::{Deserialize, Serialize};
use wccs_inventory::health::{Health, Sample};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub device_id: String,
    pub instance_id: String,
    pub sequence: i64,
    pub agent_version: Sample<String>,
    pub deployment_version: Sample<String>,
    pub collection_ms: u64,
    pub health: Health,
}
impl Report {
    pub fn valid(&self) -> bool {
        let health = &self.health;
        let times = [
            self.agent_version.sampled_at,
            self.deployment_version.sampled_at,
            health.cpu_percent.sampled_at,
            health.memory.sampled_at,
            health.displays.sampled_at,
            health.uptime_seconds.sampled_at,
            health.keyboards.sampled_at,
            health.mice.sampled_at,
        ];
        self.schema_version == 1
            && valid_id(&self.device_id)
            && valid_id(&self.instance_id)
            && self.sequence > 0
            && times.iter().all(|t| *t >= 0)
            && [&self.agent_version, &self.deployment_version]
                .iter()
                .all(|v| {
                    v.observation.value().is_none_or(|s| {
                        !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
                    })
                })
            && health
                .cpu_percent
                .observation
                .value()
                .is_none_or(|v| *v <= 100)
            && health
                .memory
                .observation
                .value()
                .is_none_or(|v| v.used_percent <= 100 && v.available_bytes <= v.total_bytes)
            && health.displays.observation.value().is_none_or(|ds| {
                ds.len() <= 64
                    && ds.iter().all(|d| {
                        d.name.len() <= 128
                            && d.description.len() <= 256
                            && d.width <= 65536
                            && d.height <= 65536
                    })
            })
            && [&health.keyboards, &health.mice]
                .iter()
                .all(|v| v.observation.value().is_none_or(|n| *n <= 1024))
    }
}
