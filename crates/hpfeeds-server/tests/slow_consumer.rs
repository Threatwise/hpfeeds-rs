use hpfeeds_core::Frame;
use hpfeeds_client::connect_and_auth;
use tokio::time::Duration;
use futures::{SinkExt, StreamExt};
use prometheus::{IntCounter, Opts, Registry};
use std::sync::{Arc, Mutex};
use tracing::warn;
use bytes::Bytes;

struct TestWriter(Arc<Mutex<Vec<String>>>);
impl std::io::Write for TestWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let mut v = self.0.lock().unwrap();
            v.push(s.to_string());
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[tokio::test]
async fn slow_consumer_drops_messages_and_logs_warning() -> Result<(), Box<dyn std::error::Error>> {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone_for_tracing = logs.clone();
    let tracing_init_ok = tracing_subscriber::fmt().with_writer(move || TestWriter(logs_clone_for_tracing.clone())).try_init().is_ok();

    let dropped = IntCounter::with_opts(Opts::new("hpfeeds_dropped_total", "Total messages dropped due to slow consumers")).unwrap();
    let registry = Registry::new();
    registry.register(Box::new(dropped.clone())).unwrap();

    {
        let (tx_test, _rx_test) = tokio::sync::mpsc::channel::<u8>(1);
        assert!(tx_test.try_send(1u8).is_ok());
        match tx_test.try_send(2u8) {
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
            other => panic!("expected Full, got {:?}", other),
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let subscribers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let dropped_clone = dropped.clone();
    let subs_for_accept = subscribers.clone();
    tokio::spawn(async move {
        loop {
            let (socket, _peer) = listener.accept().await.expect("accept");
            let dropped_clone = dropped_clone.clone();
            let subs = subs_for_accept.clone();
            tokio::spawn(async move {
                let framed = tokio_util::codec::Framed::new(socket, hpfeeds_core::HpfeedsCodec::new());
                let (mut sink, mut stream) = framed.split();
                let randbuf = vec![9u8,9,9,9];
                sink.send(Frame::Info { name: Bytes::from_static(b"slow-broker"), rand: randbuf.clone().into() }).await.expect("send info");
                if let Some(Ok(Frame::Auth { ident: _, secret_hash })) = stream.next().await {
                    assert_eq!(secret_hash, hpfeeds_core::hashsecret(&randbuf, "s3cret"));
                } else {
                    return;
                }

                let (tx, _rx) = tokio::sync::mpsc::channel::<Frame>(1);

                while let Some(Ok(frame)) = stream.next().await {
                    match frame {
                        Frame::Subscribe { ident: _, channel: _ } => {
                            subs.lock().unwrap().push(tx.clone());
                        }
                        Frame::Publish { ident: _, channel: _, payload } => {
                            let f = Frame::Publish { ident: Bytes::from_static(b"pub"), channel: Bytes::from_static(b"ch"), payload };
                            let mut to_remove = Vec::new();
                            for (i, s) in subs.lock().unwrap().iter().enumerate() {
                                match s.try_send(f.clone()) {
                                    Ok(()) => {}
                                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                        dropped_clone.inc();
                                        warn!("dropping message for slow subscriber {}", i);
                                    }
                                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                        to_remove.push(i);
                                    }
                                }
                            }
                            if !to_remove.is_empty() {
                                let mut guard = subs.lock().unwrap();
                                for idx in to_remove.into_iter().rev() {
                                    guard.remove(idx);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    });

    let mut sub = connect_and_auth(&addr.to_string(), "client1", "s3cret").await?;
    sub.send(Frame::Subscribe { ident: Bytes::from_static(b"client1"), channel: Bytes::from_static(b"ch") }).await?;

    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut pubc = connect_and_auth(&addr.to_string(), "client1", "s3cret").await?;
    for _ in 0..10 {
        pubc.send(Frame::Publish { ident: Bytes::from_static(b"pub"), channel: Bytes::from_static(b"ch"), payload: Bytes::from_static(b"\x01\x02\x03") }).await?;
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    let dropped_val = dropped.get();
    assert!(dropped_val > 0, "expected dropped messages, got {}", dropped_val);

    if tracing_init_ok {
        let logs_guard = logs.lock().unwrap();
        let found = logs_guard.iter().any(|s| s.contains("dropping message"));
        assert!(found, "expected warning log about dropping messages, logs: {:?}", *logs_guard);
    }

    Ok(())
}