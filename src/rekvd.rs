// This file will now contain the `rekvd` service setup,
// which orchestrates the gRPC and UDS servers.

use crate::config::ConfigStore;
use std::sync::Arc;

pub mod grpc;
pub mod uds;

pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let config_store = Arc::new(ConfigStore::new("/var/lib/rekv/state.json".to_string()));

    // Placeholder for starting gRPC and UDS servers
    log::info!("Starting rekvd service...");
    log::warn!("gRPC and UDS servers are not yet implemented.");

    // Example of how you might start them concurrently
    // tokio::try_join!(
    //     grpc::start_grpc_server(config_store.clone()),
    //     uds::start_uds_server(config_store.clone()),
    // )?;

    // For now, just keep the main thread alive for a bit or until interrupted
    tokio::signal::ctrl_c().await?;
    log::info!("rekvd service shutting down.");

    Ok(())
}
