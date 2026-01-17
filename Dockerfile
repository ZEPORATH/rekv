# Build stage
FROM rust:1.75-slim as builder

WORKDIR /app

# Copy dependency files
COPY Cargo.toml Cargo.lock* ./

# Create a dummy source to build dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code
COPY src ./src

# Build the application
RUN touch src/main.rs && \
    cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Create config directory
RUN mkdir -p /var/lib/myservice

# Copy binary from builder
COPY --from=builder /app/target/release/rekv /app/rekv

# Run as non-root user
RUN useradd -m -u 1000 rekv && \
    chown -R rekv:rekv /app /var/lib/myservice

USER rekv

EXPOSE 8080

# Default environment variables
ENV REKV_CONFIG_PATH=/var/lib/myservice/state.json
ENV REKV_REDIS_URL=redis://redis:6379
ENV REKV_REDIS_CHANNEL=rekv:config:updates
ENV REKV_BIND_ADDR=0.0.0.0:8080

CMD ["/app/rekv"]
