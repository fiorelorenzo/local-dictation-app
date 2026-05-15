# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
