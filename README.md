# local-dictation-app

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![CI](https://github.com/fiorelorenzo/local-dictation-app/actions/workflows/build-mac.yml/badge.svg)](https://github.com/fiorelorenzo/local-dictation-app/actions/workflows/build-mac.yml)

Fully local, open-source dictation app for macOS (Linux + Windows in v2).
Inspired by FreeFlow, Wispr Flow, Superwhisper. Zero cloud, zero account, zero telemetry.

**Status:** M0 Foundation complete - app builds and runs, but no inference yet. M1 adds Whisper + LLM cleanup.

## Getting started (development)

Requirements:
- macOS on Apple Silicon
- Rust 1.85 (managed automatically via `rust-toolchain.toml`)
- Node 22 (`.nvmrc`)
- `just` (`brew install just`)
- `cargo-nextest` and `cargo-watch` (`brew install cargo-nextest cargo-watch`)

First-time setup:

```bash
just setup
```

Run in dev mode (hot reload on both Rust sidecar and Electron renderer):

```bash
just dev
```

Build a local DMG:

```bash
just dmg
# DMG appears under app/out/make/
```

Other commands: `just test`, `just lint`, `just format`, `just clean`. Run `just` with no args for the full list.

## Documentation

Design documents (architecture spec, milestone specs, implementation plans) are kept as local-only working docs under `docs/`. The public repository tracks only code, configuration, README, CHANGELOG, LICENSE, NOTICE.

## Installing the unsigned M0 DMG

M0 ships unsigned (Apple Developer enrollment is M0.5). To open the app for the first time:

1. Drag `local-dictation-app.app` to `/Applications`.
2. Right-click the app and choose "Open" (only the first time).
3. Or remove the quarantine attribute: `xattr -d com.apple.quarantine /Applications/local-dictation-app.app`.

## Project name

The folder name `local-dictation-app` is a placeholder. The product name is intentionally not chosen yet (see "Open decisions" in the architecture design).

## License

[Apache-2.0](LICENSE). Copyright 2026 Lorenzo Fiore. See [NOTICE](NOTICE) for third-party attributions.
