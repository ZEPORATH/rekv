use std::sync::Arc;

use rekv::config_service::ConfigService;
use rekv::path_resolver::{
    resolve_delete, resolve_get, resolve_list, resolve_set, ResolveError, ResolveGetResult,
};
use rekv::protocol::{ErrorCode, RpcRequest};
use rekv::pubsub_engine::PubSubEngine;
use rekv::storage::Store;
use serde_json::json;
use tokio::sync::RwLock;

fn fixture_store() -> Store {
    Store::load_from_file("tests/fixtures/settings.json").unwrap()
}

#[test]
fn plain_path_resolves_array() {
    let store = fixture_store();
    let result = resolve_get("/platform_manager/io_devices", &store).unwrap();
    assert_eq!(result.match_count(), 1);
    assert!(result.to_json_value().is_array());
}

#[test]
fn id_selector_resolves_selected_object_and_children() {
    let store = Store::from_value(
        json!({"platform_manager":{"io_devices":[
            {"id":"ECU0","baud_rate":115200},
            {"id":"ECU1","baud_rate":9600}
        ]}}),
        None,
    );
    let selected = resolve_get("/platform_manager/io_devices[id = ECU0]", &store)
        .unwrap()
        .to_json_value();
    assert_eq!(selected["id"], json!("ECU0"));

    let baud = resolve_get("/platform_manager/io_devices[id = ECU0]/baud_rate", &store).unwrap();
    assert_eq!(baud.to_json_value(), json!(115200));

    let second = resolve_get("/platform_manager/io_devices[id=ECU1]/baud_rate", &store).unwrap();
    assert_eq!(second.to_json_value(), json!(9600));
}

#[test]
fn idx_selector_is_zero_based() {
    let store = Store::from_value(
        json!({"platform_manager":{"io_devices":[
            {"id":"ECU0","baud_rate":115200},
            {"id":"ECU1","baud_rate":9600}
        ]}}),
        None,
    );
    assert_eq!(
        resolve_get("/platform_manager/io_devices[idx = 0]/baud_rate", &store)
            .unwrap()
            .to_json_value(),
        json!(115200)
    );
    assert_eq!(
        resolve_get("/platform_manager/io_devices[idx = 1]/baud_rate", &store)
            .unwrap()
            .to_json_value(),
        json!(9600)
    );
}

#[test]
fn wildcard_lists_selected_objects_direct_children() {
    let store = Store::from_value(
        json!({
            "platform_manager": {
                "io_devices": [{
                    "id": "ECU0",
                    "baud_rate": 115200,
                    "port": "/dev/ttyUSB0",
                    "enabled": true
                }]
            }
        }),
        None,
    );
    assert_eq!(
        resolve_list("/platform_manager/io_devices[id = ECU0]/*", &store).unwrap(),
        vec!["baud_rate", "enabled", "id", "port"]
    );
}

#[test]
fn missing_id_and_index_resolve_to_not_found() {
    let store = fixture_store();
    assert!(matches!(
        resolve_get(
            "/platform_manager/io_devices[id = UNKNOWN]/baud_rate",
            &store
        )
        .unwrap(),
        ResolveGetResult::NotFound
    ));
    assert!(matches!(
        resolve_get("/platform_manager/io_devices[idx = 999]/baud_rate", &store).unwrap(),
        ResolveGetResult::NotFound
    ));
}

#[test]
fn selector_on_non_array_is_invalid_path() {
    let store = Store::from_value(json!({"platform_manager":{"device":{"id":"ECU0"}}}), None);
    assert!(matches!(
        resolve_get("/platform_manager/device[id = ECU0]", &store),
        Err(ResolveError::InvalidPath(_))
    ));
}

#[test]
fn selectors_work_for_set_delete_and_keep_natural_json() {
    let original = json!({
        "platform_manager": {
            "io_devices": [
                { "id": "ECU0", "baud_rate": 115200 },
                { "id": "ECU1", "baud_rate": 9600 }
            ]
        }
    });
    let mut store = Store::from_value(original.clone(), None);
    resolve_set(
        "/platform_manager/io_devices[id = ECU0]/baud_rate",
        json!(57600),
        &mut store,
    )
    .unwrap();
    assert_eq!(
        store.get_leaf("/platform_manager/io_devices/0/baud_rate"),
        Some(&json!(57600))
    );
    assert_eq!(
        store.tree()["platform_manager"]["io_devices"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    resolve_delete(
        "/platform_manager/io_devices[idx = 1]/baud_rate",
        &mut store,
    )
    .unwrap();
    assert!(store
        .get_leaf("/platform_manager/io_devices/1/baud_rate")
        .is_none());
    assert_eq!(
        store.tree()["platform_manager"]["io_devices"][0]["id"],
        json!("ECU0")
    );
}

#[test]
fn invalid_selector_syntax_is_rejected_before_resolution() {
    let store = fixture_store();
    assert!(matches!(
        resolve_get(
            "/platform_manager/io_devices[type = ECU0]/baud_rate",
            &store
        ),
        Err(ResolveError::InvalidPath(_))
    ));
}

#[tokio::test]
async fn selector_failures_map_to_not_found_and_invalid_path() {
    let store = fixture_store();
    let service = ConfigService::new(
        Arc::new(RwLock::new(store)),
        Arc::new(PubSubEngine::default()),
    );

    for path in [
        "/platform_manager/io_devices[id = UNKNOWN]/baud_rate",
        "/platform_manager/io_devices[idx = 999]/baud_rate",
    ] {
        let response = service
            .dispatch(RpcRequest {
                id: 1,
                method: "get".to_string(),
                path: Some(path.to_string()),
                value: None,
            })
            .await;
        assert_eq!(response.error.unwrap().code, ErrorCode::NotFound);
    }

    for path in [
        "/platform_manager/io_devices[id ECU0]/baud_rate",
        "/platform_manager/log[id = ECU0]",
    ] {
        let response = service
            .dispatch(RpcRequest {
                id: 2,
                method: "get".to_string(),
                path: Some(path.to_string()),
                value: None,
            })
            .await;
        assert_eq!(response.error.unwrap().code, ErrorCode::InvalidPath);
    }
}
