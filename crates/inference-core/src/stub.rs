use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::backend::{ModelInfo, SttBackend, SttError, SttOptions, Transcript};

/// Test/CI backend. Returns a deterministic string and never loads a model.
/// Optionally sleeps to simulate slow inference (used by concurrency tests).
pub struct StubBackend {
    sleep: Duration,
}

impl StubBackend {
    pub fn new() -> Self {
        let ms = std::env::var("SIDECAR_STUB_SLEEP_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Self { sleep: Duration::from_millis(ms) }
    }
}

impl Default for StubBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SttBackend for StubBackend {
    async fn transcribe(&self, samples: Vec<f32>, _opts: SttOptions) -> Result<Transcript, SttError> {
        let started = Instant::now();
        if !self.sleep.is_zero() {
            tokio::time::sleep(self.sleep).await;
        }
        Ok(Transcript {
            text: format!("[stub] {} samples", samples.len()),
            language: "stub".to_string(),
            duration_ms: u32::try_from(samples.len() * 1000 / 16_000).unwrap_or(u32::MAX),
            processing_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            model: "stub".to_string(),
            backend: "stub",
            segments: None,
        })
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: "stub".to_string(),
            kind: "stt",
            backend: "stub",
            path: PathBuf::from("(none)"),
            coreml: false,
            loaded: true,
        }
    }
}
