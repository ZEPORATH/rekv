use clap::Parser;

use crate::api::run_server_with_config;
use crate::cli::{
    execute_backup, execute_delete, execute_get, execute_restore, execute_set, execute_watch, Cli,
    Commands,
};
use crate::config::DaemonConfig;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();
    let address = cli.address.as_deref();
    let uds_path = &cli.uds;

    match cli.command {
        Some(Commands::Daemon {
            config,
            port,
            http_port,
            bind_address,
            uds,
        }) => {
            let daemon_config = DaemonConfig::new(config)
                .with_grpc_port(port)
                .with_http_port(http_port)
                .with_bind_address(bind_address)
                .with_uds_path(uds);
            run_server_with_config(daemon_config).await?;
        }
        Some(Commands::Get { path }) => execute_get(&path, address, uds_path).await?,
        Some(Commands::Set { path, value }) => {
            execute_set(&path, &value, address, uds_path).await?
        }
        Some(Commands::Delete { path }) => execute_delete(&path, address, uds_path).await?,
        Some(Commands::Backup) => execute_backup(address, uds_path).await?,
        Some(Commands::Restore { path }) => execute_restore(&path, address, uds_path).await?,
        Some(Commands::Watch { path }) => execute_watch(&path, address).await?,
        None => {
            if std::env::current_exe()?
                .file_stem()
                .is_some_and(|name| name == "rekv-cli")
            {
                use clap::CommandFactory;
                Cli::command().print_help()?;
                println!();
            } else {
                run_server_with_config(DaemonConfig::default()).await?;
            }
        }
    }
    Ok(())
}
