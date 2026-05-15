use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SttError {
    #[error("audio decode failed: {0}")]
    AudioDecode(String),
    #[error("unsupported audio: {0}")]
    AudioUnsupported(String),
    #[error("resample failed: {0}")]
    Resample(String),
    #[error("model not loaded")]
    ModelNotLoaded,
    #[error("backend busy (mutex timeout)")]
    Busy,
    #[error("whisper inference failed: {0}")]
    Whisper(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Default)]
pub struct SttOptions {
    pub language: Option<String>,
    pub translate: bool,
    pub want_segments: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Transcript {
    pub text: String,
    pub language: String,
    pub duration_ms: u32,
    pub processing_ms: u32,
    pub model: String,
    pub backend: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub kind: &'static str,
    pub backend: &'static str,
    pub path: PathBuf,
    pub coreml: bool,
    pub loaded: bool,
}

#[async_trait]
pub trait SttBackend: Send + Sync + 'static {
    async fn transcribe(&self, samples: Vec<f32>, opts: SttOptions) -> Result<Transcript, SttError>;
    fn model_info(&self) -> ModelInfo;
}

pub type SttBackendHandle = Arc<dyn SttBackend>;
