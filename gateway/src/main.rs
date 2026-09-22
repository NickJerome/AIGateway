use anyhow::{Context as _, Result, anyhow};
use gateway::config::Config;
use std::path::PathBuf;

fn main() {
    if let Err(err) = real_main() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn unexpected(arg: &str) -> anyhow::Error {
    anyhow!("unexpected argument \"{arg}\"; usage: gateway run [--config <path>]")
}

fn real_main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {}
        other => {
            return Err(anyhow!(
                "usage: gateway run [--config <path>] (got {other:?})"
            ));
        }
    }
    let mut config_path: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if config_path.is_some() {
                return Err(unexpected("--config"));
            }
            let value = args.next().ok_or_else(|| {
                anyhow!("--config needs a value; usage: gateway run [--config <path>]")
            })?;
            config_path = Some(PathBuf::from(value));
        } else {
            return Err(unexpected(&arg));
        }
    }
    let config_path = config_path.unwrap_or_else(|| PathBuf::from("gateway.toml"));

    let text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let config = Config::from_toml(&text)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let threads = config.threads;
        let data_dir = config.data_dir.display().to_string();
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let server = gateway::http::Server::start(config).await?;
        println!(
            "gateway ready: threads={} plugins={} loaded={} recompiled={} listen={} data_dir={}",
            threads, server.plugins, server.loaded, server.recompiled, server.address, data_dir
        );
        tokio::select! {_=terminate.recv()=>{},_=interrupt.recv()=>{}}
        server.shutdown().await?;
        println!("gateway shutdown: ok");
        Ok(())
    })
}
