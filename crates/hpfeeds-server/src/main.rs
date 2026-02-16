use clap::Parser;
use dashmap::DashMap;
use futures::StreamExt;
use hpfeeds_core::{Frame, HpfeedsCodec};
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_util::codec::Framed;
use tracing::info;

use anyhow::Result;
use prometheus::{Encoder, IntCounter, Opts, Registry};
use tokio_stream::wrappers::BroadcastStream;

mod auth;
use auth::{Authenticator, MemoryAuthenticator};
mod config;
mod db;
use bytes::{BufMut, Bytes, BytesMut};
use db::SqliteAuthenticator;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

#[derive(Parser, Debug)]
#[clap(name = "hpfeeds-server", about = "hpfeeds broker (Rust)")]
struct CliOpts {
    #[clap(long, default_value = "127.0.0.1")]
    host: String,
    #[clap(long, default_value_t = 10000)]
    port: u16,
    #[clap(long, default_value_t = 9431)]
    metrics_port: u16,
    #[clap(long = "auth")]
    auth: Vec<String>,
    #[clap(long)]
    config: Option<String>,
    #[clap(long)]
    db: Option<String>,
    #[clap(long)]
    json: bool,
    #[clap(long)]
    tls_cert: Option<String>,
    #[clap(long)]
    tls_key: Option<String>,
}

type SubscriberMap = Arc<DashMap<String, broadcast::Sender<Bytes>>>;
const CHANNEL_SIZE: usize = 65536;
const BATCH_LIMIT: usize = 128;

struct Metrics {
    registry: Registry,
    total_delivered: IntCounter,
    total_lagged: IntCounter,
    total_published: IntCounter,
    total_auth_success: IntCounter,
    total_auth_fail: IntCounter,
}

impl Metrics {
    fn new() -> Self {
        let registry = Registry::new();
        let total_delivered = IntCounter::with_opts(Opts::new(
            "hpfeeds_delivered_total",
            "Total messages successfully sent",
        ))
        .unwrap();
        let total_lagged = IntCounter::with_opts(Opts::new(
            "hpfeeds_lagged_total",
            "Total messages dropped due to lag",
        ))
        .unwrap();
        let total_published = IntCounter::with_opts(Opts::new(
            "hpfeeds_published_total",
            "Total messages received from publishers",
        ))
        .unwrap();
        let total_auth_success = IntCounter::with_opts(Opts::new(
            "hpfeeds_auth_success_total",
            "Total successful auths",
        ))
        .unwrap();
        let total_auth_fail =
            IntCounter::with_opts(Opts::new("hpfeeds_auth_fail_total", "Total failed auths"))
                .unwrap();
        registry
            .register(Box::new(total_delivered.clone()))
            .unwrap();
        registry.register(Box::new(total_lagged.clone())).unwrap();
        registry
            .register(Box::new(total_published.clone()))
            .unwrap();
        registry
            .register(Box::new(total_auth_success.clone()))
            .unwrap();
        registry
            .register(Box::new(total_auth_fail.clone()))
            .unwrap();
        Metrics {
            registry,
            total_delivered,
            total_lagged,
            total_published,
            total_auth_success,
            total_auth_fail,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = CliOpts::parse();
    if opts.json {
        tracing_subscriber::fmt().json().init();
    } else {
        tracing_subscriber::fmt::init();
    }

    let addr: SocketAddr = format!("{}:{}", opts.host, opts.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!("hpfeeds-server listening on {}", addr);

    let tls_acceptor = if let (Some(cert_path), Some(key_path)) = (&opts.tls_cert, &opts.tls_key) {
        // validate user-supplied paths to avoid path traversal / absolute path use
        if !is_safe_relative_path(cert_path) || !is_safe_relative_path(key_path) {
            eprintln!("Refusing to use absolute or parent-directory TLS paths");
            return Err(anyhow::anyhow!("unsafe TLS path"));
        }
        info!("TLS enabled with cert: {} and key: {}", cert_path, key_path);
        Some(Arc::new(load_tls_config(cert_path, key_path)?))
    } else {
        None
    };

    let subscribers: SubscriberMap = Arc::new(DashMap::new());
    let metrics = Arc::new(Metrics::new());

    let authenticator: Arc<dyn Authenticator> = if let Some(db_path) = &opts.db {
        Arc::new(SqliteAuthenticator::new(db_path).await?)
    } else {
        let mem_auth = Arc::new(MemoryAuthenticator::new());
        if let Some(config_path) = &opts.config {
            let cfg = config::load_config(config_path)?;
            for user in cfg.users {
                mem_auth
                    .add_user(
                        &user.ident,
                        &user.secret,
                        user.pub_channels,
                        user.sub_channels,
                    )
                    .await;
            }
        }
        for a in opts.auth.iter() {
            if let Some((ident, secret)) = a.split_once(':') {
                mem_auth.add(ident, secret).await;
            }
        }
        mem_auth
    };

    let metrics_registry = metrics.registry.clone();
    let metrics_addr = SocketAddr::from(([0, 0, 0, 0], opts.metrics_port));
    tokio::spawn(async move {
        let listener = TcpListener::bind(metrics_addr).await.unwrap();
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            let reg = metrics_registry.clone();
            tokio::task::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req: Request<hyper::body::Incoming>| {
                            let reg = reg.clone();
                            async move {
                                if req.uri().path() == "/metrics" {
                                    let mut buffer = vec![];
                                    prometheus::TextEncoder::new()
                                        .encode(&reg.gather(), &mut buffer)
                                        .unwrap();
                                    Ok::<_, anyhow::Error>(Response::new(Full::new(Bytes::from(
                                        buffer,
                                    ))))
                                } else {
                                    let mut res =
                                        Response::new(Full::new(Bytes::from("Not Found")));
                                    *res.status_mut() = StatusCode::NOT_FOUND;
                                    Ok(res)
                                }
                            }
                        }),
                    )
                    .await;
            });
        }
    });

    loop {
        let (socket, peer) = listener.accept().await?;
        let _ = socket.set_nodelay(true);
        let (subs, mets, auth, tls) = (
            subscribers.clone(),
            metrics.clone(),
            authenticator.clone(),
            tls_acceptor.clone(),
        );
        tokio::spawn(async move {
            if let Some(acceptor) = tls {
                if let Ok(stream) = acceptor.accept(socket).await {
                    handle_connection(stream, peer, subs, mets, auth).await;
                }
            } else {
                handle_connection(socket, peer, subs, mets, auth).await;
            }
        });
    }
}

