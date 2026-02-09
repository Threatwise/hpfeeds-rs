use hpfeeds_core::Frame;
use hpfeeds_client::connect_and_auth;
use tokio::time::{timeout, Duration};
use futures::{SinkExt, StreamExt};

#[tokio::test]
async fn unsubscribe_and_multi_subscribers() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let subscribers = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::<String, std::collections::HashMap<usize, tokio::sync::mpsc::Sender<Frame>>>::new()));
        let next_conn = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
        loop {
            let (socket, _peer) = listener.accept().await.expect("accept");
            let subscribers = subscribers.clone();
            let next_conn = next_conn.clone();
            tokio::spawn(async move {
                let framed = tokio_util::codec::Framed::new(socket, hpfeeds_core::HpfeedsCodec::new());
                let (mut sink, mut stream) = framed.split();
                // send OP_INFO
                let randbuf = vec![1u8,2,3,4];
                sink.send(Frame::Info { name: "test-broker".to_string(), rand: randbuf.clone().into() }).await.expect("send info");
                // expect auth
                if let Some(Ok(Frame::Auth { ident: _, secret_hash })) = stream.next().await {
                    assert_eq!(secret_hash, hpfeeds_core::hashsecret(&randbuf, "s3cret"));
                } else {
                    return;
                }
                let conn_id = next_conn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Frame>(1024);
                // forward task
                let mut s = sink;
                tokio::spawn(async move {
                    while let Some(f) = rx.recv().await {
                        if s.send(f).await.is_err() {
                            break;
                        }
                    }
                });
                while let Some(Ok(frame)) = stream.next().await {
                    match frame {
                        Frame::Subscribe { ident: _, channel } => {
                            let mut m = subscribers.write().await;
                            m.entry(channel).or_insert_with(std::collections::HashMap::new).insert(conn_id, tx.clone());
                        }
                        Frame::Unsubscribe { ident: _, channel } => {
                            let mut m = subscribers.write().await;
                            if let Some(entry) = m.get_mut(&channel) {
                                entry.remove(&conn_id);
                                if entry.is_empty() {
                                    m.remove(&channel);
                                }
                            }
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

    // create two subscribers
    let mut sub1 = connect_and_auth(&addr.to_string(), "client1", "s3cret").await?;
    let mut sub2 = connect_and_auth(&addr.to_string(), "client2", "s3cret").await?;
    let mut pubc = connect_and_auth(&addr.to_string(), "client3", "s3cret").await?;

    sub1.send(Frame::Subscribe { ident: "client1".to_string(), channel: "chX".to_string() }).await?;
    sub2.send(Frame::Subscribe { ident: "client2".to_string(), channel: "chX".to_string() }).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;

    pubc.send(Frame::Publish { ident: "client3".to_string(), channel: "chX".to_string(), payload: b"one".to_vec().into() }).await?;

    // both subscribers should receive the message
    let mut got1 = false;
    let mut got2 = false;
    let _r = timeout(Duration::from_secs(1), async {
        for _ in 0..2 {
            if let Some(Ok(Frame::Publish { ident: _, channel, payload })) = sub1.next().await {
                if channel == "chX" && payload == b"one".to_vec() { got1 = true; }
            }
            if let Some(Ok(Frame::Publish { ident: _, channel, payload })) = sub2.next().await {
                if channel == "chX" && payload == b"one".to_vec() { got2 = true; }
            }
        }
    }).await;

    assert!(got1 && got2, "both should have received first publish");

    // unsubscribe sub2
    sub2.send(Frame::Unsubscribe { ident: "client2".to_string(), channel: "chX".to_string() }).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;

    pubc.send(Frame::Publish { ident: "client3".to_string(), channel: "chX".to_string(), payload: b"two".to_vec().into() }).await?;

    // sub1 should get it, sub2 should not
    let mut got1b = false;
    let mut got2b = false;
    let _r2 = timeout(Duration::from_secs(1), async {
        while let Some(msg) = sub1.next().await {
            if let Ok(Frame::Publish { ident: _, channel, payload }) = msg {
                if channel == "chX" && payload == b"two".to_vec() { got1b = true; break; }
            }
        }
    }).await;

    // sub2 should not receive
    let _r3 = timeout(Duration::from_millis(200), async {
        if let Some(msg) = sub2.next().await {
            if let Ok(Frame::Publish { .. }) = msg {
                got2b = true;
            }
        }
    }).await;

    assert!(got1b && !got2b, "sub1 should get second publish, sub2 should not");

    Ok(())
}
