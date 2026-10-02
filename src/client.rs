use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::api::uds::send_rpc_request;
use crate::config_value::ConfigValue;
use crate::protocol::RpcRequest;

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub struct ConfigClient {
    socket_path: PathBuf,
}

impl ConfigClient {
    pub async fn connect<P: AsRef<Path>>(socket_path: P) -> Result<Self, String> {
        tokio::net::UnixStream::connect(socket_path.as_ref())
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            socket_path: socket_path.as_ref().to_path_buf(),
        })
    }

    pub async fn get(&self, path: &str) -> Result<ConfigValue, String> {
        let response = self.call("get", path, None).await?;
        serde_json::from_value(
            response
                .result
                .ok_or_else(|| "missing GET result".to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    pub async fn set(&self, path: &str, value: ConfigValue) -> Result<(), String> {
        self.call("set", path, Some(value)).await.map(|_| ())
    }

    pub async fn delete(&self, path: &str) -> Result<(), String> {
        self.call("delete", path, None).await.map(|_| ())
    }

    pub async fn list(&self, path: &str) -> Result<Vec<String>, String> {
        let result = self
            .call("list", path, None)
            .await?
            .result
            .ok_or_else(|| "missing LIST result".to_string())?;
        serde_json::from_value::<Vec<String>>(result).map_err(|error| error.to_string())
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        value: Option<ConfigValue>,
    ) -> Result<crate::protocol::RpcResponse, String> {
        let request = RpcRequest {
            id: REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            method: method.to_string(),
            path: Some(path.to_string()),
            value,
        };
        let response = send_rpc_request(&self.socket_path, &request)
            .await
            .map_err(|error| error.to_string())?;
        if response.ok {
            Ok(response)
        } else {
            Err(response
                .error
                .map(|error| format!("{:?}: {}", error.code, error.message))
                .unwrap_or_else(|| "request failed".to_string()))
        }
    }
}
