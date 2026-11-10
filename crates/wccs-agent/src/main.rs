#![forbid(unsafe_code)]

#[cfg(windows)]
mod windows_agent;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    return windows_agent::run();
    #[cfg(not(windows))]
    Err("工具检查入口需要实际 Windows 环境".into())
}
