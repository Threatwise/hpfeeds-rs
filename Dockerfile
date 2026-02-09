# Builder stage
FROM rust:1.80-slim-bullseye as builder

WORKDIR /app

# Install dependencies needed for build (e.g., if we needed openssl, but we use rustls)
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binaries
RUN cargo build --release

# Final stage
FROM debian:bullseye-slim

WORKDIR /app

# Copy binaries
COPY --from=builder /app/target/release/hpfeeds-server /usr/local/bin/
COPY --from=builder /app/target/release/hpfeeds-cli /usr/local/bin/

# Expose ports
EXPOSE 10000 9431

# Create volume for persistence
VOLUME ["/data"]

# Entrypoint
ENTRYPOINT ["hpfeeds-server", "--host", "0.0.0.0"]
