use crate::auth::{AccessContext, Authenticator};
use anyhow::Result;
use async_trait::async_trait;
use tokio_rusqlite::{rusqlite, Connection};
use tracing::info;

#[derive(Clone)]
pub struct SqliteAuthenticator {
    conn: Connection,
}

impl SqliteAuthenticator {
    pub async fn new(db_path: &str) -> Result<Self> {
        // Prevent path traversal attacks by rejecting paths containing '..'
        let path = std::path::Path::new(db_path);
        if path.components().any(|c| c == std::path::Component::ParentDir) {
            return Err(anyhow::anyhow!("Invalid input: {}", path.display()));
        }
        if !path.exists() {
            std::fs::File::create(path)?;
        }

        let conn = Connection::open(db_path).await?;

        conn.call(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS users (ident TEXT PRIMARY KEY, secret TEXT NOT NULL)",
                [],
            )?;
            conn.execute(
                "CREATE TABLE IF NOT EXISTS permissions (id INTEGER PRIMARY KEY AUTOINCREMENT, ident TEXT NOT NULL, channel TEXT NOT NULL, can_pub BOOLEAN DEFAULT FALSE, can_sub BOOLEAN DEFAULT FALSE, FOREIGN KEY(ident) REFERENCES users(ident))",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        }).await?;

        info!("Connected to SQLite database at {}", db_path);
        Ok(Self { conn })
    }

    #[allow(dead_code)]
    pub async fn add_user(&self, ident: &str, secret: &str) -> Result<()> {
        let ident = ident.to_string();
        let secret = secret.to_string();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO users (ident, secret) VALUES (?, ?)",
                    [&ident, &secret],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn add_permission(
        &self,
        ident: &str,
        channel: &str,
        can_pub: bool,
        can_sub: bool,
    ) -> Result<()> {
        let ident = ident.to_string();
        let channel = channel.to_string();
        self.conn
            .call(move |conn| {
                conn.execute(
                "INSERT INTO permissions (ident, channel, can_pub, can_sub) VALUES (?, ?, ?, ?)",
                rusqlite::params![&ident, &channel, can_pub, can_sub],
            )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Authenticator for SqliteAuthenticator {
    async fn authenticate(
        &self,
        ident: &str,
        secret_hash: &[u8],
        rand: &[u8],
    ) -> Option<AccessContext> {
        let ident = ident.to_string();
        let secret_hash = secret_hash.to_vec();
        let rand = rand.to_vec();

        self.conn
            .call(move |conn| {
                let secret: String = match conn.query_row(
                    "SELECT secret FROM users WHERE ident = ?",
                    [&ident],
                    |row| row.get(0),
                ) {
                    Ok(s) => s,
                    Err(_) => return Ok::<Option<AccessContext>, rusqlite::Error>(None),
                };

                let expected = hpfeeds_core::hashsecret(&rand, &secret);
                if expected.as_slice() != secret_hash.as_slice() {
                    return Ok(None);
                }

                let mut stmt = match conn
                    .prepare("SELECT channel, can_pub, can_sub FROM permissions WHERE ident = ?")
                {
                    Ok(s) => s,
                    Err(_) => return Ok(None),
                };

                let perms: Vec<(String, bool, bool)> = match stmt
                    .query_map([&ident], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                {
                    Ok(rows) => match rows.collect::<Result<Vec<_>, _>>() {
                        Ok(p) => p,
                        Err(_) => return Ok(None),
                    },
                    Err(_) => return Ok(None),
                };

                let mut pub_channels = Vec::new();
                let mut sub_channels = Vec::new();

                for (channel, can_pub, can_sub) in perms {
                    if can_pub {
                        pub_channels.push(channel.clone());
                    }
                    if can_sub {
                        sub_channels.push(channel);
                    }
                }

                Ok(Some(AccessContext {
                    ident: ident.clone(),
                    pub_channels,
                    sub_channels,
                }))
            })
            .await
            .ok()
            .flatten()
    }
}
