use anyhow::{anyhow, Result};
use hpfeeds_core::{Frame, HpfeedsCodec, hashsecret};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use futures::SinkExt;
use futures::StreamExt;

use std::sync::Arc;
use rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use rustls::pki_types::{ServerName, CertificateDer};

pub type Transport<T> = Framed<T, HpfeedsCodec>;

/// Connects to `addr` and returns a framed transport using the hpfeeds codec.
pub async fn connect(addr: &str) -> Result<Transport<TcpStream>> {
    let stream = TcpStream::connect(addr).await?;
    let framed = Framed::new(stream, HpfeedsCodec::new());
    Ok(framed)
}

/// Connects and performs the hpfeeds handshake: reads OP_INFO and sends OP_AUTH.
pub async fn connect_and_auth(addr: &str, ident: &str, secret: &str) -> Result<Transport<TcpStream>> {
    let mut framed = connect(addr).await?;

    // read OP_INFO
    if let Some(Ok(Frame::Info { name: _, rand })) = framed.next().await {
        let sh = hashsecret(&rand, secret);
        framed.send(Frame::Auth { ident: ident.to_string().into(), secret_hash: sh.into() }).await?;
        Ok(framed)
    } else {
        Err(anyhow!("Expected OP_INFO from server"))
    }
}

/// Connects using TLS to `addr` and performs the handshake. `root_cert` should be DER-formatted certificate bytes of the CA/server to trust.
pub async fn connect_tls_and_auth(addr: &str, ident: &str, secret: &str, root_cert: &[u8]) -> Result<Transport<tokio_rustls::client::TlsStream<TcpStream>>> {
    // Build rustls client config with provided root
    let mut roots = RootCertStore::empty();
    let cert = CertificateDer::from(root_cert.to_vec());
    roots.add(cert).map_err(|_| anyhow!("invalid root cert"))?;
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    let stream = TcpStream::connect(addr).await?;
    // For tests, we expect the server name to be "localhost"; parse into ServerName
    let server_name = ServerName::try_from("localhost").map_err(|_| anyhow!("invalid dnsname"))?.to_owned();
    let tls_stream = connector.connect(server_name, stream).await?;

    let mut framed = Framed::new(tls_stream, HpfeedsCodec::new());

    // read OP_INFO
    if let Some(Ok(Frame::Info { name: _, rand })) = framed.next().await {
        let sh = hashsecret(&rand, secret);
        framed.send(Frame::Auth { ident: ident.to_string().into(), secret_hash: sh.into() }).await?;
        Ok(framed)
    } else {
        Err(anyhow!("Expected OP_INFO from server"))
    }
}
