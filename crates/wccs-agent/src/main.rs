#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(windows)]
mod host;
#[cfg(windows)]
mod service;

#[cfg(windows)]
mod windows_agent;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args
            .get(1)
            .is_some_and(|a| a == "--foreground" || a == "--service")
        {
            let service = args[1] == "--service";
            let start = if service { 3 } else { 2 };
            if args.len() != start + 5
                || args[start] != "--identity"
                || args[start + 2] != "--session-exe"
                || args[start + 4] != if service { "--managed" } else { "--test-mode" }
            {
                return Err("用法 wccs-agent --foreground --identity FILE --session-exe EXE --test-mode 或 --service WCCS-NAME --identity FILE --session-exe EXE --managed".into());
            }
            let path = args[start + 1].clone().into();
            let exe = args[start + 3].clone().into();
            let result = if service {
                service::dispatch(args[2].clone(), path, exe)
            } else {
                host::run(
                    path,
                    exe,
                    true,
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                )
            };
            return result.map_err(|e| e as Box<dyn std::error::Error>);
        }
        windows_agent::run()
    }
    #[cfg(not(windows))]
    Err("工具检查入口需要实际 Windows 环境".into())
}
