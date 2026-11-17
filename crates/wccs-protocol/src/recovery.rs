//! Explicit post-contest inspection, archive, cleanup and restart contracts.
use serde::{Deserialize, Serialize};

pub const MAX_FILES: usize = 4096;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Options {
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
        ["inspect", "archive", "clean", "reboot"].contains(&self.action.as_str())
            && crate::valid_id(&self.contest_id)
            && crate::preparation::directory(&self.target)
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
        self.contest_id == other.contest_id
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
            (self.size - index as u64 * crate::distribution::CHUNK_SIZE as u64)
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
        && path.len() <= 1024
        && path
            .split('/')
            .all(|part| part == ".wccs-owner.json" || crate::distribution::filename(part))
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
impl Manifest {
    pub fn valid(&self) -> bool {
        let paths: std::collections::HashSet<_> = self
            .files
            .iter()
            .map(|f| f.path.to_lowercase())
            .chain(self.directories.iter().map(|d| d.to_lowercase()))
            .collect();
        crate::preparation::directory(&self.target)
            && self.volume.guid.len() <= 128
            && !self.volume.guid.is_empty()
            && self.volume.serial.len() <= 64
            && !self.volume.serial.is_empty()
            && self.volume.label.len() <= 256
            && self.files.len() + self.directories.len() <= MAX_FILES
            && paths.len() == self.files.len() + self.directories.len()
            && self.files.iter().all(|f| {
                relative(&f.path)
                    && f.size <= MAX_BYTES
                    && hash(&f.sha256)
                    && f.hashes.len()
                        == f.size.div_ceil(crate::distribution::CHUNK_SIZE as u64) as usize
                    && f.hashes.iter().all(|h| hash(h))
            })
            && self.files.iter().map(|f| f.size).sum::<u64>() <= MAX_BYTES
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
    }
}