fn load_tls_config(cert_path: &str, key_path: &str) -> Result<tokio_rustls::TlsAcceptor> {
    // Extra safety: check for path traversal or absolute paths
    if !is_safe_relative_path(cert_path) || !is_safe_relative_path(key_path) {
        return Err(anyhow::anyhow!(
            "Unsafe TLS file path: absolute or parent-directory component detected"
        ));
    }

    // Read and parse PEM-encoded certs
    // Prevent path traversal attacks by rejecting paths containing '..'
    let cert_path = std::path::Path::new(cert_path);
    if cert_path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(anyhow::anyhow!("Invalid input: {}", cert_path.display()));
    }
    let cert_data = std::fs::read_to_string(cert_path)?;
    let cert_pems = pem::parse_many(&cert_data)?;
    let cert_chain = cert_pems
        .into_iter()
        .filter(|p| p.tag() == "CERTIFICATE")
        .map(|p| rustls::pki_types::CertificateDer::from(p.contents().to_vec()))
        .collect::<Vec<_>>();
    if cert_chain.is_empty() {
        return Err(anyhow::anyhow!(
            "no certificates found in {}",
            cert_path.display()
        ));
    }

    // Read and parse PEM-encoded private key (support PKCS#8 / PKCS#1 / EC)
    // Prevent path traversal attacks by rejecting paths containing '..'
    let key_path = std::path::Path::new(key_path);
    if key_path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(anyhow::anyhow!("Invalid input: {}", key_path.display()));
    }
    let key_data = std::fs::read_to_string(key_path)?;
    let key_pems = pem::parse_many(&key_data)?;
    let key_pem = key_pems
        .into_iter()
        .find(|p| {
            let t = p.tag();
            t == "PRIVATE KEY" || t == "RSA PRIVATE KEY" || t == "EC PRIVATE KEY"
        })
        .ok_or_else(|| anyhow::anyhow!("no private key found"))?;
    let key = rustls::pki_types::PrivateKeyDer::try_from(key_pem.contents().to_vec())
        .map_err(|e| anyhow::anyhow!(e))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

