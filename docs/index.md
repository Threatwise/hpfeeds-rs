# Welcome to hpfeeds-rs

`hpfeeds-rs` is a modern, high-performance, async Rust implementation of the [HPFeeds](https://github.com/hpfeeds/hpfeeds) protocol.

This project provides a wire-compatible, drop-in replacement for original HPFeeds brokers, optimized for extreme throughput (3.6M+ msg/s) and production reliability.

## Getting Started

### Installation

Build the entire suite from source:

```bash
cargo build --release
```

The binaries will be found in `target/release/`.

### Quick Start

1. **Start the Broker**:
   ```bash
   ./target/release/hpfeeds-server --auth "admin:secret"
   ```

2. **Subscribe to a Channel**:
   ```bash
   ./target/release/hpfeeds-cli -i admin -s secret sub malware
   ```

3. **Publish a Message**:
   ```bash
   echo "malware payload" | ./target/release/hpfeeds-cli -i admin -s secret pub -c malware
   ```

## Documentation Sections

*   **[Architecture](architecture.md)**: Deep dive into how we achieve 3.6M msg/s.
*   **[Server](server.md)**: Configuration, TLS, and SQLite setup.
*   **[Collector](collector.md)**: Routing data to Splunk, Kafka, and Databases.
*   **[CLI](cli.md)**: Using the admin and testing tools.
