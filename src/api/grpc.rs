use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

use crate::path_resolver::{resolve_get, resolve_set};
use crate::pubsub_engine::{ChangeEvent, PubSubEngine};
use crate::storage::Store;

pub mod proto {
    tonic::include_proto!("rekv");
}

pub use proto::rekv_service_client::RekvServiceClient;
pub use proto::rekv_service_server::{RekvService, RekvServiceServer};
pub use proto::{GetRequest, GetResponse, SetRequest, SetResponse, WatchEvent, WatchRequest};

/// Implementation of the RekvService gRPC service.
#[derive(Clone)]
pub struct RekvServiceImpl {
    store: Arc<RwLock<Store>>,
    pubsub: Arc<PubSubEngine>,
}

impl RekvServiceImpl {
    pub fn new(store: Arc<RwLock<Store>>, pubsub: Arc<PubSubEngine>) -> Self {
        Self { store, pubsub }
    }
}

#[tonic::async_trait]
impl RekvService for RekvServiceImpl {
    /// Query configuration by XPath-like path.
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let req = request.into_inner();
        let store = self.store.read().await;

        match resolve_get(&req.path, &store) {
            Ok(result) => {
                let match_count = result.match_count() as u32;
                let found = match_count > 0;
                let json_val = result.to_json_value();
                let json_str = serde_json::to_string(&json_val)
                    .map_err(|e| Status::internal(format!("JSON serialization error: {}", e)))?;

                Ok(Response::new(GetResponse {
                    found,
                    json_value: json_str,
                    match_count,
                }))
            }
            Err(err_msg) => Err(Status::invalid_argument(err_msg)),
        }
    }

    /// Set configuration value at an XPath-like path.
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<SetResponse>, Status> {
        let req = request.into_inner();

        let parsed_val: serde_json::Value = serde_json::from_str(&req.json_value)
            .map_err(|e| Status::invalid_argument(format!("Invalid JSON value: {}", e)))?;

        let mut store = self.store.write().await;

        // Capture old values for affected paths if possible
        let old_val_str = store
            .get_leaf(&req.path)
            .cloned()
            .or_else(|| store.get_subtree(&req.path))
            .map(|v| serde_json::to_string(&v).unwrap_or_default());

        match resolve_set(&req.path, parsed_val.clone(), &mut store) {
            Ok(set_result) => {
                let affected_count = set_result.affected_paths.len() as u32;

                // Auto-persist changes to disk if a file path is configured
                if let Err(e) = store.persist() {
                    log::error!("Disk persistence failed after set: {}", e);
                    return Ok(Response::new(SetResponse {
                        success: false,
                        error: format!("Updated in-memory, but disk persist failed: {}", e),
                        affected_count,
                        affected_paths: set_result.affected_paths,
                    }));
                }

                // Publish bubble-up change event to subscribers
                let new_val_str = serde_json::to_string(&parsed_val).unwrap_or_default();
                for affected_path in &set_result.affected_paths {
                    let event = ChangeEvent::new(
                        affected_path.clone(),
                        old_val_str.clone(),
                        new_val_str.clone(),
                    );
                    self.pubsub.publish(event);
                }

                Ok(Response::new(SetResponse {
                    success: true,
                    error: String::new(),
                    affected_count,
                    affected_paths: set_result.affected_paths,
                }))
            }
            Err(err_msg) => Ok(Response::new(SetResponse {
                success: false,
                error: err_msg,
                affected_count: 0,
                affected_paths: vec![],
            })),
        }
    }

    type WatchStream = Pin<Box<dyn tonic::codegen::tokio_stream::Stream<Item = Result<WatchEvent, Status>> + Send>>;

    /// Watch configuration changes in real time (bubble-up hierarchical streaming).
    async fn watch(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let req = request.into_inner();
        let mut rx = self.pubsub.subscribe(&req.path);

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
    let svc = RekvServiceServer::new(RekvServiceImpl::new(store, pubsub));
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
    let svc = RekvServiceServer::new(RekvServiceImpl::new(store, pubsub));
    log::info!("gRPC server listening on {}", addr);
    tonic::transport::Server::builder()
        .add_service(svc)
        .serve_with_shutdown(addr, signal)
        .await
}
