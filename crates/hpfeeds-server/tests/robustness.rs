use hpfeeds_core::{Frame, HpfeedsCodec};
use tokio::net::TcpListener;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use tokio_util::codec::Framed;
use futures::{SinkExt, StreamExt};
use bytes::BufMut;

#[tokio::test]
async fn rejects_invalid_opcode() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(socket, HpfeedsCodec::new());
        // Send OP_INFO
        let randbuf = vec![1u8, 2, 3, 4];
        framed.send(Frame::Info { name: "test".to_string(), rand: randbuf.into() }).await.unwrap();
        // Keep reading until error
        while let Some(_) = framed.next().await {}
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    
    // Read OP_INFO manually to clear buffer
    let mut buf = vec![0u8; 1024];
    let n = stream.peek(&mut buf).await?; // just peek to know it's there
    // Actually we should perform handshake properly or just send raw bytes
    // Let's just send invalid opcode immediately after connect (server sends INFO, we send bad bytes)
    
    // Construct invalid frame: len=5, opcode=255
    let mut bad_frame = bytes::BytesMut::new();
    bad_frame.put_u32(5); // len: 4 bytes len + 1 byte op
    bad_frame.put_u8(255); // invalid opcode

    stream.write_all(&bad_frame).await?;

    // The server should close the connection or send an error
    // Let's try to read from stream. It should eventually return EOF (0 bytes)
    let mut read_buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut read_buf).await?;
        if n == 0 {
            // EOF, connection closed
            break;
        }
        // If we get data (like OP_INFO), ignore it
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
        framed.send(Frame::Info { name: "test".to_string(), rand: randbuf.into() }).await.unwrap();
        while let Some(_) = framed.next().await {}
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await?;

    // Construct frame with invalid string length
    // OP_AUTH: len(ident) = 200, but we only provide 1 byte
    let mut bad_frame = bytes::BytesMut::new();
    // length = 4 (header) + 1 (op) + 1 (str len) + 1 (str data) = 7
    bad_frame.put_u32(7);
    bad_frame.put_u8(2); // OP_AUTH
    bad_frame.put_u8(200); // Claims string is 200 bytes
    bad_frame.put_u8(65);  // Only 1 byte ('A')

    stream.write_all(&bad_frame).await?;

    // Server should close connection due to "string buffer too short"
    let mut read_buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut read_buf).await?;
        if n == 0 {
            break;
        }
    }

    Ok(())
}
