use rekv::pubsub_engine::{bubble_up_paths, ChangeEvent, PubSubEngine};

#[test]
fn test_bubble_up_path_derivation() {
    assert_eq!(bubble_up_paths("/"), vec!["/"]);
    assert_eq!(bubble_up_paths("/a"), vec!["/a", "/"]);
    assert_eq!(bubble_up_paths("/a/b"), vec!["/a/b", "/a", "/"]);
    assert_eq!(
        bubble_up_paths("/platform_manager/log/level"),
        vec![
            "/platform_manager/log/level",
            "/platform_manager/log",
            "/platform_manager",
            "/"
        ]
    );
}

#[tokio::test]
async fn test_pubsub_bubble_up_notifications() {
    let engine = PubSubEngine::default();

    // 1. Subscribe to different hierarchy levels:
    let mut rx_exact = engine.subscribe("/platform_manager/log/level");
    let mut rx_parent = engine.subscribe("/platform_manager/log");
    let mut rx_root = engine.subscribe("/");
    let mut rx_other = engine.subscribe("/platform_manager/peripherals");

    // 2. Publish change to /platform_manager/log/level
    let event = ChangeEvent::new(
        "/platform_manager/log/level",
        Some("\"info\"".to_string()),
        "\"debug\"",
    );
    let notified = engine.publish(event.clone());
    assert_eq!(notified, 3); // exact, parent, root

    // 3. Verify exact receiver got event
    let recv_exact = rx_exact.recv().await.unwrap();
    assert_eq!(recv_exact.path, "/platform_manager/log/level");
    assert_eq!(recv_exact.new_value, "\"debug\"");

    // 4. Verify parent receiver got event (bubble-up)
    let recv_parent = rx_parent.recv().await.unwrap();
    assert_eq!(recv_parent.path, "/platform_manager/log/level");

    // 5. Verify root receiver got event (bubble-up)
    let recv_root = rx_root.recv().await.unwrap();
    assert_eq!(recv_root.path, "/platform_manager/log/level");

    // 6. Verify unrelated branch did NOT receive event
    assert!(rx_other.try_recv().is_err());
}
