use hpfeeds_core::{Frame, HpfeedsCodec};
use tokio::net::TcpListener;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use tokio_util::codec::Framed;
use futures::{SinkExt, StreamExt};
use bytes::{BufMut, Bytes};

#[tokio::test]
async fn rejects_invalid_opcode() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(socket, HpfeedsCodec::new());
        let randbuf = vec![1u8, 2, 3, 4];
        framed.send(Frame::Info { name: Bytes::from_static(b"test"), rand: randbuf.into() }).await.unwrap();
        while framed.next().await.is_some() {}
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let mut buf = vec![0u8; 1024];
    let _n = stream.peek(&mut buf).await?;

    let mut bad_frame = bytes::BytesMut::new();
    bad_frame.put_u32(5);
    bad_frame.put_u8(255);

    stream.write_all(&bad_frame).await?;

    let mut read_buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut read_buf).await?;
        if n == 0 { break; }
    }
    Ok(())
}

#[tokio::test]
async fn rejects_malformed_string_length() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(socket, HpfeedsCodec::new());
        let randbuf = vec![1u8, 2, 3, 4];
        framed.send(Frame::Info { name: Bytes::from_static(b"test"), rand: randbuf.into() }).await.unwrap();
        while framed.next().await.is_some() {}
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let mut bad_frame = bytes::BytesMut::new();
    bad_frame.put_u32(7);
    bad_frame.put_u8(2);
    bad_frame.put_u8(200);
    bad_frame.put_u8(65);

    stream.write_all(&bad_frame).await?;

    let mut read_buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut read_buf).await?;
        if n == 0 { break; }
    }
    Ok(())
}
