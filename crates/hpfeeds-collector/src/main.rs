use clap::Parser;
use hpfeeds_client::connect_and_auth;
use hpfeeds_core::Frame;
use anyhow::{Result, Context};
use futures::{StreamExt, SinkExt};
use serde::{Serialize, Deserialize};
use chrono::Utc;
use tokio::io::AsyncWriteExt;
use elasticsearch::{Elasticsearch, BulkParts, BulkIndexOperation, BulkOperations};
use mongodb::{Client as MongoClient, options::ClientOptions as MongoOptions};
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;
use rskafka::client::{ClientBuilder as KafkaClientBuilder, partition::{Compression, UnknownTopicHandling}};
use rskafka::record::Record;

#[derive(Parser, Debug)]
#[clap(name = "hpfeeds-collector", about = "Universal batteries-included collector for hpfeeds")]
struct Args {
    #[clap(long, default_value = "127.0.0.1")]
    host: String,
    #[clap(long, default_value_t = 10000)]
    port: u16,
    #[clap(long, short = 'i', required = true)]
    ident: String,
    #[clap(long, short = 's', required = true)]
    secret: String,
    #[clap(long, default_value = "bench")]
    channels: String,

    /// Output mode: file, console, redis, postgres, mongo, elastic, splunk-hec, stix, kafka, syslog, tcp
    #[clap(long, default_value = "console")]
    output: String,

    #[clap(long)]
    file_path: Option<String>,
    #[clap(long, default_value = "redis://127.0.0.1/")]
    redis_url: String,
    #[clap(long, default_value = "hpfeeds.events")]
    redis_channel: String,
    #[clap(long, default_value = "postgres://postgres:password@localhost/hpfeeds")]
    postgres_url: String,
    #[clap(long, default_value = "mongodb://localhost:27017")]
    mongo_url: String,
    #[clap(long, default_value = "http://localhost:9200")]
    elastic_url: String,
    #[clap(long, default_value = "http://localhost:8088/services/collector")]
    splunk_url: String,
    #[clap(long)]
    splunk_token: Option<String>,
    #[clap(long, default_value = "localhost:9092")]
    kafka_url: String,
    #[clap(long, default_value = "hpfeeds.events")]
    kafka_topic: String,
    #[clap(long, default_value = "127.0.0.1:514")]
    syslog_addr: String,
    #[clap(long, default_value = "127.0.0.1:9999")]
    tcp_addr: String,

