use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::constants::{
    DEFAULT_CONFIG_PATH, DEFAULT_GRPC_PORT, DEFAULT_HTTP_PORT, DEFAULT_UDS_PATH,
};

/// Top-level command-line arguments.
#[derive(Parser, Debug)]
#[command(
    name = "rekv",
    author,
    version,
    about = "Rust Embedded Key-Value & Config Store"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Address of remote rekvd service (e.g. 127.0.0.1:50051)
    #[arg(short, long, global = true)]
    pub address: Option<String>,

    /// Path to Unix Domain Socket (default: /var/run/rekv.sock)
    #[arg(short, long, global = true, default_value = DEFAULT_UDS_PATH)]
    pub uds: PathBuf,
}

/// Available CLI subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the rekvd daemon service
    Daemon {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,

        /// Port for gRPC service
        #[arg(short, long, default_value_t = DEFAULT_GRPC_PORT)]
        port: u16,

        /// Port for the loopback HTTP API
        #[arg(long, default_value_t = DEFAULT_HTTP_PORT)]
        http_port: u16,

        /// Interface for gRPC and HTTP listeners (default: 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        bind_address: String,

        /// Path for Unix Domain Socket
        #[arg(short, long, default_value = DEFAULT_UDS_PATH)]
        uds: PathBuf,
    },

    /// Retrieve configuration value(s) by path
    Get {
        /// XPath-like path query (e.g. /platform_manager/grpc_port)
        path: String,
    },

    /// Set configuration value at path
    Set {
        /// XPath-like path query
        path: String,
        /// JSON value string to set
        value: String,
    },

    /// Delete a configuration value by path
    Delete { path: String },

    /// Flush changed paths to the sibling _delta.json file
    Backup,

    /// Restore one setting from the original JSON file
    Restore { path: String },

    /// Watch configuration changes on a path in real-time
    Watch {
        /// XPath-like path query to watch
        path: String,
    },
}
