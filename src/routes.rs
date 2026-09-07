use axum::{
    Router,
    extract::{Path, Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, patch},
};
use serde::Deserialize;
use serde_json::json;
use subtle::ConstantTimeEq;

use crate::db;

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub token: String,
}

pub fn serve_mux(state: AppState) -> Router {
    // Ids may contain slashes, so the item route takes a {*rest} wildcard and
    // handlers replicate the Go suffix dispatch ("/use" => PATCH-only,
    // everything else => DELETE-only).
    let api = Router::new()
        .route(
            "/api/v1/favorites",
            get(list_favorites)
                .post(post_favorite)
                .fallback(method_not_allowed),
        )
        .route(
            "/api/v1/favorites/{*rest}",
            patch(use_item)
                .delete(delete_item)
                .fallback(method_not_allowed),
        )
        .layer(middleware::from_fn_with_state(state.clone(), auth_mw));
    Router::new()
        .route("/health", get(health))
        .merge(api)
        .fallback(fallback_404)
        .with_state(state)
        .layer(middleware::from_fn(logging))
}

fn write_json(status: StatusCode, v: &serde_json::Value) -> Response {
    let body = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

fn write_err(status: StatusCode, msg: &str) -> Response {
    write_json(status, &json!({ "error": msg }))
}

async fn health() -> Response {
    write_json(StatusCode::OK, &json!({ "status": "ok" }))
}

async fn fallback_404() -> Response {
    write_err(StatusCode::NOT_FOUND, "not found")
}

async fn method_not_allowed() -> Response {
    write_err(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
}

async fn auth_mw(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.token.is_empty() {
        return write_err(StatusCode::UNAUTHORIZED, "missing or invalid X-Auth-Token");
    }
    let got = req
        .headers()
        .get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ok = got.len() == state.token.len()
        && got.as_bytes().ct_eq(state.token.as_bytes()).into();
    if !ok {
        return write_err(StatusCode::UNAUTHORIZED, "missing or invalid X-Auth-Token");
    }
    next.run(req).await
}

async fn logging(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let res = next.run(req).await;
    tracing::info!("{method} {path} {status}", status = res.status().as_u16());
    res
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<String>,
    offset: Option<String>,
}

#[allow(clippy::result_large_err)] // Err carries the JSON error response
fn parse_non_neg(q: Option<&str>, msg: &str) -> Result<i64, Response> {
    let Some(q) = q else { return Ok(0) };
    match q.parse::<i64>() {
        Ok(n) if n >= 0 => Ok(n),
        _ => Err(write_err(StatusCode::BAD_REQUEST, msg)),
    }
}

async fn list_favorites(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let limit = match parse_non_neg(q.limit.as_deref(), "invalid limit") {
        Ok(v) => v,
        Err(res) => return res,
    };
    let offset = match parse_non_neg(q.offset.as_deref(), "invalid offset") {
        Ok(v) => v,
        Err(res) => return res,
    };
    let pool = &state.pool;
    let total = match db::count(pool).await {
        Ok(n) => n,
        Err(_) => return write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
    };
    // Offset without limit still skips; a negative LIMIT means "no limit" in
    // SQLite, so -1 keeps the unbounded default while applying OFFSET.
    let sql_limit = if limit <= 0 { -1 } else { limit };
    let ids = match db::list_ids(pool, sql_limit, offset).await {
        Ok(ids) => ids,
        Err(_) => return write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
    };
    let mut items = Vec::with_capacity(ids.len());
    for id in &ids {
        match db::fetch(pool, id).await {
            Ok(Some(f)) => items.push(f),
            _ => return write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
        }
    }
    match serde_json::to_vec(&items) {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header("X-Total-Count", total.to_string())
            .body(axum::body::Body::from(body))
            .unwrap(),
        Err(_) => write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
    }
}

#[derive(Deserialize, Default)]
struct PostBody {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    preview: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    title: String,
}

async fn post_favorite(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    // Go decodes the body regardless of Content-Type; mirror that.
    let body: PostBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(_) => return write_err(StatusCode::BAD_REQUEST, "invalid JSON body"),
    };
    let id = body.id.trim().to_string();
    let url = body.url.trim().to_string();
    if id.is_empty() || url.is_empty() {
        return write_err(StatusCode::BAD_REQUEST, "id and url are required");
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return write_err(
            StatusCode::BAD_REQUEST,
            "url must start with http:// or https://",
        );
    }
    let now = chrono::Utc::now().timestamp();
    let nf = db::NewFavorite {
        id: &id,
        url: &url,
        preview: &body.preview,
        provider: &body.provider,
        title: &body.title,
    };
    if db::upsert(&state.pool, &nf, now).await.is_err() {
        return write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error");
    }
    match db::fetch(&state.pool, &id).await {
        Ok(Some(f)) => write_json(
            StatusCode::OK,
            &serde_json::to_value(&f).unwrap_or_default(),
        ),
        _ => write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
    }
}

async fn use_item(State(state): State<AppState>, Path(rest): Path<String>) -> Response {
    let Some(id) = rest.strip_suffix("/use") else {
        // In Go, a path without the "/use" suffix belongs to the DELETE-only
        // branch, so PATCH here is a wrong method.
        return method_not_allowed().await;
    };
    let id = id.trim().to_string();
    let now = chrono::Utc::now().timestamp();
    match db::record_use(&state.pool, &id, now).await {
        Ok(false) => write_err(StatusCode::NOT_FOUND, "favorite not found"),
        Ok(true) => match db::fetch(&state.pool, &id).await {
            Ok(Some(f)) => write_json(
                StatusCode::OK,
                &json!({
                    "id": id,
                    "use_count": f.use_count,
                    "last_used": f.last_used,
                }),
            ),
            _ => write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
        },
        Err(_) => write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
    }
}

async fn delete_item(State(state): State<AppState>, Path(rest): Path<String>) -> Response {
    // "/use" paths are PATCH-only in Go; DELETE there is a wrong method.
    if rest.ends_with("/use") {
        return method_not_allowed().await;
    }
    let id = rest.trim().to_string();
    match db::delete(&state.pool, &id).await {
        Ok(false) => write_err(StatusCode::NOT_FOUND, "favorite not found"),
        Ok(true) => Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::empty())
            .unwrap(),
        Err(_) => write_err(StatusCode::INTERNAL_SERVER_ERROR, "db error"),
    }
}
