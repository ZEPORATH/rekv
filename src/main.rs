use clap::Parser;
use rekv::api::run_server_with_config;
use rekv::cli::{execute_get, execute_set, execute_watch, Cli, Commands};
use rekv::config::DaemonConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let cli = Cli::parse();
    let address = cli.address.as_deref();
    let uds_path = &cli.uds;

    match cli.command {
        Some(Commands::Daemon { config, port, uds }) => {
            let daemon_cfg = DaemonConfig::new(config)
                .with_grpc_port(port)
                .with_uds_path(uds);
            run_server_with_config(daemon_cfg).await?;
        }
        Some(Commands::Get { path }) => {
            execute_get(&path, address, uds_path).await?;
        }
        Some(Commands::Set { path, value }) => {
            execute_set(&path, &value, address, uds_path).await?;
        }
        Some(Commands::Watch { path }) => {
            execute_watch(&path, address).await?;
        }
        None => {
            // Default: start the daemon with default settings
            run_server_with_config(DaemonConfig::default()).await?;
        }
    }

    Ok(())
}
