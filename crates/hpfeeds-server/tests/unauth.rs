use hpfeeds_core::Frame;
use hpfeeds_client::connect;
use tokio::time::{timeout, Duration};
use futures::{SinkExt, StreamExt};

#[tokio::test]
async fn unauth_subscribe_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _peer) = listener.accept().await.expect("accept");
        let framed = tokio_util::codec::Framed::new(socket, hpfeeds_core::HpfeedsCodec::new());
        let (mut sink, mut stream) = framed.split();
        // send OP_INFO
        let randbuf = vec![1u8,2,3,4];
        sink.send(Frame::Info { name: "test-broker".to_string(), rand: randbuf.clone().into() }).await.expect("send info");
        // do NOT expect AUTH, instead read incoming
        if let Some(Ok(frame)) = stream.next().await {
            match frame {
                Frame::Subscribe { ident: _, channel: _ } => {
                    // should be rejected
                    sink.send(Frame::Error("unauthorized".to_string())).await.expect("send error");
                }
                _ => {
                    // ignore
                }
            }
        }
    });

    // client connects raw (no auth)
    let mut raw = connect(&addr.to_string()).await?;

    // read OP_INFO
    if let Some(Ok(Frame::Info { .. })) = raw.next().await {
        // send subscribe without authenticating
        raw.send(Frame::Subscribe { ident: "client1".to_string(), channel: "ch1".to_string() }).await?;

        // expect an error frame
        let res = timeout(Duration::from_secs(1), async {
            while let Some(msg) = raw.next().await {
                if let Ok(Frame::Error(err)) = msg {
                    return Ok(err);
                }
            }
            Err("no error")
        }).await?;

        let err = res?;
        assert_eq!(err, "unauthorized".to_string());
    } else {
        panic!("no info");
    }

    Ok(())
}
