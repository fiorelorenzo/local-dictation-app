# Local Dictation App - Architecture Design (v1)

**Status:** Draft
**Date:** 2026-05-15
**Scope:** High-level architecture decisions for a brand-new, fully-local, open-source dictation app inspired by FreeFlow/Wispr Flow/Superwhisper. This document fixes the architecture and decomposes the project into 5 sub-projects; each sub-project will receive its own implementation spec later.

---

## 1. Goals & non-goals

### Goals (v1)

- **Fully local inference.** No external services (no Ollama, no cloud APIs, no Groq). Audio and text never leave the user's machine.
- **Cross-platform target.** Architecture must support macOS, Linux, and Windows. v1 ships macOS only.
- **Maximum performance.** Every available platform accelerator is used (Metal, Apple Neural Engine, MLX on Apple Silicon; CUDA/Vulkan/DirectML on other platforms in v2).
- **Streaming UX.** Live partial transcript appears while speaking; cleanup output streams token-by-token into the target text field.
- **Open source.** Permissive license (Apache-2.0 or MIT, TBD).
- **Feature parity with FreeFlow's core.** Hold-to-talk dictation, context-aware cleanup, configurable hotkeys, custom system prompt, settings, model manager. Edit Mode included in v1.

### Non-goals (v1)

- Context-aware screenshot inference (vision input) - deferred to v2.
- Custom vocabulary editor - deferred to v2 (the cleanup LLM already handles common spelling fixes).
- Linux and Windows builds - architecture is OS-abstracted from day one, but only macOS implementations land in v1.
- Mobile platforms.
- Any cloud sync, account, telemetry, or analytics.

### Localization stance (important)

v1 ships with **English UI only**. The codebase is structured from day one with full i18n infrastructure (`i18next` + `svelte-i18next`, all UI strings extracted to JSON resource files under `app/src/locales/<lang>/translation.json`, locale detection at startup via `app.getLocale()`, no hard-coded user-facing strings). Adding additional locales in v1.x or v2 is a translation effort, not a code change. The cleanup LLM itself is language-agnostic (handles any language Whisper transcribes).

---

## 2. Architecture overview

### Runtime topology

```
┌─────────────────────────────────────────────────────────────────┐
│                        DESKTOP APP (one bundle)                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Electron MAIN process (Node.js + TypeScript)             │  │
│  │  - app lifecycle (start/stop sidecar, app menu, tray)     │  │
│  │  - native addon: global hotkey, paste, permissions        │  │
│  │  - settings persistence (electron-store)                  │  │
│  │  - IPC bridge to renderer                                 │  │
│  └────────────────┬────────────────────────┬─────────────────┘  │
│                   │ ipcMain                │ stdio + UNIX sock  │
│  ┌────────────────▼─────────────┐    ┌─────▼──────────────────┐ │
│  │  Electron RENDERER (Svelte)  │    │  RUST SIDECAR          │ │
│  │  - Settings window           │    │  inference-core        │ │
│  │  - Model manager UI          │    │  - whisper-rs          │ │
│  │  - Live transcript overlay   │    │  - llama-cpp-2         │ │
│  │  - Setup wizard              │    │  - audio decode (hound)│ │
│  └──────────────────────────────┘    │  - VAD (silero-vad-rs) │ │
│                                      │  - HTTP/MsgPack server │ │
│  ┌──────────────────────────────┐    └────────────────────────┘ │
│  │  MLX SIDECAR (Python, opt-in)│                               │
│  │  - python-build-standalone   │                               │
│  │  - mlx-lm + mlx-whisper      │                               │
│  │  - FastAPI server            │                               │
│  └──────────────────────────────┘                               │
└─────────────────────────────────────────────────────────────────┘
                                              │
                       ┌──────────────────────┴────────────────┐
                       │   ~/Library/Application Support/      │
                       │   <appname>/models/                   │
                       │   - whisper/*.bin (+ .mlmodelc)       │
                       │   - llm/*.gguf                        │
                       │   - mlx/*  (MLX-format models)        │
                       └───────────────────────────────────────┘
```

### Why this shape

- **Electron for UI:** mature, cross-platform, large ecosystem. The 120-180 MB cost is acceptable for a desktop app.
- **Rust sidecar for inference:** isolates native crashes from the UI, allows shipping platform-specific accelerator builds without rebuilding Electron, gives a clean process boundary for resource accounting.
- **napi-rs native addon for OS integration:** the small set of system calls (hotkey, paste, permissions) need to share a process with the Electron main, so they're a Rust crate compiled to a `.node` addon.
- **MLX as separate Python sidecar:** the Rust MLX bindings are not yet production-ready in 2026; isolating MLX in a Python process keeps the option open without coupling to Python in the main inference path.

### Sub-project decomposition

| # | Sub-project | Language | Location | Responsibility |
|---|---|---|---|---|
| 1 | `inference-core` | Rust | `crates/inference-core` | Sidecar daemon: model loading, streaming STT (whisper-rs + VAD), streaming chat completion (llama-cpp-2), HTTP/MsgPack API on UNIX socket |
| 2 | `audio-capture` | Rust | `crates/audio-capture` | Cross-platform microphone capture (cpal), VAD chunking, push stream to `inference-core` |
| 3 | `os-integration` | Rust + napi-rs | `crates/os-integration` | OS abstraction: global hotkey, paste-at-cursor, permission checks, audio device enumeration. Exposed to Electron via N-API |
| 4 | `app-shell` | TypeScript + Svelte | `app/` | Electron main + renderer: settings UI, model manager, overlay window, hotkey wiring, sidecar lifecycle, i18n |
| 5 | `model-manager` | Rust + TypeScript | `crates/model-manager` + `app/src/lib/models/` | Model download (HuggingFace), integrity check (SHA-256), storage management, hot-swap |

