use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_cli_daemon_and_commands() {
    let fixture_path = "tests/fixtures/settings.json";
    let temp_dir = tempdir().expect("Failed to create temporary settings directory");
    let settings_path = temp_dir.path().join("settings.json");
    fs::copy(fixture_path, &settings_path).expect("Failed to copy fixture");
    let socket_path = temp_dir.path().join("rekv.sock");

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();

    // 1. Start daemon process
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "daemon",
            "--config",
            settings_path.to_str().unwrap(),
            "--port",
            &port,
            "--http-port",
            "0",
            "--uds",
            socket_path.to_str().unwrap(),
        ])
        .spawn()
        .expect("Failed to spawn daemon process");

    // Wait for daemon to initialize
    std::thread::sleep(std::time::Duration::from_millis(300));

    // 2. Test GET via CLI over UDS
    let output_uds = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--uds",
            socket_path.to_str().unwrap(),
            "get",
            "/platform_manager/grpc_port",
        ])
        .output()
        .expect("Failed to run CLI get via UDS");

    assert!(output_uds.status.success());
    let stdout_uds = String::from_utf8_lossy(&output_uds.stdout);
    assert_eq!(stdout_uds.trim(), "50051");

    // 3. Test GET via CLI over gRPC
    let output_grpc = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--address",
            &format!("127.0.0.1:{}", port),
            "get",
            "/platform_manager/peripherals[id = REED_UP]/pin",
        ])
        .output()
        .expect("Failed to run CLI get via gRPC");

    assert!(output_grpc.status.success());
    let stdout_grpc = String::from_utf8_lossy(&output_grpc.stdout);
    assert_eq!(stdout_grpc.trim(), "23");

    // 4. Test SET via CLI over UDS
    let output_set = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--uds",
            socket_path.to_str().unwrap(),
            "set",
            "/platform_manager/log/level",
            "debug",
        ])
        .output()
        .expect("Failed to run CLI set via UDS");

    assert!(output_set.status.success());

    // Verify SET updated the value via GET
    let output_verify = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--uds",
            socket_path.to_str().unwrap(),
            "get",
            "/platform_manager/log/level",
        ])
        .output()
        .expect("Failed to run CLI verify get via UDS");

    assert!(output_verify.status.success());
    let stdout_verify = String::from_utf8_lossy(&output_verify.stdout);
    assert_eq!(stdout_verify.trim(), "debug");

    let output_backup = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args(["--uds", socket_path.to_str().unwrap(), "backup"])
        .output()
        .expect("Failed to run backup");
    assert!(output_backup.status.success());
    let delta_path = temp_dir.path().join("_delta.json");
    assert!(delta_path.exists());
    let delta: serde_json::Value = serde_json::from_slice(&fs::read(&delta_path).unwrap()).unwrap();
    assert_eq!(delta["/platform_manager/log/level"]["state"], "set");

    let baud_path = "/platform_manager/io_devices[id = ECU0]/baud_rate";
    let output_baud_set = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--uds",
            socket_path.to_str().unwrap(),
            "set",
            baud_path,
            "57600",
        ])
        .output()
        .expect("Failed to change baud rate");
    assert!(output_baud_set.status.success());

    let output_restore = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args(["--uds", socket_path.to_str().unwrap(), "restore", baud_path])
        .output()
        .expect("Failed to restore backup");
    assert!(output_restore.status.success());

    let output_baud = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args(["--uds", socket_path.to_str().unwrap(), "get", baud_path])
        .output()
        .expect("Failed to read restored baud rate");
    assert!(output_baud.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output_baud.stdout).trim(),
        "115200"
    );

    let output_create = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--uds",
            socket_path.to_str().unwrap(),
            "set",
            "/temporary/setting",
            "true",
        ])
        .output()
        .expect("Failed to create temporary setting");
    assert!(output_create.status.success());
    let output_delete = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "--uds",
            socket_path.to_str().unwrap(),
            "delete",
            "/temporary/setting",
        ])
        .output()
        .expect("Failed to delete temporary setting");
    assert!(output_delete.status.success());

    // 5. Clean up daemon process
    let _ = daemon.kill();
    let _ = daemon.wait();
}
