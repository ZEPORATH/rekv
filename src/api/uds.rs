use std::fs;
use std::path::Path;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use crate::path_resolver::{resolve_get, resolve_set};
use crate::storage::Store;

/// Request payload sent over Unix Domain Socket (line-delimited JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UdsRequest {
    Get { path: String },
    Set { path: String, value: Value },
}

/// Response payload returned over Unix Domain Socket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UdsResponse {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        found: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        match_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        affected_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        affected_paths: Option<Vec<String>>,
    },
    Error {
        error: String,
    },
}

impl UdsResponse {
    pub fn get_success(found: bool, match_count: usize, value: Value) -> Self {
        UdsResponse::Ok {
            found: Some(found),
            match_count: Some(match_count),
            value: Some(value),
            affected_count: None,
            affected_paths: None,
        }
    }

    pub fn set_success(affected_paths: Vec<String>) -> Self {
        let affected_count = affected_paths.len();
        UdsResponse::Ok {
            found: None,
            match_count: None,
            value: None,
            affected_count: Some(affected_count),
            affected_paths: Some(affected_paths),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        UdsResponse::Error { error: msg.into() }
    }
}

/// Start Unix Domain Socket listener with graceful shutdown.
pub async fn start_uds_server_with_shutdown<F, P>(
    uds_path: P,
    store: Arc<RwLock<Store>>,
    signal: F,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    P: AsRef<Path>,
    F: std::future::Future<Output = ()>,
{
    let path_buf = uds_path.as_ref().to_path_buf();

    // 1. Ensure directory exists
    if let Some(parent) = path_buf.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    // 2. Remove any old stale socket file
    if path_buf.exists() {
        let _ = fs::remove_file(&path_buf);
    }

    // 3. Bind listener
    let listener = UnixListener::bind(&path_buf)?;
    log::info!("UDS listener bound at {:?}", path_buf);

    tokio::pin!(signal);

    loop {
        tokio::select! {
            _ = &mut signal => {
                log::info!("UDS server shutting down...");
                break;
            }
            res = listener.accept() => {
                match res {
                    Ok((stream, _)) => {
                        let store_clone = Arc::clone(&store);
                        tokio::spawn(async move {
                            if let Err(e) = handle_uds_client(stream, store_clone).await {
                                log::debug!("UDS client session closed: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        log::error!("UDS accept error: {}", e);
                    }
                }
            }
        }
    }

    // Cleanup socket file on exit
    let _ = fs::remove_file(&path_buf);
    Ok(())
}

/// Handle an individual UDS client connection
async fn handle_uds_client(
    stream: UnixStream,
    store: Arc<RwLock<Store>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<UdsRequest>(trimmed) {
            Ok(UdsRequest::Get { path }) => {
                let s = store.read().await;
                match resolve_get(&path, &s) {
                    Ok(result) => {
                        let match_count = result.match_count();
                        let found = match_count > 0;
                        let val = result.to_json_value();
                        UdsResponse::get_success(found, match_count, val)
                    }
                    Err(err) => UdsResponse::error(err),
                }
            }
            Ok(UdsRequest::Set { path, value }) => {
                let mut s = store.write().await;
                match resolve_set(&path, value, &mut s) {
                    Ok(set_res) => {
                        let _ = s.persist();
                        UdsResponse::set_success(set_res.affected_paths)
                    }
                    Err(err) => UdsResponse::error(err),
                }
            }
            Err(e) => UdsResponse::error(format!("Invalid request JSON: {}", e)),
        };

        let mut resp_bytes = serde_json::to_vec(&response)?;
        resp_bytes.push(b'\n');
        writer.write_all(&resp_bytes).await?;
        writer.flush().await?;
    }

    Ok(())
}

/// Send a single UDS request and wait for the response (client helper).
pub async fn send_uds_request<P: AsRef<Path>>(
    socket_path: P,
    request: &UdsRequest,
) -> Result<UdsResponse, Box<dyn std::error::Error>> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();

    let mut req_bytes = serde_json::to_vec(request)?;
    req_bytes.push(b'\n');
    writer.write_all(&req_bytes).await?;
    writer.flush().await?;

    let mut lines = BufReader::new(reader).lines();
    if let Some(line) = lines.next_line().await? {
        let resp: UdsResponse = serde_json::from_str(&line)?;
        Ok(resp)
    } else {
        Err("Connection closed before response received".into())
    }
}
