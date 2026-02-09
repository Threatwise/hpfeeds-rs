# Server Documentation

The `hpfeeds-server` is the central broker of the ecosystem.

## Usage

```bash
./hpfeeds-server [FLAGS] [OPTIONS]
```

### Authentication Modes

1. **Ephemeral**: Use `--auth ident:secret` for quick tests.
2. **JSON Config**: Use `--config users.json` for static ACLs.
3. **SQLite**: Use `--db hpfeeds.db` for dynamic user management via the CLI.

### Security (TLS)

Enable native TLS:
```bash
./hpfeeds-server --tls-cert cert.pem --tls-key key.pem
```

### Metrics

Prometheus metrics are served at `http://0.0.0.0:9431/metrics`.