/// Return false for absolute paths or any parent-directory (`..`) components.
fn is_safe_relative_path(p: &str) -> bool {
    let path = std::path::Path::new(p);
    if path.is_absolute() {
        return false;
    }
    for comp in path.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return false;
        }
    }
    true
}

async fn handle_connection<S>(
    stream: S,
    _peer: SocketAddr,
    subscribers: SubscriberMap,
    metrics: Arc<Metrics>,
    authenticator: Arc<dyn Authenticator>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut read_framed = Framed::new(reader, HpfeedsCodec::new());
    let mut codec = HpfeedsCodec::new();

    let mut randbuf = vec![0u8; 16];
    if let Ok(mut f) = File::open("/dev/urandom") {
        if f.read_exact(&mut randbuf).is_err() {
            return;
        }
    } else {
        return;
    }
    let info_bytes = codec
        .encode_to_bytes(Frame::Info {
            name: "hpfeeds-rs".to_string().into(),
            rand: randbuf.clone().into(),
        })
        .unwrap();
    if writer.write_all(&info_bytes).await.is_err() {
        return;
    }

    use auth::AccessContext;
    let access_ctx: AccessContext =
        if let Some(Ok(Frame::Auth { ident, secret_hash })) = read_framed.next().await {
            let ident_str = String::from_utf8_lossy(&ident);
            if let Some(ctx) = authenticator
                .authenticate(&ident_str, &secret_hash, &randbuf)
                .await
            {
                metrics.total_auth_success.inc();
                ctx
            } else {
                metrics.total_auth_fail.inc();
                return;
            }
        } else {
            return;
        };

    let mut write_buf = BytesMut::with_capacity(CHANNEL_SIZE);
    let mut stream_map = tokio_stream::StreamMap::new();

    loop {
        tokio::select! {
            Some((_chan, result)) = stream_map.next(), if !stream_map.is_empty() => {
                match result {
                    Ok(msg) => {
                        write_buf.put(msg);
                        metrics.total_delivered.inc();
                        let mut count = 1;
                        {
                            let waker = futures::task::noop_waker();
                            let mut cx = std::task::Context::from_waker(&waker);
                            while count < BATCH_LIMIT {
                                match stream_map.poll_next_unpin(&mut cx) {
                                    std::task::Poll::Ready(Some((_, Ok(next_msg)))) => {
                                        write_buf.put(next_msg);
                                        metrics.total_delivered.inc();
                                        count += 1;
                                    }
                                    _ => break,
                                }
                            }
                        }
                        if writer.write_all(&write_buf).await.is_err() { break; }
                        write_buf.clear();
                    }
                    Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                        metrics.total_lagged.inc_by(n);
                    }
                }
            }
            Some(Ok(frame)) = read_framed.next() => {
                match frame {
                    Frame::Subscribe { channel, .. } => {
                        let chan_str = String::from_utf8_lossy(&channel).to_string();
                        if access_ctx.can_subscribe(&chan_str) {
                            if stream_map.contains_key(&chan_str) { continue; }
                            let b_tx = subscribers.entry(chan_str.clone()).or_insert_with(|| broadcast::channel(CHANNEL_SIZE).0).value().clone();
                            stream_map.insert(chan_str, BroadcastStream::new(b_tx.subscribe()));
                        }
                    }
                    Frame::Unsubscribe { channel, .. } => {
                        stream_map.remove(String::from_utf8_lossy(&channel).as_ref());
                    }
                    Frame::Publish { channel, payload, .. } => {
                        let chan_str = String::from_utf8_lossy(&channel);
                        if access_ctx.can_publish(&chan_str) {
                            metrics.total_published.inc();
                            if let Some(b_tx) = subscribers.get(chan_str.as_ref()) {
                                let f = Frame::Publish { ident: access_ctx.ident.clone().into(), channel: channel.clone(), payload: payload.clone() };
                                if let Ok(b) = codec.encode_to_bytes(f) { let _ = b_tx.send(b); }
                            }
                        }
                    }
                    _ => {}
                }
            }
            else => { break; }
        }
    }
}
