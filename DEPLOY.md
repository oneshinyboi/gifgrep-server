Run by **Diamond**. Everything is prepared; you execute. Nothing here has been
run against the server — verify each step's output before proceeding.

Goal chain:

```
outer nginx (443/TLS)  ->  inner nginx (:8080, routes by Host)  ->  gif-favs (:8099)
                                  favs.veryshiny.net
                      ->  Portainer stack gif-favs (db: /home/Diamond/Docker/GifFavs/db)
```

---

## 0. Prereqs on the workstation (already done)

- Project dir: `/home/diamond/Projects/gifgrep-server` (this repo root).
- Binary verified locally: `go build ./src; FAV_TOKEN=test123 ./gif-favs`, full
  curl pass done, persistence across restart proven.

---

## 1. Server: create bind dir + check port 8099

```sh
ssh -i ~/.ssh/id_ed25519_veryshiny Diamond@veryshiny.net
mkdir -p /home/Diamond/Docker/GifFavs/db
sudo chown -R 1000:1000 /home/Diamond/Docker/GifFavs/db   # must be 1000 = container UID
```

Confirm 8099 is free (nothing should print):

```sh
ss -tln | grep ':8099' || echo "8099 is free"
```

> If the grep prints a line, a service already uses 8099 — pick another host
> port and change the `ports:` mapping in the stack accordingly.

---

## 2. Get the image on the server

The image is published to Docker Hub by the GitHub Action in this repo
(`.github/workflows/docker-image.yml`, manual dispatch) as
`diamondcoder295/giffavs:latest`. You don't need the source on the server —
just pull the image.

```sh
ssh -i ~/.ssh/id_ed25519_veryshiny Diamond@veryshiny.net
docker pull diamondcoder295/giffavs:latest
```

(Optional) Smoke-test it before wiring anything:

```sh
FAV_TOKEN=test123 docker run -d --rm --name gif-favs-smoke \
  -p 8101:8099 -e FAV_TOKEN=test123 -v /tmp/giffavs-db:/data diamondcoder295/giffavs:latest

# wait ~2s, then:
curl -s http://127.0.0.1:8101/health                                 # {"status":"ok"}
curl -s -X POST -H "X-Auth-Token: test123" \
  -H "Content-Type: application/json" \
  -d '{"id":"smoke-1","url":"https://example.com/a.gif","preview":"https://example.com/a_p.gif","provider":"giphy","title":"smoke"}' \
  http://127.0.0.1:8101/api/v1/favorites                              # 200 item
curl -s -H "X-Auth-Token: test123" http://127.0.0.1:8101/api/v1/favorites
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8101/api/v1/favorites  # 401 (no token)

docker rm -f gif-favs-smoke
rm -rf /tmp/giffavs-db
```

> **No source needed on the server.** There's no repo clone or Docker build on
> veryshiny.net anymore — pull + run only.

---

## 3. Portainer: add the stack

UI: https://portainer.veryshiny.net -> **Stacks** -> **Add stack**.

1. In the **Web editor**, paste the full contents of `compose.yml` from this repo.
2. In the **Environment variables** panel, add `FAV_TOKEN` with a fresh value.
   Generate one now, e.g.
   ```sh
   openssl rand -hex 32
   ```
3. (Optional) Deployment -> Force recreation: `on`.
4. Deploy.

Notes:

- **No `env_file`** is used; Portainer keeps `FAV_TOKEN` in its own stack env.
- Portainer pulls `diamondcoder295/giffavs:latest` from Docker Hub on deploy
  (tick "Pull latest image" / "Force recreation" when redeploying to update).
- Container maps host `8099` -> container `8099` and bind-mounts
  `/home/Diamond/Docker/GifFavs/db` -> `/data`.
- Healthcheck hits `/health` — expect a green dot.

Verify container + host port:

```sh
docker ps --filter name=gif-favs
ss -tln | grep ':8099'
curl -s http://127.0.0.1:8099/health            # {"status":"ok"}
```

---

## 4. Inner nginx: add the `favs.` server block to the router

You have two options — **A. envsubst (preferred, matches your flow)**, or B. plain manual edit.

The block must be added **before** the `default_server` 404 block in
`/home/Diamond/Docker/Nginx/nginx.conf.conf`:

## 4-A. envsubst patch (single-quoted var list, re-render, reload)

On the server, as Diamond:

