# syntax=docker/dockerfile:1.5

# Builder stage — uses BuildKit cache mounts for cargo registry/git and target
FROM rust:1.80-slim-bullseye AS builder

ARG PKG_CONFIG_VERSION=0.29.2-1
ARG LIBSSL_DEV_VERSION=1.1.1l-1~deb11u1

WORKDIR /app

# Install build dependencies (pinned versions)
# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config=${PKG_CONFIG_VERSION} \
        libssl-dev=${LIBSSL_DEV_VERSION} \
        ca-certificates \
        build-essential \
        binutils \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first to leverage layer caching of dependencies
COPY Cargo.toml Cargo.lock ./

# Create a dummy main so we can build dependencies and cache them
RUN mkdir -p src && echo 'fn main() { println!("dummy"); }' > src/main.rs

# Pre-build to cache dependencies (requires BuildKit): cargo registry/git cache
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --locked || true

# Copy full source and build using cache for target
COPY . .
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --locked

# Optionally strip binaries to reduce final image size
RUN strip target/release/hpfeeds-server || true && \
    strip target/release/hpfeeds-cli || true && \
    strip target/release/hpfeeds-collector || true

# Prepare runtime artifacts and ownership in builder for distroless
# (create data dir and set correct permissions so final distroless stage can be static)
RUN install -d -m 0755 /data && chown 65532:65532 /data || true
RUN chown 65532:65532 /app/target/release/hpfeeds-server || true
RUN chown 65532:65532 /app/target/release/hpfeeds-cli || true
RUN chown 65532:65532 /app/target/release/hpfeeds-collector || true

# Final minimal distroless runtime
FROM gcr.io/distroless/cc-debian11:nonroot AS runtime

ARG DEFAULT_COMPONENT=hpfeeds-server
ENV COMPONENT=${DEFAULT_COMPONENT}

WORKDIR /app

# Copy CA certificates from builder so TLS works
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# Copy release binaries, entrypoint, and data dir (ownership preserved)
COPY --from=builder /app/target/release/hpfeeds-server /usr/local/bin/
COPY --from=builder /app/target/release/hpfeeds-cli /usr/local/bin/
COPY --from=builder /app/target/release/hpfeeds-collector /usr/local/bin/
COPY --from=builder /app/target/release/hpfeeds-entrypoint /usr/local/bin/
COPY --from=builder /data /data

# Expose ports for server
EXPOSE 10000 9431

# Persisted data
VOLUME ["/data"]

# Use our tiny Rust entrypoint to pick & exec the right binary
ENTRYPOINT ["/usr/local/bin/hpfeeds-entrypoint"]
CMD ["--host", "0.0.0.0"]

# Run as non-root user provided by distroless
USER nonroot

# Basic HEALTHCHECK using the CLI binary to ensure the image can run commands
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/hpfeeds-cli","--version"]