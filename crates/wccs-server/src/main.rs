#![forbid(unsafe_code)]
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use std::{fs, path::Path};
use wccs_protocol::token;
use wccs_server::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || !matches!(args[1].as_str(), "--init" | "--config" | "--check-standby") {
        return Err(
            "用法 wccs-server (--init | --check-standby | --config) SERVER_CONFIG.json".into(),
        );
    }
    let path = Path::new(&args[2]);
    if args[1] == "--init" {
        if path.exists() || path.with_file_name("admin-access.json").exists() {
            return Err("初始化目标已有配置或登录材料，请使用独立目录".into());
        }
        let password = token();
        let salt = SaltString::encode_b64(&token().as_bytes()[..32]).map_err(|_| "salt")?;
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| "password hash")?
            .to_string();
        let config = Config { domjudge_config: None, bind: "127.0.0.1:8787".into(), origin: "http://127.0.0.1:8787".into(), database_url: "host=127.0.0.1 port=55432 user=wccs dbname=wccs connect_timeout=5 options='-c statement_timeout=10000'".into(), static_dir: "web/dist".into(), trusted_proxies: vec![], admin_username: "admin".into(), admin_password_hash: hash };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let access_path = path.with_file_name("admin-access.json");
        // create_new prevents accidentally replacing an existing deployment identity.
        let mut access = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&access_path)?;
        serde_json::to_writer_pretty(
            &mut access,
            &serde_json::json!({"username": "admin", "password": password}),
        )?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        serde_json::to_writer_pretty(&mut file, &config)?;
        println!(
            "已创建配置 {}，登录材料保存在 {}",
            path.display(),
            access_path.display()
        );
        return Ok(());
    }
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    if args[1] == "--check-standby" {
        println!("{}", wccs_server::deployment::check_standby(&config).await?);
        return Ok(());
    }
    let address = config.bind.clone();
    let listener = tokio::net::TcpListener::bind(&address).await?;
    let Some(mut lease) = wccs_server::deployment::ControllerLease::acquire(&config).await? else {
        eprintln!("此数据库已有活动控制器，请先核对并停止旧实例");
        std::process::exit(78);
    };
    tokio::select! {
        biased;
        _ = lease.lost() => Err("控制器数据库会话已中断，本进程停止服务".into()),
        result = serve(config, listener) => result,
    }
}

async fn serve(
    config: Config,
    listener: tokio::net::TcpListener,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = wccs_server::connect(config).await?;
    println!("WCCS listening on {}", listener.local_addr()?);
    axum::serve(
        listener,
        wccs_server::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