```sh
# 1) Drop a drop-in fragment with ${HOST}/${PASS_IP} placeholders
cat > /tmp/favs-router.conf <<'EOF'
    server {
        listen 8080;
        server_name favs.${HOST};
        location / {
            proxy_pass http://${PASS_IP}:8099;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $scheme;
        }
    }
EOF

# 2) Decide PASS_IP: the container IP of gif-favs, so nginx can reach it.
#    Pick ONE of:
PASS_IP=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' gif-favs)
#    or, if the router proxies to the host instead of the container directly:
# PASS_IP=127.0.0.1
HOST=veryshiny.net

# 3) Insert the rendered block BEFORE the default_server 404 block.
#    Find the line number of the default_server block start:
BN=$(grep -n 'default_server' /home/Diamond/Docker/Nginx/nginx.conf.conf \
      | head -1 | cut -d: -f1)
#    Render + insert (envsubst with single-quoted var list — only those two
#    vars are substituted, nginx $host/$remote_addr/... stay intact):
envsubst '${HOST} ${PASS_IP}' < /tmp/favs-router.conf > /tmp/favs-router.conf.rendered
sed -i "${BN}i $(cat /tmp/favs-router.conf.rendered)" /home/Diamond/Docker/Nginx/nginx.conf.conf

# 4) Check the diff looks right, then reload via Docker (adapt container name):
grep -n -A10 'server_name favs' /home/Diamond/Docker/Nginx/nginx.conf.conf
docker exec <nginx-container> nginx -t          # must say: ok
docker exec <nginx-container> nginx -s reload
```

## 4-B. Manual edit (equivalent)

Append this `server` block to `/home/Diamond/Docker/Nginx/nginx.conf.conf`
**above** the existing `server { listen 8080 default_server; ... }` 404 block,
with `server_name favs.veryshiny.net` and the real PASS_IP in `proxy_pass`:

```
    server {
        listen 8080;
        server_name favs.veryshiny.net;
        location / {
            proxy_pass http://PASS_IP:8099;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-Proto $scheme;
        }
    }
```

Then test + reload as in step 4-A-4. **Do not touch the outer nginx.**

---

## 5. Verify end-to-end through the router

From the server:

```sh
# Expect 401: routing works (auth would be checked after routing; no token here)
curl -s -o /dev/null -w "%{http_code}\n" \
  -H "Host: favs.veryshiny.net" \
  http://127.0.0.1:8080/api/v1/favorites
```

Then with the real token:

```sh
curl -s -o /dev/null -w "%{http_code}\n" \
  -H "Host: favs.veryshiny.net" -H "X-Auth-Token: <from Portainer>" \
  http://127.0.0.1:8080/api/v1/favorites        # expect 200 [] or list
```

Visit https://favs.veryshiny.net/api/v1/favorites — the browser will complain
about the certificate **until** the cert SAN is updated (next section).

---

## 6. Certificate SAN (do this or the public URL is broken)

The Let's Encrypt cert has an **explicit SAN list**. Add
`favs.veryshiny.net` to it by hand (however your renewal is set up — certbot
`--cert-name <cert> -d ... -d favs.veryshiny.net` or the ACME panel), then
renew/reissue. Until then the site returns ERR_CERT_COMMON_NAME_INVALID.

After renewal:

```sh
curl -s -o /dev/null -w "%{http_code}\n" https://favs.veryshiny.net/api/v1/favorites
# 401 with no token is the success signal (proves TLS + routing +
# (token gate) all work)
```

---

## 7. Final smoke from a workstation

```sh
curl -s -X PATCH -H "X-Auth-Token: <token>" \
  https://favs.veryshiny.net/api/v1/favorites/smoke-1/use
curl -s -H "X-Auth-Token: <token>" https://favs.veryshiny.net/api/v1/favorites
```

---

## 8. Updating the service

New images are published on demand via the GitHub Action
(`Actions` -> `Build and Push Docker Image` -> `Run workflow`), which pushes a
new `diamondcoder295/giffavs:<timestamp>` plus `:latest`. To roll it out:

1. Run the workflow on GitHub (requires `DOCKER_HUB_USERNAME` and
   `DOCKER_HUB_PASSWORD` secrets in this repo — already set).
2. On the server:
   ```sh
   docker pull diamondcoder295/giffavs:latest
   ```
3. Recreate the stack in Portainer (Stacks -> gif-favs -> Deploy with
   "Pull latest image" / "Force recreation" enabled), or stop/start from the
   stack list.

The bind-mounted DB (`/home/Diamond/Docker/GifFavs/db`) persists across
recreations — updates never touch favorites data.

---

## Rollback

- Stacks -> gif-favs -> Stop; remove the added nginx `server` block and reload.
- DB (and nothing else) lives in `/home/Diamond/Docker/GifFavs/db` — back that
  directory up; restoring it restores the favorites.

## Troubleshooting

- `docker exec gif-favs ls -la /data` — is the bind mount visible? WAL files
  (`favorites.db-wal`) should appear there after the first write.
- Portainer "unhealthy": check the stack logs (Portainer shows container logs).
- 404 vs 401 through the router: 404 => the nginx block isn't active; 401 =>
  routing + gate are fine, your token is just missing.
- If the router can't reach the container IP after a recreation, re-run step
  4-A with the new `PASS_IP`.