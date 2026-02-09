use async_trait::async_trait;
use sqlx::{SqlitePool, Row};
use crate::auth::{Authenticator, AccessContext};
use anyhow::Result;
use tracing::info;

#[derive(Clone)]
pub struct SqliteAuthenticator {
    pool: SqlitePool,
}

impl SqliteAuthenticator {
    pub async fn new(db_path: &str) -> Result<Self> {
        let conn_str = format!("sqlite:{}", db_path);
        if !std::path::Path::new(db_path).exists() {
             std::fs::File::create(db_path)?;
        }
        let pool = SqlitePool::connect(&conn_str).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS users (ident TEXT PRIMARY KEY, secret TEXT NOT NULL);").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS permissions (id INTEGER PRIMARY KEY AUTOINCREMENT, ident TEXT NOT NULL, channel TEXT NOT NULL, can_pub BOOLEAN DEFAULT FALSE, can_sub BOOLEAN DEFAULT FALSE, FOREIGN KEY(ident) REFERENCES users(ident));").execute(&pool).await?;
        info!("Connected to SQLite database at {}", db_path);
        Ok(Self { pool })
    }

    #[allow(dead_code)]
    pub async fn add_user(&self, ident: &str, secret: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO users (ident, secret) VALUES (?, ?)")
            .bind(ident).bind(secret).execute(&self.pool).await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn add_permission(&self, ident: &str, channel: &str, can_pub: bool, can_sub: bool) -> Result<()> {
        sqlx::query("INSERT INTO permissions (ident, channel, can_pub, can_sub) VALUES (?, ?, ?, ?)")
            .bind(ident).bind(channel).bind(can_pub).bind(can_sub).execute(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl Authenticator for SqliteAuthenticator {
    async fn authenticate(&self, ident: &str, secret_hash: &[u8], rand: &[u8]) -> Option<AccessContext> {
        let row = sqlx::query("SELECT secret FROM users WHERE ident = ?").bind(ident).fetch_optional(&self.pool).await.ok()??;
        let secret: String = row.get("secret");
        let expected = hpfeeds_core::hashsecret(rand, &secret);
        if expected.as_slice() != secret_hash { return None; }
        let perms = sqlx::query("SELECT channel, can_pub, can_sub FROM permissions WHERE ident = ?").bind(ident).fetch_all(&self.pool).await.ok()?;
        let (mut pub_channels, mut sub_channels) = (Vec::new(), Vec::new());
        for p in perms {
            let channel: String = p.get("channel");
            if p.get::<bool, _>("can_pub") { pub_channels.push(channel.clone()); }
            if p.get::<bool, _>("can_sub") { sub_channels.push(channel); }
        }
        Some(AccessContext { ident: ident.to_string(), pub_channels, sub_channels })
    }
}