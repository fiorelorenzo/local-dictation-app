// whisper-rs 0.16 API notes (deviations from the plan's 0.13-era pseudocode):
//
//  1. WhisperContextParameters uses setter methods: params.use_gpu(true) — not a field.
//  2. The `accelerate` feature was dropped; metal + coreml cover macOS arm64.
//  3. full_n_segments() returns c_int directly (no Result wrapping).
//  4. full_lang_id_from_state() returns c_int directly (no Result wrapping).
//  5. Segments are obtained via state.get_segment(i) → Option<WhisperSegment>;
//     text via seg.to_str(), timestamps via seg.start_timestamp() / seg.end_timestamp().
//     The old full_get_segment_text / full_get_segment_t0 / full_get_segment_t1 are gone.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::backend::{ModelInfo, Segment, SttBackend, SttError, SttOptions, Transcript};

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

pub struct WhisperBackend {
    ctx: Arc<Mutex<WhisperContext>>,
    model_path: PathBuf,
    model_id: String,
    coreml_enabled: bool,
}

impl WhisperBackend {
    pub fn load(model_path: PathBuf) -> Result<Self, SttError> {
        let coreml_path = coreml_sidecar_path(&model_path);
        let coreml_present = coreml_path.exists();
        let coreml_disabled = std::env::var("SIDECAR_WHISPER_COREML_DISABLE").is_ok();
        let coreml_enabled = coreml_present && !coreml_disabled;

        // 0.16: WhisperContextParameters uses setter methods, not struct fields.
        let mut params = WhisperContextParameters::new();
        params.use_gpu(true);

        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or_else(|| {
                SttError::Internal("model path not valid UTF-8".to_string())
            })?,
            params,
        )
        .map_err(|e| SttError::Whisper(format!("load model: {e}")))?;

        info!(?model_path, coreml_enabled, "whisper model loaded");
        if coreml_present && coreml_disabled {
            warn!(
                "coreml encoder found at {coreml_path:?} but SIDECAR_WHISPER_COREML_DISABLE is set"
            );
        }

        let model_id = model_path
            .file_stem()
            .map_or_else(|| "whisper".to_string(), |s| s.to_string_lossy().to_string());

        Ok(Self {
            ctx: Arc::new(Mutex::new(ctx)),
            model_path,
            model_id,
            coreml_enabled,
        })
    }
}

pub fn coreml_sidecar_path(model_path: &Path) -> PathBuf {
    let stem = model_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let parent = model_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}-encoder.mlmodelc"))
}

#[async_trait]
impl SttBackend for WhisperBackend {
    #[allow(clippy::cast_sign_loss)] // segment counts and indices are always non-negative
    async fn transcribe(
        &self,
        samples: Vec<f32>,
        opts: SttOptions,
    ) -> Result<Transcript, SttError> {
        let started = Instant::now();
        let model_id = self.model_id.clone();

        // Acquire the async mutex with a timeout. The guard's lifetime is tied to `self`,
        // so we keep it in scope and use `tokio::task::block_in_place` to run the
        // CPU-heavy whisper inference without moving the guard into spawn_blocking.
        // (Requires the multi-threaded tokio runtime, which we already use.)
        let ctx_guard = tokio::time::timeout(LOCK_TIMEOUT, self.ctx.lock())
            .await
            .map_err(|_| SttError::Busy)?;

        #[allow(clippy::cast_possible_truncation)]
        let duration_ms = u32::try_from(samples.len() * 1000 / 16_000).unwrap_or(u32::MAX);

        let (text, language, segments) = tokio::task::block_in_place(|| {
            // 0.16 API: create_state, full(), full_n_segments(), get_segment(i)
            let mut state = ctx_guard
                .create_state()
                .map_err(|e| SttError::Whisper(format!("create state: {e}")))?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            // set_language takes Option<&str>; use as_deref() to convert Option<String>.
            params.set_language(opts.language.as_deref());
            params.set_translate(opts.translate);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);

            state
                .full(params, &samples)
                .map_err(|e| SttError::Whisper(format!("full: {e}")))?;

            // 0.16: full_n_segments() returns c_int directly, no Result.
            let n = state.full_n_segments();

            let mut text = String::new();
            let mut segs: Vec<Segment> = Vec::new();
            for i in 0..n {
                // 0.16: get_segment(i) returns Option<WhisperSegment>.
                let seg = state.get_segment(i).ok_or_else(|| {
                    SttError::Whisper(format!("segment {i} out of range"))
                })?;
                let seg_text = seg
                    .to_str()
                    .map_err(|e| SttError::Whisper(format!("segment text: {e}")))?;
                text.push_str(seg_text);
                if opts.want_segments {
                    // start_timestamp / end_timestamp return i64 centiseconds.
                    let t0 = seg.start_timestamp();
                    let t1 = seg.end_timestamp();
                    segs.push(Segment {
                        start_ms: ms_from_centiseconds(t0),
                        end_ms: ms_from_centiseconds(t1),
                        text: seg_text.to_owned(),
                    });
                }
            }

            // 0.16: full_lang_id_from_state() returns c_int directly, no Result.
            // get_lang_str(id) remains stable: Option<&'static str>.
            let lang_id = state.full_lang_id_from_state();
            let language = whisper_rs::get_lang_str(lang_id)
                .map_or_else(|| "und".to_string(), ToString::to_string);

            Ok::<(String, String, Vec<Segment>), SttError>((
                text.trim().to_string(),
                language,
                segs,
            ))
        })?;

        Ok(Transcript {
            text,
            language,
            duration_ms,
            processing_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            model: model_id,
            backend: "whisper-rs",
            segments: if opts.want_segments { Some(segments) } else { None },
        })
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.model_id.clone(),
            kind: "stt",
            backend: "whisper-rs",
            path: self.model_path.clone(),
            coreml: self.coreml_enabled,
            loaded: true,
        }
    }
}

fn ms_from_centiseconds(cs: i64) -> u32 {
    if cs < 0 {
        return 0;
    }
    let ms = cs.saturating_mul(10);
    u32::try_from(ms).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn coreml_sidecar_path_uses_encoder_mlmodelc_suffix() {
        let p = coreml_sidecar_path(Path::new("/models/ggml-large-v3-turbo.bin"));
        assert_eq!(
            p,
            PathBuf::from("/models/ggml-large-v3-turbo-encoder.mlmodelc")
        );
    }

    #[test]
    fn coreml_sidecar_path_handles_no_parent() {
        let p = coreml_sidecar_path(Path::new("ggml-tiny.bin"));
        assert_eq!(p, PathBuf::from("ggml-tiny-encoder.mlmodelc"));
    }
}
