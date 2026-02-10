#!/bin/bash
set -e

echo "Building hpfeeds-rs..."
cargo build --release

echo "Starting hpfeeds-server..."
# Use ephemeral auth for benchmark simplicity
./target/release/hpfeeds-server --auth "bench:benchsecret" --port 10001 --metrics-port 9432 > server.log 2>&1 &
SERVER_PID="$!"

# Wait for server to start
sleep 2

echo "Running sustained 60s benchmark..."
# All clients use the same 'bench' user which has wildcard access by default via --auth
./target/release/hpfeeds-bench --port 10001 --ident "bench" --secret "benchsecret" --subs 50 --pubs 5 --duration 60 --msgs 100000000 --payload-size 1024

echo "Stopping server..."
kill "${SERVER_PID}"
wait "${SERVER_PID}" 2>/dev/null || true

echo "Done. Results above."
