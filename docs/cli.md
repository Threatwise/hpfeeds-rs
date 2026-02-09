# CLI Tools

## testing

Use `hpfeeds-cli` to interact with the broker.

```bash
./hpfeeds-cli sub malware
./hpfeeds-cli pub -c malware -p "threat"
```

## Administration

Manage users in the SQLite database:

```bash
./hpfeeds-cli admin --db hpfeeds.db add-user sensor1 secret
./hpfeeds-cli admin --db hpfeeds.db list-users
```
