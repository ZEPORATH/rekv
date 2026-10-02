# Go client and demo gateway

The Go package provides runtime-typed Get, Set, Delete, List, and Watch calls. The demo gateway serves the React UI and relays gRPC Watch events to browsers over SSE.

```sh
cd clients/go
go test ./...
go run ./cmd/demo-server --target 127.0.0.1:50051 --listen 127.0.0.1:8090 --web-root ../../web/dist
```

For local UI development, run `npm --prefix web run dev` from the repository root; Vite proxies `/api` to the Go gateway.