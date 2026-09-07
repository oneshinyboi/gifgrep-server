use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::db;
use crate::routes::{AppState, serve_mux};

struct TestApp {
    state: AppState,
    _dir: tempfile::TempDir,
}

async fn new_test_app() -> TestApp {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = db::open_db(&dir.path().join("favorites.db").to_string_lossy())
        .await
        .expect("openDB");
    TestApp {
        state: AppState {
            pool,
            token: "test-token".to_string(),
        },
        _dir: dir,
    }
}

async fn seed(app: &TestApp, id: &str, use_count: i64, last_used: i64) {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO favorites (id, url, preview, provider, title, added_at, last_used, use_count) \
         VALUES (?, ?, '', 'giphy', ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(format!("https://example.com/{id}.gif"))
    .bind(id)
    .bind(now)
    .bind(last_used)
    .bind(use_count)
    .execute(&app.state.pool)
    .await
    .expect("seed");
}

fn get_req(path: &str) -> Request<Body> {
    Request::get(path)
        .header("X-Auth-Token", "test-token")
        .body(Body::empty())
        .unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let res = app.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, headers, json)
}

async fn get(app: &TestApp, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<Value>) {
    let (status, headers, json) = send(&serve_mux(app.state.clone()), get_req(path)).await;
    let items = json.as_array().cloned().unwrap_or_default();
    (status, headers, items)
}

fn ids(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|f| f["id"].as_str().map(str::to_string))
        .collect()
}

// Ordering is use_count DESC, last_used DESC, id ASC; limit+offset windows it.
#[tokio::test]
async fn test_list_favorites_limit_offset() {
    let app = new_test_app().await;
    // Expected order: f3 (uc=3), f2 (uc=2), f1 (uc=1), b (uc=0, newer), a (uc=0, older).
    seed(&app, "a", 0, 100).await;
    seed(&app, "f1", 1, 100).await;
    seed(&app, "f2", 2, 100).await;
    seed(&app, "f3", 3, 100).await;
    seed(&app, "b", 0, 200).await;

    let (status, headers, items) = get(&app, "/api/v1/favorites").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&items), vec!["f3", "f2", "f1", "b", "a"]);
    assert_eq!(headers["X-Total-Count"].to_str().unwrap(), "5");

    let (_, _, items) = get(&app, "/api/v1/favorites?limit=2").await;
    assert_eq!(ids(&items), vec!["f3", "f2"]);

    let (_, _, items) = get(&app, "/api/v1/favorites?limit=2&offset=2").await;
    assert_eq!(ids(&items), vec!["f1", "b"]);

    // Offset past the end returns an empty (non-null) array.
    let (_, _, items) = get(&app, "/api/v1/favorites?limit=10&offset=50").await;
    assert!(items.is_empty(), "offset past end should be empty array");

    // Offset without limit still skips.
    let (_, _, items) = get(&app, "/api/v1/favorites?offset=4").await;
    assert_eq!(ids(&items), vec!["a"]);
}

