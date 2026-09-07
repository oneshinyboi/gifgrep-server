# API v1

Base path: `/api/v1` — the reverse proxy maps `favs.veryshiny.net` to this
service.

Auth: header `X-Auth-Token: <token>` required on **all** `/api/v1` routes.
Only `/health` is unauthenticated.

All responses are JSON, `Content-Type: application/json` (including errors).
404 for unknown ids and unknown routes. 405 for a wrong method on a valid
route. 400 for malformed input. 401 for missing/invalid token.

---

## GET /health

Unauthenticated. Liveness check for load balancers / healthchecks.

```
GET /health
→ 200 {"status":"ok"}
```

```
curl -s http://127.0.0.1:8099/health
{"status":"ok"}
```

---

## GET /api/v1/favorites

Authenticated. Returns the full list, sorted by `use_count` DESC then
`last_used` DESC.

Optional query params:

- `?limit=N` (non-negative integer) truncates the list.
- `?offset=N` (non-negative integer) skips the first N items. Combine with
  `limit` for page-by-page browsing, e.g. `?limit=50&offset=50` is page 2.
  An empty result means the end of the list.

Every list response carries an `X-Total-Count` header (integer) with the
total number of favorites, so clients can compute the page count.

Item shape:

```json
{
  "id": "giphy-abc123",
  "url": "https://media1.giphy.com/media/abc123/giphy.gif",
  "preview": "https://media1.giphy.com/media/abc123/200.gif",
  "provider": "giphy",
  "title": "funny cat",
  "use_count": 4,
  "added_at": "2026-09-06T12:00:00Z",
  "last_used": "2026-09-06T18:30:00Z"
}
```

`last_used` is `null` for items that have never been used. `title` is `""`
when not provided.

```
curl -s -H "X-Auth-Token: $FAV_TOKEN" http://127.0.0.1:8099/api/v1/favorites
curl -s -H "X-Auth-Token: $FAV_TOKEN" 'http://127.0.0.1:8099/api/v1/favorites?limit=10'
curl -s -H "X-Auth-Token: $FAV_TOKEN" 'http://127.0.0.1:8099/api/v1/favorites?limit=50&offset=50'
```

---

## POST /api/v1/favorites

Authenticated. Upserts a favorite keyed by `id`.

Request body:

```json
{
  "id": "giphy-abc123",
  "url": "https://media1.giphy.com/media/abc123/giphy.gif",
  "preview": "https://media1.giphy.com/media/abc123/200.gif",
  "provider": "giphy",
  "title": "funny cat"
}
```

Rules:

- `id` — required, trimmed of surrounding whitespace.
- `url` — required, must start with `http://` or `https://`.
- `preview` — optional (defaults to empty string).
- `provider` — optional (defaults to empty string).
- `title` — optional (defaults to empty string).

Response: `200` with the upserted item (same shape as a list item). On
re-save of an existing id, `added_at` and `use_count` are **preserved**.

```
curl -s -X POST -H "X-Auth-Token: $FAV_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"id":"giphy-abc123","url":"https://media1.giphy.com/media/abc123/giphy.gif","preview":"https://media1.giphy.com/media/abc123/200.gif","provider":"giphy","title":"funny cat"}' \
  http://127.0.0.1:8099/api/v1/favorites
```

Errors:

- `400` on malformed JSON.
- `400` on missing `id`/`url`.
- `400` on `url` not starting with `http(s)://`.

---

## PATCH /api/v1/favorites/{id}/use

Authenticated. Records a "use": increments `use_count` and sets
`last_used = now` (UTC).

Response: `200`

```json
{
  "id": "giphy-abc123",
  "use_count": 5,
  "last_used": "2026-09-06T19:00:00Z"
}
```

`404` if the id is unknown.

```
curl -s -X PATCH -H "X-Auth-Token: $FAV_TOKEN" \
  http://127.0.0.1:8099/api/v1/favorites/giphy-abc123/use
```

---

## DELETE /api/v1/favorites/{id}

Authenticated. Deletes the favorite.

Response: `204` (no body). `404` if the id is unknown.

```
curl -s -o /dev/null -w "%{http_code}" -X DELETE \
  -H "X-Auth-Token: $FAV_TOKEN" \
  http://127.0.0.1:8099/api/v1/favorites/giphy-abc123
```

---

## Errors

```json
401 {"error":"missing or invalid X-Auth-Token"}
400 {"error":"..."}       // validation
405 {"error":"method not allowed"}
404 {"error":"favorite not found"}
500 {"error":"db error"}
```