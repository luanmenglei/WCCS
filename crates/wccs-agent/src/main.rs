#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(windows)]
mod connection;
#[cfg(windows)]
mod diagnostics;
#[cfg(windows)]
mod file_transaction;
#[cfg(windows)]
mod health;
#[cfg(windows)]
mod host;
#[cfg(windows)]
mod login;
#[cfg(windows)]
mod login_broker;
#[cfg(windows)]
mod login_native;
#[cfg(windows)]
mod login_windows;
#[cfg(windows)]
mod maintenance;
#[cfg(windows)]
mod preparation;
#[cfg(windows)]
mod presentation;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod waiting_policy;

#[cfg(windows)]
mod windows_agent;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).is_some_and(|a| a == "--health") && args.len() == 3 {
            println!(
                "{}",
                serde_json::to_string_pretty(&health::sample(
                    std::path::Path::new(&args[2]),
                    &wccs_protocol::token(),
                    1,
                    &mut wccs_inventory::HealthCollector::default()
                )?)?
            );
            return Ok(());
        }
        if args.get(1).is_some_and(|a| a == "--identify") {
            if args.len() != 5 {
                return Err("用法 --identify IDENTITY SESSION_EXE SECONDS".into());
            }
            return maintenance::identify_local(
                std::path::Path::new(&args[2]),
                std::path::Path::new(&args[3]),
                args[4].parse()?,
            );
        }
        if args.get(1).is_some_and(|a| {
            [
                "--setup",
                "--setup-status",
                "--configure",
                "--diagnose",
                "--diagnose-test",
            ]
            .contains(&a.as_str())
        }) {
            return diagnostics::run(&args[1..]);
        }
        if args.get(1).is_some_and(|a| a == "--provision-login-native") {
            if args.len() != 4 {
                return Err("用法 --provision-login-native CONFIG SECRET_FILE".into());
            }
            login_native::provision(
                std::path::Path::new(&args[2]),
                std::path::Path::new(&args[3]),
            )
            .map_err(|_| "native_provision_failed")?;
            println!("native_recovery_provisioned");
            return Ok(());
        }
        if args.get(1).is_some_and(|a| a == "--recover-login-native") {
            if args.len() != 4 {
                return Err("用法 --recover-login-native IDENTITY SESSION_EXE".into());
            }
            use std::os::windows::fs::OpenOptionsExt;
            let _lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(std::path::Path::new(&args[2]).with_extension("lock"))?;
            let policy_recovery = waiting_policy::recover(
                &std::path::Path::new(&args[2]).with_extension("waiting-policy.json"),
            );
            let identity: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[2])?)?;
            let mut native = login_native::Native::open(
                &std::path::Path::new(&args[2]).with_extension("login-native.json"),
                identity["device_id"].as_str().ok_or("device_missing")?,
                args[3].clone().into(),
            )
            .map_err(|_| "native_recovery_open_failed")?;
            native.recover().map_err(|_| "native_recovery_failed")?;
            policy_recovery.map_err(|_| "waiting_policy_recovery_failed")?;
            println!("{}", serde_json::to_string(&native.state())?);
            return Ok(());
        }
        if args
            .get(1)
            .is_some_and(|a| a == "--recover-login-simulation")
        {
            if args.len() != 4 {
                return Err("用法 --recover-login-simulation DIRECTORY DEVICE_ID".into());
            }
            let directory = std::path::PathBuf::from(&args[2]).canonicalize()?;
            let result = login::Simulator::open(directory, &args[3]);
            let mut simulator = result.map_err(|_| "simulation_recovery_open_failed")?;
            simulator
                .recover()
                .map_err(|_| "simulation_recovery_failed")?;
            println!("{}", serde_json::to_string(&simulator.state())?);
            return Ok(());
        }
        let simulate_login = args.last().is_some_and(|a| a == "--simulate-login");
        let args = if simulate_login {
            &args[..args.len() - 1]
        } else {
            &args[..]
        };
        if args
            .get(1)
            .is_some_and(|a| a == "--foreground" || a == "--service")
        {
            let service = args[1] == "--service";
            if service && simulate_login {
                return Err("simulation_requires_foreground".into());
            }
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
                    simulate_login,
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
