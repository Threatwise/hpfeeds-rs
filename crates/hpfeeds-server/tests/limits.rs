use bytes::Bytes;
use futures::SinkExt;
use hpfeeds_client::connect_and_auth;
use hpfeeds_core::{Frame, MAXBUF};
use tokio::net::TcpListener;
use tokio_util::codec::Framed;

#[tokio::test]
async fn rejects_oversized_messages() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(socket, hpfeeds_core::HpfeedsCodec::new());
        let randbuf = vec![1u8, 2, 3, 4];
        framed
            .send(Frame::Info {
                name: Bytes::from_static(b"test"),
                rand: randbuf.into(),
            })
            .await
            .unwrap();
        while futures::StreamExt::next(&mut framed).await.is_some() {}
    });

    let mut client = connect_and_auth(&addr.to_string(), "client", "secret").await?;
    let huge_payload = vec![0u8; MAXBUF + 100];

    let frame = Frame::Publish {
        ident: Bytes::from_static(b"client"),
        channel: Bytes::from_static(b"test"),
        payload: huge_payload.into(),
    };

    client.send(frame).await?;

    let _result = client
        .send(Frame::Publish {
            ident: Bytes::from_static(b"client"),
            channel: Bytes::from_static(b"test"),
            payload: Bytes::from_static(b"small"),
        })
        .await;

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let next_read = futures::StreamExt::next(&mut client).await;
    match next_read {
        None => Ok(()),
        Some(Err(_)) => Ok(()),
        Some(Ok(_)) => panic!("Connection should have closed!"),
    }
}