#[tokio::test]
async fn test_list_favorites_validation() {
    let app = new_test_app().await;
    seed(&app, "x", 1, 100).await;

    let (status, _, _) = get(&app, "/api/v1/favorites?offset=-1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = get(&app, "/api/v1/favorites?limit=-1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = get(&app, "/api/v1/favorites?offset=abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // No token -> 401.
    let router = serve_mux(app.state.clone());
    let req = Request::get("/api/v1/favorites")
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&router, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_health_unauthenticated() {
    let app = new_test_app().await;
    let router = serve_mux(app.state.clone());
    let req = Request::get("/health").body(Body::empty()).unwrap();
    let (status, headers, json) = send(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(json["status"], "ok");
}

fn post_req(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header("X-Auth-Token", "test-token")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn test_post_favorite_upsert() {
    let app = new_test_app().await;
    let router = serve_mux(app.state.clone());
    let body = r#"{"id":"giphy-abc123","url":"https://media1.giphy.com/media/abc123/giphy.gif","preview":"https://media1.giphy.com/media/abc123/200.gif","provider":"giphy","title":"funny cat"}"#;
    let (status, _, item) = send(&router, post_req("/api/v1/favorites", body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(item["id"], "giphy-abc123");
    assert_eq!(
        item["url"],
        "https://media1.giphy.com/media/abc123/giphy.gif"
    );
    assert_eq!(
        item["preview"],
        "https://media1.giphy.com/media/abc123/200.gif"
    );
    assert_eq!(item["provider"], "giphy");
    assert_eq!(item["title"], "funny cat");
    assert_eq!(item["use_count"], 0);
    assert_eq!(item["last_used"], Value::Null);
    assert!(item["added_at"].as_str().is_some());

    // Re-save preserves added_at and use_count.
    let (_, _, patch) = send(
        &router,
        Request::patch("/api/v1/favorites/giphy-abc123/use")
            .header("X-Auth-Token", "test-token")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(patch["use_count"], 1);
    assert!(patch["last_used"].as_str().is_some());

    let resave = r#"{"id":"giphy-abc123","url":"https://example.com/new.gif"}"#;
    let (status, _, item) = send(&router, post_req("/api/v1/favorites", resave)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(item["use_count"], 1);
    assert_eq!(item["last_used"], patch["last_used"]);
    assert_eq!(item["added_at"], item["added_at"]);
    assert_eq!(item["title"], "");
}

#[tokio::test]
async fn test_post_favorite_validation() {
    let app = new_test_app().await;
    let router = serve_mux(app.state.clone());

    let (status, _, json) = send(&router, post_req("/api/v1/favorites", "not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "invalid JSON body");

    let (status, _, json) = send(
        &router,
        post_req("/api/v1/favorites", r#"{"id":"a","url":"ftp://x"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "url must start with http:// or https://");

    let (status, _, json) = send(
        &router,
        post_req("/api/v1/favorites", r#"{"url":"https://x.com/a.gif"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "id and url are required");
}

#[tokio::test]
async fn test_use_favorite() {
    let app = new_test_app().await;
    seed(&app, "giphy-abc123", 4, 100).await;
    let router = serve_mux(app.state.clone());

    let (status, _, json) = send(
        &router,
        Request::patch("/api/v1/favorites/giphy-abc123/use")
            .header("X-Auth-Token", "test-token")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], "giphy-abc123");
    assert_eq!(json["use_count"], 5);
    assert!(json["last_used"].as_str().is_some());

    // Unknown id -> 404.
    let (status, _, json) = send(
        &router,
        Request::patch("/api/v1/favorites/missing/use")
            .header("X-Auth-Token", "test-token")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"], "favorite not found");
}

#[tokio::test]
async fn test_delete_favorite() {
    let app = new_test_app().await;
    seed(&app, "giphy-abc123", 1, 100).await;
    let router = serve_mux(app.state.clone());

    let req = Request::delete("/api/v1/favorites/giphy-abc123")
        .header("X-Auth-Token", "test-token")
        .body(Body::empty())
        .unwrap();
    let (status, headers, json) = send(&router, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(json, Value::Null);

    // Gone now.
    let (_, _, items) = get(&app, "/api/v1/favorites").await;
    assert!(items.is_empty());

    // Unknown id -> 404.
    let req = Request::delete("/api/v1/favorites/giphy-abc123")
        .header("X-Auth-Token", "test-token")
        .body(Body::empty())
        .unwrap();
    let (status, _, json) = send(&router, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"], "favorite not found");
}

#[tokio::test]
async fn test_method_not_allowed() {
    let app = new_test_app().await;
    let router = serve_mux(app.state.clone());

    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/favorites")
        .header("X-Auth-Token", "test-token")
        .body(Body::empty())
        .unwrap();
    let (status, _, json) = send(&router, req).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(json["error"], "method not allowed");

    // Unknown route -> 404 JSON.
    let req = Request::get("/nope")
        .header("X-Auth-Token", "test-token")
        .body(Body::empty())
        .unwrap();
    let (status, _, json) = send(&router, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json["error"].is_string());
}
