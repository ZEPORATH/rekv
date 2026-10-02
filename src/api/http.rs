use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_http::cors::CorsLayer;

use crate::config_service::ConfigService;
use crate::protocol::{ErrorCode, RpcRequest, RpcResponse};

#[derive(Debug, Deserialize)]
struct PathQuery {
    path: String,
    #[serde(default)]
    id: u64,
}

#[derive(Debug, Deserialize)]
struct SetBody {
    path: String,
    value: crate::config_value::ConfigValue,
    #[serde(default)]
    id: u64,
}

pub fn router(service: ConfigService) -> Router {
    let cors = CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://localhost:5173"),
            HeaderValue::from_static("http://127.0.0.1:5173"),
            HeaderValue::from_static("http://localhost:3000"),
            HeaderValue::from_static("http://127.0.0.1:3000"),
        ])
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE]);

    Router::new()
        .route(
            "/api/config",
            get(get_value).post(set_value).delete(delete_value),
        )
        .route("/api/config/list", get(list_values))
        .route("/api/rpc", post(rpc))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(cors)
        .with_state(service)
}

pub async fn start_http_server<F>(
    port: u16,
    service: ConfigService,
    shutdown: F,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::Server::from_tcp(listener.into_std()?)?
        .serve(router(service).into_make_service())
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

async fn get_value(
    Query(query): Query<PathQuery>,
    State(service): State<ConfigService>,
) -> impl IntoResponse {
    response(
        service
            .dispatch(RpcRequest {
                id: query.id,
                method: "get".to_string(),
                path: Some(query.path),
                value: None,
            })
            .await,
    )
}

async fn list_values(
    Query(query): Query<PathQuery>,
    State(service): State<ConfigService>,
) -> impl IntoResponse {
    response(
        service
            .dispatch(RpcRequest {
                id: query.id,
                method: "list".to_string(),
                path: Some(query.path),
                value: None,
            })
            .await,
    )
}

async fn set_value(
    State(service): State<ConfigService>,
    Json(body): Json<SetBody>,
) -> impl IntoResponse {
    response(
        service
            .dispatch(RpcRequest {
                id: body.id,
                method: "set".to_string(),
                path: Some(body.path),
                value: Some(body.value),
            })
            .await,
    )
}

async fn delete_value(
    Query(query): Query<PathQuery>,
    State(service): State<ConfigService>,
) -> impl IntoResponse {
    response(
        service
            .dispatch(RpcRequest {
                id: query.id,
                method: "delete".to_string(),
                path: Some(query.path),
                value: None,
            })
            .await,
    )
}

async fn rpc(
    State(service): State<ConfigService>,
    Json(request): Json<RpcRequest>,
) -> impl IntoResponse {
    response(service.dispatch(request).await)
}

fn response(response: RpcResponse) -> (StatusCode, Json<RpcResponse>) {
    let status = if response.ok {
        StatusCode::OK
    } else {
        match response.error.as_ref().map(|error| error.code) {
            Some(ErrorCode::InvalidRequest | ErrorCode::InvalidPath | ErrorCode::InvalidValue) => {
                StatusCode::BAD_REQUEST
            }
            Some(ErrorCode::NotFound) => StatusCode::NOT_FOUND,
            Some(ErrorCode::InternalError) | None => StatusCode::INTERNAL_SERVER_ERROR,
        }
    };
    (status, Json(response))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use serde_json::{json, Value};
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use crate::api::http::router;
    use crate::pubsub_engine::PubSubEngine;
    use crate::storage::Store;

    #[tokio::test]
    async fn http_routes_share_rpc_crud_contract() {
        let service = crate::config_service::ConfigService::new(
            Arc::new(RwLock::new(Store::from_value(
                json!({ "device": { "name": "rpi" } }),
                None,
            ))),
            Arc::new(PubSubEngine::default()),
        );
        let app = router(service);

        let get = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config?path=%2Fdevice%2Fname")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), axum::http::StatusCode::OK);
        let get_body = hyper::body::to_bytes(get.into_body()).await.unwrap();
        let get_json: Value = serde_json::from_slice(&get_body).unwrap();
        assert_eq!(get_json["result"], json!({"type":"string", "value":"rpi"}));

        let set = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path":"/device/name","value":{"type":"string","value":"demo"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(set.status(), axum::http::StatusCode::OK);
        let set_body = hyper::body::to_bytes(set.into_body()).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&set_body).unwrap()["ok"],
            true
        );

        let list = app
            .oneshot(
                Request::builder()
                    .uri("/api/config/list?path=%2Fdevice")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_body = hyper::body::to_bytes(list.into_body()).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&list_body).unwrap()["result"],
            json!(["name"])
        );
    }
}
