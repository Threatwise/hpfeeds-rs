use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::{SinkExt, StreamExt};
use hpfeeds_client::connect_and_auth;
use hpfeeds_core::Frame;
use tokio::io::{self, AsyncReadExt};
use tokio_rusqlite::{Connection, rusqlite};

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
    },
}

#[derive(Subcommand, Debug)]
enum AdminCommands {
    /// Add a user
    AddUser { ident: String, secret: String },
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
    RemoveUser { ident: String },
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
                client
                    .send(Frame::Subscribe {
                        ident: args.ident.clone().into(),
                        channel: c.into(),
                    })
                    .await?;
            }

            println!("Waiting for messages...");
            while let Some(msg) = client.next().await {
                match msg {
                    Ok(Frame::Publish {
                        ident,
                        channel,
                        payload,
                    }) => {
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
            client
                .send(Frame::Publish {
                    ident: args.ident.clone().into(),
                    channel: channel.into(),
                    payload: data.into(),
                })
                .await?;
            println!("Done.");
        }
        Commands::Admin { db, cmd } => {
            if !std::path::Path::new(&db).exists() {
                anyhow::bail!(
                    "Database file {} not found. Start the server with --db to create it.",
                    db
                );
            }
            let conn = Connection::open(&db).await?;

            match cmd {
                AdminCommands::AddUser { ident, secret } => {
                    let ident_display = ident.clone();
                    let ident = ident.clone();
                    let secret = secret.clone();
                    conn.call(move |conn| {
                        conn.execute(
                            "INSERT OR REPLACE INTO users (ident, secret) VALUES (?, ?)",
                            [&ident, &secret],
                        )?;
                        Ok::<(), rusqlite::Error>(())
                    })
                    .await?;
                    println!("User {} added/updated.", ident_display);
                }
                AdminCommands::AddAcl {
                    ident,
                    channel,
                    pub_allowed,
                    sub_allowed,
                } => {
                    let ident_display = ident.clone();
                    let channel_display = channel.clone();
                    let ident = ident.clone();
                    let channel = channel.clone();
                    conn.call(move |conn| {
                            conn.execute(
                                "INSERT INTO permissions (ident, channel, can_pub, can_sub) VALUES (?, ?, ?, ?)",
                                rusqlite::params![&ident, &channel, pub_allowed, sub_allowed],
                            )?;
                            Ok::<(), rusqlite::Error>(())
                        }).await?;
                    println!(
                        "ACL added for {} on {}: pub={}, sub={}",
                        ident_display, channel_display, pub_allowed, sub_allowed
                    );
                }
                AdminCommands::ListUsers => {
                    let users = conn
                        .call(|conn| {
                            let mut stmt = conn.prepare("SELECT ident, secret FROM users")?;
                            let rows = stmt.query_map([], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                            })?;
                            rows.collect::<Result<Vec<_>, _>>()
                        })
                        .await?;

                    println!("{:<20}", "IDENT");
                    println!("{:-<20}", "");
                    for (ident, _secret) in users {
                        println!("{:<20}", ident);

                        // fetch permissions
                        let ident_clone = ident.clone();
                        let perms = conn.call(move |conn| {
                                let mut stmt = conn.prepare("SELECT channel, can_pub, can_sub FROM permissions WHERE ident = ?")?;
                                let rows = stmt.query_map([&ident_clone], |row| {
                                    Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?))
                                })?;
                                rows.collect::<Result<Vec<_>, _>>()
                            }).await?;

                        for (chan, can_pub, can_sub) in perms {
                            println!("  -> ACL: {:<15} pub={} sub={}", chan, can_pub, can_sub);
                        }
                    }
                }
                AdminCommands::RemoveUser { ident } => {
                    let ident_display = ident.clone();
                    let ident = ident.clone();
                    let rows_affected = conn
                        .call(move |conn| {
                            let tx = conn.transaction()?;
                            tx.execute("DELETE FROM permissions WHERE ident = ?", [&ident])?;
                            let affected =
                                tx.execute("DELETE FROM users WHERE ident = ?", [&ident])?;
                            tx.commit()?;
                            Ok::<usize, rusqlite::Error>(affected)
                        })
                        .await?;

                    if rows_affected > 0 {
                        println!("User {} removed.", ident_display);
                    } else {
                        println!("User {} not found.", ident_display);
                    }
                }
            }
        }
    }

    Ok(())
}
