# Benchmarking and Testing Strategies for rekv

This document outlines the planned strategies for benchmarking and testing the `rekv` project to ensure its performance, reliability, and correctness.

## 1. Benchmarking Strategies

Benchmarking is crucial for identifying performance bottlenecks and ensuring `rekv` meets its low-latency and resource-efficient goals for embedded systems. We will focus on key performance indicators (KPIs) relevant to a configuration service.

### 1.1 What to Benchmark

*   **Read Performance**: Latency and throughput for `GET` operations on various path complexities (e.g., root, deep paths, paths with multiple attributes).
*   **Write Performance**: Latency and throughput for `SET` operations, including atomic writes and broadcast writes.
*   **Attribute Query Performance**: Efficiency of `GET` requests involving complex attribute predicates.
*   **Watch Notification Latency**: The delay between a `SET` operation and the corresponding `WATCH` notification being received by clients.
*   **Memory Usage**: Monitor the memory footprint of the `rekvd` service under different configuration sizes and load conditions.
*   **CPU Utilization**: Measure CPU usage during peak read/write operations and watch notification bursts.
*   **File Persistence Overhead**: The impact of atomic persistence on write operation latency.

### 1.2 Tools and Methodologies

*   **Unit-level Benchmarks (`criterion.rs`)**: For granular performance analysis of individual components like `path_parser`, `path_resolver`, and core `BTreeMap` operations. This will help optimize critical algorithms.
*   **API Benchmarks (`wrk`, custom clients)**: For measuring the performance of the gRPC and UDS APIs under simulated load. Custom Rust clients can be developed to accurately simulate client behavior, especially for `WATCH` streams.
*   **Load Testing**: Gradually increasing the number of concurrent clients and request rates to identify breaking points and observe scaling behavior.
*   **Long-running Stability Tests**: Running benchmarks over extended periods to detect memory leaks or performance degradation over time.
*   **Raspberry Pi Specific Benchmarks**: Performing benchmarks directly on target `armv7` hardware to get realistic performance figures.

### 1.3 Environment Considerations

*   **Isolated Environment**: Benchmarks should run in an environment with minimal interference from other processes.
*   **Consistent Data**: Use a consistent dataset for comparisons across different optimization iterations.
*   **Cold vs. Warm Cache**: Test both scenarios where the configuration is freshly loaded (cold cache) and when it's already in memory (warm cache).

### 1.4 Local Baseline (2026-10-02)

Criterion was run against `tests/fixtures/settings.json` (136 indexed leaves) on x86_64, Intel Core Ultra 7 265K, Rust 1.97.1. Each case used a 100 ms warm-up, 500 ms measurement window, and 10 samples:

```sh
cargo bench --bench config_ops -- --warm-up-time 0.1 --measurement-time 0.5 --sample-size 10
```

| Operation | Median estimate |
| :--- | ---: |
| JSON decode | 8.28 us |
| JSON encode | 1.62 us |
| GET `/platform_manager/io_devices[id = ECU0]/baud_rate` | 15.48 us |
| SET path lookup | 15.36 us |
| Path parse | 153 ns |
| SET operation without transport or persistence | 71.64 us |

These are a local baseline, not a Raspberry Pi performance claim. The SET benchmark clones the in-memory store outside the measured operation and excludes transport, delta-file I/O, and fsync.

### 1.5 Depth-10 / 10k Delta Stress Run (2026-10-02)

The standalone harness generates a 980,220-byte natural JSON file with 10,000 ID-keyed device objects at tree depth 10. Cluster/rack/slot/device levels include arrays, strings, booleans, and numeric leaves. It measures base-file I/O/load, in-memory operations, random selector GET/SET, watcher timing, overlay load/read cost, path restore, and concurrent-reader atomicity.

```sh
cargo run --release --example benchmark_10k
```

On x86_64, Intel Core Ultra 7 265K, Rust 1.97.1 (7 samples for load, 256 random GETs, 32 random SETs, 16 restores):

