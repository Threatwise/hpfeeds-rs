use hpfeeds_core::{Frame, HpfeedsCodec, hashsecret};
use hpfeeds_client::connect_and_auth;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;
use futures::{SinkExt, StreamExt};
use bytes::Bytes;

#[tokio::test]
async fn handshake_integration() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _peer) = listener.accept().await.expect("accept");
        let mut framed = Framed::new(socket, HpfeedsCodec::new());
        let randbuf = vec![9u8,8,7,6];
        framed.send(Frame::Info { name: Bytes::from_static(b"test-broker"), rand: randbuf.clone().into() }).await.expect("send info");
        if let Some(Ok(Frame::Auth { ident: _, secret_hash })) = framed.next().await {
            let expected = hashsecret(&randbuf, "s3cret");
            assert_eq!(secret_hash, expected);
            framed.send(Frame::Info { name: Bytes::from_static(b"ack"), rand: vec![].into() }).await.expect("send ack");
        } else {
            panic!("expected AUTH");
        }
    });

    let mut transport = connect_and_auth(&addr.to_string(), "client1", "s3cret").await?;

    if let Some(Ok(Frame::Info { name, .. })) = transport.next().await {
        assert_eq!(name, Bytes::from_static(b"ack"));
    } else {
        panic!("expected ack info");
    }

    Ok(())
}
