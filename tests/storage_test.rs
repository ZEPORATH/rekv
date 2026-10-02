use rekv::storage::Store;
use serde_json::json;
use tempfile::NamedTempFile;

#[test]
fn test_empty_store() {
    let store = Store::empty();
    assert_eq!(store.leaf_count(), 0);
    assert!(store.exists("/"));
}

#[test]
fn test_basic_leaf_get_and_set() {
    let mut store = Store::empty();
    store.set("/server/port", json!(8080)).unwrap();
    store.set("/server/host", json!("0.0.0.0")).unwrap();

    assert_eq!(store.get_leaf("/server/port"), Some(&json!(8080)));
    assert_eq!(store.get_leaf("/server/host"), Some(&json!("0.0.0.0")));
    assert_eq!(store.get_leaf("/server/missing"), None);

    let subtree = store.get_subtree("/server").unwrap();
    assert_eq!(subtree, json!({ "port": 8080, "host": "0.0.0.0" }));
}

#[test]
fn test_real_fixture_loading_and_lookups() {
    let fixture_path = "tests/fixtures/settings.json";
    let store = Store::load_from_file(fixture_path).expect("Failed to load settings.json fixture");

    // Verify leaf count matches our analysis (136 leaves)
    assert_eq!(store.leaf_count(), 136);

    // Test direct leaf lookups (O(1))
    assert_eq!(store.get_leaf("/platform_manager/grpc_port"), Some(&json!(50051)));
    assert_eq!(store.get_leaf("/platform_manager/log/level"), Some(&json!("info")));
    assert_eq!(store.get_leaf("/platform_manager/constants/pyro_coefficient"), Some(&json!(4.5)));

    // Test primary-key ID lookups (O(1))
    let reed_up_idx = store.find_index_by_id("/platform_manager/peripherals", "REED_UP");
    assert_eq!(reed_up_idx, Some(0));

    let reed_up = store.get_item_by_id("/platform_manager/peripherals", "REED_UP").unwrap();
    assert_eq!(reed_up["pin"], json!(23));
    assert_eq!(reed_up["type"], json!("reed"));

    let ecu0_idx = store.find_index_by_id("/platform_manager/io_devices", "ECU0");
    assert_eq!(ecu0_idx, Some(0));

    // Test predicate filtering (O(1))
    let reed_devices = store.get_items_by_predicate("/platform_manager/peripherals", "type", "reed");
    assert_eq!(reed_devices.len(), 2);
    assert_eq!(reed_devices[0]["id"], json!("REED_UP"));
    assert_eq!(reed_devices[1]["id"], json!("REED_DOWN"));

    let relays = store.get_items_by_predicate("/platform_manager/peripherals", "type", "relay");
    assert_eq!(relays.len(), 3);

    let buff_reads = store.find_indices_by_predicate("/platform_manager/peripherals", "io_mode", "buff_read");
    assert_eq!(buff_reads.len(), 7);
}

#[test]
fn test_go_settings_fixture_lookups() {
    let fixture_path = "tests/fixtures/settings.go.json";
    let store = Store::load_from_file(fixture_path).expect("Failed to load settings.go.json fixture");

    assert_eq!(store.leaf_count(), 174);
    assert_eq!(store.get_leaf("/server/port"), Some(&json!(9090)));
    assert_eq!(store.get_leaf("/edge/static_address"), Some(&json!("127.0.0.1:50051")));

    // Test primary-key channel ID lookups:
    let heat_surf = store.get_item_by_id("/experiment_defaults/channels", "HEAT_SURF").unwrap();
    assert_eq!(heat_surf["unit"], json!("°C"));
    assert_eq!(heat_surf["decimals"], json!(2));

    // Test user role predicate lookups:
    let admins = store.get_items_by_predicate("/auth/default_users", "role", "admin");
    assert_eq!(admins.len(), 1);
    assert_eq!(admins[0]["username"], json!("admin"));
}

#[test]
fn test_prefix_scanning() {
    let fixture_path = "tests/fixtures/settings.json";
    let store = Store::load_from_file(fixture_path).unwrap();

    let log_keys = store.scan_prefix("/platform_manager/log");
    assert_eq!(log_keys.len(), 5);

    let mut key_names: Vec<&str> = log_keys.iter().map(|(k, _)| k.as_str()).collect();
    key_names.sort();
    assert_eq!(
        key_names,
        vec![
            "/platform_manager/log/dir",
            "/platform_manager/log/filename",
            "/platform_manager/log/level",
            "/platform_manager/log/max_file_count",
            "/platform_manager/log/rotate_bytes",
        ]
    );
}

#[test]
fn test_atomic_persistence_and_reload() {
    let temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path().to_path_buf();

    let mut store = Store::empty();
    store.set_file_path(temp_path.clone());
    store.set("/device/name", json!("SmartSensor")).unwrap();
    store.set("/device/baud", json!(115200)).unwrap();

    // Persist to disk:
    store.persist().unwrap();

    // Reload from disk into a new Store:
    let reloaded = Store::load_from_file(&temp_path).unwrap();
    assert_eq!(reloaded.get_leaf("/device/name"), Some(&json!("SmartSensor")));
    assert_eq!(reloaded.get_leaf("/device/baud"), Some(&json!(115200)));
}
