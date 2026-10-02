use crate::constants::{
    DEFAULT_CONFIG_PATH, DEFAULT_GRPC_PORT, DEFAULT_HTTP_PORT, DEFAULT_LOCAL_HOST,
    DEFAULT_UDS_PATH,
};
use std::path::PathBuf;

/// Daemon configuration options.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Path to JSON settings file on disk.
    pub settings_path: PathBuf,
    /// Port for the gRPC server (default: 50051).
    pub grpc_port: u16,
    /// Port for the HTTP API (default: 8080).
    pub http_port: u16,
    /// Network interface used by gRPC and HTTP (loopback by default).
    pub bind_address: String,
    /// Path to Unix Domain Socket (default: /var/run/rekv.sock).
    pub uds_path: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            settings_path: PathBuf::from(DEFAULT_CONFIG_PATH),
            grpc_port: DEFAULT_GRPC_PORT,
            http_port: DEFAULT_HTTP_PORT,
            bind_address: DEFAULT_LOCAL_HOST.to_string(),
            uds_path: PathBuf::from(DEFAULT_UDS_PATH),
        }
    }
}

impl DaemonConfig {
    pub fn new<P: Into<PathBuf>>(settings_path: P) -> Self {
        Self {
            settings_path: settings_path.into(),
            ..Default::default()
        }
    }

    pub fn with_grpc_port(mut self, port: u16) -> Self {
        self.grpc_port = port;
        self
    }

    pub fn with_http_port(mut self, port: u16) -> Self {
        self.http_port = port;
        self
    }

    pub fn with_bind_address(mut self, address: impl Into<String>) -> Self {
        self.bind_address = address.into();
        self
    }

    pub fn with_uds_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.uds_path = path.into();
        self
    }
}
