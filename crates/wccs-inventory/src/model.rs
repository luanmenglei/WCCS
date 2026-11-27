#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    PermissionDenied,
    Missing,
    NotFoundInSearchScope,
    NotApplicable,
    NotConfigured,
    NotAFile,
    Unsupported,
    SystemError,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Observation<T> {
    Available { value: T },
    Unavailable { reason: Reason, code: Option<u32> },
}

impl<T> Observation<T> {
    pub fn available(value: T) -> Self {
        Self::Available { value }
    }

    pub fn unavailable(reason: Reason) -> Self {
        Self::Unavailable { reason, code: None }
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Available { value } => Some(value),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn from_win32(code: u32) -> Self {
        let reason = match code {
            5 | 1314 => Reason::PermissionDenied,
            2 | 3 | 13 | 1812..=1815 => Reason::Missing,
            50 | 120 => Reason::Unsupported,
            _ => Reason::SystemError,
        };
        Self::Unavailable {
            reason,
            code: Some(code),
        }
    }

    pub fn from_io(error: &io::Error) -> Self {
        let reason = match error.kind() {
            io::ErrorKind::PermissionDenied => Reason::PermissionDenied,
            io::ErrorKind::NotFound => Reason::Missing,
            _ => Reason::SystemError,
        };
        Self::Unavailable {
            reason,
            code: error.raw_os_error().map(|code| code as u32),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceContext {
    Development,
    Lab,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Report {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<crate::health::Health>,
    pub schema_version: u32,
    pub collector_version: String,
    pub evidence_context: EvidenceContext,
    pub collected_at_unix_ms: u128,
    pub collection_mode: String,
    pub system: System,
    pub account: Account,
    pub volumes: VolumeInventory,
    pub network: Observation<Vec<NetworkAdapter>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_configuration: Option<Observation<String>>,
    // Retained only for reading and re-exporting Schema 1/2 evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_search_directories: Vec<String>,
    pub tools: Vec<ToolReport>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct System {
    pub host_name: Observation<String>,
    pub version: Observation<String>,
    pub edition: Observation<String>,
    pub display_version: Observation<String>,
    pub update_build_revision: Observation<u32>,
    pub native_architecture: String,
    pub process_architecture: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Account {
    pub user_name: Observation<String>,
    pub sid: Observation<String>,
    pub elevated: Observation<bool>,
    pub elevation_type: Observation<String>,
    pub administrators_group: Observation<String>,
    pub session_id: Observation<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VolumeInventory {
    pub entries: Vec<Volume>,
    pub enumeration: Observation<()>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Volume {
    pub volume_guid_path: String,
    pub mount_paths: Observation<Vec<String>>,
    pub drive_type: u32,
    pub identity: Observation<VolumeIdentity>,
    pub space: Observation<VolumeSpace>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VolumeIdentity {
    pub label: String,
    pub serial_number: String,
    pub file_system: String,
    pub file_system_flags: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VolumeSpace {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub available_to_caller_bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NetworkAdapter {
    pub name: String,
    pub description: String,
    pub interface_index: u32,
    pub ipv6_interface_index: u32,
    pub interface_type: u32,
    pub operational_status: i32,
    pub mac_address: String,
    pub mtu: u32,
    pub addresses: Vec<IpAddress>,
    pub dns_servers: Vec<Observation<String>>,
    pub gateways: Vec<Observation<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IpAddress {
    pub address: Observation<String>,
    pub prefix_length: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Executable,
    Script,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPath {
    pub id: String,
    pub name: String,
    pub kind: ToolKind,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolReport {
    Configured(ConfiguredTool),
    Legacy(Tool),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfiguredTool {
    #[serde(flatten)]
    pub tool: ToolPath,
    pub check: Observation<ToolMetadata>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolMetadata {
    pub file_version: Observation<String>,
}

// Schema 1/2 historical reports, never used to resolve current tool paths.
#[derive(Debug, Deserialize, Serialize)]
pub struct Tool {
    pub executable: String,
    pub detection: Observation<Vec<ToolFile>>,
    pub search_errors: Vec<ToolSearchError>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolSearchError {
    pub directory: String,
    pub error: Observation<()>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolFile {
    pub path: String,
    pub file_version: Observation<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_missing_and_unexpected_errors_are_distinct() {
        for (code, expected) in [
            (5, Reason::PermissionDenied),
            (1314, Reason::PermissionDenied),
            (2, Reason::Missing),
            (1813, Reason::Missing),
            (50, Reason::Unsupported),
            (1117, Reason::SystemError),
        ] {
            assert_eq!(
                Observation::<bool>::from_win32(code),
                Observation::Unavailable {
                    reason: expected,
                    code: Some(code)
                }
            );
        }
    }

    #[test]
    fn unknown_permission_is_never_false() {
        let unknown = Observation::<bool>::from_win32(5);
        let json = serde_json::to_string(&unknown).unwrap();
        assert!(!json.contains("false"));
        assert!(!json.contains("value"));
        assert_eq!(
            serde_json::from_str::<Observation<bool>>(&json).unwrap(),
            unknown
        );
    }
}
