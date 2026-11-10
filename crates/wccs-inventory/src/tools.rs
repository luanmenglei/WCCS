#![forbid(unsafe_code)]

use crate::model::{
    ConfiguredTool, Observation, Reason, ToolKind, ToolMetadata, ToolPath, ToolReport,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io, path::Path};

pub const TOOL_CATALOG: &str = include_str!("../../../config/tool-catalog.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfiguration {
    pub schema_version: u32,
    pub config_version: String,
    pub tools: Vec<ToolPath>,
}

impl ToolConfiguration {
    pub fn parse(json: &str) -> io::Result<Self> {
        let configuration: Self = serde_json::from_str(json).map_err(io::Error::other)?;
        configuration.validate()?;
        Ok(configuration)
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("工具配置 schema_version 必须为 1"));
        }
        if self.config_version.trim().is_empty() {
            return Err(invalid("工具配置必须提供 config_version"));
        }
        let mut ids = HashSet::new();
        for tool in &self.tools {
            if tool.id.trim().is_empty() || tool.name.trim().is_empty() {
                return Err(invalid("工具 id 和 name 必须非空"));
            }
            if !ids.insert(&tool.id) {
                return Err(invalid("工具配置包含重复 id"));
            }
            if let Some(path) = &tool.path
                && (path.contains('\0') || !Path::new(path).is_absolute())
            {
                return Err(invalid("工具路径必须为完整文件绝对路径，未配置时使用 null"));
            }
        }
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

// The caller supplies the entire effective configuration. Only these exact paths
// reach the filesystem boundary, so missing paths cannot trigger discovery.
pub fn check_configuration(
    configuration: Option<&ToolConfiguration>,
    inspect: impl Fn(&Path) -> io::Result<bool>,
    version: impl Fn(&Path) -> Observation<String>,
) -> io::Result<(Observation<String>, Vec<ToolReport>)> {
    let Some(configuration) = configuration else {
        return Ok((Observation::unavailable(Reason::NotConfigured), Vec::new()));
    };
    // Validate the full snapshot before the first file access, including direct callers.
    configuration.validate()?;
    let tools = configuration
        .tools
        .iter()
        .map(|tool| {
            let check = match &tool.path {
                None => Observation::unavailable(Reason::NotConfigured),
                Some(path) => match inspect(Path::new(path)) {
                    Ok(true) => Observation::available(ToolMetadata {
                        file_version: match tool.kind {
                            ToolKind::Executable => version(Path::new(path)),
                            ToolKind::Script => Observation::unavailable(Reason::NotApplicable),
                        },
                    }),
                    Ok(false) => Observation::unavailable(Reason::NotAFile),
                    Err(error) => Observation::from_io(&error),
                },
            };
            ToolReport::Configured(ConfiguredTool {
                tool: tool.clone(),
                check,
            })
        })
        .collect();
    Ok((
        Observation::available(configuration.config_version.clone()),
        tools,
    ))
}

