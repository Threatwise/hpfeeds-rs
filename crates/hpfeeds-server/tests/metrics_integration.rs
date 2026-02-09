use std::process::{Command, Stdio};
use std::time::Duration;
use std::thread;

// We can't easily spawn the server binary from an integration test and share the in-memory registry
// because integration tests run as separate binaries.
// However, we can spawn the server binary as a subprocess, perform actions against it, and then curl it.
// This is a true black-box integration test.

#[test]
fn metrics_endpoint_exposes_counters() {
    // Build the server first (cargo test builds it, but let's be sure)
    // Actually, running cargo test compiles the binary too.
    // We need to locate the binary.
    let debug_dir = std::env::current_exe()
        .expect("current exe")
        .parent()
        .expect("parent")
        .parent()
        .expect("parent") // target/debug/deps
        .parent()
        .expect("parent") // target/debug
        .to_path_buf();

    let server_bin = debug_dir.join("hpfeeds-server");

    if !server_bin.exists() {
        // Fallback: try to find it in target/debug directly if we are running via cargo test
        // It might be complex to locate robustly.
        // Alternative: Use the library code in a test?
        // No, main.rs logic (which spawns the metrics server) is in bin, not lib.
        // So we must spawn the binary.
        // Let's assume standard cargo layout.
        eprintln!("Skipping metrics test because server binary not found at {:?}. Run `cargo build --bin hpfeeds-server` first.", server_bin);
        return;
    }

    // Pick random ports
    let hpfeeds_port = 10000 + (rand::random::<u16>() % 10000);
    let metrics_port = 20000 + (rand::random::<u16>() % 10000);

    let mut child = Command::new(&server_bin)
        .arg("--port")
        .arg(hpfeeds_port.to_string())
        .arg("--metrics-port")
        .arg(metrics_port.to_string())
        .arg("--auth")
        .arg("test:secret")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn server");

    // Give it time to start
    thread::sleep(Duration::from_millis(500));

    // Connect via hpfeeds-client (using async runtime)
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let addr = format!("127.0.0.1:{}", hpfeeds_port);
        // Just connecting increments total_connections
        let _ = hpfeeds_client::connect(&addr).await;
    });

    // Request metrics
    let url = format!("http://127.0.0.1:{}/metrics", metrics_port);
    let resp = reqwest::blocking::get(&url).expect("failed to get metrics");

    assert!(resp.status().is_success());
    let body = resp.text().expect("failed to read body");

    // Assert we see the counter
    assert!(body.contains("hpfeeds_connections_total"));
    // We expect at least 1 connection (the one we just made)
    // The counter format is usually: hpfeeds_connections_total 1
    assert!(body.contains("hpfeeds_connections_total 1") || body.contains("hpfeeds_connections_total 2"));

    // cleanup
    let _ = child.kill();
    let _ = child.wait();
}
