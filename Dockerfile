# syntax=docker/dockerfile:1

# ---- build stage ----
# go.mod + deps require go 1.26.7, matching the host toolchain.
FROM golang:1.26-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY src/ ./
RUN go build -trimpath -ldflags="-s -w -buildid=" -o /out/gif-favs .

# ---- runtime stage ----
FROM alpine:3.20
RUN addgroup -S -g 1000 giffavs \
 && adduser -S -D -H -u 1000 -G giffavs giffavs \
 && mkdir -p /srv /data \
 && chown giffavs:giffavs /data
WORKDIR /srv
COPY --from=build /out/gif-favs /srv/gif-favs
USER giffavs

ENV FAV_HOST=0.0.0.0 \
    FAV_PORT=8099 \
    FAV_DB_PATH=/data/favorites.db

EXPOSE 8099
# HEALTHCHECK optional (busybox wget is present in alpine)
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD wget -q -O /dev/null http://127.0.0.1:8099/health || exit 1
CMD ["/srv/gif-favs"]