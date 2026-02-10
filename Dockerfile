# Builder stage
FROM rust:1.80-slim-bullseye as builder

WORKDIR /app

# Install dependencies needed for build
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests and source
COPY . .

# Build release binaries
RUN cargo build --release

# Final stage
FROM debian:bullseye-slim

WORKDIR /app

# Install runtime dependencies if any
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

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

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["--host", "0.0.0.0"]