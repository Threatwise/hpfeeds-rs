use hpfeeds_core::Frame;
use hpfeeds_client::connect;
use tokio::time::{timeout, Duration};
use futures::{SinkExt, StreamExt};
use bytes::Bytes;
use tokio_util::codec::Decoder;

#[tokio::test]
async fn unauth_subscribe_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _peer) = listener.accept().await.expect("accept");
        let framed = hpfeeds_core::HpfeedsCodec::new().framed(socket);
        let (mut sink, mut stream) = framed.split();
        let randbuf = vec![1u8,2,3,4];
        sink.send(Frame::Info { name: Bytes::from_static(b"test-broker"), rand: randbuf.clone().into() }).await.expect("send info");
        if let Some(Ok(Frame::Subscribe { .. })) = stream.next().await {
            sink.send(Frame::Error(Bytes::from_static(b"unauthorized"))).await.expect("send error");
        }
    });

    let mut raw = connect(&addr.to_string()).await?;

    if let Some(Ok(Frame::Info { .. })) = raw.next().await {
        raw.send(Frame::Subscribe { ident: Bytes::from_static(b"client1"), channel: Bytes::from_static(b"ch1") }).await?;

        let res = timeout(Duration::from_secs(1), async {
            while let Some(msg) = raw.next().await {
                if let Ok(Frame::Error(err)) = msg { return Ok(err); }
            }
            Err("no error")
        }).await?;

        let err = res?;
        assert_eq!(err, Bytes::from_static(b"unauthorized"));
    } else {
        panic!("no info");
    }

    Ok(())
}
