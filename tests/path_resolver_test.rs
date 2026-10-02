use rekv::path_resolver::{resolve_get, resolve_set};
use rekv::storage::Store;
use serde_json::json;

#[test]
fn test_resolve_get_simple_and_subtree() {
    let fixture_path = "tests/fixtures/settings.json";
    let store = Store::load_from_file(fixture_path).unwrap();

    // 1. Direct leaf
    let res = resolve_get("/platform_manager/grpc_port", &store).unwrap();
    assert_eq!(res.to_json_value(), json!(50051));

    // 2. Subtree
    let res_sub = resolve_get("/platform_manager/constants", &store).unwrap();
    assert_eq!(
        res_sub.to_json_value(),
        json!({
            "active_area": 1.0,
            "active_thickness": 0.1,
            "pyro_coefficient": 4.5,
            "window_size": 10
        })
    );
}

#[test]
fn test_resolve_get_primary_key_id_shorthand() {
    let fixture_path = "tests/fixtures/settings.json";
    let store = Store::load_from_file(fixture_path).unwrap();

    // Primary key shorthand: #REED_UP
    let res = resolve_get("/platform_manager/peripherals#REED_UP", &store).unwrap();
    let val = res.to_json_value();
    assert_eq!(val["id"], json!("REED_UP"));
    assert_eq!(val["pin"], json!(23));

    // Suffix path after primary key ID: #REED_UP/pin
    let res_pin = resolve_get("/platform_manager/peripherals#REED_UP/pin", &store).unwrap();
    assert_eq!(res_pin.to_json_value(), json!(23));

    // Another device: #ECU0/port
    let res_ecu = resolve_get("/platform_manager/io_devices#ECU0/port", &store).unwrap();
    assert_eq!(res_ecu.to_json_value(), json!("/dev/ttyUSB0"));
}

#[test]
fn test_resolve_get_predicates() {
    let fixture_path = "tests/fixtures/settings.json";
    let store = Store::load_from_file(fixture_path).unwrap();

    // [type="reed"]
    let res = resolve_get("/platform_manager/peripherals[type=\"reed\"]/id", &store).unwrap();
    assert_eq!(res.to_json_value(), json!(["REED_UP", "REED_DOWN"]));

    // Comparison operator: [pin >= 22]
    let res_gt = resolve_get("/platform_manager/peripherals[pin>=22]/id", &store).unwrap();
    let gt_ids = res_gt.to_json_value();
    assert_eq!(
        gt_ids,
        json!(["REED_UP", "REED_DOWN", "RELAY_PELTIER"])
    );

    // Multiple chained predicates: [type="relay"][default=0]
    let res_relays = resolve_get("/platform_manager/peripherals[type=\"relay\"][default=0]/id", &store).unwrap();
    assert_eq!(
        res_relays.to_json_value(),
        json!(["RELAY_DIMMER", "RELAY_FAN_0", "RELAY_PELTIER"])
    );

    // Float comparison in safety_rules: [threshold >= 85.0]
    let res_safe = resolve_get("/platform_manager/safety_rules[threshold>=85.0]/target", &store).unwrap();
    assert_eq!(res_safe.to_json_value(), json!("RELAY_PELTIER"));
}

#[test]
fn test_resolve_get_wildcards() {
    let fixture_path = "tests/fixtures/settings.json";
    let store = Store::load_from_file(fixture_path).unwrap();

    // 1. Single level wildcard: /io_devices/*/id
    let res = resolve_get("/platform_manager/io_devices/*/id", &store).unwrap();
    assert_eq!(res.to_json_value(), json!(["ECU0", "arduino_ecu_1", "rpi_gpio"]));

    // 2. Recursive multi-level wildcard: /**/pin
    let res_pin = resolve_get("/**/pin", &store).unwrap();
    assert_eq!(res_pin.to_json_value(), json!([23, 24, 18, 17, 22]));
}

#[test]
fn test_resolve_set_single_and_broadcast() {
    let fixture_path = "tests/fixtures/settings.json";
    let content = std::fs::read_to_string(fixture_path).unwrap();
    let tree: serde_json::Value = serde_json::from_str(&content).unwrap();
    let mut store = Store::from_value(tree, None);

    // 1. Set single leaf
    let set_res = resolve_set("/platform_manager/log/level", json!("debug"), &mut store).unwrap();
    assert_eq!(set_res.affected_paths, vec!["/platform_manager/log/level"]);
    assert_eq!(store.get_leaf("/platform_manager/log/level"), Some(&json!("debug")));

    // 2. Broadcast write: update all relays default to 1
    let broadcast_res = resolve_set(
        "/platform_manager/peripherals[type=\"relay\"]/default",
        json!(1),
        &mut store,
    )
    .unwrap();

    assert_eq!(broadcast_res.affected_paths.len(), 3);
    assert_eq!(
        store.get_leaf("/platform_manager/peripherals/2/default"),
        Some(&json!(1))
    );
    assert_eq!(
        store.get_leaf("/platform_manager/peripherals/3/default"),
        Some(&json!(1))
    );
    assert_eq!(
        store.get_leaf("/platform_manager/peripherals/4/default"),
        Some(&json!(1))
    );
}

#[test]
fn test_resolve_set_new_nested_path() {
    let mut store = Store::empty();
    let res = resolve_set("/app/config/feature_flags/new_ui", json!(true), &mut store).unwrap();
    assert_eq!(res.affected_paths, vec!["/app/config/feature_flags/new_ui"]);
    assert_eq!(store.get_leaf("/app/config/feature_flags/new_ui"), Some(&json!(true)));
}
