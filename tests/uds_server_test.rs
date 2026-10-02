use std::fs;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{oneshot, RwLock};

use rekv::api::uds::{
    send_rpc_request, send_uds_request, start_uds_server_with_shutdown, UdsRequest, UdsResponse,
};
use rekv::client::ConfigClient;
use rekv::config_value::ConfigValue;
use rekv::protocol::RpcRequest;
use rekv::storage::Store;
use serde_json::json;

#[tokio::test]
async fn test_uds_server_get_and_set() {
    // 1. Prepare isolated settings file
    let fixture_path = "tests/fixtures/settings.json";
    let temp_dir = tempdir().expect("Failed to create temporary settings directory");
    let temp_settings = temp_dir.path().join("settings.json");
    fs::copy(fixture_path, &temp_settings).expect("Failed to copy fixture");

    let store = Store::load_from_file(&temp_settings).expect("Failed to load store");
    let shared_store = Arc::new(RwLock::new(store));

    // 2. Prepare isolated socket path
    let socket_path = temp_dir.path().join("rekv.sock");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server_store = Arc::clone(&shared_store);
    let s_path = socket_path.clone();
    let server_handle = tokio::spawn(async move {
        start_uds_server_with_shutdown(s_path, server_store, async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    // Wait briefly for UDS listener to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // 3. Test UDS GET
    let resp = send_uds_request(
        &socket_path,
        &UdsRequest::Get {
            path: "/platform_manager/grpc_port".to_string(),
        },
    )
    .await
    .expect("UDS GET failed");

    match resp {
        UdsResponse::Ok {
            found,
            value,
            match_count,
            ..
        } => {
            assert_eq!(found, Some(true));
            assert_eq!(match_count, Some(1));
            assert_eq!(value, Some(json!(50051)));
        }
        other => panic!("Unexpected response: {:?}", other),
    }

    // 4. Test UDS GET with primary-key ID shorthand
    let resp_id = send_uds_request(
        &socket_path,
        &UdsRequest::Get {
            path: "/platform_manager/peripherals[id = REED_UP]/pin".to_string(),
        },
    )
    .await
    .expect("UDS GET with id selector failed");

    match resp_id {
        UdsResponse::Ok { found, value, .. } => {
            assert_eq!(found, Some(true));
            assert_eq!(value, Some(json!(23)));
        }
        other => panic!("Unexpected response: {:?}", other),
    }

    // 5. Test UDS SET
    let set_resp = send_uds_request(
        &socket_path,
        &UdsRequest::Set {
            path: "/platform_manager/log/level".to_string(),
            value: json!("debug"),
        },
    )
    .await
    .expect("UDS SET failed");

    match set_resp {
        UdsResponse::Ok {
            affected_count,
            affected_paths,
            ..
        } => {
            assert_eq!(affected_count, Some(1));
            assert_eq!(
                affected_paths,
                Some(vec!["/platform_manager/log/level".to_string()])
            );
        }
        other => panic!("Unexpected set response: {:?}", other),
    }

    // Verify changed value via UDS GET
    let verify_resp = send_uds_request(
        &socket_path,
        &UdsRequest::Get {
            path: "/platform_manager/log/level".to_string(),
        },
    )
    .await
    .expect("UDS verify GET failed");

    match verify_resp {
        UdsResponse::Ok { value, .. } => {
            assert_eq!(value, Some(json!("debug")));
        }
        other => panic!("Unexpected verify response: {:?}", other),
    }

    let client = ConfigClient::connect(&socket_path).await.unwrap();
    let typed = client.get("/platform_manager/grpc_port").await.unwrap();
    assert_eq!(
        serde_json::to_value(typed).unwrap(),
        json!({"type":"integer", "value":50051})
    );
    client
        .set("/demo/enabled", ConfigValue::Boolean(true))
        .await
        .unwrap();
    assert_eq!(client.list("/demo").await.unwrap(), vec!["enabled"]);
    client.delete("/demo/enabled").await.unwrap();
    assert!(client.get("/demo/enabled").await.is_err());

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut watcher = BufReader::new(reader);
    let watch_request = serde_json::json!({
        "id": 91,
        "method": "watch",
        "path": "/platform_manager/io_devices[id = ECU0]/baud_rate"
    });
    writer
        .write_all(format!("{}\n", watch_request).as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    watcher.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["event"],
        "watching"
    );

    let update = RpcRequest {
        id: 92,
        method: "set".to_string(),
        path: Some("/platform_manager/io_devices[id = ECU0]/baud_rate".to_string()),
        value: Some(ConfigValue::from_json(json!(57600))),
    };
    assert!(send_rpc_request(&socket_path, &update).await.unwrap().ok);
    line.clear();
    watcher.read_line(&mut line).await.unwrap();
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["event"], "change");
    assert_eq!(event["path"], "/platform_manager/io_devices/0/baud_rate");
    assert_eq!(event["new_value"], json!(57600));

    writer
        .write_all(b"{\"id\":93,\"method\":\"unwatch\"}\n")
        .await
        .unwrap();
    line.clear();
    watcher.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["event"],
        "unwatched"
    );

    let oversized = UnixStream::connect(&socket_path).await.unwrap();
    let (reader, mut writer) = oversized.into_split();
    writer.write_all(&vec![b'x'; 64 * 1024 + 1]).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
    let mut response_line = String::new();
    BufReader::new(reader)
        .read_line(&mut response_line)
        .await
        .unwrap();
    let oversized_response: serde_json::Value = serde_json::from_str(&response_line).unwrap();
    assert_eq!(oversized_response["ok"], false);
    assert_eq!(oversized_response["error"]["code"], "INVALID_REQUEST");

    // 6. Shutdown cleanly
    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}
