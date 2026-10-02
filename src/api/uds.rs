use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{unix::OwnedWriteHalf, UnixListener, UnixStream};
use tokio::sync::RwLock;

use crate::config_service::ConfigService;
use crate::config_value::ConfigValue;
use crate::protocol::{ErrorCode, RpcRequest, RpcResponse};
use crate::pubsub_engine::PubSubEngine;
use crate::storage::Store;

const MAX_REQUEST_BYTES: usize = 64 * 1024;

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
    let service = ConfigService::new(store, Arc::new(PubSubEngine::default()));
    start_uds_server_with_service(uds_path, service, signal).await
}

pub async fn start_uds_server_with_service<F, P>(
    uds_path: P,
    service: ConfigService,
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

    // 2. Remove only an existing socket; never unlink an arbitrary file.
    if path_buf.exists() {
        use std::os::unix::fs::FileTypeExt;
        let metadata = fs::symlink_metadata(&path_buf)?;
        if !metadata.file_type().is_socket() {
            return Err(format!("refusing to replace non-socket path {:?}", path_buf).into());
        }
        fs::remove_file(&path_buf)?;
    }

    // 3. Bind listener
    let listener = UnixListener::bind(&path_buf)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path_buf, fs::Permissions::from_mode(0o600))?;
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
                        let service_clone = service.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_uds_client(stream, service_clone).await {
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
    service: ConfigService,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read = (&mut reader)
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .await?;
        if bytes_read == 0 {
            break;
        }
        if line.len() > MAX_REQUEST_BYTES {
            let response =
                RpcResponse::failure(0, ErrorCode::InvalidRequest, "request exceeds maximum size");
            let mut bytes = serde_json::to_vec(&response)?;
            bytes.push(b'\n');
            writer.write_all(&bytes).await?;
            break;
        }
        let trimmed = std::str::from_utf8(&line)?.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(raw) = serde_json::from_str::<Value>(trimmed) {
            if raw.get("method").and_then(Value::as_str) == Some("watch") {
                let request: RpcRequest = serde_json::from_value(raw.clone())?;
                if let Some(path) = request.path.as_deref() {
                    watch_uds_client(&mut reader, &mut writer, &service, request.id, path).await?;
                } else {
                    let error = RpcResponse::failure(
                        request.id,
                        ErrorCode::InvalidRequest,
                        "path is required",
                    );
                    let mut bytes = serde_json::to_vec(&error)?;
                    bytes.push(b'\n');
                    writer.write_all(&bytes).await?;
                }
                break;
            }
        }

        let response = dispatch_uds_line(trimmed, &service).await;

        let mut resp_bytes = serde_json::to_vec(&response)?;
        if resp_bytes.len() > MAX_REQUEST_BYTES {
            resp_bytes = serde_json::to_vec(&RpcResponse::failure(
                0,
                ErrorCode::InternalError,
                "response exceeds maximum size",
            ))?;
        }
        resp_bytes.push(b'\n');
        writer.write_all(&resp_bytes).await?;
        writer.flush().await?;
    }

    Ok(())
}

