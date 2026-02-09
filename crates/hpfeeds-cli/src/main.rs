use clap::{Parser, Subcommand};
use hpfeeds_client::connect_and_auth;
use hpfeeds_core::Frame;
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use tokio::io::{self, AsyncReadExt};
use sqlx::SqlitePool;

#[derive(Parser, Debug)]
#[clap(name = "hpfeeds-cli", about = "CLI tool for hpfeeds")]
struct Cli {
    /// Broker host
    #[clap(long, default_value = "127.0.0.1")]
    host: String,

    /// Broker port
    #[clap(long, default_value_t = 10000)]
    port: u16,

    /// Identity
    #[clap(long, short = 'i', default_value = "anonymous")]
    ident: String,

    /// Secret
    #[clap(long, short = 's', default_value = "")]
    secret: String,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Subscribe to channels
    Sub {
        /// Channels to subscribe to (space separated)
        #[clap(required = true)]
        channels: Vec<String>,
    },
    /// Publish data to a channel
    Pub {
        /// Channel to publish to
        #[clap(long, short = 'c')]
        channel: String,

        /// Payload (string). If not provided, reads from stdin.
        #[clap(long, short = 'p')]
        payload: Option<String>,
    },
    /// Admin commands (Direct DB access)
    Admin {
        /// Path to hpfeeds.db
        #[clap(long, default_value = "hpfeeds.db")]
        db: String,

        #[clap(subcommand)]
        cmd: AdminCommands,
    }
}

#[derive(Subcommand, Debug)]
enum AdminCommands {
    /// Add a user
    AddUser {
        ident: String,
        secret: String,
    },
        /// Add ACL permission
        AddAcl {
            ident: String,
            channel: String,
            #[clap(long)]
            pub_allowed: bool,
            #[clap(long)]
            sub_allowed: bool,
        },
        /// List users
        ListUsers,
        /// Remove a user (and their permissions)
        RemoveUser {
            ident: String,
        }
    }
    
    #[tokio::main]
    async fn main() -> Result<()> {
        let args = Cli::parse();
        
        match args.command {
                    Commands::Sub { channels } => {
                        let addr = format!("{}:{}", args.host, args.port);
                        let mut client = connect_and_auth(&addr, &args.ident, &args.secret).await?;
                        println!("Connected and authenticated as {}", args.ident);
                        for c in channels {
                            println!("Subscribing to {}", c);
                            client.send(Frame::Subscribe { ident: args.ident.clone().into(), channel: c.into() }).await?;
                        }
            
                        println!("Waiting for messages...");
                        while let Some(msg) = client.next().await {
                            match msg {
                                Ok(Frame::Publish { ident, channel, payload }) => {
                                    let data = String::from_utf8_lossy(&payload);
                                    let ident_str = String::from_utf8_lossy(&ident);
                                    let chan_str = String::from_utf8_lossy(&channel);
                                    println!("[{}] {}: {}", chan_str, ident_str, data);
                                }
                                Ok(Frame::Error(e)) => {
                                    eprintln!("Error from server: {}", String::from_utf8_lossy(&e));
                                }
                                Ok(other) => {
                                    println!("Received frame: {:?}", other);
                                }
                                Err(e) => {
                                    eprintln!("Connection error: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                    Commands::Pub { channel, payload } => {
                        let addr = format!("{}:{}", args.host, args.port);
                        let mut client = connect_and_auth(&addr, &args.ident, &args.secret).await?;
                        println!("Connected and authenticated as {}", args.ident);
                        let data = match payload {
                            Some(p) => p.into_bytes(),
                            None => {
                                let mut buf = Vec::new();
                                io::stdin().read_to_end(&mut buf).await?;
                                buf
                            }
                        };
            
                        println!("Publishing {} bytes to {}", data.len(), channel);
                        client.send(Frame::Publish { 
                            ident: args.ident.clone().into(), 
                            channel: channel.into(), 
                            payload: data.into() 
                        }).await?;
                        println!("Done.");
                    }            Commands::Admin { db, cmd } => {
                if !std::path::Path::new(&db).exists() {
                    anyhow::bail!("Database file {} not found. Start the server with --db to create it.", db);
                }
                let conn_str = format!("sqlite:{}", db);
                let pool = SqlitePool::connect(&conn_str).await?;
    
                match cmd {
                    AdminCommands::AddUser { ident, secret } => {
                        sqlx::query("INSERT OR REPLACE INTO users (ident, secret) VALUES (?, ?)")
                            .bind(&ident)
                            .bind(&secret)
                            .execute(&pool)
                            .await?;
                        println!("User {} added/updated.", ident);
                    }
                    AdminCommands::AddAcl { ident, channel, pub_allowed, sub_allowed } => {
                        sqlx::query("INSERT INTO permissions (ident, channel, can_pub, can_sub) VALUES (?, ?, ?, ?)")
                            .bind(&ident)
                            .bind(&channel)
                            .bind(pub_allowed)
                            .bind(sub_allowed)
                            .execute(&pool)
                            .await?;
                        println!("ACL added for {} on {}: pub={}, sub={}", ident, channel, pub_allowed, sub_allowed);
                    }
                    AdminCommands::ListUsers => {
                        use sqlx::Row;
                        let rows = sqlx::query("SELECT ident, secret FROM users").fetch_all(&pool).await?;
                        println!("{:<20} | {:<20}", "IDENT", "SECRET");
                        println!("{:-<20}-+-{:-<20}", "", "");
                        for r in rows {
                            let ident: String = r.get("ident");
                            let secret: String = r.get("secret");
                            println!("{:<20} | {:<20}", ident, secret);
                            
                            // fetch permissions
                            let perms = sqlx::query("SELECT channel, can_pub, can_sub FROM permissions WHERE ident = ?")
                                .bind(&ident)
                                .fetch_all(&pool)
                                .await?;
                            for p in perms {
                                let chan: String = p.get("channel");
                                let can_pub: bool = p.get("can_pub");
                                let can_sub: bool = p.get("can_sub");
                                println!("  -> ACL: {:<15} pub={} sub={}", chan, can_pub, can_sub);
                            }
                        }
                    }
                    AdminCommands::RemoveUser { ident } => {
                        let mut tx = pool.begin().await?;
                        sqlx::query("DELETE FROM permissions WHERE ident = ?").bind(&ident).execute(&mut *tx).await?;
                        let res = sqlx::query("DELETE FROM users WHERE ident = ?").bind(&ident).execute(&mut *tx).await?;
                        tx.commit().await?;
                        if res.rows_affected() > 0 {
                            println!("User {} removed.", ident);
                        } else {
                            println!("User {} not found.", ident);
                        }
                    }
                }
            }
        }
    
        Ok(())
    }
    