use hpfeeds_core::Frame;
use hpfeeds_client::connect_tls_and_auth;
use futures::{SinkExt, StreamExt};

use rcgen::generate_simple_self_signed;
use rustls::Certificate;
use rustls::{ServerConfig, PrivateKey};
use tokio_rustls::TlsAcceptor;
use std::sync::Arc;

#[tokio::test]
async fn tls_handshake_and_auth() -> Result<(), Box<dyn std::error::Error>> {
    // generate self-signed cert for "localhost"
    let cert = generate_simple_self_signed(vec!["localhost".into()])?;
    // get cert and private key in DER form
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    let cert_chain = vec![Certificate(cert_der.clone())];
    let privkey = PrivateKey(key_der);

    let server_config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, privkey)?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    // bind listener
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // spawn server task
    tokio::spawn(async move {
        let (socket, _peer) = listener.accept().await.expect("accept");
        // upgrade to TLS
        let tls_stream = acceptor.accept(socket).await.expect("tls accept");
        let framed = tokio_util::codec::Framed::new(tls_stream, hpfeeds_core::HpfeedsCodec::new());
        let (mut sink, mut stream) = framed.split();
        // send OP_INFO with known rand
        let randbuf = vec![5u8,6,7,8];
        sink.send(Frame::Info { name: "tls-broker".to_string(), rand: randbuf.clone().into() }).await.expect("send info");
        // expect OP_AUTH
        if let Some(Ok(Frame::Auth { ident: _, secret_hash })) = stream.next().await {
            assert_eq!(secret_hash, hpfeeds_core::hashsecret(&randbuf, "s3cret"));
            sink.send(Frame::Info { name: "ack".to_string(), rand: vec![].into() }).await.expect("send ack");
        }
    });

    // client: connect using TLS and trust the generated cert
    let mut transport = connect_tls_and_auth(&addr.to_string(), "client1", "s3cret", &cert_der).await?;

    // after auth, expect ack info from server
    if let Some(Ok(Frame::Info { name, .. })) = transport.next().await {
        assert_eq!(name, "ack");
    } else {
        panic!("expected ack info");
    }

    Ok(())
}

