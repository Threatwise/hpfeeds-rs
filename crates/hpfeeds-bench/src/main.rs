use clap::Parser;
use hpfeeds_client::connect_and_auth;
use hpfeeds_core::Frame;
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::Barrier;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Parser, Debug)]
#[clap(name = "hpfeeds-bench", about = "Benchmarking tool for hpfeeds")]
struct Args {
    /// Broker host
    #[clap(long, default_value = "127.0.0.1")]
    host: String,

    /// Broker port
    #[clap(long, default_value_t = 10000)]
    port: u16,

    /// Number of subscribers
    #[clap(long, default_value_t = 10)]
    subs: usize,

    /// Number of publishers
    #[clap(long, default_value_t = 1)]
    pubs: usize,

    /// Messages per publisher (ignored if duration is set)
    #[clap(long, default_value_t = 1000)]
    msgs: usize,

    /// Duration to run the test in seconds (overrides msgs)
    #[clap(long)]
    duration: Option<u64>,

    /// Payload size in bytes
    #[clap(long, default_value_t = 1024)]
    payload_size: usize,

    /// Identity prefix
    #[clap(long, default_value = "bench")]
    ident: String,

    /// Secret
    #[clap(long, default_value = "benchsecret")]
    secret: String,

    /// Channel to use
    #[clap(long, default_value = "bench")]
    channel: String,

    /// Path to server SQLite DB to seed users (optional)
    #[clap(long)]
    db: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr = format!("{}:{}", args.host, args.port);

    if let Some(db_path) = &args.db {
        println!("Seeding database {}...", db_path);
        use tokio_rusqlite::{Connection, rusqlite};
        let conn = Connection::open(db_path).await?;

        // Seed sub users
        for i in 0..args.subs {
            let ident = format!("{}-sub-{}", args.ident, i);
            let secret = args.secret.clone();
            let channel = args.channel.clone();
            let ident_clone = ident.clone();
            conn.call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO users (ident, secret) VALUES (?, ?)",
                    [&ident, &secret],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO permissions (ident, channel, can_pub, can_sub) VALUES (?, ?, 0, 1)",
                    [&ident_clone, &channel],
                )?;
                Ok::<(), rusqlite::Error>(())
            }).await?;
        }
        // Seed pub users
        for i in 0..args.pubs {
            let ident = format!("{}-pub-{}", args.ident, i);
            let secret = args.secret.clone();
            let channel = args.channel.clone();
            let ident_clone = ident.clone();
            conn.call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO users (ident, secret) VALUES (?, ?)",
                    [&ident, &secret],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO permissions (ident, channel, can_pub, can_sub) VALUES (?, ?, 1, 0)",
                    [&ident_clone, &channel],
                )?;
                Ok::<(), rusqlite::Error>(())
            }).await?;
        }
        println!("Database seeded.");
    }

    println!("Starting benchmark with {} subs, {} pubs, {} msgs/pub, payload {} bytes",
             args.subs, args.pubs, args.msgs, args.payload_size);

    let total_expected = (args.pubs * args.msgs * args.subs) as u64;
    let received_count = Arc::new(AtomicU64::new(0));
    let start_barrier = Arc::new(Barrier::new(args.subs + args.pubs + 1));

    // Spawn subscribers
    for i in 0..args.subs {
        let addr = addr.clone();
        let ident = args.ident.clone(); // Use same ident
        let secret = args.secret.clone();
        let channel = args.channel.clone();
        let counter = received_count.clone();
        let barrier = start_barrier.clone();

        tokio::spawn(async move {
            let mut client = match connect_and_auth(&addr, &ident, &secret).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Sub {} connect failed: {}", i, e);
                    barrier.wait().await;
                    return;
                }
            };
            if let Err(e) = client.send(Frame::Subscribe {
                ident: ident.clone().into(),
                channel: channel.into()
            }).await {
                eprintln!("Sub {} subscribe failed: {}", i, e);
                barrier.wait().await;
                return;
            }

            barrier.wait().await;

            while let Some(msg) = client.next().await {
                if let Ok(Frame::Publish { .. }) = msg {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }

    // Give subscribers time to connect before starting publishers spawning
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Spawn publishers
    let payload = vec![0u8; args.payload_size];
    let payload = bytes::Bytes::from(payload);
    let run_duration = args.duration.map(Duration::from_secs);

    for i in 0..args.pubs {
        let addr = addr.clone();
        let ident = args.ident.clone(); // Use same ident
        let secret = args.secret.clone();
        let channel = args.channel.clone();
        let msgs = args.msgs;
        let barrier = start_barrier.clone();
        let p = payload.clone();

        tokio::spawn(async move {
            let mut client = match connect_and_auth(&addr, &ident, &secret).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Pub {} connect failed: {}", i, e);
                    barrier.wait().await;
                    return;
                }
            };

            barrier.wait().await;

            let start = Instant::now();
            let mut count = 0usize;
            loop {
                // Check termination condition
                if let Some(d) = run_duration {
                    if start.elapsed() >= d { break; }
                } else if count >= msgs {
                    break;
                }

                // Tight loop for publishing
                if let Err(e) = client.send(Frame::Publish {
                    ident: ident.clone().into(),
                    channel: channel.clone().into(),
                    payload: p.clone()
                }).await {
                    eprintln!("Pub {} failed: {}", i, e);
                    break;
                }
                count += 1;
            }
        });
    }

    // Wait for all to be ready
    println!("Waiting for all clients to connect...");
    start_barrier.wait().await;
    let start_time = Instant::now();
    println!("Benchmark started.");

    let mut last_report = Instant::now();
    let mut last_count = 0u64;
    let test_limit = run_duration.unwrap_or(Duration::from_secs(3600)); // Default 1h if msgs mode

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let current_count = received_count.load(Ordering::Relaxed);
        let elapsed = last_report.elapsed().as_secs_f64();
        let delta = current_count - last_count;
        println!("Received: {} ({:.2} msg/s)", current_count, delta as f64 / elapsed);

        last_report = Instant::now();
        last_count = current_count;

        if start_time.elapsed() >= test_limit {
            println!("Benchmark finished after duration.");
            break;
        }

        if run_duration.is_none() && current_count >= total_expected {
            println!("Benchmark finished after msg count.");
            break;
        }
    }

    let total_elapsed = start_time.elapsed();
    let final_count = received_count.load(Ordering::Relaxed);

    println!("--- Benchmark Results ---");
    println!("Total Messages Received: {}", final_count);
    println!("Total Time: {:.2?}", total_elapsed);
    println!("Throughput: {:.2} msg/s", final_count as f64 / total_elapsed.as_secs_f64());
    println!("Data Rate: {:.2} MB/s", (final_count * args.payload_size as u64) as f64 / (1024.0 * 1024.0 * total_elapsed.as_secs_f64()));

    Ok(())
}