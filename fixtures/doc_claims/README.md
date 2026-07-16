# Runtime

The `GET /health` route reports service health.
Runtime configuration lives in `config/app.toml`.
The environment variable `APP_PORT` controls the listener.

```sh
cargo test
```

# Guidance

Prefer simple deployments.
TODO: document the future failover service.
The system is reliable.
