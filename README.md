# gifgrep-server

Small, self-hosted GIF-favorites sync server.

A personal REST API (Go stdlib + pure-Go SQLite) that stores favorite GIFs as
URL bookmarks: the server half of a "Discord GIF-picker favorites" — but
self-hosted and terminal-first. A lightweight GIF picker client on two Linux
machines shares one favorites list through it.

- Go 1.26 (stdlib `net/http` only, no HTTP framework)
- SQLite via pure-Go `modernc.org/sqlite` (pinned to `v1.40.0`; no cgo bindings)
- Single binary, ~9.5 MiB, WAL mode, private single-user service
  (bearer token; TLS is terminated by the reverse proxy; no CORS)

API contract: see [api.md](api.md). Deploy runbook: see [DEPLOY.md](DEPLOY.md).

## Quick start (local)

```sh
go build -trimpath -ldflags="-s -w -buildid=" -o gif-favs ./src

FAV_TOKEN=test123 FAV_PORT=8099 ./gif-favs
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
GET    /health                                    -> 200 {"status":"ok"}
GET    /api/v1/favorites[?limit=N]                -> 200 sorted array
POST   /api/v1/favorites                          -> 200 upserted item
PATCH  /api/v1/favorites/{id}/use                 -> 200 {id,use_count,last_used}
DELETE /api/v1/favorites/{id}                     -> 204
```

`X-Auth-Token: <token>` is required on all `/api/v1/*` routes.
Formal contract + curl examples: [api.md](api.md).

## Layout

```
gifgrep-server/
├── .gitignore
├── .dockerignore
├── api.md              # the v1 contract + curl examples
├── src/main.go         # the whole server (one file)
├── go.mod / go.sum     # pin: modernc.org/sqlite v1.40.0
├── Dockerfile          # multi-stage: golang:1.26-alpine -> alpine:3.20, uid 1000
├── compose.yml         # Portainer-ready stack (no env_file)
├── stack.env.example   # FAV_TOKEN=change-me
└── DEPLOY.md           # exact deploy runbook for veryshiny.net
```

## Docker

```sh
docker build -t giffavs:latest .
docker run -d --name gif-favs -p 8099:8099 \
  -e FAV_TOKEN=test123 \
  -v "$(pwd)"/db:/data \
  giffavs:latest
```

## Notes

- **Build image version**: the build stage uses `golang:1.26-alpine` because
  the pinned deps require Go 1.26.7 (matches the toolchain this was developed
  with — Go 1.25+).
- Binary is built with `-trimpath -ldflags="-s -w -buildid="` to keep it under
  10 MB (~9.5 MiB).
- `last_used` is `null` and `title` is `""` for never-used / untitled items.
- No TLS, no users, no CORS — that is deliberate; the nginx proxy ahead of it
  handles TLS and the bearer token is the whole auth story.