# syntax=docker/dockerfile:1

# ---- build stage ----
# Static musl build; sqlx bundles the SQLite C sources, so only a C compiler
# is needed (no system libsqlite3, matching the old pure-Go driver).
FROM rust:1-alpine AS build
WORKDIR /src
RUN apk add --no-cache musl-dev gcc
COPY Cargo.toml Cargo.lock ./
# Build a stub first so dependency artifacts are cached between source edits.
RUN mkdir src \
 && echo "" > src/db.rs \
 && echo "" > src/routes.rs \
 && echo "fn main() {}" > src/main.rs \
 && cargo build --release
COPY src/ ./src/
RUN touch src/main.rs && cargo build --release

# ---- runtime stage ----
FROM alpine:3.20
RUN addgroup -S -g 1000 giffavs \
 && adduser -S -D -H -u 1000 -G giffavs giffavs \
 && mkdir -p /srv /data \
 && chown giffavs:giffavs /data
WORKDIR /srv
COPY --from=build /src/target/release/gif-favs /srv/gif-favs
USER giffavs

ENV FAV_HOST=0.0.0.0 \
    FAV_PORT=8099 \
    FAV_DB_PATH=/data/favorites.db

EXPOSE 8099
# HEALTHCHECK optional (busybox wget is present in alpine)
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD wget -q -O /dev/null http://127.0.0.1:8099/health || exit 1
CMD ["/srv/gif-favs"]
