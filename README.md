# rekv

A lightweight hierarchical configuration service for Raspberry Pi and embedded systems. rekv provides a simple HTTP API to manage JSON-based configuration with Redis pub/sub notifications for config changes.

## Features

- **Hierarchical JSON configuration** - Store nested configuration using dot-separated paths (e.g., `device1.gpio.17.value`)
- **Atomic persistence** - Config is saved to disk atomically (temp file + rename) to prevent corruption
- **Redis notifications** - Publishes config update events to Redis channels (path, old value, new value)
- **Single-writer, multi-reader** - Uses `RwLock` for safe concurrent access
- **Simple HTTP API** - RESTful endpoints for getting and setting configuration
- **Raspberry Pi friendly** - Minimal dependencies, low resource usage

## Architecture

- **In-memory store**: Configuration is kept in memory as `serde_json::Value` for fast access
- **File persistence**: Config is persisted to `/var/lib/myservice/state.json` (configurable)
- **Redis pub/sub**: Used only for notifications, not as a datastore
- **Concurrency**: Single-writer, multi-reader pattern using `std::sync::RwLock`

## API Endpoints

### GET /config
Returns the full configuration as JSON.

**Example:**
```bash
curl http://localhost:8080/config
```

### GET /config/:path
Returns a subtree by dot-separated path. Returns 404 if the path doesn't exist.

**Example:**
```bash
curl http://localhost:8080/config/device1.gpio
```

### POST /config/:path
Sets a value at the given path. The request body should be a JSON value. Returns the old value (or `null` if it didn't exist).

**Example:**
```bash
curl -X POST http://localhost:8080/config/device1.gpio.17.value \
  -H "Content-Type: application/json" \
  -d '1'
```

## Configuration

The service can be configured via environment variables:

- `REKV_CONFIG_PATH` - Path to the config file (default: `/var/lib/myservice/state.json`)
- `REKV_REDIS_URL` - Redis connection URL (default: `redis://127.0.0.1:6379`)
- `REKV_REDIS_CHANNEL` - Redis channel for notifications (default: `rekv:config:updates`)
- `RUST_LOG` - Log level (default: `info`)

## Redis Events

When a config value is updated, an event is published to the Redis channel with the following structure:

```json
{
  "path": "device1.gpio.17.value",
  "old_value": 0,
  "new_value": 1,
  "timestamp": "2024-01-01T12:00:00Z"
}
```

## Building

```bash
cargo build --release
```

## Running

### Prerequisites
- Redis server running (default: `localhost:6379`)
- Write access to the config file directory

### Local Development
```bash
# Start Redis (if not already running)
redis-server

# Run the service
cargo run

# Or with custom config
REKV_CONFIG_PATH=./state.json cargo run
```

### Production (Raspberry Pi)
```bash
# Create config directory
sudo mkdir -p /var/lib/myservice
sudo chown $USER:$USER /var/lib/myservice

# Run the service
./target/release/rekv
```

## Docker

See the `Dockerfile` and `docker-compose.yml` for containerized deployment.

```bash
# Build the image
docker build -t rekv .

# Run with docker-compose
docker-compose up
```

## Project Structure

```
rekv/
├── src/
│   ├── main.rs      # Entry point, server setup
│   ├── config.rs    # Config store, persistence, path operations
│   ├── api.rs       # HTTP handlers and routing
│   └── redis.rs     # Redis pub/sub notifications
├── Cargo.toml       # Dependencies
├── Dockerfile       # Container image
└── README.md        # This file
```

## Concurrency Notes

- **Read operations** (`get_full`, `get_path`) acquire a read lock, allowing concurrent reads
- **Write operations** (`set_path`) acquire a write lock, blocking all readers during the update
- The write lock is held only during the in-memory update, then released before disk I/O
- For single-writer scenarios (typical for config services), this provides good performance

## Error Handling

- Config file I/O errors are propagated and logged
- Redis publish failures are logged but don't block config updates (fire-and-forget)
- Invalid paths return appropriate HTTP status codes (400 Bad Request, 404 Not Found)
- JSON parsing errors return 500 Internal Server Error

## Limitations

- No authentication or authorization
- No clustering or distributed locking
- Redis failures don't block config updates (notifications may be lost)
- Single process only (not designed for horizontal scaling)

## License

[Add your license here]

## Contributing

[Add contribution guidelines here]