    /// Batch size for flushes
    #[clap(long, default_value_t = 1000)]
    batch_size: usize,
    /// Max time to wait before flushing (seconds)
    #[clap(long, default_value_t = 5)]
    flush_interval: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Event {
    timestamp: chrono::DateTime<Utc>,
    channel: String,
    source: String,
    #[serde(with = "serde_bytes")]
    payload: Vec<u8>,
}

mod serde_bytes {
    use serde::{Serializer, Deserialize};
    pub fn serialize<S>(v: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        match std::str::from_utf8(v) {
            Ok(s) => serializer.serialize_str(s),
            Err(_) => serializer.serialize_str(&base64::encode(v)),
        }
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error> where D: serde::Deserializer<'de> {
        let s = String::deserialize(deserializer)?;
        Ok(base64::decode(&s).unwrap_or_else(|_| s.into_bytes()))
    }
}

fn to_stix_bundle(events: &[Event]) -> serde_json::Value {
    let bundle_id = format!("bundle--{}", Uuid::new_v4());
    let mut objects = Vec::new();
    for event in events {
        let observed_data_id = format!("observed-data--{}", Uuid::new_v4());
        objects.push(serde_json::json!({
            "type": "observed-data", "id": observed_data_id, "spec_version": "2.1",
            "first_observed": event.timestamp.to_rfc3339(), "last_observed": event.timestamp.to_rfc3339(),
            "number_observed": 1, "external_references": [{"source_name": "hpfeeds", "external_id": event.source}],
            "x_hpfeeds_channel": event.channel, "x_hpfeeds_payload": base64::encode(&event.payload)
        }));
        objects.push(serde_json::json!({
            "type": "sighting", "id": format!("sighting--{}", Uuid::new_v4()), "spec_version": "2.1",
            "sighting_of_ref": observed_data_id, "last_seen": event.timestamp.to_rfc3339(), "count": 1
        }));
    }
    serde_json::json!({"type": "bundle", "id": bundle_id, "objects": objects})
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr = format!("{}:{}", args.host, args.port);

    let mut client = connect_and_auth(&addr, &args.ident, &args.secret).await?;
    println!("Collector connected to broker at {}", addr);

    for channel in args.channels.split(',') {
        client.send(Frame::Subscribe { 
            ident: args.ident.clone().into(), 
            channel: channel.trim().to_string().into() 
        }).await?;
    }

    // Initialize Sinks
    let mut file_sink = if args.output == "file" || args.output == "stix" {
        let p = args.file_path.as_ref().context("--file-path required")?;
        Some(tokio::fs::OpenOptions::new().create(true).append(true).open(p).await?)
    } else { None };

    let mut redis_conn = if args.output == "redis" {
        Some(redis::Client::open(args.redis_url.clone())?.get_async_connection().await?)
    } else { None };

    let pg_pool = if args.output == "postgres" {
        let pool = PgPoolOptions::new().max_connections(5).connect(&args.postgres_url).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS events (id SERIAL PRIMARY KEY, ts TIMESTAMPTZ, channel TEXT, source TEXT, payload BYTEA)").execute(&pool).await?;
        Some(pool)
    } else { None };

    let mongo_coll = if args.output == "mongo" {
        let c = MongoClient::with_options(MongoOptions::parse(&args.mongo_url).await?)?;
        Some(c.database("hpfeeds").collection::<Event>("events"))
    } else { None };

    let es_client = if args.output == "elastic" {
        Some(Elasticsearch::new(elasticsearch::http::transport::Transport::single_node(&args.elastic_url)?))
    } else { None };

    let kafka_producer = if args.output == "kafka" {
        let client = KafkaClientBuilder::new(vec![args.kafka_url.clone()]).build().await?;
        let partition_client = client.partition_client(args.kafka_topic.clone(), 0, UnknownTopicHandling::Retry).await?;
        Some(partition_client)
    } else { None };

    let syslog_socket = if args.output == "syslog" {
        Some(tokio::net::UdpSocket::bind("0.0.0.0:0").await?)
    } else { None };

    let mut tcp_stream = if args.output == "tcp" {
        Some(tokio::net::TcpStream::connect(&args.tcp_addr).await?)
    } else { None };

    let http_client = reqwest::Client::new();
    let mut buffer: Vec<Event> = Vec::with_capacity(args.batch_size);
    let mut last_flush = Instant::now();

    println!("Starting collection loop using output mode: {}", args.output);
    while let Some(msg) = client.next().await {
        if let Ok(Frame::Publish { ident, channel, payload }) = msg {
            buffer.push(Event {
                timestamp: Utc::now(),
                channel: String::from_utf8_lossy(&channel).to_string(),
                source: String::from_utf8_lossy(&ident).to_string(),
                payload: payload.to_vec(),
            });
        }

        if buffer.len() >= args.batch_size || (last_flush.elapsed() >= Duration::from_secs(args.flush_interval) && !buffer.is_empty()) {
            match args.output.as_str() {
                "console" => { for e in &buffer { println!("{}", serde_json::to_string(e)?); } }
                "file" => {
                    if let Some(f) = file_sink.as_mut() {
                        let mut d = String::new();
                        for e in &buffer { d.push_str(&serde_json::to_string(e)?); d.push('\n'); }
                        f.write_all(d.as_bytes()).await?;
                    }
                }
                "stix" => {
                    if let Some(f) = file_sink.as_mut() {
                        let bundle = to_stix_bundle(&buffer);
                        f.write_all(serde_json::to_string_pretty(&bundle)?.as_bytes()).await?;
                        f.write_all(b"\n").await?;
                    }
                }
                "redis" => {
                    if let Some(conn) = redis_conn.as_mut() {
                        for e in &buffer { let _: () = redis::AsyncCommands::publish(conn, &args.redis_channel, serde_json::to_string(e)?).await?; }
                    }
                }
                "postgres" => {
                    if let Some(pool) = &pg_pool {
                        for e in &buffer {
                            sqlx::query("INSERT INTO events (ts, channel, source, payload) VALUES ($1, $2, $3, $4)")
                                .bind(e.timestamp).bind(&e.channel).bind(&e.source).bind(&e.payload).execute(pool).await?;
                        }
                    }
                }
                "mongo" => { if let Some(coll) = &mongo_coll { coll.insert_many(&buffer).await?; } }
                "elastic" => {
                    if let Some(es) = &es_client {
                        let mut ops = BulkOperations::new();
                        for e in &buffer { ops.push(BulkIndexOperation::new(e.clone())).unwrap(); }
                        es.bulk(BulkParts::Index("hpfeeds-events")).body(vec![ops]).send().await?;
                    }
                }
                "kafka" => {
                    if let Some(p) = &kafka_producer {
                        let records: Vec<Record> = buffer.iter().map(|e| Record {
                            key: Some(e.channel.as_bytes().to_vec()),
                            value: Some(serde_json::to_vec(e).unwrap()),
                            timestamp: rskafka::chrono::Utc::now(),
                            headers: Default::default(),
                        }).collect();
                        p.produce(records, Compression::NoCompression).await?;
                    }
                }
                "syslog" => {
                    if let Some(s) = &syslog_socket {
                        for e in &buffer {
                            let msg = format!("<134>1 {} {} hpfeeds - - - {}", e.timestamp.to_rfc3339(), e.source, serde_json::to_string(e)?);
                            s.send_to(msg.as_bytes(), &args.syslog_addr).await?;
                        }
                    }
                }
                "tcp" => {
                    if let Some(s) = tcp_stream.as_mut() {
                        let mut d = String::new();
                        for e in &buffer { d.push_str(&serde_json::to_string(e)?); d.push('\n'); }
                        s.write_all(d.as_bytes()).await?;
                    }
                }
                "splunk-hec" => {
                    let token = args.splunk_token.as_ref().context("--splunk-token required")?;
                    let mut b = String::new();
                    for e in &buffer { b.push_str(&serde_json::json!({"time": e.timestamp.timestamp(), "event": e, "sourcetype": "_json"}).to_string()); b.push('\n'); }
                    http_client.post(&args.splunk_url).header("Authorization", format!("Splunk {}", token)).body(b).send().await?;
                }
                _ => {}
            }
            buffer.clear();
            last_flush = Instant::now();
        }
    }
    Ok(())
}
