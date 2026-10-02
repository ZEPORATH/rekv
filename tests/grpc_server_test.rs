use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::{oneshot, RwLock};

use rekv::api::grpc::{
    start_grpc_server_with_shutdown, GetRequest, RekvServiceClient, RpcCallRequest, SetRequest,
};
use rekv::storage::Store;

#[tokio::test]
async fn test_grpc_server_get_and_set() {
    // 1. Prepare Store with isolated temp file copy of fixture
    let fixture_path = "tests/fixtures/settings.json";
    let temp_dir = tempdir().expect("Failed to create temporary settings directory");
    let temp_file = temp_dir.path().join("settings.json");
    fs::copy(fixture_path, &temp_file).expect("Failed to copy fixture to temp file");

    let store = Store::load_from_file(&temp_file).expect("Failed to load temp settings");
    let shared_store = Arc::new(RwLock::new(store));

    // 2. Select port and shutdown channel
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let pubsub = Arc::new(rekv::pubsub_engine::PubSubEngine::default());
    let server_store = Arc::clone(&shared_store);
    let server_handle = tokio::spawn(async move {
        start_grpc_server_with_shutdown(addr, server_store, pubsub, async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    // Wait briefly for server to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // 3. Connect gRPC client
    let channel = tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{}", port))
        .unwrap()
        .connect()
        .await
        .expect("Failed to connect to gRPC server");
    let mut client = RekvServiceClient::new(channel);

    let typed_call = client
        .call(RpcCallRequest {
            id: 7,
            method: "get".to_string(),
            path: "/platform_manager/grpc_port".to_string(),
            json_value: String::new(),
        })
        .await
        .expect("typed gRPC Call failed")
        .into_inner();
    assert!(typed_call.ok);
    assert_eq!(typed_call.id, 7);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&typed_call.result_json).unwrap(),
        serde_json::json!({"type":"integer", "value":50051})
    );

    // 4. Test GET direct leaf:
    let resp = client
        .get(GetRequest {
            path: "/platform_manager/grpc_port".to_string(),
        })
        .await
        .expect("gRPC GET failed")
        .into_inner();

    assert!(resp.found);
    assert_eq!(resp.match_count, 1);
    assert_eq!(resp.json_value, "50051");

    // 5. Test GET with an id selector:
    let resp_id = client
        .get(GetRequest {
            path: "/platform_manager/peripherals[id = REED_UP]/pin".to_string(),
        })
        .await
        .expect("gRPC GET with id selector failed")
        .into_inner();

    assert!(resp_id.found);
    assert_eq!(resp_id.json_value, "23");

    // 6. Test GET with an id selector:
    let resp_pred = client
        .get(GetRequest {
            path: "/platform_manager/peripherals[id = REED_DOWN]/id".to_string(),
        })
        .await
        .expect("gRPC GET with second id selector failed")
        .into_inner();

    assert!(resp_pred.found);
    assert_eq!(resp_pred.match_count, 1);
    assert_eq!(resp_pred.json_value, "\"REED_DOWN\"");

    // 7. Test SET single leaf:
    let set_resp = client
        .set(SetRequest {
            path: "/platform_manager/log/level".to_string(),
            json_value: "\"debug\"".to_string(),
        })
        .await
        .expect("gRPC SET failed")
        .into_inner();

    assert!(set_resp.success);
    assert_eq!(set_resp.affected_count, 1);
    assert_eq!(set_resp.affected_paths, vec!["/platform_manager/log/level"]);

    // Verify changed value via GET:
    let verify_resp = client
        .get(GetRequest {
            path: "/platform_manager/log/level".to_string(),
        })
        .await
        .expect("gRPC GET failed")
        .into_inner();

    assert_eq!(verify_resp.json_value, "\"debug\"");

    // 8. Test SET on one selected array element:
    let broadcast_resp = client
        .set(SetRequest {
            path: "/platform_manager/peripherals[id = RELAY_PELTIER]/default".to_string(),
            json_value: "1".to_string(),
        })
        .await
        .expect("gRPC selector SET failed")
        .into_inner();

    assert!(broadcast_resp.success);
    assert_eq!(broadcast_resp.affected_count, 1);

    // 9. Shutdown server cleanly
    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}
