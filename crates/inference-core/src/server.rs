#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::UnixListener;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};

use crate::audio;
use crate::backend::{SttBackendHandle, SttError, SttOptions};
use crate::wire::{error_response, ErrorBody, Wire, WireResponse};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_SHA: &str = match option_env!("BUILD_SHA") {
    Some(sha) => sha,
    None => "unknown",
};
pub const BACKEND_NAME: &str = "whisper-rs";

const MAX_BODY_BYTES: usize = 50 * 1024 * 1024; // 50 MiB

#[derive(Clone)]
pub struct AppState {
    pub started_at: Instant,
    pub stt: Option<SttBackendHandle>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_ms: u128,
    stt_ready: bool,
}

#[derive(Serialize)]
struct VersionResponse {
    version: &'static str,
    build: &'static str,
    backend: &'static str,
}

async fn healthz(headers: HeaderMap, State(state): State<AppState>) -> WireResponse<HealthResponse> {
    WireResponse::ok(
        Wire::from_accept(&headers),
        HealthResponse {
            status: "ok",
            version: VERSION,
            uptime_ms: state.started_at.elapsed().as_millis(),
            stt_ready: state.stt.is_some(),
        },
    )
}

async fn version(headers: HeaderMap) -> WireResponse<VersionResponse> {
    WireResponse::ok(
        Wire::from_accept(&headers),
        VersionResponse {
            version: VERSION,
            build: BUILD_SHA,
            backend: BACKEND_NAME,
        },
    )
}

#[derive(Debug, Deserialize, Default)]
pub struct SttQuery {
    pub language: Option<String>,
    #[serde(default)]
    pub translate: bool,
    #[serde(default)]
    pub segments: bool,
}

async fn stt(
    headers: HeaderMap,
    Query(q): Query<SttQuery>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let wire = Wire::from_accept(&headers);

    // Content-Type check
    let ct_ok = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_ascii_lowercase().starts_with("audio/wav"))
        .unwrap_or(false);
    if !ct_ok {
        return error_response(
            wire,
            StatusCode::BAD_REQUEST,
            "bad_audio",
            "Content-Type must be audio/wav",
        )
        .into_response();
    }
    if body.len() > MAX_BODY_BYTES {
        return error_response(
            wire,
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            format!("body {} bytes exceeds {MAX_BODY_BYTES}", body.len()),
        )
        .into_response();
    }

    let Some(stt_handle) = state.stt.clone() else {
        return error_response(
            wire,
            StatusCode::SERVICE_UNAVAILABLE,
            "stt_unavailable",
            "model not loaded",
        )
        .into_response();
    };

    // Audio processing is CPU-bound: run on the blocking pool.
    let body_vec = body.to_vec();
    let samples = match tokio::task::spawn_blocking(move || audio::process_wav(&body_vec)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return stt_error_to_response(wire, e).into_response(),
        Err(join_err) => {
            return error_response(
                wire,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("audio task panicked: {join_err}"),
            )
            .into_response();
        }
    };

    let opts = SttOptions {
        language: q.language,
        translate: q.translate,
        want_segments: q.segments,
    };
    match stt_handle.transcribe(samples, opts).await {
        Ok(transcript) => WireResponse::ok(wire, transcript).into_response(),
        Err(e) => stt_error_to_response(wire, e).into_response(),
    }
}

fn stt_error_to_response(wire: Wire, err: SttError) -> WireResponse<ErrorBody> {
    let (status, code) = match err {
        SttError::AudioDecode(_) => (StatusCode::BAD_REQUEST, "bad_audio"),
        SttError::AudioUnsupported(_) => (StatusCode::BAD_REQUEST, "unsupported_audio"),
        SttError::ModelNotLoaded => (StatusCode::SERVICE_UNAVAILABLE, "stt_unavailable"),
        SttError::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
        SttError::Resample(_) | SttError::Whisper(_) | SttError::Internal(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ),
    };
    error_response(wire, status, code, err.to_string())
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/v1/stt", axum::routing::post(stt))
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
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

pub async fn run(socket_path: PathBuf, stt: Option<SttBackendHandle>) -> Result<()> {
    if socket_path.exists() {
        warn!(?socket_path, "removing stale socket file");
        std::fs::remove_file(&socket_path).context("remove stale socket")?;
    }

    let listener = UnixListener::bind(&socket_path).context("bind unix listener")?;
    info!(?socket_path, "listening on unix socket");

    let state = AppState { started_at: Instant::now(), stt };
    let app = build_router(state);

    let shutdown = shutdown_signal(socket_path.clone());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("axum serve")?;
    Ok(())
}
