#![forbid(unsafe_code)]

use crate::model::{EvidenceContext, Observation, Reason, Report};
use std::{
    fmt::Display,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn field<T: Display>(observation: &Observation<T>) -> String {
    match observation {
        Observation::Available { value } => value.to_string(),
        Observation::Unavailable { reason, .. } => reason_label(reason).into(),
    }
}

fn reason_label(reason: &Reason) -> &'static str {
    match reason {
        Reason::PermissionDenied => "权限不足",
        Reason::Missing => "字段缺失",
        Reason::NotFoundInSearchScope => "搜索范围内未发现",
        Reason::Unsupported => "不支持",
        Reason::SystemError => "系统错误",
    }
}

pub fn summary(report: &Report) -> Result<String, serde_json::Error> {
    let context = match report.evidence_context {
        EvidenceContext::Development => "开发环境证据",
        EvidenceContext::Lab => "机房采集证据（运行者指定）",
    };
    let mut lines = vec![
        format!("WCCS 环境报告 · {context}"),
        format!(
            "Schema {} · 采集器 {} · Unix 毫秒 {}",
            report.schema_version, report.collector_version, report.collected_at_unix_ms
        ),
        "只读采集 · 摘要已省略机器名、账户名、SID、卷身份、IP、MAC 和文件路径".into(),
        format!(
            "Windows 内核版本 {} · {} · 显示版本 {} · 修订 {}",
            field(&report.system.version),
            field(&report.system.edition),
            field(&report.system.display_version),
            field(&report.system.update_build_revision)
        ),
        format!(
            "系统架构 {} · 进程架构 {}",
            report.system.native_architecture, report.system.process_architecture
        ),
        format!(
            "当前进程提升 {} · 令牌类型 {} · 管理员组 {}",
            field(&report.account.elevated),
            field(&report.account.elevation_type),
            field(&report.account.administrators_group)
        ),
        format!("磁盘卷 {} 个", report.volumes.entries.len()),
    ];
    for (index, volume) in report.volumes.entries.iter().enumerate() {
        let file_system = volume
            .identity
            .value()
            .map(|identity| identity.file_system.as_str())
            .unwrap_or("未知");
        let capacity = volume
            .space
            .value()
            .map(|space| {
                format!(
                    "总计 {:.1} GiB，可用 {:.1} GiB",
                    space.total_bytes as f64 / 1073741824.0,
                    space.available_to_caller_bytes as f64 / 1073741824.0
                )
            })
            .unwrap_or_else(|| "容量未知".into());
        lines.push(format!(
            "  卷 {} · 类型 {} · {file_system} · {capacity}",
            index + 1,
            volume.drive_type
        ));
    }
    match &report.network {
        Observation::Available { value } => {
            lines.push(format!(
                "网络适配器 {} 个，以下列出具有地址配置的网卡",
                value.len()
            ));
            for (index, adapter) in value
                .iter()
                .enumerate()
                .filter(|(_, adapter)| !adapter.addresses.is_empty())
            {
                lines.push(format!(
                    "  网卡 {} · 运行状态 {} · 地址 {} 个 · DNS {} 个 · 网关 {} 个",
                    index + 1,
                    adapter.operational_status,
                    adapter.addresses.len(),
                    adapter.dns_servers.len(),
                    adapter.gateways.len()
                ));
            }
        }
        Observation::Unavailable { reason, .. } => {
            lines.push(format!("网络采集 {}", reason_label(reason)))
        }
    }
    lines.push("竞赛工具文件探测（文件存在性与 PE 文件版本）".into());
    for tool in &report.tools {
        let status = match &tool.detection {
            Observation::Available { value } => format!(
                "发现 {} 处，文件版本 {}",
                value.len(),
                value
                    .iter()
                    .map(|file| field(&file.file_version))
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
            Observation::Unavailable { reason, .. } => reason_label(reason).into(),
        };
        lines.push(format!("  {} · {status}", tool.executable));
    }
    let json = serde_json::to_value(report)?;
    let mut issues = Vec::new();
    unavailable_fields(&json, "", &mut issues);
    lines.push(format!(
        "缺失或失败字段 {} 项（详情含字段路径和错误码）",
        issues.len()
    ));
    lines.extend(issues);
    lines.push(
        "现场待验证项包括登录方式、复原软件、网络隔离与放行、D 盘重解析点及编译运行能力。".into(),
    );
    Ok(lines.join("\n") + "\n")
}

fn unavailable_fields(value: &serde_json::Value, path: &str, issues: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if object.get("status").and_then(|value| value.as_str()) == Some("unavailable") {
                issues.push(format!(
                    "  {path} · {} · code {}",
                    object["reason"].as_str().unwrap_or("unknown"),
                    object["code"]
                ));
            } else {
                for (name, child) in object {
                    let child_path = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}.{name}")
                    };
                    unavailable_fields(child, &child_path, issues);
                }
            }
        }
        serde_json::Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                unavailable_fields(child, &format!("{path}[{index}]"), issues);
            }
        }
        _ => {}
    }
}

pub fn export(report: &Report, output_directory: &Path) -> io::Result<PathBuf> {
    // Serialize before creating any files so serialization failures leave no partial bundle.
    let json = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
    let summary = summary(report).map_err(io::Error::other)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let name = format!("report-{timestamp}-{}", std::process::id());
    fs::create_dir_all(output_directory)?;
    let partial = output_directory.join(format!("{name}.partial"));
    fs::create_dir(&partial)?;
    write_new(&partial.join("report.json"), &json)?;
    write_new(&partial.join("summary.txt"), summary.as_bytes())?;
    let destination = output_directory.join(name);
    // Both files are complete before publication. Interrupted runs retain an explicitly
    // marked .partial directory. Existing reports are never opened for writing.
    fs::rename(&partial, &destination)?;
    Ok(destination)
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Report {
        serde_json::from_str(include_str!("../tests/fixtures/report.json")).unwrap()
    }

    #[test]
    fn summary_redacts_machine_identity_and_exposes_partial_failure() {
        let summary = summary(&fixture()).unwrap();
        for private in [
            "SYNTHETIC-HOST",
            "synthetic-user",
            "S-1-5-21-123",
            "192.0.2.42",
            "02:00:00:00:00:01",
            "PRIVATE-LABEL",
            "C:\\synthetic",
        ] {
            assert!(!summary.contains(private), "leaked {private}");
        }
        assert!(summary.contains("开发环境证据"));
        assert!(summary.contains("volumes.entries[0].space"));
        assert!(summary.contains("permission_denied"));
        assert!(summary.contains("搜索范围内未发现"));
    }

    #[test]
    fn export_is_repeatable_without_overwriting_and_rejects_file_destination() {
        let root = std::env::temp_dir().join(format!(
            "wccs-export-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = export(&fixture(), &root).unwrap();
        let before = fs::read(first.join("report.json")).unwrap();
        let second = export(&fixture(), &root).unwrap();
        assert_ne!(first, second);
        assert_eq!(before, fs::read(first.join("report.json")).unwrap());
        let decoded: Report = serde_json::from_slice(&before).unwrap();
        assert_eq!(decoded.schema_version, 1);
        assert!(first.join("summary.txt").is_file());
        assert!(export(&fixture(), &first.join("report.json")).is_err());
        // Only the uniquely created test directory is removed.
        fs::remove_dir_all(root).unwrap();
    }
}
