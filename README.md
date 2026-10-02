# rekv

rekv is a Rust-owned configuration service backed by an original JSON settings file and a sparse sibling `_delta.json`. Clients use newline-delimited JSON over a Unix socket, a loopback HTTP API, or the retained gRPC compatibility API. All transports call one Rust configuration service and share one in-memory store.

The Go demo gateway calls gRPC, serves the React editor, and forwards gRPC Watch events to browsers over server-sent events. The Rust service remains the sole owner of settings and persistence.

## Build and run

Rust gRPC bindings are checked in, so builds need no system or vendored `protoc`. The compiled service needs only its settings file and local socket/HTTP ports at runtime, including on Raspberry Pi ARMv7.

```sh
cargo build --release
cargo test
mkdir -p /tmp/rekv-settings
cp tests/fixtures/settings.json /tmp/rekv-settings/settings.json
cargo run --bin rekv -- daemon \
  --config /tmp/rekv-settings/settings.json \
  --uds /tmp/rekv.sock \
  --port 50051 \
  --http-port 8080
```

The service never rewrites the source settings file; it writes edits to the sibling `/tmp/rekv-settings/_delta.json`.

In a second terminal, a normal settings operation uses the provided device entry:

```sh
rekv-cli get '/platform_manager/io_devices[id = ECU0]/baud_rate'
rekv-cli set '/platform_manager/io_devices[id = ECU0]/baud_rate' 57600
rekv-cli backup
rekv-cli restore '/platform_manager/io_devices[id = ECU0]/baud_rate'
rekv-cli delete /platform_manager/new_setting
```

The C++ subscriber prints that exact change; `read_value` prints the current value once:

```sh
make -C clients/cpp
./clients/cpp/watch_baud_rate /tmp/rekv.sock
```

## Local Compose test

The local stack runs the Rust service, Go/React editor, and C++ watcher. It seeds a named settings volume from the real fixture on first start, so UI and CLI writes do not modify the checked-in fixture.

```sh
docker compose -f docker-compose.local-test.yml up --build -d
docker compose -f docker-compose.local-test.yml ps
```

Open `http://<server-LAN-IP>:8099` from another machine. Change the selected ECU0 baud rate in the form, then inspect the C++ subscriber output:

```sh
docker compose -f docker-compose.local-test.yml logs -f cpp-watcher
```

CLI operations use the same daemon and UDS:

```sh
docker compose -f docker-compose.local-test.yml exec configd rekv --uds /run/rekv/rekv.sock get '/platform_manager/io_devices[id = ECU0]/baud_rate'
docker compose -f docker-compose.local-test.yml exec configd rekv --uds /run/rekv/rekv.sock set '/platform_manager/io_devices[id = ECU0]/baud_rate' 57600
docker compose -f docker-compose.local-test.yml exec configd rekv --uds /run/rekv/rekv.sock backup
docker compose -f docker-compose.local-test.yml exec configd rekv --uds /run/rekv/rekv.sock restore '/platform_manager/io_devices[id = ECU0]/baud_rate'
```

Stop the stack with `docker compose -f docker-compose.local-test.yml down`. Add `-v` to `down` to discard the test settings volume and reseed from the fixture next time.

The service binds gRPC and HTTP to `127.0.0.1` by default. The UDS socket is owner-only. The Rust HTTP routes are:

```text
GET    /api/config?path=/device/name
POST   /api/config                { "path": ..., "value": { "type": ..., "value": ... } }
DELETE /api/config?path=/device/name
GET    /api/config/list?path=/network/wifi
POST   /api/rpc                   newline-JSON RPC request envelope
```

Successful responses carry `id`, `ok`, and an optional typed `result`; errors use `INVALID_REQUEST`, `INVALID_PATH`, `NOT_FOUND`, `INVALID_VALUE`, or `INTERNAL_ERROR`.

## Language clients and editor

The Rust client is `rekv::client::ConfigClient` and uses the UDS JSON contract. The Go client uses the typed gRPC `Call` operation. The C++ client uses UDS and nlohmann/json. The TypeScript client uses HTTP.

Build the React editor and start the Go gateway:

```sh
npm --prefix web install
npm --prefix web run build
cd clients/go
go test ./...
go run ./cmd/demo-server --target 127.0.0.1:50051 --listen 127.0.0.1:8090 --web-root ../../web/dist
```

For live UI development, run `npm --prefix web run dev` from the repository root and open the Vite URL. The dev proxy forwards `/api` to the Go gateway. For a production demo, open `http://127.0.0.1:8090`.

Build both C++ examples with `make -C clients/cpp`. They connect to the same UDS and use the real fixture's ECU0 baud-rate path.

## Runtime values and persistence

Wire values are schema-less envelopes: `string`, `integer`, `float`, `boolean`, `object`, `array`, `str_list`, `numeric_list`, and `null`. `str_list` and `numeric_list` are validated homogeneous arrays. The configured settings JSON is the immutable baseline. Saves write only changed canonical leaf paths to its sibling `_delta.json`; each entry is either `set` with a typed value or `unused` for a deleted baseline value. New settings are `set` entries whose path is absent from the baseline. On startup, rekv loads the baseline and overlays the delta. Restoring one path removes only that delta entry, revealing the original value or removing an added value. The baseline file is never rewritten by client changes.

Object-array elements are selected with `[id = value]` or `[idx = n]`, for example `/platform_manager/io_devices[id = ECU0]/baud_rate`. `*` lists direct children of the selected object. Writes stage a candidate store and atomically update only the corresponding `_delta.json` entry before publishing changes to watchers.

The retained gRPC API is a deliberate compatibility exception to the original MVP's no-gRPC constraint, requested for the Go demo. The new typed `Call` operation carries runtime envelopes; the legacy Get/Set/Watch methods remain available.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
cd clients/go && go test ./...
npm --prefix web run build
```

For ARMv7 container builds, use `docker build -f Dockerfile.rpi-armv7 -t rekv:armv7 .`. The service does not require Go, Node, or C++ on its target device.