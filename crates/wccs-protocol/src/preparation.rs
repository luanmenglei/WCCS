//! Versioned, fixed desktop preparation inputs. Packages are deployed as data.
use crate::{digest, valid_id, windows_absolute_file};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub path: String,
    pub content: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub configuration_version: String,
    pub runtime_tool_id: String,
    pub runtime_file_version: String,
    pub source: String,
    pub license: String,
    pub files: Vec<PackageFile>,
}
pub fn relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                && ![
                    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
                    "LPT8", "LPT9",
                ]
                .contains(
                    &part
                        .split('.')
                        .next()
                        .unwrap()
                        .to_ascii_uppercase()
                        .as_str(),
                )
        })
}
pub fn directory(path: &str) -> bool {
    windows_absolute_file(path)
        && path.as_bytes().get(1) == Some(&b':')
        && path[3..].split(['\\', '/']).all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with(['.', ' '])
                && !p.contains([':', '"', '*', '?', '<', '>', '|'])
        })
}
impl Package {
    pub fn valid(&self) -> bool {
        let total: usize = self.files.iter().map(|f| f.content.len()).sum();
        self.schema_version == 1
            && valid_id(&self.id)
            && valid_id(&self.version)
            && valid_id(&self.configuration_version)
            && valid_id(&self.runtime_tool_id)
            && !self.runtime_file_version.is_empty()
            && self.runtime_file_version.len() <= 100
            && !self.source.is_empty()
            && self.source.len() <= 300
            && !self.license.is_empty()
            && self.license.len() <= 300
            && !self.files.is_empty()
            && self.files.len() <= 64
            && total <= 128 * 1024
            && self.files.iter().all(|f| {
                relative_path(&f.path)
                    && !f.path.eq_ignore_ascii_case("manifest.json")
                    && digest(&f.content) == f.sha256
            })
            && self
                .files
                .iter()
                .map(|f| f.path.to_ascii_lowercase())
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.files.len()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Link {
    Tool { name: String, tool_id: String },
    Website { name: String, url: String },
    Resource { name: String, path: String },
}
impl Link {
    pub fn name(&self) -> &str {
        match self {
            Self::Tool { name, .. } | Self::Website { name, .. } | Self::Resource { name, .. } => {
                name
            }
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub tool_version: i64,
    pub work_directory: String,
    pub desktop_directory: String,
    pub script_directory: String,
    pub links: Vec<Link>,
    pub package: Option<Package>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub competition: Option<Competition>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Competition {
    pub compiler_tool_id: String,
    pub compiler_arguments: Vec<String>,
    pub judge_url: String,
    pub contest_id: String,
    pub language_id: String,
}

pub fn domjudge_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 36
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

impl Competition {
    pub fn valid(&self) -> bool {
        valid_id(&self.compiler_tool_id)
            && domjudge_id(&self.contest_id)
            && domjudge_id(&self.language_id)
            && self.judge_url.starts_with("https://")
            && self.judge_url.len() <= 2048
            && !self.judge_url.chars().any(char::is_control)
            && self.compiler_arguments.len() <= 32
            && self.compiler_arguments.iter().all(|argument| {
                !argument.is_empty()
                    && argument.len() <= 256
                    && !argument.chars().any(char::is_control)
            })
    }
}
impl Profile {
    pub fn valid(&self) -> bool {
        self.schema_version == 1
            && self.tool_version > 0
            && [
                &self.work_directory,
                &self.desktop_directory,
                &self.script_directory,
            ]
            .iter()
            .all(|p| directory(p))
            && self.links.len() <= 32
            && self.links.iter().all(|l| {
                !l.name().is_empty()
                    && l.name().len() <= 80
                    && !l
                        .name()
                        .chars()
                        .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
                    && !l.name().ends_with(['.', ' '])
            })
            && self
                .links
                .iter()
                .map(|l| l.name().to_lowercase())
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.links.len()
            && self.package.as_ref().is_none_or(Package::valid)
            && self.competition.as_ref().is_none_or(|competition| {
                competition.valid()
                    && self.package.as_ref().is_some_and(|package| {
                        ["compilecpp.ps1", "submit.ps1", "contest-common.ps1"]
                            .iter()
                            .all(|name| package.files.iter().any(|file| file.path == *name))
                            && !package
                                .files
                                .iter()
                                .any(|file| file.path.eq_ignore_ascii_case("contest.json"))
                    })
            })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedTool {
    pub id: String,
    pub path: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: i64,
    pub profile: Profile,
    pub tools: Vec<ResolvedTool>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub phase: String,
    pub path: String,
    pub outcome: String,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn domjudge_ids_match_clics_length_and_boundaries() {
        for id in ["a", "a-b.c_d", &"a".repeat(36)] {
            assert!(super::domjudge_id(id));
        }
        for id in ["", "-a", "a_", "a/b", "中文", &"a".repeat(37)] {
            assert!(!super::domjudge_id(id));
        }
    }

    #[test]
    fn rejects_package_traversal_device_names_and_path_aliases() {
        for path in [
            "../x", "a/../x", "C:/x", "a\\x", "nul.txt", "COM1.py", "a/", "a.",
        ] {
            assert!(!super::relative_path(path), "{path}");
        }
        assert!(super::relative_path("bin/compilecpp.ps1"));
        assert!(!super::directory("D:\\Contest\\..\\Windows"));
        assert!(!super::directory("D:\\"));
    }
}