async fn watch_uds_client(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: &mut OwnedWriteHalf,
    service: &ConfigService,
    id: u64,
    path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Err(error) = crate::path_parser::QueryPath::parse(path) {
        let response = RpcResponse::failure(id, ErrorCode::InvalidPath, error);
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        writer.write_all(&bytes).await?;
        return Ok(());
    }

    let canonical_path = {
        let store = service.store().read().await;
        let query = crate::path_parser::QueryPath::parse(path).unwrap();
        crate::path_resolver::resolve_paths(&query, &store)?
            .into_iter()
            .next()
    };
    let Some(canonical_path) = canonical_path else {
        let response = RpcResponse::failure(
            id,
            ErrorCode::NotFound,
            format!("watch path not found: {}", path),
        );
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        writer.write_all(&bytes).await?;
        return Ok(());
    };
    let mut receiver = service.pubsub().subscribe(&canonical_path);
    let mut acknowledgement = json!({"id": id, "ok": true, "watching": path});
    acknowledgement["event"] = Value::String("watching".to_string());
    let mut bytes = serde_json::to_vec(&acknowledgement)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;

    let mut control_line = Vec::new();
    loop {
        tokio::select! {
            event = receiver.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        let mut response = json!({"id": id, "ok": false, "event": "error", "error": {
                            "code": "INTERNAL_ERROR",
                            "message": format!("watch subscriber lagged by {} events", skipped)
                        }});
                        response["id"] = json!(id);
                        let mut bytes = serde_json::to_vec(&response)?;
                        bytes.push(b'\n');
                        writer.write_all(&bytes).await?;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let old_value = event.old_value
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .unwrap_or(Value::Null);
                let new_value = serde_json::from_str::<Value>(&event.new_value)
                    .unwrap_or(Value::String(event.new_value));
                let mut bytes = serde_json::to_vec(&json!({
                    "id": id,
                    "ok": true,
                    "event": "change",
                    "path": event.path,
                    "old_value": old_value,
                    "new_value": new_value,
                    "timestamp_ms": event.timestamp_ms,
                }))?;
                bytes.push(b'\n');
                writer.write_all(&bytes).await?;
                writer.flush().await?;
            }
            read = reader.read_until(b'\n', &mut control_line) => {
                if read? == 0 {
                    break;
                }
                if control_line.len() > MAX_REQUEST_BYTES {
                    break;
                }
                let control: Value = match serde_json::from_slice(&control_line) {
                    Ok(control) => control,
                    Err(_) => {
                        control_line.clear();
                        continue;
                    }
                };
                control_line.clear();
                if control.get("method").and_then(Value::as_str) == Some("unwatch") {
                    let mut response = json!({"id": id, "ok": true, "event": "unwatched"});
                    if let Some(request_id) = control.get("id") {
                        response["id"] = request_id.clone();
                    }
                    let mut bytes = serde_json::to_vec(&response)?;
                    bytes.push(b'\n');
                    writer.write_all(&bytes).await?;
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn dispatch_uds_line(line: &str, service: &ConfigService) -> Value {
    let raw: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return serde_json::to_value(RpcResponse::failure(
                0,
                ErrorCode::InvalidRequest,
                format!("invalid JSON request: {}", error),
            ))
            .unwrap_or_else(|_| json!({"id":0,"ok":false}));
        }
    };

    if raw.get("method").is_some() {
        let request: RpcRequest = match serde_json::from_value(raw.clone()) {
            Ok(request) => request,
            Err(error) => {
                let id = raw.get("id").and_then(Value::as_u64).unwrap_or(0);
                return serde_json::to_value(RpcResponse::failure(
                    id,
                    ErrorCode::InvalidRequest,
                    format!("invalid request: {}", error),
                ))
                .unwrap_or_else(|_| json!({"id":id,"ok":false}));
            }
        };
        return serde_json::to_value(service.dispatch(request).await)
            .unwrap_or_else(|_| json!({"id":0,"ok":false}));
    }

    let request: UdsRequest = match serde_json::from_value(raw) {
        Ok(request) => request,
        Err(error) => {
            return serde_json::to_value(RpcResponse::failure(
                0,
                ErrorCode::InvalidRequest,
                format!("invalid request: {}", error),
            ))
            .unwrap_or_else(|_| json!({"id":0,"ok":false}));
        }
    };
    let (method, path, value) = match request {
        UdsRequest::Get { path } => ("get", path, None),
        UdsRequest::Set { path, value } => ("set", path, Some(ConfigValue::from_json(value))),
    };
    let response = service
        .dispatch(RpcRequest {
            id: 0,
            method: method.to_string(),
            path: Some(path.clone()),
            value,
        })
        .await;
    if !response.ok {
        return serde_json::to_value(UdsResponse::error(
            response
                .error
                .map(|error| error.message)
                .unwrap_or_default(),
        ))
        .unwrap_or_else(|_| json!({"status":"error","error":"internal error"}));
    }
    if method == "get" {
        let match_count = response.match_count as usize;
        let typed: Option<ConfigValue> = response
            .result
            .and_then(|value| serde_json::from_value(value).ok());
        let value = typed
            .and_then(|value| value.into_json().ok())
            .unwrap_or(Value::Null);
        return serde_json::to_value(UdsResponse::get_success(true, match_count, value))
            .unwrap_or_else(|_| json!({"status":"error","error":"internal error"}));
    }
    let affected_paths = if response.affected_paths.is_empty() {
        vec![path]
    } else {
        response.affected_paths
    };
    serde_json::to_value(UdsResponse::set_success(affected_paths))
        .unwrap_or_else(|_| json!({"status":"error","error":"internal error"}))
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

pub async fn send_rpc_request<P: AsRef<Path>>(
    socket_path: P,
    request: &RpcRequest,
) -> Result<RpcResponse, Box<dyn std::error::Error + Send + Sync>> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;

    let mut reader = BufReader::new(reader);
    let mut response = Vec::new();
    let read = reader.read_until(b'\n', &mut response).await?;
    if read == 0 {
        return Err("connection closed before response received".into());
    }
    if response.len() > MAX_REQUEST_BYTES {
        return Err("response exceeds maximum size".into());
    }
    Ok(serde_json::from_slice(&response)?)
}
