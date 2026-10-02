use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::{oneshot, RwLock};

use rekv::api::grpc::{
    start_grpc_server_with_shutdown, RekvServiceClient, SetRequest, WatchRequest,
};
use rekv::pubsub_engine::PubSubEngine;
use rekv::storage::Store;

#[tokio::test]
async fn test_grpc_watch_bubble_up_streaming() {
    // 1. Prepare Store with isolated temp copy of fixture
    let fixture_path = "tests/fixtures/settings.json";
    let temp_dir = tempdir().expect("Failed to create temporary settings directory");
    let temp_settings = temp_dir.path().join("settings.json");
    fs::copy(fixture_path, &temp_settings).expect("Failed to copy fixture");

    let store = Store::load_from_file(&temp_settings).expect("Failed to load store");
    let shared_store = Arc::new(RwLock::new(store));
    let pubsub = Arc::new(PubSubEngine::default());

    // 2. Bind and start gRPC server
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server_store = Arc::clone(&shared_store);
    let server_pubsub = Arc::clone(&pubsub);
    let server_handle = tokio::spawn(async move {
        start_grpc_server_with_shutdown(addr, server_store, server_pubsub, async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    // Wait for server to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // 3. Client 1 watches parent path: `/platform_manager/log`
    let endpoint = format!("http://127.0.0.1:{}", port);
    let channel1 = tonic::transport::Endpoint::from_shared(endpoint.clone())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client1 = RekvServiceClient::new(channel1);
    let mut stream_parent = client1
        .watch(WatchRequest {
            path: "/platform_manager/log".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    // 4. Client 2 watches root path: `/`
    let channel2 = tonic::transport::Endpoint::from_shared(endpoint.clone())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client2 = RekvServiceClient::new(channel2);
    let mut stream_root = client2
        .watch(WatchRequest {
            path: "/".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    // 5. Client 3 watches sibling branch: `/platform_manager/peripherals`
    let channel3 = tonic::transport::Endpoint::from_shared(endpoint.clone())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client3 = RekvServiceClient::new(channel3);
    let mut stream_peripherals = client3
        .watch(WatchRequest {
            path: "/platform_manager/peripherals".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut stream_baud = client3
        .watch(WatchRequest {
            path: "/platform_manager/io_devices[id = ECU0]/baud_rate".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 6. Client 4 modifies a child path: `/platform_manager/log/level` -> `"warn"`
    let channel4 = tonic::transport::Endpoint::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client4 = RekvServiceClient::new(channel4);

    let set_resp = client4
        .set(SetRequest {
            path: "/platform_manager/log/level".to_string(),
            json_value: "\"warn\"".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(set_resp.success);

    // 7. Verify Client 1 receives bubble-up event on parent `/platform_manager/log`:
    let event1 = tokio::time::timeout(tokio::time::Duration::from_secs(1), stream_parent.message())
        .await
        .expect("Timeout waiting for parent stream event")
        .unwrap()
        .expect("Stream closed unexpectedly");

    assert_eq!(event1.path, "/platform_manager/log/level");
    assert_eq!(event1.new_value, "\"warn\"");

    // 8. Verify Client 2 receives bubble-up event on root `/`:
    let event2 = tokio::time::timeout(tokio::time::Duration::from_secs(1), stream_root.message())
        .await
        .expect("Timeout waiting for root stream event")
        .unwrap()
        .expect("Stream closed unexpectedly");

    assert_eq!(event2.path, "/platform_manager/log/level");
    assert_eq!(event2.new_value, "\"warn\"");

    // 9. Client 4 modifies a peripheral selected by its zero-based index.
    let set_resp2 = client4
        .set(SetRequest {
            path: "/platform_manager/peripherals[idx = 0]/pin".to_string(),
            json_value: "25".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(set_resp2.success);

    // 10. Verify Client 3 receives the event on `/platform_manager/peripherals`:
    let event3 = tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        stream_peripherals.message(),
    )
    .await
    .expect("Timeout waiting for peripherals stream event")
    .unwrap()
    .expect("Stream closed unexpectedly");

    assert_eq!(event3.path, "/platform_manager/peripherals/0/pin");
    assert_eq!(event3.new_value, "25");

    // 11. Verify Client 2 receives the root event:
    let event_root2 =
        tokio::time::timeout(tokio::time::Duration::from_secs(1), stream_root.message())
            .await
            .expect("Timeout waiting for root second stream event")
            .unwrap()
            .expect("Stream closed unexpectedly");

    assert_eq!(event_root2.path, "/platform_manager/peripherals/0/pin");
    assert_eq!(event_root2.new_value, "25");

    let set_baud = client4
        .set(SetRequest {
            path: "/platform_manager/io_devices[id = ECU0]/baud_rate".to_string(),
            json_value: "57600".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(set_baud.success);
    let baud_event =
        tokio::time::timeout(tokio::time::Duration::from_secs(1), stream_baud.message())
            .await
            .expect("Timeout waiting for ID-addressed baud-rate event")
            .unwrap()
            .expect("Baud-rate stream closed unexpectedly");
    assert_eq!(baud_event.path, "/platform_manager/io_devices/0/baud_rate");
    assert_eq!(baud_event.new_value, "57600");

    // 12. Drop client streams so Tonic HTTP/2 server can terminate cleanly
    drop(stream_parent);
    drop(stream_root);
    drop(stream_peripherals);
    drop(stream_baud);
    drop(client1);
    drop(client2);
    drop(client3);
    drop(client4);

    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}
