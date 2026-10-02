use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

use crate::config_service::ConfigService;
use crate::config_value::ConfigValue;
use crate::path_parser::QueryPath;
use crate::path_resolver::{resolve_paths, ResolveError};
use crate::protocol::{ErrorCode, RpcRequest, RpcResponse};
use crate::pubsub_engine::PubSubEngine;
use crate::storage::Store;

pub mod proto {
    include!("rekv_proto.rs");
}

pub use proto::rekv_service_client::RekvServiceClient;
pub use proto::rekv_service_server::{RekvService, RekvServiceServer};
pub use proto::{GetRequest, GetResponse, SetRequest, SetResponse, WatchEvent, WatchRequest};
pub use proto::{RpcCallRequest, RpcCallResponse};

/// Implementation of the RekvService gRPC service.
#[derive(Clone)]
pub struct RekvServiceImpl {
    service: ConfigService,
}

impl RekvServiceImpl {
    pub fn new(store: Arc<RwLock<Store>>, pubsub: Arc<PubSubEngine>) -> Self {
        Self {
            service: ConfigService::new(store, pubsub),
        }
    }

    pub fn with_service(service: ConfigService) -> Self {
        Self { service }
    }
}

#[tonic::async_trait]
impl RekvService for RekvServiceImpl {
    /// Query configuration by XPath-like path.
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let req = request.into_inner();
        let response = self
            .service
            .dispatch(RpcRequest {
                id: 0,
                method: "get".to_string(),
                path: Some(req.path),
                value: None,
            })
            .await;
        if response.ok {
            let raw_value = response
                .result
                .and_then(|value| serde_json::from_value::<ConfigValue>(value).ok())
                .ok_or_else(|| Status::internal("missing typed GET result"))?
                .into_json()
                .map_err(Status::internal)?;
            let json_value = serde_json::to_string(&raw_value)
                .map_err(|error| Status::internal(error.to_string()))?;
            Ok(Response::new(GetResponse {
                found: true,
                json_value,
                match_count: response.match_count,
            }))
        } else if response
            .error
            .as_ref()
            .is_some_and(|error| error.code == ErrorCode::NotFound)
        {
            Ok(Response::new(GetResponse {
                found: false,
                json_value: "null".to_string(),
                match_count: 0,
            }))
        } else {
            Err(error_status(response))
        }
    }

    /// Set configuration value at an XPath-like path.
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<SetResponse>, Status> {
        let req = request.into_inner();

        let raw_value: serde_json::Value = serde_json::from_str(&req.json_value)
            .map_err(|error| Status::invalid_argument(format!("Invalid JSON value: {}", error)))?;
        let value = serde_json::from_value::<ConfigValue>(raw_value.clone())
            .unwrap_or_else(|_| ConfigValue::from_json(raw_value));
        let response = self
            .service
            .dispatch(RpcRequest {
                id: 0,
                method: "set".to_string(),
                path: Some(req.path),
                value: Some(value),
            })
            .await;
        Ok(Response::new(SetResponse {
            success: response.ok,
            error: response
                .error
                .map(|error| error.message)
                .unwrap_or_default(),
            affected_count: response.affected_paths.len() as u32,
            affected_paths: response.affected_paths,
        }))
    }

    async fn call(
        &self,
        request: Request<RpcCallRequest>,
    ) -> Result<Response<RpcCallResponse>, Status> {
        let req = request.into_inner();
        let value = if req.json_value.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str::<ConfigValue>(&req.json_value).map_err(|error| {
                    Status::invalid_argument(format!("Invalid typed JSON value: {}", error))
                })?,
            )
        };
        let response = self
            .service
            .dispatch(RpcRequest {
                id: req.id,
                method: req.method,
                path: Some(req.path),
                value,
            })
            .await;
        let result_json = response
            .result
            .map(|result| serde_json::to_string(&result))
            .transpose()
            .map_err(|error| Status::internal(error.to_string()))?
            .unwrap_or_default();
        let (error_code, error_message) = response
            .error
            .map(|error| (error_code_name(error.code).to_string(), error.message))
            .unwrap_or_default();
        Ok(Response::new(RpcCallResponse {
            id: req.id,
            ok: response.ok,
            result_json,
            error_code,
            error_message,
        }))
    }

    type WatchStream = Pin<
        Box<dyn tonic::codegen::tokio_stream::Stream<Item = Result<WatchEvent, Status>> + Send>,
    >;

    /// Watch configuration changes in real time (bubble-up hierarchical streaming).
    async fn watch(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let req = request.into_inner();
        let query = QueryPath::parse(&req.path).map_err(Status::invalid_argument)?;
        let store = self.service.store().read().await;
        let canonical_path = resolve_paths(&query, &store)
            .map_err(resolve_error_status)?
            .into_iter()
            .next()
            .ok_or_else(|| Status::not_found(format!("watch path not found: {}", req.path)))?;
        drop(store);
        let mut rx = self.service.pubsub().subscribe(&canonical_path);

        let stream = async_stream::try_stream! {
            while let Ok(event) = rx.recv().await {
                yield WatchEvent {
                    path: event.path,
                    old_value: event.old_value.unwrap_or_default(),
                    new_value: event.new_value,
                    timestamp_ms: event.timestamp_ms,
                };
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}

/// Start gRPC server on the given address.
pub async fn start_grpc_server(
    addr: SocketAddr,
    store: Arc<RwLock<Store>>,
    pubsub: Arc<PubSubEngine>,
) -> Result<(), tonic::transport::Error> {
    start_grpc_server_with_service(addr, ConfigService::new(store, pubsub)).await
}

pub async fn start_grpc_server_with_service(
    addr: SocketAddr,
    service: ConfigService,
) -> Result<(), tonic::transport::Error> {
    let svc = RekvServiceServer::new(RekvServiceImpl::with_service(service));
    log::info!("gRPC server listening on {}", addr);
    tonic::transport::Server::builder()
        .add_service(svc)
        .serve(addr)
        .await
}

/// Start gRPC server with graceful shutdown signal.
pub async fn start_grpc_server_with_shutdown<F>(
    addr: SocketAddr,
    store: Arc<RwLock<Store>>,
    pubsub: Arc<PubSubEngine>,
    signal: F,
) -> Result<(), tonic::transport::Error>
where
    F: std::future::Future<Output = ()>,
{
    start_grpc_server_with_service_and_shutdown(addr, ConfigService::new(store, pubsub), signal)
        .await
}

pub async fn start_grpc_server_with_service_and_shutdown<F>(
    addr: SocketAddr,
    service: ConfigService,
    signal: F,
) -> Result<(), tonic::transport::Error>
where
    F: std::future::Future<Output = ()>,
{
    let svc = RekvServiceServer::new(RekvServiceImpl::with_service(service));
    log::info!("gRPC server listening on {}", addr);
    tonic::transport::Server::builder()
        .add_service(svc)
        .serve_with_shutdown(addr, signal)
        .await
}

fn error_status(response: RpcResponse) -> Status {
    match response.error {
        Some(error) => match error.code {
            ErrorCode::InvalidRequest | ErrorCode::InvalidPath | ErrorCode::InvalidValue => {
                Status::invalid_argument(error.message)
            }
            ErrorCode::NotFound => Status::not_found(error.message),
            ErrorCode::InternalError => Status::internal(error.message),
        },
        None => Status::internal("operation failed without an error"),
    }
}

fn resolve_error_status(error: ResolveError) -> Status {
    match error {
        ResolveError::InvalidPath(message) | ResolveError::InvalidValue(message) => {
            Status::invalid_argument(message)
        }
        ResolveError::NotFound(message) => Status::not_found(message),
    }
}

fn error_code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::InvalidRequest => "INVALID_REQUEST",
        ErrorCode::InvalidPath => "INVALID_PATH",
        ErrorCode::NotFound => "NOT_FOUND",
        ErrorCode::InvalidValue => "INVALID_VALUE",
        ErrorCode::InternalError => "INTERNAL_ERROR",
    }
}
