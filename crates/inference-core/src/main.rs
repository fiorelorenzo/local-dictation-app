#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod backend;
mod audio;
mod server;
mod stub;
mod wire;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::backend::SttBackendHandle;
use crate::stub::StubBackend;

fn socket_path_from_env() -> Result<PathBuf> {
    let path = env::var("SIDECAR_SOCKET_PATH")
        .context("SIDECAR_SOCKET_PATH env var is required")?;
    Ok(PathBuf::from(path))
}

/// Picks the STT backend at startup based on env.
/// Precedence: `SIDECAR_STT_BACKEND=stub` > whisper (real backend lands in T11).
fn load_stt_backend() -> Option<SttBackendHandle> {
    let kind = env::var("SIDECAR_STT_BACKEND").unwrap_or_else(|_| "whisper".to_string());
    match kind.as_str() {
        "stub" => Some(Arc::new(StubBackend::new()) as SttBackendHandle),
        "whisper" => {
            // WhisperBackend wired in T11. Until then, fall back to None when no model.
            None
        }
        other => {
            tracing::warn!(backend=%other, "unknown SIDECAR_STT_BACKEND, ignoring");
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = env::var("SIDECAR_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();

    let socket_path = socket_path_from_env()?;
    let stt = load_stt_backend();
    server::run(socket_path, stt).await
}
