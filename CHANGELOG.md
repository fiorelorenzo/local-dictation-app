# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - <set release date at merge> - M1a STT

### Added
- `inference-core` ora espone `POST /v1/stt` (WAV in body → testo trascritto) e `GET /v1/models`.
- `WhisperBackend` su `whisper-rs` con features `metal + coreml`. Auto-detect dell'encoder CoreML accanto al `.bin`. Provisioning via env `SIDECAR_WHISPER_MODEL_PATH`.
- `StubBackend` selezionabile via `SIDECAR_STT_BACKEND=stub` per CI/testing senza modello reale.
- Pipeline audio interna: hound + rubato per resamplare ogni WAV (8-96 kHz, 1-2 canali) a 16 kHz mono f32.
- Content negotiation JSON ↔ MsgPack su tutti gli endpoint via header `Accept`.
- Serializzazione delle richieste concorrenti via `tokio::sync::Mutex` con timeout 30s → 503 `busy`.
- Crate nuovo `crates/lda-cli`: subcommands `health`, `version`, `models`, `stt`.
- `just test-real` per i test integration con modello reale (marker `#[ignore]`).
- README: sezioni "Provisioning del modello Whisper" e "Usare lda-cli".

### Changed
- `/healthz` include `stt_ready: bool`. `/version` riporta `backend: "whisper-rs"` (era `"hello-world"`).
- Sidecar binary release passa da ~3 MB a ~15 MB (whisper.cpp statico + bridge CoreML).
- Rust toolchain bumped 1.85 → 1.88 (richiesto da `whisper-rs-sys 0.15`).

### Notes
- Niente streaming, niente download manager modelli, niente LLM cleanup: rispettivamente M4, M3, M1b.
- Modelli e `.mlmodelc` non sono bundlati nel DMG: l'utente li scarica e li punta via env.
- La feature `accelerate` di whisper-rs è stata rimossa upstream nella 0.16 (l'accelerazione macOS arm64 è ora gestita internamente da whisper.cpp).

## [0.0.1] - 2026-05-15 - M0 Foundation

### Added
- Monorepo with `Cargo workspace` (crate `inference-core`) and Electron Forge app (`app/`) using Vite + Svelte 5 + TypeScript strict.
- `inference-core` Rust sidecar with `/healthz` and `/version` endpoints over a UNIX socket (axum). Graceful SIGTERM / SIGINT shutdown with socket file cleanup.
- Electron main process spawns and supervises the sidecar, polls `/healthz`, and exposes its state to the renderer via a contextBridge.
- macOS menu bar tray with sidecar status indicator.
- Svelte renderer that displays the live sidecar status.
- `just` orchestrator with commands: `dev`, `build`, `dmg`, `test`, `lint`, `format`, `clean`, `setup`.
- GitHub Actions CI workflow that builds an unsigned arm64 DMG on every push and PR.
- Apache-2.0 LICENSE and NOTICE at the repo root (per-file headers intentionally omitted).
- ESLint + Prettier configs for TypeScript and Svelte. Clippy pedantic on Rust.

### Notes
- DMG is unsigned in M0. Signing and notarization land in M0.5 after Apple Developer enrollment.
- x86_64 (Intel Mac) build is deferred to M0.5 or M1 (driven by user demand).
- Rust toolchain pinned to 1.85 (bumped from the original 1.84 target because transitive deps require `edition2024`, stabilized in 1.85).
