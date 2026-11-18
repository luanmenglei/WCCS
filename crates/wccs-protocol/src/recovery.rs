//! Explicit post-contest inspection, archive, cleanup and restart contracts.
use serde::{Deserialize, Serialize};

pub const MAX_FILES: usize = 4096;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_MANIFEST_BYTES: usize = 128 * 1024 * 1024;

pub fn volume_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

pub fn deployment_volume(path: &str) -> bool {
    volume_root(path) && path.as_bytes()[0].eq_ignore_ascii_case(&b'D')
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_entries: usize,
    pub archive_bytes: u64,
    pub timeout_seconds: u64,
}
impl Limits {
    pub fn valid(&self) -> bool {
        (1..=MAX_ENTRIES).contains(&self.max_entries)
            && self.archive_bytes <= MAX_BYTES
            && (60..=86400).contains(&self.timeout_seconds)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    pub action: String,
    pub contest_id: String,
    pub target: String,
    pub scope: String,
    pub extensions: Vec<String>,
    pub skip_reason: String,
    pub previous_request: Option<String>,
    pub confirm_session_end: bool,
}
impl Options {
    pub fn valid(&self) -> bool {
        self.limits.as_ref().is_none_or(Limits::valid)
            && ["inspect", "archive", "clean", "reboot"].contains(&self.action.as_str())
            && crate::valid_id(&self.contest_id)
            && ((self.scope == "volume" && deployment_volume(&self.target))
                || (self.scope == "workspace" && crate::preparation::directory(&self.target)))
            && ["workspace", "volume"].contains(&self.scope.as_str())
            && self.extensions.len() <= 32
            && self.extensions.iter().all(|e| {
                !e.is_empty() && e.len() <= 16 && e.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            && ((self.extensions.is_empty() && !self.skip_reason.trim().is_empty())
                || (!self.extensions.is_empty() && self.skip_reason.is_empty()))
            && self.skip_reason.len() <= 1000
            && match &self.previous_request {
                None => self.action == "inspect",
                Some(id) => self.action != "inspect" && crate::valid_id(id),
            }
            && (!["clean", "reboot"].contains(&self.action.as_str()) || self.confirm_session_end)
    }
    pub fn collects(&self, path: &str) -> bool {
        path.rsplit_once('.').is_some_and(|(_, suffix)| {
            self.extensions
                .iter()
                .any(|e| e.eq_ignore_ascii_case(suffix))
        })
    }
    pub fn same_scope(&self, other: &Self) -> bool {
        self.limits == other.limits
            && self.contest_id == other.contest_id
            && self.target == other.target
            && self.scope == other.scope
            && self.extensions == other.extensions
            && self.skip_reason == other.skip_reason
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    pub guid: String,
    pub serial: String,
    pub label: String,
    pub system: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub hashes: Vec<String>,
}
impl File {
    pub fn chunk_length(&self, index: usize) -> Option<usize> {
        self.hashes.get(index)?;
        Some(
            (self
                .size
                .checked_sub((index as u64).checked_mul(crate::distribution::CHUNK_SIZE as u64)?)?)
            .min(crate::distribution::CHUNK_SIZE as u64) as usize,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub target: String,
    pub volume: Volume,
    pub files: Vec<File>,
    pub directories: Vec<String>,
}
pub fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.encode_utf16().count() <= 32_767
        && path.split('/').all(crate::distribution::windows_component)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
impl Manifest {
    pub fn valid_for(&self, options: &Options) -> bool {
        let count = options
            .limits
            .as_ref()
            .map_or(MAX_FILES, |limits| limits.max_entries);
        let budget = options
            .limits
            .as_ref()
            .map_or(MAX_BYTES, |limits| limits.archive_bytes);
        self.valid()
            && options.valid()
            && self.target == options.target
            && self.files.len() + self.directories.len() <= count
            && self
                .files
                .iter()
                .filter(|file| options.limits.is_none() || options.collects(&file.path))
                .try_fold(0u64, |sum, file| sum.checked_add(file.size))
                .is_some_and(|sum| sum <= budget)
            && self.files.iter().all(|file| {
                (!options.collects(&file.path) && options.limits.is_some())
                    || file.hashes.len() as u64
                        == file.size.div_ceil(crate::distribution::CHUNK_SIZE as u64)
            })
    }
    pub fn valid(&self) -> bool {
        let paths: std::collections::HashSet<_> = self
            .files
            .iter()
            .map(|f| f.path.to_lowercase())
            .chain(self.directories.iter().map(|d| d.to_lowercase()))
            .collect();
        (crate::preparation::directory(&self.target) || volume_root(&self.target))
            && self.volume.guid.len() <= 128
            && !self.volume.guid.is_empty()
            && self.volume.serial.len() <= 64
            && !self.volume.serial.is_empty()
            && self.volume.label.len() <= 256
            && self.files.len() + self.directories.len() <= MAX_ENTRIES
            && paths.len() == self.files.len() + self.directories.len()
            && self.files.iter().all(|f| {
                relative(&f.path)
                    && hash(&f.sha256)
                    && (f.hashes.is_empty()
                        || (f.size <= MAX_BYTES
                            && f.hashes.len()
                                == f.size.div_ceil(crate::distribution::CHUNK_SIZE as u64)
                                    as usize))
                    && f.hashes.iter().all(|h| hash(h))
            })
            && self
                .files
                .iter()
                .try_fold(0u64, |sum, f| sum.checked_add(f.size))
                .is_some()
            && self.directories.iter().all(|d| relative(d))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub inspection_job: String,
    pub archive_job: Option<String>,
    pub manifest: Manifest,
}

#[cfg(test)]
mod tests {
    #[test]
    fn paths_cannot_escape_or_alias_windows_names() {
        for p in [
            "../main.cpp",
            "C:/main.cpp",
            "/a",
            "a//b",
            "a/../b",
            "a\\b",
            "a:stream",
            "NUL.cpp",
            "a.",
        ] {
            assert!(!super::relative(p), "{p}");
        }
        assert!(super::relative("题目 A/main.cpp"));
        assert!(super::relative(&format!("{}.cpp", "题".repeat(60))));
        assert!(super::deployment_volume("d:/"));
        for other in ["C:/", "E:/", "D:/sub"] {
            assert!(!super::deployment_volume(other));
        }
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    fn options() -> Options {
        serde_json::from_value(serde_json::json!({"action":"inspect","contest_id":"test","target":"D:\\","scope":"volume","extensions":["cpp"],"skip_reason":"","previous_request":null,"confirm_session_end":false})).unwrap()
    }
    fn manifest() -> Manifest {
        Manifest {
            target: "D:\\".into(),
            volume: Volume {
                guid: "volume".into(),
                serial: "1234".into(),
                label: String::new(),
                system: false,
            },
            files: vec![File {
                path: "large.bin".into(),
                size: MAX_BYTES + 1,
                sha256: crate::bytes_digest(b"synthetic"),
                hashes: vec![],
            }],
            directories: vec![],
        }
    }
    #[test]
    fn legacy_contract_is_preserved_and_extended_cleanup_has_separate_budget() {
        let mut options = options();
        assert!(options.valid());
        assert!(
            serde_json::to_value(&options)
                .unwrap()
                .get("limits")
                .is_none()
        );
        let mut manifest = manifest();
        assert!(!manifest.valid_for(&options));
        options.limits = Some(Limits {
            max_entries: 10000,
            archive_bytes: MAX_BYTES,
            timeout_seconds: 3600,
        });
        assert!(manifest.valid_for(&options));
        manifest.files[0].path = "large.cpp".into();
        assert!(!manifest.valid_for(&options));
        manifest.files[0].path = "large.bin".into();
        manifest.directories = (0..5000).map(|i| format!("directory-{i}")).collect();
        assert!(manifest.valid_for(&options));
        options.limits.as_mut().unwrap().max_entries = 4096;
        assert!(!manifest.valid_for(&options));
    }
    #[test]
    fn limits_are_frozen_bounded_and_sizes_do_not_overflow() {
        let mut options = options();
        options.limits = Some(Limits {
            max_entries: MAX_ENTRIES + 1,
            archive_bytes: MAX_BYTES,
            timeout_seconds: 3600,
        });
        assert!(!options.valid());
        options.limits.as_mut().unwrap().max_entries = 10000;
        assert!(options.valid());
        let mut changed = options.clone();
        changed.limits.as_mut().unwrap().timeout_seconds = 7200;
        assert!(!options.same_scope(&changed));
        let mut manifest = manifest();
        manifest.files[0].size = u64::MAX;
        manifest.files.push(File {
            path: "other.bin".into(),
            size: 1,
            sha256: crate::bytes_digest(b"x"),
            hashes: vec![],
        });
        assert!(!manifest.valid());
        manifest.files[0].hashes = vec![crate::bytes_digest(b"x")];
        manifest.files[0].size = 0;
        assert!(!manifest.valid());
    }
}
