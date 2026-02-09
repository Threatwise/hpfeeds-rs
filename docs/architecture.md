# Architecture of hpfeeds-rs

This document describes the high-performance design principles used in `hpfeeds-rs` to achieve 3.6M+ messages per second.

## 1. Zero-Copy Message Path
The broker uses the `bytes` crate for all internal message handling. When a message arrives from a publisher:
- It is decoded into a `Frame` structure where strings and payloads are `Bytes` handles.
- During broadcast, these handles are cloned using atomic reference counting—**no data is copied**.
- The same memory buffer received from the publisher is eventually sent to all subscribers.

## 2. Greedy Draining & Batching
The most significant bottleneck in high-frequency messaging is the system call overhead (writing to the socket). `hpfeeds-rs` implements a greedy draining strategy in the connection's writer loop:
- When a message is ready, the server pulls it from the internal channel.
- It then attempts to "drain" up to 128 more messages that are already waiting in the buffer.
- All these messages are coalesced into a single large memory buffer (`BytesMut`).
- A single `write_all` system call is made, significantly reducing CPU context switching.

## 3. Lock-Free Concurrency
- **Sharded State**: The global subscriber map is managed by `DashMap`, which uses fine-grained sharding instead of a single global lock. This allows 12+ cores to access the map simultaneously without contention.
- **Tokio Broadcast**: We utilize Tokio's native `broadcast` channels for the fan-out logic, which is highly optimized for MPMC (Multiple-Producer, Multiple-Consumer) scenarios.

## 4. Universal Collector
The `hpfeeds-collector` acts as a high-speed adapter. It uses an internal buffered pipeline to ensure that slow downstream databases (like Postgres or MongoDB) do not cause backpressure that stalls the broker.

### Data Flow
`Honeypot` -> `hpfeeds-server` -> `hpfeeds-collector (Batching)` -> `[Splunk / Kafka / DB]`
