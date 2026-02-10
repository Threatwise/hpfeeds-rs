# Builder stage
FROM rust:1.80-slim-bullseye as builder

ARG PKG_CONFIG_VERSION=0.29.2-1
ARG LIBSSL_DEV_VERSION=1.1.1l-1~deb11u1

WORKDIR /app

# Install dependencies needed for build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config=${PKG_CONFIG_VERSION} libssl-dev=${LIBSSL_DEV_VERSION} && rm -rf /var/lib/apt/lists/*

# Copy manifests and source
COPY . .

# Build release binaries
RUN cargo build --release

# Final stage
FROM debian:bullseye-slim

ARG CA_CERTIFICATES_VERSION=20210119~deb11u3
ARG CURL_VERSION=7.74.0-1.3+deb11u4

WORKDIR /app

# Install runtime dependencies if any (versions pinned)
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates=${CA_CERTIFICATES_VERSION} curl=${CURL_VERSION} && rm -rf /var/lib/apt/lists/*

# Copy binaries
COPY --from=builder /app/target/release/hpfeeds-server /usr/local/bin/
COPY --from=builder /app/target/release/hpfeeds-cli /usr/local/bin/
COPY --from=builder /app/target/release/hpfeeds-collector /usr/local/bin/

# Expose ports for server
EXPOSE 10000 9431

# Create volume for persistence
VOLUME ["/data"]

# Default to server, but can be overridden
ENV COMPONENT=hpfeeds-server

# Entrypoint script to handle different components
RUN echo '#!/bin/sh\nexec "$COMPONENT" "$@"' > /usr/local/bin/entrypoint.sh && \
    chmod +x /usr/local/bin/entrypoint.sh

# Add non-root user and set ownership for runtime
RUN groupadd -r hpfeeds && useradd -r -g hpfeeds -d /nonexistent -s /usr/sbin/nologin -c "hpfeeds user" hpfeeds && \
    mkdir -p /data && chown -R hpfeeds:hpfeeds /data /usr/local/bin/hpfeeds-server /usr/local/bin/hpfeeds-cli /usr/local/bin/hpfeeds-collector

USER hpfeeds

# Healthcheck to ensure the container is healthy
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD curl -f http://127.0.0.1:9431/ || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["--host", "0.0.0.0"]