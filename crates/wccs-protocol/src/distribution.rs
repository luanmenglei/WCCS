//! Chunked HTTPS distribution. Payloads are files and are never executed.
use serde::{Deserialize, Serialize};

pub const CHUNK_SIZE: usize = 256 * 1024;
pub const MAX_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub hashes: Vec<String>,
}
impl Manifest {
    pub fn valid(&self) -> bool {
        crate::valid_id(&self.id)
            && filename(&self.name)
            && self.size > 0
            && self.size <= MAX_SIZE
            && self.hashes.len() == self.size.div_ceil(CHUNK_SIZE as u64) as usize
            && self.hashes.iter().all(|hash| {
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
    }
    pub fn chunk_length(&self, index: usize) -> Option<usize> {
        self.hashes.get(index)?;
        Some((self.size - index as u64 * CHUNK_SIZE as u64).min(CHUNK_SIZE as u64) as usize)
    }
}
pub fn filename(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    !name.is_empty()
        && !name.eq_ignore_ascii_case(".wccs-owner.json")
        && name.len() <= 180
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        && ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
            "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³",
        ]
        .contains(&stem.as_str())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub job_id: String,
    pub device_id: String,
    pub generation: i64,
    pub manifest: Manifest,
    pub directory: String,
    pub publish: bool,
    pub bytes_per_second: u64,
    pub adapter: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub job_id: String,
    pub stage: String,
    pub verified_chunks: usize,
    pub detail: String,
}
impl Progress {
    pub fn valid(&self) -> bool {
        crate::valid_id(&self.job_id)
            && ["downloading", "ready", "published", "failed"].contains(&self.stage.as_str())
            && [
                "chunks_verified",
                "awaiting_publication",
                "file_published",
                "transfer_failed",
                "target_changed",
                "target_reparse_point",
                "file_conflict",
                "cache_corrupt",
            ]
            .contains(&self.detail.as_str())
            && self.verified_chunks <= (MAX_SIZE / CHUNK_SIZE as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_names_and_chunks_are_bounded() {
        for name in [
            "../x",
            "a/b",
            "a\\b",
            "C:x",
            "nul.pdf",
            "con",
            "a.",
            "a ",
            "",
            "..",
            "COM¹.txt",
        ] {
            assert!(!super::filename(name));
        }
        assert!(super::filename("竞赛资料 v1.pdf"));
        let manifest = super::Manifest {
            id: "test".into(),
            name: "notes.pdf".into(),
            size: 262145,
            hashes: vec![crate::digest("a"), crate::digest("b")],
        };
        assert!(manifest.valid());
        assert_eq!(manifest.chunk_length(0), Some(262144));
        assert_eq!(manifest.chunk_length(1), Some(1));
        assert_eq!(manifest.chunk_length(2), None);
    }
}
