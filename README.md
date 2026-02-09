# hpfeeds-rs

A modern, high-performance, async Rust implementation of the [HPFeeds](https://github.com/hpfeeds/hpfeeds) protocol.

This project is a wire-compatible, drop-in replacement for original HPFeeds brokers, optimized for extreme throughput (3.6M+ msg/s) and production reliability.

## Key Components

- **`hpfeeds-server`**: The central broker. Supports SQLite/JSON auth, granular ACLs, native TLS, and Prometheus metrics.
- **`hpfeeds-collector`**: A universal data bridge with "batteries included" sinks for Splunk, Kafka, Postgres, Mongo, Elastic, and more.
- **`hpfeeds-cli`**: Command-line tool for publishing, subscribing, and managing the SQLite user database.
- **`hpfeeds-client`**: Robust Rust library for building high-speed honeypots.

## Features

- **Extreme Performance**: Sustained 3.6M msg/s (3.5 GB/s) on standard hardware.
- **Zero-Copy Architecture**: Uses `bytes::Bytes` for efficient message broadcasting.
- **Greedy Batching**: All database and log outputs use intelligent buffering to maximize I/O efficiency.
- **Strict Protocol Safety**: Enforces 1MB limits and strict UTF-8 validation to prevent DoS and crashes.
- **Pure Rust**: 100% Rust stack including pure-Rust Kafka and TLS (rustls) for easy deployment.

## Installation

```bash
cargo build --release
```

## Universal Collector (`hpfeeds-collector`)

The collector is the bridge between the broker and your analysis stack. It supports multiple output sinks:

| Sink | Use Case | Config Flag |
| :--- | :--- | :--- |
| **Splunk HEC** | Modern direct Splunk ingestion | `--output splunk-hec` |
| **Kafka** | High-volume data lakes (Redpanda/Kafka) | `--output kafka` |
| **PostgreSQL** | Relational long-term storage | `--output postgres` |
| **MongoDB** | Flexible NoSQL storage | `--output mongo` |
| **Elasticsearch**| Search and Dashboards (Kibana) | `--output elastic` |
| **Redis** | Real-time Honeymaps (NodeJS/Go) | `--output redis` |
| **STIX 2.1** | Standard Threat Intel Feeds (MISP) | `--output stix` |
| **Syslog** | Legacy Enterprise SIEMs (RFC 5424) | `--output syslog` |
| **TCP JSON** | Custom developer streaming | `--output tcp` |
| **File** | JSON-Lines logs for local analysis | `--output file` |

### Example: Feed your Honeymap via Redis
```bash
./target/release/hpfeeds-collector -i admin -s secret --output redis --redis-url "redis://localhost/"
```

### Example: Professional Archival to Postgres
```bash
./target/release/hpfeeds-collector -i admin -s secret --output postgres --postgres-url "postgres://user:pass@host/db"
```

## Running the Broker

```bash
# Start with SQLite backend and TLS enabled
./target/release/hpfeeds-server --db hpfeeds.db --tls-cert cert.pem --tls-key key.pem
```

## Admin Tooling

```bash
# Add a new sensor user to the database
./target/release/hpfeeds-cli admin --db hpfeeds.db add-user sensor1 secret123

# Allow sensor1 to publish to the 'malware' channel
./target/release/hpfeeds-cli admin --db hpfeeds.db add-acl sensor1 malware --pub-allowed
```

## Benchmarking

A saturation test script is included to verify performance on your hardware:
```bash
./saturation_test.sh
```