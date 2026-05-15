#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::{routing::get, Json, Router};
use serde::Serialize;
use tokio::net::UnixListener;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_SHA: &str = match option_env!("BUILD_SHA") {
    Some(sha) => sha,
    None => "unknown",
};
pub const BACKEND: &str = "hello-world";

#[derive(Clone)]
pub struct AppState {
    pub started_at: Instant,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_ms: u128,
}

#[derive(Serialize)]
struct VersionResponse {
    version: &'static str,
    build: &'static str,
    backend: &'static str,
}

async fn healthz(axum::extract::State(state): axum::extract::State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: VERSION,
        uptime_ms: state.started_at.elapsed().as_millis(),
    })
}

async fn version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: VERSION,
        build: BUILD_SHA,
        backend: BACKEND,
    })
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .with_state(state)
}

pub async fn shutdown_signal(socket_path: PathBuf) {
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = sigterm.recv() => info!("received SIGTERM"),
        _ = sigint.recv()  => info!("received SIGINT"),
    }
    if socket_path.exists() {
        if let Err(e) = std::fs::remove_file(&socket_path) {
            warn!(?socket_path, ?e, "failed to remove socket file during shutdown");
        } else {
            info!(?socket_path, "removed socket file");
        }
    }
}

pub async fn run(socket_path: PathBuf) -> Result<()> {
    if socket_path.exists() {
        warn!(?socket_path, "removing stale socket file");
        std::fs::remove_file(&socket_path).context("remove stale socket")?;
    }

    let listener = UnixListener::bind(&socket_path).context("bind unix listener")?;
    info!(?socket_path, "listening on unix socket");

    let state = AppState { started_at: Instant::now() };
    let app = build_router(state);

    let shutdown = shutdown_signal(socket_path.clone());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("axum serve")?;
    Ok(())
}
