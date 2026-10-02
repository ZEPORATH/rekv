use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config_value::ConfigValue;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub value: Option<ConfigValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    InvalidPath,
    NotFound,
    InvalidValue,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    #[serde(skip)]
    pub match_count: u32,
    #[serde(skip)]
    pub affected_paths: Vec<String>,
}

impl RpcResponse {
    pub fn success(id: u64, result: Option<Value>) -> Self {
        Self {
            id,
            ok: true,
            result,
            error: None,
            match_count: 0,
            affected_paths: Vec::new(),
        }
    }

    pub fn failure(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
            match_count: 0,
            affected_paths: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, RpcRequest, RpcResponse};
    use serde_json::json;

    #[test]
    fn rpc_contract_uses_request_ids_and_fixed_error_codes() {
        let request: RpcRequest = serde_json::from_value(json!({
            "id": 12,
            "method": "get",
            "path": "/device/name"
        }))
        .unwrap();
        assert_eq!(request.id, 12);

        let response = RpcResponse::failure(12, ErrorCode::NotFound, "missing");
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "id": 12,
                "ok": false,
                "error": { "code": "NOT_FOUND", "message": "missing" }
            })
        );
    }
}
