use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::config::DaemonConfig;
use crate::storage::Store;
use crate::api::{grpc, uds};

/// Run daemon with default configuration.
pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    run_server_with_config(DaemonConfig::default()).await
}

/// Run daemon with explicit configuration.
pub async fn run_server_with_config(config: DaemonConfig) -> Result<(), Box<dyn std::error::Error>> {
    let (store, config_status) = if config.settings_path.exists() {
        let s = Store::load_from_file(&config.settings_path)?;
        let count = s.leaf_count();
        (s, format!("Loaded {:?} ({} keys)", config.settings_path, count))
    } else {
        let mut s = Store::empty();
        s.set_file_path(config.settings_path.clone());
        (s, format!("Empty store ({:?} not found)", config.settings_path))
    };

    let grpc_addr = SocketAddr::from(([0, 0, 0, 0], config.grpc_port));

    println!("rekv daemon started:");
    println!("  Config : {}", config_status);
    println!("  gRPC   : {}", grpc_addr);
    println!("  UDS    : {:?}", config.uds_path);
    println!("Ready for requests (Ctrl+C to stop)");

    let shared_store = Arc::new(RwLock::new(store));

    let pubsub = Arc::new(crate::pubsub_engine::PubSubEngine::default());

    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Spawn ctrl+c watcher
    let shutdown_tx_ctrlc = shutdown_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            log::error!("Failed to listen for ctrl+c: {}", e);
        }
        println!("\nStopping rekv daemon...");
        let _ = shutdown_tx_ctrlc.send(());
    });

    // Spawn gRPC server
    let grpc_store = Arc::clone(&shared_store);
    let grpc_pubsub = Arc::clone(&pubsub);
    let mut grpc_shutdown_rx = shutdown_tx.subscribe();
    let shutdown_tx_grpc = shutdown_tx.clone();
    let grpc_task = tokio::spawn(async move {
        if let Err(e) = grpc::start_grpc_server_with_shutdown(
            grpc_addr,
            grpc_store,
            grpc_pubsub,
            async move {
                let _ = grpc_shutdown_rx.recv().await;
            },
        )
        .await
        {
            eprintln!("Error: Failed to start gRPC server on {}: {}", grpc_addr, e);
            eprintln!("Hint: Port {} may already be in use. Stop the existing rekv process or pass '--port <PORT>'.", grpc_addr.port());
            let _ = shutdown_tx_grpc.send(());
        }
    });

    // Spawn UDS server
    let uds_store = Arc::clone(&shared_store);
    let uds_path = config.uds_path.clone();
    let mut uds_shutdown_rx = shutdown_tx.subscribe();
    let shutdown_tx_uds = shutdown_tx.clone();
    let uds_task = tokio::spawn(async move {
        if let Err(e) = uds::start_uds_server_with_shutdown(uds_path.clone(), uds_store, async move {
            let _ = uds_shutdown_rx.recv().await;
        })
        .await
        {
            eprintln!("Error: Failed to start Unix socket on {:?}: {}", uds_path, e);
            let _ = shutdown_tx_uds.send(());
        }
    });

    let _ = tokio::join!(grpc_task, uds_task);
    println!("rekv daemon stopped.");

    Ok(())
}
