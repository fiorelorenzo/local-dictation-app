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

## Provisioning del modello Whisper (M1a)

M1a esegue speech-to-text via [whisper.cpp](https://github.com/ggerganov/whisper.cpp) (bridge `whisper-rs`, build con features `metal + coreml`). Il modello non è bundlato nel DMG: lo fornisci tu.

1. Scarica un modello ggml dalla [repo HF di whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp/tree/main). Esempio consigliato: `ggml-large-v3-turbo.bin` (~1.5 GB, ottimo trade-off qualità/velocità su M-series).
2. (Opzionale, raccomandato su M-series) Scarica l'encoder CoreML corrispondente, es. `ggml-large-v3-turbo-encoder.mlmodelc.zip`, ed estrailo **nella stessa cartella** del `.bin`. Il sidecar lo rileva automaticamente cercando `<basename>-encoder.mlmodelc/` accanto al `.bin`.
3. Esporta la variabile d'ambiente:
   ```bash
   export SIDECAR_WHISPER_MODEL_PATH=/percorso/assoluto/ggml-large-v3-turbo.bin
   ```
4. Avvia in dev: `just dev`, oppure lancia il sidecar standalone:
   ```bash
   SIDECAR_SOCKET_PATH=/tmp/s.sock cargo run -p inference-core
   ```

Se il modello manca o il path non esiste, il sidecar gira comunque ma `/v1/stt` ritorna `503 stt_unavailable` e `/healthz` riporta `stt_ready: false`.

Per disabilitare l'encoder CoreML (utile su alcuni M1 con bug noti su ANE):
```bash
export SIDECAR_WHISPER_COREML_DISABLE=1
```

## Usare `lda-cli`

Il CLI vive nel crate `crates/lda-cli` e parla col sidecar via UNIX socket. Risoluzione del socket: flag `--socket` > env `SIDECAR_SOCKET_PATH` > default macOS `$HOME/Library/Application Support/app/sidecar.sock` (stesso path della Electron app, così puoi parlare col sidecar mentre la app gira).

Esempi:

```bash
# Salute del sidecar
lda-cli health
# status=ok  version=0.0.1  uptime_ms=12345  stt_ready=true

# Modelli caricati
lda-cli models

# Trascrivi un file WAV
lda-cli stt sample.wav
# stderr: [whisper-rs] ggml-large-v3-turbo (it) 30000ms audio, 4120ms processing (rtf 0.14x)
# stdout: ciao mondo, questo è un test.

# Stesso comando, JSON intero in stdout
lda-cli stt sample.wav --json

# Con segments e una lingua forzata
lda-cli stt sample.wav --language en --segments --json

# Forza MsgPack sulla risposta (debug)
lda-cli --msgpack stt sample.wav
```

Exit codes: `0` success, `2` server unreachable, `3` HTTP 4xx, `4` HTTP 5xx, `5` bad input file.

## Project name

The folder name `local-dictation-app` is a placeholder. The product name is intentionally not chosen yet (see "Open decisions" in the architecture design).

## License

[Apache-2.0](LICENSE). Copyright 2026 Lorenzo Fiore. See [NOTICE](NOTICE) for third-party attributions.
