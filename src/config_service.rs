use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::config_value::ConfigValue;
use crate::path_resolver::{
    resolve_delete, resolve_get, resolve_list, resolve_paths, resolve_set, ResolveError,
    ResolveGetResult,
};
use crate::protocol::{ErrorCode, RpcRequest, RpcResponse};
use crate::pubsub_engine::{ChangeEvent, PubSubEngine};
use crate::storage::{normalize_path, StorageError, Store};

#[derive(Clone)]
pub struct ConfigService {
    store: Arc<RwLock<Store>>,
    pubsub: Arc<PubSubEngine>,
    read_only_paths: Arc<Vec<String>>,
}

impl ConfigService {
    pub fn new(store: Arc<RwLock<Store>>, pubsub: Arc<PubSubEngine>) -> Self {
        Self {
            store,
            pubsub,
            read_only_paths: Arc::new(Vec::new()),
        }
    }

    /// Subtrees that clients can read but never change. Any set/delete/restore whose result
    /// would alter one of them (including a whole-document set on `/`) is rejected.
    pub fn with_read_only_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.read_only_paths = Arc::new(
            paths
                .into_iter()
                .map(|path| normalize_path(path.as_ref()))
                .collect(),
        );
        self
    }

    fn read_only_violation(
        &self,
        id: u64,
        current: &Store,
        candidate: &Store,
    ) -> Option<RpcResponse> {
        self.read_only_paths
            .iter()
            .find(|path| current.get_subtree_ref(path) != candidate.get_subtree_ref(path))
            .map(|path| {
                RpcResponse::failure(
                    id,
                    ErrorCode::InvalidRequest,
                    format!("'{path}' is read-only"),
                )
            })
    }

    pub fn store(&self) -> &Arc<RwLock<Store>> {
        &self.store
    }

    pub fn pubsub(&self) -> &Arc<PubSubEngine> {
        &self.pubsub
    }

    pub async fn dispatch(&self, request: RpcRequest) -> RpcResponse {
        match request.method.as_str() {
            "get" => self.get(request.id, request.path).await,
            "set" => self.set(request.id, request.path, request.value).await,
            "delete" => self.delete(request.id, request.path).await,
            "list" => self.list(request.id, request.path).await,
            "backup" => self.backup(request.id, request.path).await,
            "restore" => self.restore(request.id, request.path).await,
            _ => RpcResponse::failure(request.id, ErrorCode::InvalidRequest, "unsupported method"),
        }
    }

    async fn backup(&self, id: u64, _path: Option<String>) -> RpcResponse {
        let store = self.store.read().await;
        match store.persist() {
            Ok(()) => RpcResponse::success(
                id,
                store
                    .delta_file_path()
                    .map(|path| Value::String(path.display().to_string())),
            ),
            Err(error) => storage_failure(id, error),
        }
    }

    async fn restore(&self, id: u64, path: Option<String>) -> RpcResponse {
        let Some(path) = path.filter(|path| !path.trim().is_empty()) else {
            return RpcResponse::failure(id, ErrorCode::InvalidRequest, "setting path is required");
        };
        let mut store = self.store.write().await;
        let mut candidate = store.clone();
        let query = match crate::path_parser::QueryPath::parse(&path) {
            Ok(query) => query,
            Err(error) => return RpcResponse::failure(id, ErrorCode::InvalidPath, error),
        };
        let canonical_path = resolve_paths(&query, &candidate)
            .ok()
            .and_then(|paths| paths.into_iter().next())
            .or_else(|| candidate.resolve_base_path(&path).ok().flatten());
        let Some(canonical_path) = canonical_path else {
            return RpcResponse::failure(id, ErrorCode::NotFound, "setting path not found");
        };
        let old_value = candidate.get_subtree(&canonical_path);
        if let Err(error) = candidate.restore_delta(&canonical_path) {
            return storage_failure(id, error);
        }
        if let Some(rejection) = self.read_only_violation(id, &store, &candidate) {
            return rejection;
        }
        if let Err(error) = candidate.persist() {
            return storage_failure(id, error);
        }
        let new_value = candidate.get_subtree(&canonical_path);
        let event = ChangeEvent::new(
            canonical_path,
            old_value.and_then(|value| serde_json::to_string(&value).ok()),
            new_value
                .and_then(|value| serde_json::to_string(&value).ok())
                .unwrap_or_else(|| "null".to_string()),
        );
        *store = candidate;
        drop(store);
        self.pubsub.publish(event);
        RpcResponse::success(id, None)
    }

    async fn get(&self, id: u64, path: Option<String>) -> RpcResponse {
        let Some(path) = path else {
            return RpcResponse::failure(id, ErrorCode::InvalidRequest, "path is required");
        };
        let store = self.store.read().await;
        match resolve_get(&path, &store) {
            Ok(result) if result.match_count() > 0 => {
                let match_count = result.match_count() as u32;
                let value = match result {
                    ResolveGetResult::Single(node) => ConfigValue::from_json_with_types(
                        node.value,
                        &node.path,
                        store.typed_lists(),
                    ),
                    ResolveGetResult::Multiple(nodes) => nodes
                        .into_iter()
                        .map(|node| {
                            ConfigValue::from_json_with_types(
                                node.value,
                                &node.path,
                                store.typed_lists(),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map(ConfigValue::Array),
                    ResolveGetResult::NotFound => unreachable!(),
                };
                let value = match value {
                    Ok(value) => value,
                    Err(error) => return RpcResponse::failure(id, ErrorCode::InternalError, error),
                };
                match serde_json::to_value(value) {
                    Ok(result) => {
                        let mut response = RpcResponse::success(id, Some(result));
                        response.match_count = match_count;
                        response
                    }
                    Err(error) => {
                        RpcResponse::failure(id, ErrorCode::InternalError, error.to_string())
                    }
                }
            }
            Ok(_) => {
                RpcResponse::failure(id, ErrorCode::NotFound, "configuration path does not exist")
            }
            Err(error) => resolve_failure(id, error),
        }
    }

    async fn set(&self, id: u64, path: Option<String>, value: Option<ConfigValue>) -> RpcResponse {
        let Some(path) = path else {
            return RpcResponse::failure(id, ErrorCode::InvalidRequest, "path is required");
        };
        let Some(value) = value else {
            return RpcResponse::failure(id, ErrorCode::InvalidRequest, "value is required");
        };
        let normalized_path = normalize_path(&path);
        let (value, value_types) = match value.into_json_with_types(&normalized_path) {
            Ok(value) => value,
            Err(error) => return RpcResponse::failure(id, ErrorCode::InvalidValue, error),
        };

        let mut store = self.store.write().await;
        let mut candidate = store.clone();
        let affected_paths = match resolve_set(&path, value.clone(), &mut candidate) {
            Ok(result) => result.affected_paths,
            Err(error) => return resolve_failure(id, error),
        };

        let mut persisted_types = std::collections::BTreeMap::new();
        let type_prefix = format!("{}/", normalized_path.trim_end_matches('/'));
        for affected_path in &affected_paths {
            for (typed_path, kind) in &value_types {
                let suffix = if typed_path == &normalized_path {
                    ""
                } else if normalized_path == "/" {
                    typed_path.trim_start_matches('/')
                } else if let Some(suffix) = typed_path.strip_prefix(&type_prefix) {
                    suffix
                } else {
                    continue;
                };
                let mapped_path = if suffix.is_empty() {
                    affected_path.clone()
                } else if affected_path == "/" {
                    format!("/{}", suffix)
                } else {
                    format!("{}/{}", affected_path, suffix)
                };
                persisted_types.insert(mapped_path, kind.clone());
            }
        }
        candidate.set_typed_lists(persisted_types);

        if let Some(rejection) = self.read_only_violation(id, &store, &candidate) {
            return rejection;
        }
        if candidate.file_path().is_some() {
            if let Err(error) = candidate.persist() {
                return storage_failure(id, error);
            }
        }

        let events: Vec<_> = affected_paths
            .iter()
            .map(|affected_path| {
                let old_value = store
                    .get_subtree(affected_path)
                    .and_then(|value| serde_json::to_string(&value).ok());
                let new_value = candidate
                    .get_subtree(affected_path)
                    .and_then(|value| serde_json::to_string(&value).ok())
                    .unwrap_or_else(|| "null".to_string());
                ChangeEvent::new(affected_path.clone(), old_value, new_value)
            })
            .collect();
        *store = candidate;
        drop(store);
        for event in events {
            self.pubsub.publish(event);
        }
        let mut response = RpcResponse::success(id, None);
        response.affected_paths = affected_paths;
        response
    }

    async fn delete(&self, id: u64, path: Option<String>) -> RpcResponse {
        let Some(path) = path else {
            return RpcResponse::failure(id, ErrorCode::InvalidRequest, "path is required");
        };
        let mut store = self.store.write().await;
        let mut candidate = store.clone();
        let old_value = crate::path_parser::QueryPath::parse(&path)
            .ok()
            .and_then(|query| resolve_paths(&query, &candidate).ok())
            .and_then(|paths| paths.into_iter().next())
            .and_then(|resolved_path| candidate.get_subtree(&resolved_path));
        let resolved_path = match resolve_delete(&path, &mut candidate) {
            Ok(resolved_path) => resolved_path,
            Err(error) => return resolve_failure(id, error),
        };
        if let Some(rejection) = self.read_only_violation(id, &store, &candidate) {
            return rejection;
        }
        if candidate.file_path().is_some() {
            if let Err(error) = candidate.persist() {
                return storage_failure(id, error);
            }
        }
        let old_value = old_value.and_then(|value| serde_json::to_string(&value).ok());
        let event = ChangeEvent::new(
            resolved_path.clone(),
            old_value.and_then(|value| serde_json::to_string(&value).ok()),
            "null",
        );
        *store = candidate;
        drop(store);
        self.pubsub.publish(event);
        let mut response = RpcResponse::success(id, None);
        response.affected_paths = vec![resolved_path];
        response
    }

    async fn list(&self, id: u64, path: Option<String>) -> RpcResponse {
        let Some(path) = path else {
            return RpcResponse::failure(id, ErrorCode::InvalidRequest, "path is required");
        };
        let store = self.store.read().await;
        match resolve_list(&path, &store) {
            Ok(children) => RpcResponse::success(
                id,
                Some(Value::Array(
                    children.into_iter().map(Value::String).collect(),
                )),
            ),
            Err(error) => resolve_failure(id, error),
        }
    }
}

fn resolve_failure(id: u64, error: ResolveError) -> RpcResponse {
    let code = match error {
        ResolveError::InvalidPath(_) => ErrorCode::InvalidPath,
        ResolveError::NotFound(_) => ErrorCode::NotFound,
        ResolveError::InvalidValue(_) => ErrorCode::InvalidValue,
    };
    RpcResponse::failure(id, code, error.to_string())
}

fn storage_failure(id: u64, error: StorageError) -> RpcResponse {
    let code = match error {
        StorageError::NotFound(_) => ErrorCode::NotFound,
        StorageError::InvalidPath(_) => ErrorCode::InvalidPath,
        StorageError::InvalidValue(_) => ErrorCode::InvalidValue,
        StorageError::Io(_) | StorageError::Json(_) | StorageError::NoFilePath => {
            ErrorCode::InternalError
        }
    };
    RpcResponse::failure(id, code, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::{tempdir, NamedTempFile};
    use tokio::sync::RwLock;

    use crate::config_service::ConfigService;
    use crate::protocol::RpcRequest;
    use crate::pubsub_engine::PubSubEngine;
    use crate::storage::Store;

    fn request(id: u64, method: &str, path: &str, value: Option<serde_json::Value>) -> RpcRequest {
        RpcRequest {
            id,
            method: method.to_string(),
            path: Some(path.to_string()),
            value: value.map(serde_json::from_value).transpose().unwrap(),
        }
    }

    #[tokio::test]
    async fn dispatches_crud_and_lists_id_selectors() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        fs::write(
            &settings_path,
            json!({"sensors": [{"id": "temperature", "threshold": 50.5}]}).to_string(),
        )
        .unwrap();
        let store = Store::load_from_file(&settings_path).unwrap();
        let shared = Arc::new(RwLock::new(store));
        let service = ConfigService::new(shared.clone(), Arc::new(PubSubEngine::default()));

        let listed = service.dispatch(request(1, "list", "/sensors", None)).await;
        assert_eq!(listed.result.unwrap(), json!(["temperature"]));
        let fields = service
            .dispatch(request(7, "list", "/sensors[id = temperature]/*", None))
            .await;
        assert_eq!(fields.result.unwrap(), json!(["id", "threshold"]));
        let got = service
            .dispatch(request(
                2,
                "get",
                "/sensors[id = temperature]/threshold",
                None,
            ))
            .await;
        assert_eq!(got.result.unwrap(), json!({"type":"float", "value":50.5}));

        let set = service
            .dispatch(request(
                3,
                "set",
                "/sensors[id = temperature]/threshold",
                Some(json!({"type":"integer", "value":51})),
            ))
            .await;
        assert!(set.ok);
        let deleted = service
            .dispatch(request(
                4,
                "delete",
                "/sensors[id = temperature]/threshold",
                None,
            ))
            .await;
        assert!(deleted.ok);
        assert!(shared
            .read()
            .await
            .get_subtree("/sensors/0/threshold")
            .is_none());

        let typed_list = service
            .dispatch(request(
                5,
                "set",
                "/device/labels",
                Some(json!({"type":"str_list", "value":[]})),
            ))
            .await;
        assert!(typed_list.ok);
        let reloaded = Store::load_from_file(&settings_path).unwrap();
        let reloaded_service = ConfigService::new(
            Arc::new(RwLock::new(reloaded)),
            Arc::new(PubSubEngine::default()),
        );
        let labels = reloaded_service
            .dispatch(request(6, "get", "/device/labels", None))
            .await;
        assert_eq!(
            labels.result.unwrap(),
            json!({"type":"str_list", "value":[]})
        );
    }

    #[tokio::test]
    async fn failed_persistence_does_not_change_memory_or_publish() {
        let parent_file = NamedTempFile::new().unwrap();
        let store = Store::from_value(
            json!({"device": {"name": "before"}}),
            Some(parent_file.path().join("settings.json")),
        );
        let shared = Arc::new(RwLock::new(store.clone()));
        let pubsub = Arc::new(PubSubEngine::default());
        let mut events = pubsub.subscribe("/device");
        let service = ConfigService::new(shared.clone(), pubsub);

        let response = service
            .dispatch(request(
                1,
                "set",
                "/device/name",
                Some(json!({"type":"string", "value":"after"})),
            ))
            .await;
        assert!(!response.ok);
        assert_eq!(
            shared.read().await.get_leaf("/device/name"),
            Some(&json!("before"))
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn delta_override_and_path_restore_round_trip() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let original =
            json!({"platform_manager":{"io_devices":[{"id":"ECU0","baud_rate":115200}]}});
        fs::write(&settings_path, original.to_string()).unwrap();
        let store = Store::load_from_file(&settings_path).unwrap();
        let shared = Arc::new(RwLock::new(store));
        let pubsub = Arc::new(PubSubEngine::default());
        let mut baud_events = pubsub.subscribe("/platform_manager/io_devices/0/baud_rate");
        let service = ConfigService::new(shared.clone(), pubsub);

        let changed = service
            .dispatch(request(
                2,
                "set",
                "/platform_manager/io_devices[id = ECU0]/baud_rate",
                Some(json!({"type":"integer","value":57600})),
            ))
            .await;
        assert!(changed.ok);
        let change = baud_events.recv().await.unwrap();
        assert_eq!(change.new_value, "57600");
        let restored = service
            .dispatch(request(
                3,
                "restore",
                "/platform_manager/io_devices[id = ECU0]/baud_rate",
                None,
            ))
            .await;
        assert!(restored.ok);
        let restore_event = baud_events.recv().await.unwrap();
        assert_eq!(restore_event.old_value.as_deref(), Some("57600"));
        assert_eq!(restore_event.new_value, "115200");
        assert_eq!(
            shared
                .read()
                .await
                .get_leaf("/platform_manager/io_devices/0/baud_rate"),
            Some(&json!(115200))
        );
        let disk = Store::load_from_file(&settings_path).unwrap();
        assert_eq!(
            disk.get_leaf("/platform_manager/io_devices/0/baud_rate"),
            Some(&json!(115200))
        );
        assert_eq!(
            fs::read_to_string(&settings_path).unwrap(),
            original.to_string()
        );
        let delta: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("_delta.json")).unwrap())
                .unwrap();
        assert!(delta.as_object().unwrap().is_empty());

        let added = service
            .dispatch(request(
                4,
                "set",
                "/platform_manager/new_setting",
                Some(json!({"type":"boolean","value":true})),
            ))
            .await;
        assert!(added.ok);
        let added_delta: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("_delta.json")).unwrap())
                .unwrap();
        assert_eq!(added_delta["/platform_manager/new_setting"]["state"], "set");
        let restore_added = service
            .dispatch(request(5, "restore", "/platform_manager/new_setting", None))
            .await;
        assert!(restore_added.ok);
        assert!(shared
            .read()
            .await
            .get_subtree("/platform_manager/new_setting")
            .is_none());

        let deleted = service
            .dispatch(request(
                6,
                "delete",
                "/platform_manager/io_devices[id = ECU0]/baud_rate",
                None,
            ))
            .await;
        assert!(deleted.ok);
        let deleted_delta: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("_delta.json")).unwrap())
                .unwrap();
        assert_eq!(
            deleted_delta["/platform_manager/io_devices/0/baud_rate"]["state"],
            "unused"
        );
        let restore_deleted = service
            .dispatch(request(
                7,
                "restore",
                "/platform_manager/io_devices[id = ECU0]/baud_rate",
                None,
            ))
            .await;
        assert!(restore_deleted.ok);
        assert_eq!(
            shared
                .read()
                .await
                .get_leaf("/platform_manager/io_devices/0/baud_rate"),
            Some(&json!(115200))
        );
    }

    #[tokio::test]
    async fn read_only_paths_reject_every_kind_of_write() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        fs::write(
            &settings_path,
            json!({
                "error_codes": [{"code": 100, "name": "ERR_CONFIG_PARSING"}],
                "device": {"name": "rig"}
            })
            .to_string(),
        )
        .unwrap();
        let store = Arc::new(RwLock::new(Store::load_from_file(&settings_path).unwrap()));
        let service = ConfigService::new(store.clone(), Arc::new(PubSubEngine::default()))
            .with_read_only_paths(["/error_codes"]);
        let request =
            |id: u64, method: &str, path: &str, value: Option<serde_json::Value>| RpcRequest {
                id,
                method: method.to_string(),
                path: Some(path.to_string()),
                value: value.map(crate::config_value::ConfigValue::from_json),
            };

        for (method, path, value) in [
            (
                "set",
                "/error_codes[idx = 0]/name",
                Some(json!("ERR_OTHER")),
            ),
            ("set", "/error_codes", Some(json!([]))),
            ("set", "/", Some(json!({"device": {"name": "rig"}}))),
            ("delete", "/error_codes[idx = 0]", None),
            ("delete", "/error_codes", None),
        ] {
            let response = service.dispatch(request(1, method, path, value)).await;
            assert!(!response.ok, "{method} {path} must be rejected");
            let message = response.error.unwrap().message;
            assert!(message.contains("read-only"), "{method} {path}: {message}");
        }
        assert_eq!(
            store.read().await.get_subtree("/error_codes").unwrap(),
            json!([{"code": 100, "name": "ERR_CONFIG_PARSING"}])
        );

        // Reads and writes elsewhere still work, including a root set that keeps the subtree.
        assert!(
            service
                .dispatch(request(2, "get", "/error_codes[idx = 0]/code", None))
                .await
                .ok
        );
        assert!(
            service
                .dispatch(request(3, "set", "/device/name", Some(json!("rig-2"))))
                .await
                .ok
        );
        let whole = json!({
            "error_codes": [{"code": 100, "name": "ERR_CONFIG_PARSING"}],
            "device": {"name": "rig-3"}
        });
        assert!(
            service
                .dispatch(request(4, "set", "/", Some(whole)))
                .await
                .ok
        );
    }
}