Each sub-project will have its own implementation spec (separate brainstorming session) and can be built largely independently.

### Module boundaries (strict)

- `inference-core` does not know that Electron exists. It only exposes a socket API.
- `audio-capture` talks to `inference-core` via the same socket (push audio bytes; receive partial transcripts via streaming events).
- `os-integration` is "the Electron main process's hands." It never calls `inference-core` directly; everything is orchestrated by the Electron main.
- `app-shell` orchestrates: spawns the sidecar, holds the socket connection, reacts to events from `os-integration`, forwards work to `inference-core`, distributes streaming events to the renderer and to `os-integration` for paste.
- `model-manager` can run standalone (future CLI). Electron uses it via N-API or via a dedicated socket endpoint.

### Repository

Single monorepo with `Cargo workspace` for the Rust crates and `npm workspaces` for the Electron app. Build orchestrated via `just` (Justfile).

---

## 3. Streaming pipeline

### Standard dictation sequence (hold-to-talk)

```
t=0    USER presses hotkey (Fn)
       │
       ▼ os-integration emits "hotkey_press"
t=10ms MAIN process:
       - spawns overlay window (borderless, click-through, top-of-screen)
       - tells audio-capture: start(48 kHz mic → resample to 16 kHz mono PCM)
       - tells inference-core: POST /v1/stt/stream (open streaming channel)
       │
t=50ms USER starts speaking
       │
       ▼ audio-capture: 20 ms PCM frames → inference-core socket
t=100..2000ms (while speaking)
       │
       inference-core loop (every 200 ms or on silence-end VAD):
       1. VAD (silero-vad-rs) classifies frame: speech vs silence
       2. When silence > 400 ms after speech → commit chunk
       3. whisper-rs decode chunk (large-v3-turbo, language=auto)
       4. emit "partial_transcript" {text, segment_id, confidence}
       │
       ▼ MAIN receives partial → ipcMain.send to renderer overlay
       OVERLAY: updates text with join of all accumulated segments
       │
t=3500ms USER releases hotkey
       │
       ▼ os-integration emits "hotkey_release"
       MAIN:
       - audio-capture.stop()
       - inference-core: PUT /v1/stt/stream/{session_id}/finalize
       │
       inference-core:
       - decodes FINAL pass on full audio (best accuracy, not only the
         last 200 ms chunk that the streaming path produced)
       - emit "final_transcript" {text}
       │
       ▼ MAIN:
       - overlay shows "Cleaning..." (placeholder, ~150 ms)
       - POST /v1/chat/completions {stream: true, model, system_prompt,
         messages: [{role: user, content: transcript}]}
       │
       inference-core streams cleanup tokens (llama-cpp-2 streaming):
       - "delta": "Allora "
       - "delta": "oggi "
       - "delta": "devo "
       - ...
       │
       ▼ MAIN: on each delta:
       - if first delta: overlay.hide()
       - os-integration.pasteIncremental(delta) → writes at cursor
       │
t=4500ms STT done, first paste token visible in target
t=5500ms cleanup finished, full text pasted, overlay hidden
```

### MAIN process state machine

```
   IDLE
    │ hotkey_press
    ▼
RECORDING ──(release)──▶ FINALIZING ──▶ CLEANUP_STREAMING ──▶ IDLE
    │                       │                   │
    │ (esc / 5 min timeout) │ (error)           │ (error)
    ▼                       ▼                   ▼
  ABORTED ─────────────▶ IDLE              IDLE + paste raw transcript
```

### Edit Mode differs

```
RECORDING (no overlay text, only "listening" icon)
    │ release
    ▼
LLM_TRANSFORM (read selected text via os-integration.focused_selection)
    │
    inference-core: chat completion stream with system prompt =
    "Transform SELECTED_TEXT according to VOICE_COMMAND"
    + user message = {selected: "...", command: "..."}
    │
    ▼
TYPING (incremental replace selection via os-integration.replace_focused_selection)
```

### Backpressure and limits

- **Max recording duration:** 5 minutes hard cap. Beyond, auto-stop with warning. Prevents OOM on infinite audio.
- **Max audio buffer in inference-core:** ring buffer of 6 min @ 16 kHz mono PCM = ~11 MB.
- **VAD chunk min/max:** min 200 ms (no decode on brief noise), max 8 s (force commit on long monologue).
- **Cleanup token streaming:** if llama-cpp-2 produces tokens faster than the paste throughput (rare), `pasteIncremental` has an internal buffer that flushes every 50 ms or 5 tokens, whichever first.

### IPC protocol (Electron MAIN ↔ Rust sidecar)

UNIX socket locally (macOS/Linux: `~/Library/Application Support/<app>/inference.sock`; Windows: named pipe `\\.\pipe\<app>-inference`). On top: HTTP/1.1 with MsgPack request/response bodies, SSE for streaming events.

**Minimal v1 API:**

- `POST /v1/stt/stream` - open SSE-like channel
- `POST /v1/stt/stream/{session_id}/frames` - push audio frames
- `PUT /v1/stt/stream/{session_id}/finalize` - close and run final pass
- `POST /v1/chat/completions` - OpenAI-compatible schema (reusable by future CLI clients)
- `GET /v1/models` - list installed models
- `POST /v1/models/download` - download with SSE progress
- `DELETE /v1/models/{name}` - remove model
- `GET /healthz` - liveness

### Audio device handling

- Default: system input (cpal `default_input_device()`).
- Settings: dropdown of available input devices.
- Hot-swap during recording: graceful error with notification.
- Echo cancellation: not in v1 (cpal raw mode). v2: optional `webrtc-audio-processing`.

