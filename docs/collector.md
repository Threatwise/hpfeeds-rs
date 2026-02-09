# Universal Collector

The `hpfeeds-collector` routes threat data from the broker to your downstream analytics stack.

## Supported Sinks

| Sink | Flag |
| :--- | :--- |
| **Redis** | `--output redis` |
| **PostgreSQL** | `--output postgres` |
| **Splunk HEC** | `--output splunk-hec` |
| **Kafka** | `--output kafka` |
| **STIX 2.1** | `--output stix` |

## Batching

The collector automatically buffers messages and flushes them in batches to improve performance.
You can tune this with:
- `--batch-size`: Max messages per batch (default 1000).
- `--flush-interval`: Max seconds to wait before flushing (default 5).
