use hpfeeds_core::{Frame, MAXBUF};
use hpfeeds_client::connect_and_auth;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;
use futures::SinkExt;
use bytes::Bytes;

#[tokio::test]
async fn rejects_oversized_messages() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Spawn a dummy server that just performs the handshake
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(socket, hpfeeds_core::HpfeedsCodec::new());
        
        // Send OP_INFO
        let randbuf = vec![1u8, 2, 3, 4];
        framed.send(Frame::Info { name: "test".to_string(), rand: randbuf.into() }).await.unwrap();
        
        // We expect the connection to be closed or an error to occur when the client sends a huge message
        // We just keep the loop open to read
        while let Some(_) = futures::StreamExt::next(&mut framed).await {}
    });

    // Connect client
    let mut client = connect_and_auth(&addr.to_string(), "client", "secret").await?;

    // Create a payload that is slightly too large
    // len includes the 4-byte header? No, `len` in codec is the u32 read from the first 4 bytes.
    // The `len` field in hpfeeds covers the *entire* message (header + opcode + payload).
    // So if we make a payload of MAXBUF, the total len will be > MAX.
    let huge_payload = vec![0u8; MAXBUF + 100];
    
    // Attempt to send. The client codec might encode it, but the server should reject it.
    // Actually, if we use the same codec on client, the *client* might fail to encode/send if we enforced it there too?
    // Let's check the codec implementation. 
    // The Codec `decode` checks limit. `encode` does not explicitly check limit in my previous edit, 
    // but the `strpack8` checks 255 chars. The payload is just bytes.
    // So the client *will* send it. The server should close connection.
    
    let frame = Frame::Publish { 
        ident: "client".to_string(), 
        channel: "test".to_string(), 
        payload: huge_payload.into() 
    };

    // Sending might succeed because it just writes to socket buffer
    client.send(frame).await?;

    // But subsequent operations should fail because server closed connection
    let _result = client.send(Frame::Publish { 
        ident: "client".to_string(), 
        channel: "test".to_string(), 
        payload: Bytes::from_static(b"small") 
    }).await;

    // Give it a split second to fail
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // We expect the connection to be broken
    let next_read = futures::StreamExt::next(&mut client).await;
    match next_read {
        None => Ok(()), // Connection closed (EOF)
        Some(Err(_)) => Ok(()), // Connection reset
        Some(Ok(_)) => panic!("Connection should have closed!"),
    }
}
