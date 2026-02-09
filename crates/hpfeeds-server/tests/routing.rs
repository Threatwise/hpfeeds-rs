use hpfeeds_core::Frame;
use hpfeeds_client::connect_and_auth;
use tokio::time::{timeout, Duration};
use futures::{SinkExt, StreamExt};
use bytes::Bytes;

#[tokio::test]
async fn routing_publish_to_subscriber() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let subscribers = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::<Bytes, std::collections::HashMap<usize, tokio::sync::mpsc::Sender<Frame>>>::new()));
        let next_conn = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
        loop {
            let (socket, _peer) = listener.accept().await.expect("accept");
            let subscribers = subscribers.clone();
            let next_conn = next_conn.clone();
            tokio::spawn(async move {
                let framed = tokio_util::codec::Framed::new(socket, hpfeeds_core::HpfeedsCodec::new());
                let (mut sink, mut stream) = framed.split();
                let randbuf = vec![1u8,2,3,4];
                sink.send(Frame::Info { name: Bytes::from_static(b"test-broker"), rand: randbuf.clone().into() }).await.expect("send info");
                if let Some(Ok(Frame::Auth { ident: _, secret_hash })) = stream.next().await {
                    assert_eq!(secret_hash, hpfeeds_core::hashsecret(&randbuf, "s3cret"));
                } else {
                    return;
                }
                let conn_id = next_conn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Frame>(1024);
                let mut s = sink;
                tokio::spawn(async move {
                    while let Some(f) = rx.recv().await {
                        if s.send(f).await.is_err() { break; }
                    }
                });
                while let Some(Ok(frame)) = stream.next().await {
                    match frame {
                        Frame::Subscribe { channel, .. } => {
                            let mut m = subscribers.write().await;
                            m.entry(channel).or_insert_with(std::collections::HashMap::new).insert(conn_id, tx.clone());
                        }
                        Frame::Publish { ident, channel, payload } => {
                            let m = subscribers.read().await;
                            if let Some(entry) = m.get(&channel) {
                                for (_cid, sender) in entry.iter() {
                                    let f = Frame::Publish { ident: ident.clone(), channel: channel.clone(), payload: payload.clone() };
                                    let _ = sender.try_send(f);
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
    let mut pubc = connect_and_auth(&addr.to_string(), "client1", "s3cret").await?;

    sub.send(Frame::Subscribe { ident: Bytes::from_static(b"client1"), channel: Bytes::from_static(b"ch1") }).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;

    pubc.send(Frame::Publish { ident: Bytes::from_static(b"client1"), channel: Bytes::from_static(b"ch1"), payload: Bytes::from_static(b"hello") }).await?;

    let res = timeout(Duration::from_secs(1), async {
        while let Some(msg) = sub.next().await {
            if let Ok(Frame::Publish { channel, payload, .. }) = msg {
                if channel == Bytes::from_static(b"ch1") && payload == Bytes::from_static(b"hello") {
                    return Ok(());
                }
            }
        }
        Err("no message")
    }).await?;

    res.map_err(Box::<dyn std::error::Error>::from)
}