### Edge cases

| Edge | Handling |
|---|---|
| Whisper model not loaded | Overlay shows "Loading model...", auto-pull from model manager if model is default, prompt otherwise |
| inference-core down / crashed | MAIN auto-restarts sidecar, current request fails gracefully |
| Audio device disconnected mid-recording | Stop + notification, partial transcript still cleaned |
| LLM cleanup fails | Fallback: paste raw transcript (no cleanup), tray notify error |
| Accidental hotkey (< 200 ms) | Filtered as false positive, no overlay |
| Target app refuses accessibility paste | Warn once, suggest enabling permissions |

---

## 4. OS abstraction layer

### Three system touchpoints in v1 (screenshot deferred to v2)

```
                   ┌─────────────────────────────┐
                   │  os-integration (Rust crate)│
                   │  exposed via napi-rs to Node│
                   └──────┬──────────────────────┘
                          │
              ┌───────────┼────────────────┬─────────────┐
              ▼           ▼                ▼             ▼
        HotkeyBackend  TextBackend   PermissionsBackend  AudioDevices
              │           │                │             │
              │           │                │             └─ cpal in
              │           │                │                Rust, no
              │           │                │                OS-specific
              │           │                │
   ┌──────────┴──┐  ┌─────┴────────┐  ┌────┴──────────┐
   │ Mac (v1)    │  │ Mac (v1)     │  │ Mac (v1)      │
   │ CGEventTap +│  │ AXUIElement  │  │ TCC + AppKit  │
   │ NSEvent.add │  │ + NSPasteboard│  │ status check  │
   │ GlobalMon   │  │ + CGEvent    │  │               │
   └─────────────┘  └──────────────┘  └───────────────┘
```

### Abstract interface (Rust traits)

```rust
// Crate: os-integration

pub trait HotkeyBackend {
    fn register(
        &self,
        binding: HotkeyBinding,
        on_event: Box<dyn Fn(HotkeyEvent) + Send + Sync>,
    ) -> Result<HotkeyId>;
    fn unregister(&self, id: HotkeyId) -> Result<()>;
}

pub enum HotkeyEvent { Press, Release }

pub trait TextBackend {
    fn focused_selection(&self) -> Result<Option<String>>;
    fn replace_focused_selection(&self, text: &str) -> Result<()>;
    fn insert_at_cursor(&self, text: &str) -> Result<()>;
}

pub trait PermissionsBackend {
    fn accessibility_status(&self) -> PermissionStatus;
    fn microphone_status(&self) -> PermissionStatus;
    fn prompt_accessibility(&self) -> Result<()>;
    fn prompt_microphone(&self) -> Result<()>;
}

pub enum PermissionStatus { Granted, Denied, NotDetermined }
```

### Mac v1 concrete implementations

| Trait | Crate / API | Notes |
|---|---|---|
| `HotkeyBackend` | `core-graphics::event::CGEventTap` on key event stream + filter for Fn keycode (63) | Requires Accessibility permission. For `Cmd+Fn` parse modifiers via `CGEventFlags` |
| `TextBackend` (preferred) | `accessibility-sys` → `AXUIElementCreateSystemWide` → `kAXFocusedUIElementAttribute` → `kAXSelectedTextAttribute` get/set | In-place edit, no clipboard pollution |
| `TextBackend` (fallback) | `objc2-app-kit::NSPasteboard` (preserving previous contents) + `CGEventCreateKeyboardEvent` for `Cmd+V` | For apps that ignore AX API (some Electron apps, certain JetBrains IDE modes). Restore clipboard after 100 ms |
| `PermissionsBackend` | `core-foundation` + `AXIsProcessTrustedWithOptions` + `AVCaptureDevice.authorizationStatus` | `prompt_accessibility` opens `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` |

### N-API export to Node

```rust
#[napi]
pub fn register_hotkey(
    binding: HotkeyBindingJs,
    callback: ThreadsafeFunction<HotkeyEventJs>,
) -> Result<u32> { /* ... */ }

#[napi]
pub fn focused_selection() -> Result<Option<String>> { /* ... */ }

#[napi]
pub fn insert_at_cursor(text: String) -> Result<()> { /* ... */ }

#[napi]
pub fn check_permissions() -> PermissionsJs { /* ... */ }
```

Build: napi-rs produces `os-integration.<platform>-<arch>.node` (e.g. `os-integration.darwin-arm64.node`). electron-builder bundles into the `.app`.

### What does NOT live in os-integration