pub fn inspect_file(path: &Path) -> io::Result<bool> {
    fs::metadata(path).map(|metadata| metadata.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn configuration() -> ToolConfiguration {
        ToolConfiguration {
            schema_version: 1,
            config_version: "synthetic-v1".into(),
            tools: vec![ToolPath {
                id: "custom-compiler".into(),
                name: "自定义编译器".into(),
                kind: ToolKind::Executable,
                path: Some(
                    std::env::temp_dir()
                        .join("自定义 工具.exe")
                        .to_string_lossy()
                        .into_owned(),
                ),
            }],
        }
    }

    fn checked(tools: &[ToolReport]) -> &ConfiguredTool {
        let ToolReport::Configured(tool) = &tools[0] else {
            panic!("current report required")
        };
        tool
    }

    #[test]
    fn catalog_contains_logical_tools_with_unconfigured_paths() {
        let catalog = ToolConfiguration::parse(TOOL_CATALOG).unwrap();
        assert!(catalog.tools.iter().all(|tool| tool.path.is_none()));
        for id in ["gcc-cpp", "python", "powershell", "compilecpp", "submit"] {
            assert!(catalog.tools.iter().any(|tool| tool.id == id));
        }
    }

    #[test]
    fn no_configuration_and_unconfigured_paths_perform_no_file_access() {
        let (status, reports) = check_configuration(
            None,
            |_| panic!("unexpected lookup"),
            |_| panic!("unexpected version query"),
        )
        .unwrap();
        assert_eq!(status, Observation::unavailable(Reason::NotConfigured));
        assert!(reports.is_empty());
        let catalog = ToolConfiguration::parse(TOOL_CATALOG).unwrap();
        let (_, reports) = check_configuration(
            Some(&catalog),
            |_| panic!("unexpected lookup"),
            |_| panic!("unexpected version query"),
        )
        .unwrap();
        for report in reports {
            let ToolReport::Configured(tool) = report else {
                panic!("current report required")
            };
            assert!(matches!(
                tool.check,
                Observation::Unavailable {
                    reason: Reason::NotConfigured,
                    ..
                }
            ));
        }
    }

    #[test]
    fn checks_only_configured_path_and_keeps_missing_version() {
        let config = configuration();
        let accesses = RefCell::new(Vec::new());
        let (status, reports) = check_configuration(
            Some(&config),
            |path| {
                accesses.borrow_mut().push(path.to_path_buf());
                Ok(true)
            },
            |_| Observation::unavailable(Reason::Missing),
        )
        .unwrap();
        assert_eq!(status, Observation::available("synthetic-v1".into()));
        assert_eq!(
            *accesses.borrow(),
            vec![std::path::PathBuf::from(
                config.tools[0].path.as_ref().unwrap()
            )]
        );
        assert_eq!(
            checked(&reports).check.value().unwrap().file_version,
            Observation::unavailable(Reason::Missing)
        );
    }

    #[test]
    fn configured_kind_controls_version_query_for_arbitrary_script_names() {
        let mut config = configuration();
        config.tools[0].kind = ToolKind::Script;
        let (_, reports) = check_configuration(
            Some(&config),
            |_| Ok(true),
            |_| panic!("script version query"),
        )
        .unwrap();
        assert_eq!(
            checked(&reports).check.value().unwrap().file_version,
            Observation::unavailable(Reason::NotApplicable)
        );
    }

    #[test]
    fn missing_denied_and_directory_paths_have_distinct_results() {
        for (result, expected) in [
            (
                Err(io::Error::from(io::ErrorKind::NotFound)),
                Reason::Missing,
            ),
            (
                Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Reason::PermissionDenied,
            ),
            (Ok(false), Reason::NotAFile),
        ] {
            let (_, reports) = check_configuration(
                Some(&configuration()),
                |_| match &result {
                    Ok(value) => Ok(*value),
                    Err(error) => Err(error.kind().into()),
                },
                |_| panic!("unavailable file"),
            )
            .unwrap();
            assert!(
                matches!(&checked(&reports).check, Observation::Unavailable { reason, .. } if *reason == expected)
            );
        }
    }

    #[test]
    fn invalid_configuration_is_rejected_before_file_access() {
        assert!(ToolConfiguration::parse(include_str!("../tests/fixtures/report.json")).is_err());
        let mut duplicate = configuration();
        duplicate.tools.push(duplicate.tools[0].clone());
        assert!(
            check_configuration(
                Some(&duplicate),
                |_| panic!("unexpected lookup"),
                |_| panic!("unexpected version query")
            )
            .is_err()
        );
        for path in ["submit.py", "C:submit.py", "", "\0"] {
            let mut config = configuration();
            config.tools[0].path = Some(path.into());
            assert!(config.validate().is_err());
        }
        for (field, value) in [
            ("schema_version", serde_json::json!(99)),
            ("config_version", serde_json::json!("")),
            ("search_directories", serde_json::json!([])),
        ] {
            let mut json = serde_json::to_value(configuration()).unwrap();
            json[field] = value;
            assert!(ToolConfiguration::parse(&json.to_string()).is_err());
        }
    }
}
