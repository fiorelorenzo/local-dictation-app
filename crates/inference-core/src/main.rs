// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Lorenzo Fiore

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::{routing::get, Json, Router};
use serde::Serialize;
use tokio::net::UnixListener;
use tracing::{info, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_SHA: &str = match option_env!("BUILD_SHA") {
    Some(sha) => sha,
    None => "unknown",
};
const BACKEND: &str = "hello-world";

#[derive(Clone)]
struct AppState {
    started_at: Instant,
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

fn socket_path_from_env() -> Result<PathBuf> {
    let path = env::var("SIDECAR_SOCKET_PATH")
        .context("SIDECAR_SOCKET_PATH env var is required")?;
    Ok(PathBuf::from(path))
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = env::var("SIDECAR_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();

    let socket_path = socket_path_from_env()?;
    if socket_path.exists() {
        warn!(?socket_path, "removing stale socket file");
        std::fs::remove_file(&socket_path).context("remove stale socket")?;
    }

    let listener = UnixListener::bind(&socket_path).context("bind unix listener")?;
    info!(?socket_path, "listening on unix socket");

    let state = AppState { started_at: Instant::now() };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .with_state(state);

    axum::serve(listener, app).await.context("axum serve")?;
    Ok(())
}