- Audio capture (cpal in `audio-capture` crate).
- Inference (separate sidecar).
- Arbitrary clipboard manipulation (only as `TextBackend` fallback).
- Screen capture (v2).
- Notifications (Electron's `Notification` API).

### v1 paying the abstraction cost up front

Even though only macOS implementations ship in v1, the traits and Linux/Windows stubs exist. When v2 begins:

- Implement 3 new struct types per OS (`HotkeyBackendLinux`, etc.).
- Conditional compile (`#[cfg(target_os = "linux")]`).
- Electron code does not change.

---

## 5. Inference backends and hardware acceleration

### Philosophy: pluggable backends, runtime selection

```
inference-core
    │
    ▼
┌─────────────────────────────────────────────────┐
│  trait Backend (Rust)                           │
│  - capabilities() → AccelInfo                   │
│  - load_whisper(...) → WhisperHandle            │
│  - load_llm(...) → LlmHandle                    │
│  - stream_stt(...) → Stream<Partial>            │
│  - stream_chat(...) → Stream<Token>             │
└──┬──────────────┬──────────────┬───────────────┬┘
   │              │              │               │
   ▼              ▼              ▼               ▼
GgmlMetalBackend  GgmlCoreMLBackend   MlxBackend   GgmlCudaBackend / Vulkan / DirectML
(Mac default)     (Mac whisper ANE)   (Mac v1)     (Win+Linux v2)
```

### Mac v1 includes EVERY Mac optimization

| Feature | v1 status |
|---|---|
| Metal GPU via ggml (whisper.cpp + llama.cpp) | Default |
| CoreML encoder for Whisper (Apple Neural Engine, 3x encoder speedup) | Default on M-series, auto-enabled in setup wizard |
| Accelerate / BNNS / AMX (ggml `-DGGML_ACCELERATE=ON -DGGML_BLAS=ON`) | Default |
| MLX backend (mlx-lm + mlx-whisper via Python sidecar) | Bundled in app, opt-in toggle in Settings |
| QoS `userInteractive` for sidecar | Default |
| Memory pressure handler (`DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`) | Default |
| Thermal state monitoring (`NSProcessInfo.thermalState`) | Default |
| Energy/Low Power Mode awareness (auto-switch to lite profile) | Default |

### Stack composition

```
inference-core (Rust) bundled per Mac arm64 / x86_64

whisper-rs (features: metal, coreml, accelerate)
  ggml backend matrix:
   - GGML_METAL=ON      (GPU, encoder+decoder)
   - GGML_COREML=ON     (ANE for encoder, 3x speedup)
   - GGML_ACCELERATE=ON (CPU fallback ops via BNNS/AMX)
   - GGML_BLAS=ON       (Accelerate BLAS for CPU path)

llama-cpp-2 (features: metal, accelerate)
  ggml backend matrix:
   - GGML_METAL=ON      (GPU full: matmul, attention)
   - GGML_ACCELERATE=ON
   - GGML_BLAS=ON

mlx-sidecar (Python embedded, opt-in)
  - Python 3.12 standalone (~30 MB)
  - mlx + mlx-lm + mlx-whisper wheels (~200 MB)
  - FastAPI + uvicorn
```

### CoreML encoder (default on M-series in v1)

The real free win on Apple Silicon. The ANE handles the Whisper encoder ~3x faster than the GPU.

Strategy: the catalog manifest includes both `ggml_url` (the `.bin`) and `coreml_url` (the `.mlmodelc.zip`) for each Whisper model. We download both for M-series machines. whisper.cpp auto-detects the `.mlmodelc` if it sits next to the `.bin` with the matching basename.

```json
{
  "id": "whisper-large-v3-turbo",
  "ggml_url": "https://huggingface.co/.../ggml-large-v3-turbo.bin",
  "ggml_sha256": "...",
  "coreml_url": "https://huggingface.co/.../ggml-large-v3-turbo-encoder.mlmodelc.zip",
  "coreml_sha256": "...",
  "size_bytes_ggml": 1610612736,
  "size_bytes_coreml": 173015040
}
```

Latency impact: Whisper encoder pass goes from ~200 ms to ~65 ms on M4.

### MLX backend (bundled in v1)

Architecture:

```
                                  ┌──────────────────────┐
                                  │  inference-core      │
   ┌─────────┐                    │  (Rust, ggml stack)  │
   │ Electron│  ─ chat/stt req ──▶│  port 21931 (sock)   │
   │  main   │                    └──────────────────────┘
   └────┬────┘
        │
        │ if Settings.backend == "mlx":
        │
        ▼
   ┌──────────────────────────────────┐
   │ mlx-sidecar (Python embedded)    │
   │ - python-build-standalone 3.12   │
   │ - mlx-lm, mlx-whisper            │
   │ - FastAPI server                 │
   │ - same API contract as           │
   │   inference-core                 │
   │                                  │
   │ port 21932 (sock)                │
   └──────────────────────────────────┘
```

Settings UI:

```
┌─────────────────────────────────────────────────┐
│ Inference backend                               │
│ ◉ GGML Metal + CoreML (stable, default)         │
│ ○ MLX (experimental, 1.5-2x faster on M3/M4)    │
│                                                 │
│ ⚠ MLX requires MLX-format models (separate      │
│   download). Prompted on first activation.      │
└─────────────────────────────────────────────────┘
```

mlx-sidecar lifecycle: lazy spawn (only when toggle is on), auto-shutdown after 10 minutes idle, logs to `~/Library/Logs/<app>/mlx-sidecar.log`.

MLX model catalog points to `mlx-community/*` HuggingFace repos.

### Runtime Mac optimizations

```rust
// inference-core/src/macos_optim.rs

// 1. QoS user-interactive: scheduled as UI thread, max priority
pub fn set_qos_user_interactive() {
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
    }
}

// 2. Memory pressure: release models on warning/critical
pub fn install_memory_pressure_handler(unload_cb: impl Fn()) {
    let source = dispatch_source_create(
        DISPATCH_SOURCE_TYPE_MEMORYPRESSURE, 0,
        DISPATCH_MEMORYPRESSURE_WARN | DISPATCH_MEMORYPRESSURE_CRITICAL,
        dispatch_get_main_queue(),
    );
    /* ... */
}

// 3. Thermal state: downgrade model under thermal stress
pub fn current_thermal_pressure() -> ThermalState {
    NSProcessInfo::processInfo().thermalState()
}

// 4. Energy mode: use small model on battery + lowPowerMode
pub fn should_use_lite_profile() -> bool {
    NSProcessInfo::processInfo().isLowPowerModeEnabled()
}
```

### Adaptive profile (Mac v1)

Background loop in inference-core (every 30 s):

1. Check `thermalState`, `lowPowerMode`, memory pressure.
2. If NORMAL: use the user's selected models.
3. If WARN/SERIOUS/lowPower: temporarily switch to "lite profile" (whisper-small + gemma3:1b) **if both are installed**.
4. If CRITICAL: pause inference, notify user.

Settings: "Adaptive profile" on/off (default on).

### Quantization defaults

| Platform | LLM quant | Whisper |
|---|---|---|
| Mac Apple Silicon | Q4_K_M | fp16 (Metal benefits more from fp16 than from q5_0; verified) |
| Mac Intel | Q4_K_M | q5_0 (CPU benefits) |
| Win/Linux NVIDIA (v2) | Q4_K_M or Q5_K_M | fp16 |
| Win/Linux AMD/Intel Vulkan (v2) | Q4_K_M | q5_0 |
| CPU only | Q4_K_M or Q3_K_S | small.bin q5_0 |

### Memory tier awareness (auto-suggest profile)

| Total RAM | Profile | Recommended models |
|---|---|---|
| < 8 GB | Lite | whisper-small + gemma3:1b |
| 8-16 GB | Balanced | whisper-large-v3-turbo + gemma3:1b |
| 16-32 GB | **Standard** | whisper-large-v3-turbo + gemma3:4b |
| > 32 GB | Performance | whisper-large-v3-turbo + gemma3:12b or 27b |

### Performance targets (Mac M4 16 GB, defaults)

| Stage | Without CoreML | With CoreML | With MLX |
|---|---|---|---|
| Audio capture start | 10 ms | 10 ms | 10 ms |
| First partial transcript | ~250 ms | **~90 ms** | **~70 ms** |
| Final STT pass (3 s audio) | ~800 ms | ~280 ms | ~220 ms |
| First cleanup token | ~250 ms | ~250 ms | ~180 ms |
| Paste latency | < 20 ms | < 20 ms | < 20 ms |
| **Total user-perceived (3 s dictation)** | ~2.0 s | **~1.0 s** | **~0.7 s** |

These are QA gate targets.

### Bundle size (single Mac DMG, MLX always bundled)

| Component | Size |
|---|---|
| Electron + UI assets | ~120 MB |
| Rust sidecar (inference-core + audio-capture + os-integration) | ~25 MB |
| Python embedded (python-build-standalone 3.12) | ~30 MB |
| MLX framework + mlx-lm + mlx-whisper wheels | ~200 MB |
| FastAPI + uvicorn + dependencies | ~15 MB |
| **Total DMG** | **~390 MB** |

Single DMG for Mac arm64. Mac x86_64 build excludes mlx-sidecar (MLX is Apple Silicon only) - smaller (~340 MB).

---

## 6. Model management and packaging

### Model storage

```
macOS:   ~/Library/Application Support/<appname>/models/
Linux:   ~/.local/share/<appname>/models/    (XDG_DATA_HOME)
Windows: %LOCALAPPDATA%\<appname>\models\

Subdirs:
  models/whisper/    *.bin (GGML, whisper.cpp native)
                     *.mlmodelc/  (CoreML encoder, optional)
  models/llm/        *.gguf (llama.cpp native)
  models/mlx/        *  (MLX-format models, opt-in)
  models/manifest.json   {installed_models, sha256, size, source_url}
```

### What we bundle vs download

| | Decision | Reason |
|---|---|---|
| App bundle (.dmg) | ~390 MB, **zero models** | Fast install, models are user-choice |
| Default models | Downloaded at first launch via setup wizard | Update models without app re-release; user picks size/type |
| Catalog manifest | Embedded fallback + fetched from GitHub Pages at startup | Add models without app updates |

### Catalog (`assets/model-catalog.json`)

```json
{
  "version": 1,
  "models": [
    {
      "id": "whisper-large-v3-turbo",
      "kind": "whisper",
      "name": "Whisper large-v3-turbo",
      "ggml_url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
      "ggml_sha256": "...",
      "coreml_url": "https://.../ggml-large-v3-turbo-encoder.mlmodelc.zip",
      "coreml_sha256": "...",
      "size_bytes_ggml": 1610612736,
      "size_bytes_coreml": 173015040,
      "languages": ["multi"],
      "recommended": true,
      "min_ram_mb": 2048
    },
    {
      "id": "gemma3-4b-it-q4",
      "kind": "llm",
      "name": "Gemma 3 4B (Q4_K_M)",
      "url": "https://huggingface.co/.../gemma-3-4b-it-q4_k_m.gguf",
      "sha256": "...",
      "size_bytes": 2700000000,
      "context_length": 8192,
      "recommended_for": ["dictation_cleanup", "edit_mode"],
      "min_ram_mb": 4096
    }
  ]
}
```

App fetches `<github-pages-url>/model-catalog.json` at startup (24 h cache). Offline → embedded copy.

### v1 default model recommendations

- **STT:** `whisper-large-v3-turbo` (1.5 GB) - proven sweet spot on Apple Silicon Metal.
- **LLM cleanup:** `gemma3:4b` Q4_K_M (~2.7 GB) - validated in benchmarks for quality + speed on M4 16 GB.

First-launch total download: ~4.2 GB. The setup wizard shows progress and lets the user pick alternatives (smaller models for low-RAM machines).

### Setup wizard model step

```
┌─────────────────────────────────────────────────┐
│ Choose your models                              │
│                                                 │
│ Transcription:                                  │
│  ◉ Whisper large-v3-turbo   (1.5 GB)  recommended│
│    ☑ + CoreML encoder (165 MB, 3x faster, ANE)  │
│  ○ Whisper small            (488 MB)            │
│                                                 │
│ Cleanup:                                        │
│  ◉ Gemma 3 4B               (2.7 GB)  recommended│
│  ○ Gemma 3 1B               (815 MB)  faster    │
│  ○ No cleanup (Whisper only)                    │
│                                                 │
│ Total: 4.2 GB                                   │
│ Estimated time: ~3 min @ 30 MB/s                │
│                                                 │
│           [ Back ]            [ Download ]      │
└─────────────────────────────────────────────────┘
```

Download via Rust sidecar (`POST /v1/models/download` with SSE progress). Resumable (Range header). SHA-256 verify, retry on mismatch.

### Model swap

- **Hot-swap:** changing model in Settings unloads the previous one (LRU eviction) and loads the new one on the next request. No app restart.
- **Memory budget:** Settings exposes "Max loaded models in RAM" (default 2). Over budget, LRU evict.
- **Keep-alive:** after 5 min idle, auto-unload to free RAM. Configurable.

### Disk space management

- Settings shows total installed model size + per-model "Remove" button.
- Warning if < 5 GB free before starting download.
- Never auto-remove models without explicit user confirmation.

### Packaging Mac v1

| Aspect | Choice |
|---|---|
| Bundler | `electron-builder` (handles signing + notarize) |
| Format | DMG with drag-to-Applications |
| Arch | Separate DMGs for arm64 and x86_64 (universal not feasible due to MLX) |
| Signing | Apple Developer ID Application cert + Hardened Runtime + entitlements |
| Notarization | `notarytool` via electron-builder `afterSign` hook |
| Update | `electron-updater` against GitHub Releases |
| Bundle size target | ~390 MB DMG (Apple Silicon, MLX included) |

### Required entitlements (macOS)

```xml
<key>com.apple.security.device.audio-input</key><true/>
<key>com.apple.security.automation.apple-events</key><true/>
<key>com.apple.security.cs.allow-jit</key><true/>
<key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
```

Info.plist must include `NSMicrophoneUsageDescription` and clear copy explaining the dictation use case.

### Update lifecycle

- Auto-check every 24 h, notify in tray. Never auto-install.
- Update is opt-in (tray click).
- App update does NOT touch downloaded models.
- Settings schema is versioned; migrations run in main process at startup.

### Disaster recovery

- If `models/manifest.json` is corrupted: app rebuilds by scanning present files + asking for re-confirmation.
- If sidecar binary is missing/corrupted: app shows error screen with "Reinstall" button (re-extracts sidecar from app bundle).
- If Accessibility is revoked at runtime: app enters `PERMISSIONS_REVOKED` state until restored.

---

## 7. Build order and milestones

### Philosophy: walking skeleton, not bottom-up

Build vertical slices that produce a shippable artifact per milestone. Discover integration issues early.

### Critical path (1 engineer full-time, realistic estimate)

```
M0 (2w) ─ Foundation
   ▼
M1 (4w) ─ inference-core MVP (batch, Metal)
   ▼
M2 (3w) ─ audio-capture + os-integration MVP (CLI prototype dictation)
   ▼
M3 (3w) ─ app-shell + model-manager (Electron app, batch only)
   ▼
M4 (3w) ─ Streaming pipeline
   ▼
M5 (2w) ─ CoreML + adaptive profile + memory/thermal awareness
   ▼
M6 (2w) ─ Edit Mode
   ▼
M7 (3w) ─ MLX integration
   ▼
M8 (3w) ─ Beta + polish + QA + docs

   TOTAL: ~25 weeks = ~6 months v1 Mac (1 engineer)
   With 2 engineers in parallel: ~4 months
```

### Per-milestone details

**M0 - Foundation (2 weeks)**
Monorepo, CI (matrix arm64 + x86_64), code signing in CI, electron-builder config, Electron + Rust hello-world spawning via UNIX socket, automated notarization pipeline. Acceptance: CI emits a signed/notarized DMG that opens and shows "hello from sidecar" in the menu bar.

**M1 - inference-core MVP (4 weeks)**
whisper-rs (features: metal, coreml, accelerate), llama-cpp-2 (features: metal, accelerate), HTTP/MsgPack server on UNIX socket, batch endpoints (`/v1/stt`, `/v1/chat/completions`, `/v1/models`), CLI test client. Acceptance: CLI does transcription + chat completion with real models in target latency.

**M2 - audio-capture + os-integration (3 weeks)**
audio-capture: cpal capture, resample, frame stream. os-integration (Mac): CGEventTap hotkey, AXUIElement text backend with NSPasteboard fallback, TCC permission checks, napi-rs exports. CLI prototype "prototype-dictation". Acceptance: CLI prototype usable for real dictation, no UI.

**M3 - app-shell + model-manager (3 weeks)**
Electron + Svelte scaffold, menu bar icon, setup wizard, settings window, model manager UI, sidecar lifecycle (spawn/respawn/cleanup), settings persistence (electron-store), hotkey wiring. i18n infrastructure scaffolded from day one (i18next, English-only translations). **Batch mode only - no streaming.** Acceptance: installable DMG, user can dictate via hotkey end-to-end in batch.

**M4 - Streaming pipeline (3 weeks)** - the magic milestone
Whisper streaming with VAD chunking (silero-vad-rs), live partial transcripts via SSE, overlay window (borderless Electron BrowserWindow, click-through, top-of-screen), LLM cleanup streaming, incremental paste via os-integration. State machine implementation. Acceptance: live overlay during recording, streamed cleanup directly into target field.

**M5 - Mac optimizations (2 weeks)**
CoreML encoder auto-download for M-series, memory pressure handler (dispatch source), thermal state monitor, adaptive profile auto-switcher, QoS user-interactive. Acceptance: Section 5 latency targets met (cleanup ~1 s, first partial ~90 ms with CoreML).

**M6 - Edit Mode (2 weeks)**
focused_selection detection, Edit toggle in Settings, modifier-extension hotkey for Edit (e.g. `Cmd+Fn`), dedicated system prompt for transform, replace_focused_selection. Acceptance: select text, speak "make it shorter", text is replaced.

**M7 - MLX integration (3 weeks)**
Python embedded (python-build-standalone), mlx-lm + mlx-whisper bundled, mlx-sidecar (FastAPI + uvicorn), backend switcher in Settings, MLX model catalog. Acceptance: user can toggle MLX, download MLX models, dictate with measurably better latency on M3/M4.

**M8 - Beta + polish (3 weeks)**
QA matrix execution, internal bug bash, external beta (20-50 users), docs (README, getting-started, troubleshooting, architecture, CONTRIBUTING), onboarding polish, release notes automation, v1.0.0 release.

### Parallelization with 2 engineers

| Engineer 1 | Engineer 2 |
|---|---|
| M1 inference-core | M2 audio-capture + os-integration |
| M3 app-shell glue | M3 model-manager UI |
| M4 streaming inference side | M4 overlay UI side |
| M5 Mac optim | M6 Edit Mode (independent) |
| M7 MLX | M8 docs + QA |

### Merge discipline

- M1-M2: feature branches, merge after manual CLI validation.
- M3+: trunk-based with feature flags for incomplete features.
- M4 streaming: feature flag `enable_streaming` (default off until stable).
- M7 MLX: feature flag `enable_mlx_backend`.

### M4 gate review

At end of M4, go/no-go: is the streaming UX measurably better than batch? If not, v1 ships batch-only; streaming becomes v1.1.

### Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| napi-rs ABI break on Electron upgrade | Medium | Medium | Pin Electron version, test on staging before bumping |
| AXUIElement broken in specific target apps | High | Medium | NSPasteboard+CGEvent fallback path, known-bad-apps list in docs |
| MLX Python sidecar startup latency | High | Low | Lazy spawn, not on app startup |
| Whisper streaming accuracy worse than batch | High | High | Final pass on release reconciles partial → final |
| Notarization fails in CI | Medium | Low | Fallback staging notarization, rebuild |
| Hardware detection misclassifies (no Metal) | Low | High | Graceful degrade to CPU + warning UI |

---

## 8. Error handling and testing

### Error taxonomy

| Category | Examples | Recovery |
|---|---|---|
| Resource missing | Whisper model not downloaded; sidecar binary missing | User prompt, link to model manager or reinstall |
| Resource busy/contended | Insufficient RAM; GPU memory pressure | Auto-degrade to smaller model (adaptive profile); user notify |
| External integration failure | AX API refused by target app; mic disconnected mid-recording | Fallback (NSPasteboard); graceful stop + partial result; tray notify |
| Internal logic error | Invalid state machine state; corrupted MsgPack | Reset state, log full context, show "Something went wrong" + report |
| Process crash | Sidecar OOM/segfault; renderer crash | Auto-restart (max 3 retries with backoff), reload renderer; preserve in-flight audio if possible |

### Recovery patterns

**Sidecar supervisor with exponential backoff:**

```typescript
class SidecarSupervisor {
  async ensureRunning(): Promise<void> {
    if (await this.healthCheck()) return;
    if (this.attempts >= 3) {
      this.tray.showError("Inference backend failed repeatedly. Check logs.");
      return;
    }
    await sleep(Math.pow(4, this.attempts) * 1000);
    this.spawn();
    this.attempts++;
  }
}
```

**Partial transcript preservation:** if inference-core crashes mid-recording, the overlay shows "Recording interrupted, transcript so far: ..." with "Save to clipboard" / "Retry" options.

**Permission revoke at runtime:** an `AXObserver` listens for accessibility permission changes; app enters `PERMISSIONS_REVOKED` state with clear remediation UI.

### Observability and logging

Logs per layer, JSON Lines (via `tracing` in Rust, `electron-log` in TS):

| Layer | Path (macOS) | Default level |
|---|---|---|
| Electron main | `~/Library/Logs/<app>/main.log` | info |
| Electron renderer | `~/Library/Logs/<app>/renderer.log` | warn |
| inference-core | `~/Library/Logs/<app>/inference-core.log` | info |
| mlx-sidecar | `~/Library/Logs/<app>/mlx-sidecar.log` | info |

Rotation: 10 MB/file, keep last 5.

**Diagnostic panel** (Settings → Advanced):
- Sidecar status + uptime
- Loaded model + RAM usage
- Last 50 pipeline events
- "Open logs folder", "Copy diagnostics", "Send report" (manual only)

### Crash reporting

**No automatic telemetry.** Consistent with "completely local" + open-source positioning.

On crash:
- Native crash dumps go to `~/Library/Application Support/<app>/crashes/`
- At restart, app detects fresh dump and shows dialog: *"<app> crashed last time. Want to email a report to the developers?"*
- If YES: opens mail client with attachment + diagnostic info pre-filled.
- Never upload automatically.

### Privacy invariants (explicitly QA-tested)

- Only outbound network: GitHub Releases (updates), HuggingFace (model downloads), GitHub Pages (catalog manifest).
- Test: `tcpdump` during dictation must show zero outbound traffic.
- Test: airplane mode → all dictation, edit mode, inference continues to work.
- Raw audio never persisted to disk.
- Transcripts stored only in opt-in History panel, max 100 entries, "Clear all" button.

### Testing strategy

```
                    ┌───────────────────┐
                    │   E2E (manual QA) │  ~20 scenarios per release
                    └──────┬────────────┘
                  ┌────────┴───────────┐
                  │  Integration tests │  ~30 / crate
                  │  (real mini models,│
                  │   socket I/O end-  │
                  │   to-end)          │
                  └────────┬───────────┘
              ┌────────────┴────────────┐
              │     Unit tests          │  ~80% coverage
              │  (pure functions, state │
              │   machines, parsers)    │
              └─────────────────────────┘
```

**Per crate:**

- **inference-core:** unit (parsers, state machines, VAD frame classification, MsgPack codecs); integration (load whisper-tiny + gemma3:270m, transcribe/complete, assert outputs). CI uses smallest models for speed. Large-model perf tests run on self-hosted Mac runners.
- **os-integration:** unit (input parsing, ABI surface); integration (Mac: register hotkey, verify callback fires; AX tests against TextEdit on Mac CI runner).
- **app-shell:** unit (Svelte stores, settings serialization, IPC contracts); integration (Playwright vs the actual Electron app - window opens, settings persist, hotkey configurable); visual regression (Playwright screenshots).
- **model-manager:** unit (catalog parsing, sha256 verify, resume logic); integration (download from local mock HTTP, verify integrity, install in tmp dir, reload).

### Performance regression suite

CI benches fail if regression > 15%:

```rust
#[bench]
fn bench_full_dictation_5s_italian(b: &mut Bencher) {
    let audio = load_fixture("dictation-5s-italian.wav");
    b.iter(|| {
        let transcript = inference.transcribe(&audio, "whisper-large-v3-turbo").unwrap();
        let cleanup = inference.chat_complete(&transcript, "gemma3:4b").unwrap();
        black_box(cleanup);
    });
}
```

Thresholds:
- Mac M4 16 GB: < 1.0 s end-to-end (with CoreML)
- Mac M2 8 GB: < 2.5 s
- Mac Intel i5: < 4.0 s

Run on self-hosted Mac runners (one per arch).

### Manual QA matrix (per release)

At least 20 scenarios covered. Selection:

| # | Scenario | OS | Expected |
|---|---|---|---|
| 1 | First install + setup wizard fresh user | clean Mac | Wizard guides step-by-step, models downloaded, first dictation works |
| 2 | Permission denial cycle | Mac | Denied → retry prompt → Open Settings → Grant → works |
| 3 | Custom hotkey (`Cmd+Shift+Space`) | Mac | Applied after Settings save, works in all targets |
| 4 | Dictation in Safari URL bar | Mac | Live overlay + correct paste |
| 5 | Dictation in JetBrains IDE (AX-problematic) | Mac | NSPasteboard fallback active, paste OK |
| 6 | Edit Mode on TextEdit selection | Mac | Replace selection correct |
| 7 | Italian dictation with self-correction | Mac | Cleanup applies self-correction per benchmark |
| 8 | English technical jargon | Mac | Cleanup preserves acronyms and names |
| 9 | Mic disconnected mid-recording | Mac | Graceful stop + notify, partial transcript saved |
| 10 | Memory pressure (Chrome + 10 tabs) | Mac M4 16 GB | Adaptive profile downgrade, no crash |
| 11 | Thermal throttling (20 rapid dictations) | Mac | Adaptive profile or pause |
| 12 | Airplane mode | Mac | All features work (no network calls) |
| 13 | Model download fail (kill network mid-download) | Mac | Resume on retry, sha256 check |
| 14 | Switch backend GGML → MLX | Mac M4 | Prompt MLX model download, switch works, dictation continues |
| 15 | Crash sidecar (manual kill) | Mac | Auto-restart, retry current, no app crash |
| 16 | Recording > 5 min hard cap | Mac | Auto-stop with notify |
| 17 | App quit during recording | Mac | Graceful cleanup, no orphan sidecar |
| 18 | App update via electron-updater | Mac | Update without losing settings/models |
| 19 | Uninstall + reinstall | Mac | Models persist in Application Support, app recognizes them |
| 20 | Live overlay z-order + click-through | Mac | Overlay above all but clicks pass through |

### Beta program (M8)

- 20-50 beta testers from HN / Twitter / r/MacApps.
- Sparkle/electron-updater "beta" channel.
- Feedback via GitHub Issues (structured template) + opt-in diagnostic submit.
- ~3 weeks, at least 2 release iterations.

### Documentation deliverables

- `README.md` - 1-pager, install + first dictation.
- `docs/getting-started.md` - setup wizard walkthrough.
- `docs/troubleshooting.md` - common errors, AX/mic permissions, sidecar restart.
- `docs/architecture.md` - condensed from this spec, for contributors.
- `CONTRIBUTING.md` - dev setup, build, test, code style.
- `CHANGELOG.md` - keep-a-changelog format.

All v1 docs are English. Translation is a v1.x or v2 effort.

---

## 9. Open decisions (non-blocking for architecture)

- **App name** - not chosen. Placeholder `<app>`. Affects bundle identifier (`com.<vendor>.<name>`), branding, repo name, launch URL scheme. Pick before M0.
- **License** - Apache-2.0 or MIT. Pick before first commit.
- **Org / repo location** - GitHub org name. Pick before M0.
- **Code signing identity** - Apple Developer account name. Required for M0 CI.
- **Update channel naming** - `stable`/`beta`/`canary`? Cosmetic, pick before M8.

None of these block architecture or sub-project implementation specs.

---

## 10. Sub-project specs to follow

Each sub-project listed in §2 will receive its own implementation spec via a separate brainstorming session, in the order dictated by §7:

1. `inference-core` - first, blocks everything else.
2. `audio-capture` - parallel with #3 if multiple engineers.
3. `os-integration` - Mac implementations only; cross-OS in v2 spec.
4. `app-shell` - depends on #1 and #3.
5. `model-manager` - overlaps with #4 model UI work.

This architecture spec is the source of truth for cross-cutting decisions; sub-project specs must conform to it.
