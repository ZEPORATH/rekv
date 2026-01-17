mod api;
mod config;
mod redis;

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::net::TcpListener;

use api::AppState;
use config::ConfigStore;
use redis::RedisNotifier;

/// Default configuration file path
const DEFAULT_CONFIG_PATH: &str = "/var/lib/myservice/state.json";

/// Default Redis URL
const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

/// Default Redis channel for notifications
const DEFAULT_REDIS_CHANNEL: &str = "rekv:config:updates";

/// Default HTTP server bind address
const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

#[tokio::main]
async fn main() -> Result<()> {
    // Simple logging setup (you can replace with env_logger or tracing if needed)
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    // Get configuration from environment or use defaults
    let config_path = std::env::var("REKV_CONFIG_PATH")
        .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
    let redis_url = std::env::var("REKV_REDIS_URL")
        .unwrap_or_else(|_| DEFAULT_REDIS_URL.to_string());
    let redis_channel = std::env::var("REKV_REDIS_CHANNEL")
        .unwrap_or_else(|_| DEFAULT_REDIS_CHANNEL.to_string());
    let bind_addr = std::env::var("REKV_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());

    println!("Starting rekv service...");
    println!("  Config path: {}", config_path);
    println!("  Redis URL: {}", redis_url);
    println!("  Redis channel: {}", redis_channel);
    println!("  Bind address: {}", bind_addr);

    // Initialize config store
    let config_store = Arc::new(ConfigStore::new(config_path.clone()));
    config_store.load()
        .context("Failed to load initial configuration")?;
    println!("Loaded configuration from {}", config_path);

    // Initialize Redis notifier
    let redis_notifier = Arc::new(
        RedisNotifier::new(&redis_url, redis_channel.clone())
            .context("Failed to initialize Redis client")?
    );
    println!("Connected to Redis at {}", redis_url);

    // Create application state
    let app_state = Arc::new(AppState {
        config: config_store,
        redis: redis_notifier,
    });

    // Build the API router
    let app = api::create_router(app_state);

    // Start the HTTP server
    let listener = TcpListener::bind(&bind_addr).await
        .context(format!("Failed to bind to {}", bind_addr))?;
    
    println!("HTTP server listening on http://{}", bind_addr);
    println!("API endpoints:");
    println!("  GET  /config          - Get full configuration");
    println!("  GET  /config/:path    - Get subtree by path");
    println!("  POST /config/:path    - Set value at path");

    axum::serve(listener, app).await
        .context("HTTP server error")?;

    Ok(())
}
