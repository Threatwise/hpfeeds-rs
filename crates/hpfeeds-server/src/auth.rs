use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use hpfeeds_core::hashsecret;

/// Permissions for an authenticated user
#[derive(Debug, Clone, PartialEq)]
pub struct AccessContext {
    pub ident: String,
    pub pub_channels: Vec<String>,
    pub sub_channels: Vec<String>,
}

impl AccessContext {
    pub fn can_publish(&self, channel: &str) -> bool {
        self.pub_channels.iter().any(|c| c == channel || c == "*")
    }

    pub fn can_subscribe(&self, channel: &str) -> bool {
        self.sub_channels.iter().any(|c| c == channel || c == "*")
    }
}

/// Authenticator trait used by the server to verify client credentials.
#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, ident: &str, secret_hash: &[u8], rand: &[u8]) -> Option<AccessContext>;
}

struct UserData {
    secret: String,
    pub_channels: Vec<String>,
    sub_channels: Vec<String>,
}

/// In-memory authenticator which stores a map of ident -> UserData.
#[derive(Clone)]
pub struct MemoryAuthenticator {
    inner: Arc<RwLock<HashMap<String, UserData>>>,
}

impl MemoryAuthenticator {
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn add(&self, ident: &str, secret: &str) {
        // Default: allow all for backwards compat until we have config
        self.add_user(ident, secret, vec!["*".to_string()], vec!["*".to_string()]).await;
    }

    pub async fn add_user(&self, ident: &str, secret: &str, pub_channels: Vec<String>, sub_channels: Vec<String>) {
        let mut m = self.inner.write().await;
        m.insert(ident.to_string(), UserData {
            secret: secret.to_string(),
            pub_channels,
            sub_channels,
        });
    }
}

#[async_trait]
impl Authenticator for MemoryAuthenticator {
    async fn authenticate(&self, ident: &str, secret_hash: &[u8], rand: &[u8]) -> Option<AccessContext> {
        let m = self.inner.read().await;
        if let Some(user) = m.get(ident) {
            let expected = hashsecret(rand, &user.secret);
            if expected.as_slice() == secret_hash {
                return Some(AccessContext {
                    ident: ident.to_string(),
                    pub_channels: user.pub_channels.clone(),
                    sub_channels: user.sub_channels.clone(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_authenticator_validates() {
        let auth = MemoryAuthenticator::new();
        auth.add("u1", "secret1").await;

        // compute hash like client: sha1(rand + secret)
        let rand = b"rand";
        let secret_hash = hpfeeds_core::hashsecret(rand, "secret1");
        let ctx = auth.authenticate("u1", &secret_hash, rand).await;
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().ident, "u1");

        let bad = hpfeeds_core::hashsecret(rand, "wrong");
        let fail = auth.authenticate("u1", &bad, rand).await;
        assert!(fail.is_none());

        let missing = auth.authenticate("missing", &bad, rand).await;
        assert!(missing.is_none());
    }

    #[test]
    fn access_context_checks() {
        let ctx = AccessContext {
            ident: "u".into(),
            pub_channels: vec!["pub1".into()],
            sub_channels: vec!["sub1".into(), "*".into()],
        };
        assert!(ctx.can_publish("pub1"));
        assert!(!ctx.can_publish("pub2"));
        assert!(ctx.can_subscribe("any")); // because of *
    }
}