| Operation | p50 | p95 |
| :--- | ---: | ---: |
| Base file write + fsync (one run) | 0.19 ms | n/a |
| Base file read only | 33.25 us | 80.57 us |
| Base load (read + JSON + indexes) | 40.56 ms | 54.46 ms |
| In-memory store clone | 16.45 ms | 20.85 ms |
| Selector SET + index rebuild (clone excluded) | 19.34 ms | 21.08 ms |
| Base depth-10 random ID GET | 4.44 us | 6.47 us |
| Service SET including delta write + fsync | 39.92 ms | 47.31 ms |
| SET start to watcher receive | 40.07 ms | 47.35 ms |
| Service response to watcher handoff | 119.94 us | 247.09 us |
| Overlay load (base + 32-entry delta + indexes) | 65.82 ms | 70.08 ms |
| Overlay depth-10 random ID GET | 4.44 us | 6.63 us |
| Path restore including delta rewrite + fsync | 43.10 ms | 54.29 ms |
| Restore while 512 reads ran | 45.19 us | n/a |

The delta was 4,575 bytes for 32 changed leaves. During restore, 512 concurrent reads observed only the baseline or override value; no torn values were observed. This is a local stress sample, not a target-device guarantee.

### 1.6 Raspberry Pi 4B AArch64 Depth-10 / 10k Run (2026-10-02)

The same harness was cross-compiled for `aarch64-unknown-linux-gnu`, copied to a Raspberry Pi 4B running a 64-bit OS, and run as `./benchmark_10k`. The OS/glibc version and storage medium were not recorded. The dataset contained 10,000 records at depth 10 and occupied 980,220 bytes. Sample counts were 7 for reads/loads, 256 for random GETs, 32 for clones/SETs, and 16 for restores.

| Operation | p50 | p95 |
| :--- | ---: | ---: |
| Base file write + fsync (one run) | 0.95 ms | n/a |
| Base file read only | 662.87 us | 753.16 us |
| Base load (read + JSON + indexes) | 341.62 ms | 373.21 ms |
| In-memory store clone | 144.04767 ms | 145.82004 ms |
| Selector SET + full index rebuild (clone excluded) | 172.98663 ms | 175.38717 ms |
| Base depth-10 random ID GET | 29.69 us | 33.33 us |
| Service SET including delta write + fsync | 364.83126 ms | 366.16873 ms |
| SET start to watcher receive | 364.86658 ms | 366.21641 ms |
| Service response to watcher handoff | 49.26 us | 66.35 us |
| Overlay load (base + 32-entry delta + indexes) | 618.88 ms | 627.96 ms |
| Overlay depth-10 random ID GET | 29.91 us | 32.22 us |
| Path restore including delta rewrite + fsync | 404.94324 ms | 419.74511 ms |
| Restore while 512 reads ran (one run; see caveat below) | 410.52 us | n/a |

The delta contained 32 changed leaves and occupied 4,575 bytes. The original settings file remained unchanged, and 512 concurrent reads observed only the baseline or override value; no torn values were observed in this run.

Store cloning and full index rebuilding together account for approximately 87% of the durable SET p50, based on separately measured medians rather than a per-operation trace. Incremental indexing should reduce the rebuild cost, but the transaction clone remains a separate bottleneck. Overlay GET latency was essentially unchanged (29.69 us base versus 29.91 us overlay), while overlay startup was slower (341.62 ms versus 618.88 ms).

This binary still uses full index rebuilding; it does not measure the requested incremental-index optimization. Repeated file reads likely benefit from the filesystem cache and must not be interpreted as cold SD-card timings. The final concurrent-reader restore reported 410.52 us, substantially below the earlier restore p50 of 404.94 ms; retain it as reported but investigate the discrepancy before using it as a representative restore latency.

#### Build and Run on Raspberry Pi 4B (AArch64)

Use a 64-bit Linux OS on the Pi 4B. Verify that `uname -m` reports `aarch64`:

