#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod server;
// `backend` is unused until T6 (StubBackend) — silenced to avoid dead_code error under clippy::pedantic
#[allow(dead_code)]
mod backend;
// `wire` is unused by handlers until T5 — silenced to avoid dead_code error under clippy::pedantic
#[allow(dead_code)]
mod wire;

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

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
    server::run(socket_path).await
}
