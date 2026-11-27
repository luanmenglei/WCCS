//! 凭据、任务和恢复日志共用的本机文件写入操作。
use crate::AgentResult as Result;
use std::{fs, path::Path};

pub(crate) fn persist(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("durable.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}
