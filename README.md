# rekv: Rust Embedded Key Value store

A lightweight hierarchical configuration service for Raspberry Pi and embedded systems. rekv provides a flexible API to manage JSON-based configuration.

## Features

- **Hierarchical JSON configuration with XPath-like addressing** - Store nested configuration using XPath-like paths with multi-attribute support (e.g., `/device[id="sensor1"][type="temp"]/gpio/17/value`).
- **Atomic persistence** - Config is saved to disk atomically (temp file + rename) to prevent corruption.
- **rekvd as a Service** - A daemon process (`rekvd`) managing configurations.
- **CLI for interaction** - A command-line interface to interact with the `rekvd` service.
- **Single-writer, multi-reader** - Uses `RwLock` for safe concurrent access.

## Architecture

- **In-memory store**: Configuration is kept in memory as `serde_json::Value` for fast access.
- **File persistence**: Config is persisted to a configurable file path.
- **`rekvd` service**: A daemon process that exposes the configuration API.
- **CLI**: A client application that interacts with `rekvd` service using gRPC or UDS.
- **Storage Layer**: Utilizes `BTreeMap` for efficient storage of canonical keys and `HashMap` for attribute indexing.
- **Concurrency**: Single-writer, multi-reader pattern using `std::sync::RwLock`.

## CLI Usage (Planned)

The `rekv` CLI tool will allow users to:

- **Connect to `rekvd`**: The CLI will prioritize connecting via Unix Domain Sockets (UDS) for local communication. If a UDS connection is not possible or an alternative address is specified, it will attempt a gRPC connection.

- **Specify service address**: Users can explicitly provide the address of the `rekvd` service using a flag (e.g., `--address` or `-a`). This will allow connecting to `rekvd` instances running on different hosts or ports.

  ```bash
  # Connect via local UDS (default)
  rekv get /

  # Connect via gRPC to a local address and port
  rekv --address 127.0.0.1:50051 get /

  # Connect via gRPC to a remote address and port
  rekv --address my.remote.server:50051 get /device[id="sensor1"]/gpio
  ```

- **Get configuration**: Retrieve full configuration or a specific value by path.
  ```bash
  rekv get /
  rekv get /device[id="sensor1"]/gpio
  ```
- **Set configuration**: Set a value at a given path.
  ```bash
  rekv set /device[id="sensor1"]/gpio/17/value 1
  ```
- **Watch for changes**: Subscribe to configuration updates.
  ```bash
  rekv watch /
  ```

## `rekvd` Service (Planned)

The `rekvd` service will run as a background daemon, providing:

- **gRPC API**: For remote and cross-language communication.
- **Unix Domain Socket (UDS) API**: For ultra-low latency local communication with Rust/C applications.

## Project Structure (Planned)

```
rekv/
├── src/
│   ├── main.rs          # Entry point, `rekvd` service setup, CLI entry point
│   ├── config.rs        # Config store, persistence, path operations
│   ├── api/             # gRPC and UDS API definitions and handlers
│   ├── cli/             # CLI command parsing and execution logic
│   ├── path_parser.rs   # Nom based parser for hierarchical paths and predicates
│   └── path_resolver.rs # Resolves virtual XPaths to physical keys
├── Cargo.toml           # Dependencies
├── LICENSE              # Apache 2.0 License
├── benchmarking.md      # Benchmarking and testing strategies
└── README.md            # This file
```

## Deployment

### 1. Obtaining Compiled Binaries

The `rekv` project compiles into two main executables:
- `rekvd`: The daemon service.
- `rekv`: The command-line interface (CLI) tool.

**Local Compilation (for your host machine):**

```bash
cargo build --release
```
The binaries will be located at `target/release/rekvd` and `target/release/rekv`.

**Cross-Compilation for Raspberry Pi (ARMv7):**

To get the `armv7` binaries, you can either:

*   **Use the Docker `armv7` build (recommended for CI/CD):** The `Dockerfile.rpi-armv7` and CircleCI setup will produce a Docker image. You can extract the binaries from this image.
    ```bash
    # After a successful CircleCI build or local docker build -f Dockerfile.rpi-armv7 -t rekv:latest-armv7 .
    docker create --name rekv_armv7_extractor rekv:latest-armv7
    docker cp rekv_armv7_extractor:/usr/local/bin/rekv ./rekv_armv7
    docker rm rekv_armv7_extractor
    # The 'rekv_armv7' file is your compiled ARMv7 binary
    # You would typically have separate executables for rekvd and rekv CLI.
    # For now, assuming 'rekv' is the daemon and CLI is built into it or separate.
    # If separate, adjust Dockerfile.rpi-armv7 to copy both.
    ```
*   **Compile directly on Raspberry Pi:** The simplest way to get native binaries for a Raspberry Pi is to compile the project directly on the device.
    ```bash
    # On your Raspberry Pi
    git clone <your_repo_url>
    cd rekv
    cargo build --release
    ```
    The binaries will be in `target/release/`.

### 2. Deploying `rekvd` as a System Service

For production deployments, `rekvd` should run as a background service. This example uses `systemd`, common on Linux distributions like Raspberry Pi OS.

**A. Create a `systemd` service file:**

Create a file named `/etc/systemd/system/rekvd.service` with the following content. Adjust `User`, `Group`, and `ExecStart` paths as necessary.

```
[Unit]
Description=rekv Daemon Service
After=network.target

[Service]
User=rekvuser          # Create this user or use an existing one
Group=rekvgroup        # Create this group or use an existing one
ExecStart=/usr/local/bin/rekvd  # Path to your rekvd binary
WorkingDirectory=/var/lib/rekv # Directory for config file persistence
StandardOutput=journal
StandardError=journal
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

**B. Install and start the service:**

```bash
sudo systemctl daemon-reload       # Reload systemd manager configuration
sudo systemctl enable rekvd.service # Enable the service to start on boot
sudo systemctl start rekvd.service  # Start the service immediately
sudo systemctl status rekvd.service # Check the service status
```

**C. Create necessary directories and user/group:**

```bash
sudo useradd -r -s /bin/false rekvuser  # Create a system user for rekvd
sudo mkdir -p /var/lib/rekv           # Create directory for config persistence
sudo chown rekvuser:rekvuser /var/lib/rekv # Set ownership
```

### 3. Shipping the `rekv` CLI Tool

The `rekv` CLI tool is a standalone executable.

**A. For individual users:**

Simply place the compiled `rekv` binary in a directory that's included in the user's `PATH` environment variable (e.g., `/usr/local/bin`).

```bash
sudo cp /path/to/your/compiled/rekv /usr/local/bin/rekv
```

**B. For wider distribution (e.g., package managers):**

For more formal distribution, you would typically create a package (e.g., `.deb` for Debian/Ubuntu, `RPM` for Fedora/RHEL, or use cargo's `install` command for Rust projects if publishing to crates.io) that handles placing the `rekvd` daemon and `rekv` CLI tool in appropriate system directories and setting up the `systemd` service file. This is beyond the scope of a simple README but is the standard practice for robust deployments.

## License

This project is licensed under the Apache License, Version 2.0. See the [LICENSE](LICENSE) file for details.

## Contributing

[Add contribution guidelines here]