```sh
ssh pi@raspberrypi 'uname -m'
```

On the development machine, install Rust/Cargo and Docker, ensure the Docker daemon is running and accessible to your user, then install `cross` once:

```sh
docker info
cargo install cross --locked
```

From the repository root on the development machine, build and copy only the benchmark executable. No repository clone or Rust installation is needed on the Pi:

```sh
cross build --release --example benchmark_10k --target aarch64-unknown-linux-gnu
scp target/aarch64-unknown-linux-gnu/release/examples/benchmark_10k pi@raspberrypi:~/benchmark_10k
ssh pi@raspberrypi 'chmod +x ~/benchmark_10k && ~/benchmark_10k | tee ~/rekv-benchmark-results.txt'
```

Replace `pi@raspberrypi` with your Pi's SSH user and hostname or IP address. Results are printed to the terminal and saved to `~/rekv-benchmark-results.txt` on the Pi. Run while the Pi is otherwise idle and keep storage, cooling, and power conditions consistent when comparing results.

The executable generates its dataset in a temporary directory and removes it on exit. Its Linux/glibc runtime must be compatible with the binary. A Pi 4B running a 32-bit OS requires `armv7-unknown-linux-gnueabihf` instead; select the target by OS architecture, not just the board model.

## 2. Testing Strategies

A robust testing strategy ensures the correctness, reliability, and maintainability of the `rekv` project.

### 2.1 Unit Testing

*   **Focus**: Individual functions, methods, and small modules.
*   **Tools**: Rust's built-in `#[test]` attribute.
*   **Coverage**: Aim for high code coverage for core logic (e.g., `path_parser`, `path_resolver`, `config_store` operations).
*   **Examples**: Testing specific path parsing scenarios, attribute matching, `BTreeMap` manipulations, and atomic file write logic.

### 2.2 Integration Testing

*   **Focus**: Interactions between different modules and external interfaces.
*   **Tools**: Rust's `#[test]` attribute, potentially with `tokio::test` for asynchronous components.
*   **Scenarios**: 
    *   **CLI to `rekvd`**: Test end-to-end flows for `get`, `set`, and `watch` commands through both UDS and gRPC.
    *   **gRPC API**: Verify correct request/response handling, streaming behavior for `Watch`.
    *   **UDS API**: Test low-latency local communication.
    *   **Persistence**: Ensure configuration is correctly saved and loaded from disk after `rekvd` restarts.
    *   **Pub/Sub Engine**: Verify that `WATCH` clients receive correct and timely notifications, including bubble-up events.

### 2.3 System Testing (End-to-End)

*   **Focus**: The entire `rekv` system deployed as it would be in production.
*   **Scenarios**: 
    *   Deployment and startup of `rekvd` service via `systemd`.
    *   Interaction of multiple `rekv` CLI clients with a running `rekvd` instance.
    *   Concurrent read/write operations and their impact on system stability.
    *   Error handling for network failures, invalid input, and file system issues.

### 2.4 Performance Testing

*   **Focus**: Validate that the system meets its performance requirements under various loads.
*   **Methodology**: Utilize the benchmarking strategies outlined in Section 1 to run automated performance tests.
*   **Regression**: Integrate performance tests into CI/CD to prevent performance regressions.

### 2.5 Durability and Stress Testing

*   **Focus**: Verify the system's resilience to high load, unexpected events, and long-term operation.
*   **Scenarios**: 
    *   Simulate sudden power loss during write operations (to test atomic persistence).
    *   Run `rekvd` for extended periods with continuous read/write activity.
    *   Introduce network interruptions or resource constraints.

### 2.6 Test Data Management

*   **Synthetic Data**: Generate realistic, varied configuration data for testing different path complexities and value types.
*   **Versioned Data**: Maintain different versions of test data to easily reproduce and track issues.

By following these strategies, we aim to build a robust and high-performing `rekv` service that is suitable for embedded system environments.
