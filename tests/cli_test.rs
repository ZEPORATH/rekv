use std::fs;
use std::process::Command;
use tempfile::NamedTempFile;

#[test]
fn test_cli_daemon_and_commands() {
    let fixture_path = "tests/fixtures/settings.json";
    let temp_settings = NamedTempFile::new().expect("Failed to create temp settings file");
    fs::copy(fixture_path, temp_settings.path()).expect("Failed to copy fixture");

    let socket_temp = NamedTempFile::new().expect("Failed to create socket temp file");
    let socket_path = socket_temp.path().to_path_buf();
    drop(socket_temp);

    let port = "50071";

    // 1. Start daemon process
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_rekv"))
        .args([
            "daemon",
            "--config",
            temp_settings.path().to_str().unwrap(),
            "--port",
            port,
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
            "/platform_manager/peripherals#REED_UP/pin",
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

    // 5. Clean up daemon process
    let _ = daemon.kill();
    let _ = daemon.wait();
}
