# Rust API (hpfeeds-client)

Use the `hpfeeds-client` library to integrate your own sensors or tools.

## Example

```rust
use hpfeeds_client::{connect_and_auth, Frame};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = connect_and_auth("127.0.0.1:10000", "ident", "secret").await?;
    
    // Publish
    client.send(Frame::Publish {
        ident: "ident".into(),
        channel: "malware".into(),
        payload: b"threat-data".to_vec().into(),
    }).await?;
    
    Ok(())
}
```
