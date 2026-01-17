use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde_json::Value;
use std::sync::Arc;

use crate::config::ConfigStore;
use crate::redis::RedisNotifier;

/// Application state shared across all handlers.
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub redis: Arc<RedisNotifier>,
}

/// GET /config - Return the full configuration.
pub async fn get_full_config(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.config.get_full())
}

/// GET /config/:path - Return a subtree by path (e.g., "device1.gpio").
/// Returns 404 if the path doesn't exist.
pub async fn get_config_path(
    State(state): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<Value>, StatusCode> {
    let path_decoded = urlencoding::decode(&path)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string();
    
    match state.config.get_path(&path_decoded) {
        Some(value) => Ok(Json(value)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// POST /config/:path - Set a value at a path.
/// Body should be a JSON value.
/// Returns the old value (or null if it didn't exist).
pub async fn set_config_path(
    State(state): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
    Json(value): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let path_decoded = urlencoding::decode(&path)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string();

    // Get old value before update
    let old_value = state.config.get_path(&path_decoded);

    // Update config (this acquires write lock and persists to disk)
    let old_value_clone = old_value.clone();
    let path_for_redis = path_decoded.clone();
    state.config.set_path(&path_decoded, value.clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Publish Redis notification (non-blocking, errors are logged but don't fail the request)
    let redis_clone = Arc::clone(&state.redis);
    let value_for_redis = value.clone();
    tokio::spawn(async move {
        if let Err(e) = redis_clone.publish_update(&path_for_redis, old_value_clone.as_ref(), &value_for_redis).await {
            eprintln!("Warning: Failed to publish Redis notification: {}", e);
        }
    });

    Ok(Json(old_value.unwrap_or(Value::Null)))
}

/// Build the API router.
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/config", get(get_full_config))
        .route("/config/*path", get(get_config_path).post(set_config_path))
        .with_state(state)
}
