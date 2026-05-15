# Justfile - orchestrator for local-dictation-app
# Run `just` (no args) for the command list.

default:
    @just --list

# ---- dev ----

# Run the app in dev mode (sidecar watch + electron with hot reload).
# Kills both on Ctrl+C.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    just sidecar-dev &
    SIDECAR_PID=$!
    trap "kill $SIDECAR_PID 2>/dev/null || true" EXIT
    just app-dev

# Watch the Rust sidecar and re-run on change.
sidecar-dev:
    cd crates/inference-core && cargo watch -x run

# Run electron in dev mode.
app-dev:
    cd app && npm start

# ---- build ----

# Release build of sidecar + electron production bundle (.app, not DMG).
build:
    cargo build --release --target aarch64-apple-darwin -p inference-core
    mkdir -p app/resources
    cp target/aarch64-apple-darwin/release/inference-core app/resources/inference-core
    cd app && npm run package

# Full DMG build (M0 acceptance target).
dmg: build
    cd app && npm run make

# ---- quality gates ----

# Run unit and integration tests across all crates and the app.
test:
    cargo nextest run -p inference-core
    cd app && npm test

# Lint everything (clippy + eslint).
lint:
    cargo clippy -p inference-core --all-targets -- -D warnings
    cd app && npm run lint

# Format everything.
format:
    cargo fmt
    cd app && npm run format

# Remove build artifacts.
clean:
    cargo clean
    cd app && rm -rf out node_modules/.vite

# One-time setup for a fresh clone.
setup:
    cd app && npm install
    cargo fetch
