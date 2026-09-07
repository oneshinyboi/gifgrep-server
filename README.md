# gifdeck-server

Small, self-hosted GIF-favorites sync server.

A REST API (Rust + axum + SQLite) that stores favorite GIFs as URL bookmarks —
the server half of a GIF-picker favorites feature, self-hosted and
terminal-first. The companion terminal client
[gifdeck](https://github.com/oneshinyboi/gifdeck) speaks this API out of the
box: search, preview, favorite, and sync one list across machines. Any HTTP
client works too.

- Rust (axum + tokio, no HTTP framework beyond axum's router)
- SQLite via `sqlx` (SQLite C sources bundled at build time; no system SQLite)
- Single static binary, ~4 MiB, WAL mode, single connection, private
  single-user service (bearer token; TLS is terminated by the reverse proxy;
  no CORS)

API contract: see [api.md](api.md).

## Quick start (local)

```sh
cargo build --release

FAV_TOKEN=test123 FAV_PORT=8099 ./target/release/gif-favs
```

Config comes from environment variables only (no config file):

| Var          | Required | Default          |
|--------------|----------|------------------|
| `FAV_TOKEN`  | yes      | — (refuses to start without it) |
| `FAV_PORT`   | no       | `8099`           |
| `FAV_HOST`   | no       | `0.0.0.0`        |
| `FAV_DB_PATH`| no       | `./favorites.db` |

DB is created on first run at `FAV_DB_PATH` (WAL mode, single connection).

## Routes

```
GET    /health                                   -> 200 {"status":"ok"}
GET    /api/v1/favorites[?limit=N[&offset=N]]    -> 200 sorted array
POST   /api/v1/favorites                         -> 200 upserted item
PATCH  /api/v1/favorites/{id}/use                -> 200 {id,use_count,last_used}
DELETE /api/v1/favorites/{id}                    -> 204
```

`X-Auth-Token: <token>` is required on all `/api/v1/*` routes.
Formal contract + curl examples: [api.md](api.md).

## Layout

```
gifdeck-server/
├── .gitignore
├── .dockerignore
├── api.md              # the v1 contract + curl examples
├── src/main.rs         # env config, listener, graceful shutdown
├── src/routes.rs       # router, auth middleware, handlers, logging
├── src/db.rs           # SQLite open (WAL) + all queries
├── src/tests.rs        # list/pagination, validation, upsert, auth tests
├── Cargo.toml / Cargo.lock
├── Dockerfile          # multi-stage: rust:1-alpine (musl) -> alpine:3.20, uid 1000
├── compose.yml         # example stack
├── stack.env.example   # FAV_TOKEN=change-me
└── .github/workflows   # builds + pushes the Docker image on release
```

## Docker

```sh
docker build -t giffavs:latest .
docker run -d --name gif-favs -p 8099:8099 \
  -e FAV_TOKEN=test123 \
  -v "$(pwd)"/db:/data \
  giffavs:latest
```

Prebuilt images are published to Docker Hub as
`diamondcoder295/giffavs:<version>` (plus `:latest`) on every GitHub release.

## Tests

```sh
cargo test
```

## Notes

- **Build image version**: the build stage uses `rust:1-alpine` (musl target,
  static binary). `sqlx` compiles the bundled SQLite C sources, so the image
  needs `gcc`/`musl-dev` but the runtime has no SQLite dependency.
- Binary is built with `strip = true`, `lto = true` in `[profile.release]`
  (~4 MiB).
- `last_used` is `null` and `title` is `""` for never-used / untitled items.
- List responses carry an `X-Total-Count` header with the total number of
  favorites, so paged clients can compute the page count.
- No TLS, no users, no CORS — that is deliberate; put a reverse proxy ahead of
  it for TLS and the bearer token is the whole auth story.
