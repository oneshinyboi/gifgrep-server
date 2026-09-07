mod db;
mod routes;
#[cfg(test)]
mod tests;

use axum::serve;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, oneshot};

use routes::{AppState, serve_mux};

fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => default.to_string(),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .init();

    let token = std::env::var("FAV_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("gif-favs: FAV_TOKEN is required; refusing to start");
        std::process::exit(1);
    }
    let port = env_or("FAV_PORT", "8099");
    let host = env_or("FAV_HOST", "0.0.0.0");
    let db_path = env_or("FAV_DB_PATH", "./favorites.db");

    let pool = match db::open_db(&db_path).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("gif-favs: open db: {err}");
            std::process::exit(1);
        }
    };

    let addr = format!("{host}:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(err) => {
            eprintln!("gif-favs: server: {err}");
            std::process::exit(1);
        }
    };
    let app = serve_mux(AppState { pool, token });
    tracing::info!("gif-favs: listening on {addr} (db={db_path})");

    // SIGINT/SIGTERM -> graceful shutdown, mirroring the Go server: a hard
    // 5s deadline, after which the process force-exits.
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let (sig_tx, mut sig_rx) = mpsc::channel::<()>(1);
    let (done_tx, mut done_rx) = oneshot::channel();

    tokio::spawn(async move {
        let shutdown = async move {
            tokio::select! {
                _ = sigint.recv() => {},
                _ = sigterm.recv() => {},
            }
            tracing::info!("gif-favs: shutting down");
            let _ = sig_tx.send(()).await;
        };
        let res = serve(listener, app).with_graceful_shutdown(shutdown).await;
        let _ = done_tx.send(res);
    });

    tokio::select! {
        res = &mut done_rx => {
            if let Err(err) = res.unwrap_or_else(|e| Err(std::io::Error::other(e))) {
                eprintln!("gif-favs: server: {err}");
                std::process::exit(1);
            }
        }
        _ = async {
            let _ = sig_rx.recv().await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            eprintln!("gif-favs: forced exit");
            std::process::exit(1);
        } => {}
    }
}
