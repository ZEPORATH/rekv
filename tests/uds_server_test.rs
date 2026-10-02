use std::fs;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::{oneshot, RwLock};

use rekv::api::uds::{send_uds_request, start_uds_server_with_shutdown, UdsRequest, UdsResponse};
use rekv::storage::Store;
use serde_json::json;

#[tokio::test]
async fn test_uds_server_get_and_set() {
    // 1. Prepare isolated settings file
    let fixture_path = "tests/fixtures/settings.json";
    let temp_settings = NamedTempFile::new().expect("Failed to create temp settings file");
    fs::copy(fixture_path, temp_settings.path()).expect("Failed to copy fixture");

    let store = Store::load_from_file(temp_settings.path()).expect("Failed to load store");
    let shared_store = Arc::new(RwLock::new(store));

    // 2. Prepare isolated socket path
    let socket_temp = NamedTempFile::new().expect("Failed to create socket temp file");
    let socket_path = socket_temp.path().to_path_buf();
    // Drop the file handle so the Unix socket can bind to that path
    drop(socket_temp);

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
            path: "/platform_manager/peripherals#REED_UP/pin".to_string(),
        },
    )
    .await
    .expect("UDS GET #ID failed");

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
            assert_eq!(affected_paths, Some(vec!["/platform_manager/log/level".to_string()]));
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

    // 6. Shutdown cleanly
    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}
