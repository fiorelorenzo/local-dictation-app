# M0 - Foundation - Design

**Status:** Draft
**Date:** 2026-05-15
**Parent spec:** [Architecture design](2026-05-15-architecture-design.md) section 7 (M0).
**Scope:** Implementation design for the M0 Foundation milestone. M0 produces a working monorepo, a CI pipeline that builds an unsigned DMG, and a runnable Electron app that spawns and talks to a Rust sidecar via a UNIX socket. M0 ships zero inference functionality (that is M1).

---

## 0. Decisions taken before drafting

| Decision | Choice | Note |
|---|---|---|
| Apple Developer Program | Not yet enrolled | M0 acceptance relaxed: unsigned DMG. Signing + notarization moved to M0.5 |
| GitHub repo | `github.com/fiorelorenzo/local-dictation-app`, public | Open source from day one |
| License | Apache-2.0 | Standard for the Rust-heavy stack, SPDX short headers in source files |
| App bundler | Electron Forge (official) + Vite plugin + Svelte 5 | Replaces the prior electron-vite + electron-builder proposal |
| Folder/repo name | `local-dictation-app` (placeholder) | Final product name TBD, see open decisions in arch spec |

---

## 1. Repository structure and toolchain

### Target tree at end of M0

```
local-dictation-app/
├── .github/
│   └── workflows/
│       └── build-mac.yml
├── .gitignore
├── Cargo.toml                     # workspace root
├── crates/
│   └── inference-core/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
├── app/
│   ├── package.json
│   ├── forge.config.ts
│   ├── vite.main.config.ts
│   ├── vite.preload.config.ts
│   ├── vite.renderer.config.ts
│   ├── tsconfig.json
│   ├── svelte.config.js
│   ├── src/
│   │   ├── main.ts                # Electron main entry
│   │   ├── preload.ts
│   │   ├── sidecar.ts             # SidecarSupervisor
│   │   └── renderer/
│   │       ├── index.html
│   │       ├── main.ts            # Svelte mount
│   │       └── App.svelte
│   └── resources/                 # extra resources (sidecar binary lands here)
├── docs/
│   └── specs/
│       ├── 2026-05-15-architecture-design.md
│       └── 2026-05-15-m0-foundation-design.md   (this doc)
├── Justfile
├── LICENSE                        # Apache-2.0 full text
├── NOTICE
├── README.md
├── CHANGELOG.md
└── rust-toolchain.toml
```

### Pinned toolchain versions

| Tool | Version | Pin location |
|---|---|---|
| Rust | 1.84 stable | `rust-toolchain.toml` |
| Node | 22 LTS | `.nvmrc` |
| Electron | 35.x latest stable | `app/package.json` peerDependency |
| Electron Forge | 7.x latest | `app/package.json` devDependency |
| Vite | 6.x | `app/package.json` devDependency |
| Svelte | 5.x | `app/package.json` devDependency |
| TypeScript | 5.7 | `app/package.json` devDependency |
| `just` | latest | user installs via `brew install just` (documented in README) |

### Cargo workspace root

`Cargo.toml`:

```toml
[workspace]
members = ["crates/inference-core"]
resolver = "2"

[workspace.package]
version = "0.0.1"
edition = "2021"
license = "Apache-2.0"
authors = ["Lorenzo Fiore <tech@sencare.io>"]
repository = "https://github.com/fiorelorenzo/local-dictation-app"

[profile.release]
lto = "thin"
codegen-units = 1
```

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.84.0"
components = ["clippy", "rustfmt"]
targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
```

### Electron Forge bootstrap

The `app/` workspace is initialized via:

```
npx create-electron-app@latest app --template=vite-typescript
```

then Svelte 5 is added manually:

```
cd app
npm install --save-dev svelte @sveltejs/vite-plugin-svelte
```

`vite.renderer.config.ts`:

```typescript
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: { target: 'esnext' },
});
```

### Coding conventions

- TypeScript: `strict: true`, `noUncheckedIndexedAccess: true`, no `any` without an inline comment justifying it.
- Rust: `#![warn(clippy::pedantic)]` with documented exceptions in `clippy.toml`. `cargo fmt` enforced in CI.
- Prettier + ESLint for TS / Svelte. Configs in `app/`.
- Pre-commit hook via `husky` is optional and not gating in M0.

---

## 2. Hello-world protocol and Justfile

### Sidecar lifecycle

The Electron main process owns the sidecar lifecycle. The handshake sequence:

```
1. Electron main computes socket path:
   socketPath = path.join(
     app.getPath('userData'),          # ~/Library/Application Support/local-dictation-app/
     'sidecar.sock'
   )

2. Electron main spawns sidecar process:
   spawn(sidecarBinaryPath, [], {
     env: {
       SIDECAR_SOCKET_PATH: socketPath,
       SIDECAR_LOG_LEVEL: 'info',
       RUST_BACKTRACE: '1',
     },
     stdio: ['ignore', 'pipe', 'pipe'],
   })

3. Sidecar (Rust) at startup:
   - reads SIDECAR_SOCKET_PATH env var
   - removes any stale socket file
   - binds tokio::net::UnixListener on that path
   - serves HTTP/1.1 (axum) on the socket
   - logs "listening on <path>" to stdout

4. Electron main polls GET /healthz over the socket:
   - retries every 100 ms, max 30 attempts (3 s total)
   - on success: menu bar updates to "Sidecar: connected, v0.0.1"
   - on timeout: menu bar updates to "Sidecar: failed to start (check logs)"

5. Shutdown:
   - app 'before-quit' event sends SIGTERM to sidecar
   - waits up to 2 s, then SIGKILL
   - cleans up socket file
```

### Sidecar API in M0 (minimal)

| Endpoint | Method | Response | Purpose |
|---|---|---|---|
| `/healthz` | GET | `{"status":"ok","version":"0.0.1","uptime_ms":<n>}` | Liveness check |
| `/version` | GET | `{"version":"0.0.1","build":"<git-sha>","backend":"hello-world"}` | Debug info |

Nothing else. Real inference endpoints (`/v1/stt`, `/v1/chat/completions`) arrive in M1.

### Sidecar binary location at runtime

- **Dev (`just dev`):** the sidecar is built by `cargo build` and lives at `crates/inference-core/target/debug/inference-core`. The Electron main resolves the path from a `SIDECAR_BIN` env var when present.
- **Packaged DMG:** Forge places the sidecar at `<App>.app/Contents/Resources/inference-core` via `extraResource`. The Electron main detects packaged mode (`app.isPackaged`) and uses that path.

### Justfile commands

```just
# Justfile

default:
    @just --list

# Run app in dev mode (sidecar + electron with hot reload)
dev:
    just sidecar-dev & just app-dev; kill %1

sidecar-dev:
    cd crates/inference-core && cargo watch -x run

app-dev:
    cd app && npm start

# Production build (binaries only, no DMG)
build:
    cd crates/inference-core && cargo build --release
    cd app && npm run package

# Full DMG build (M0 acceptance target)
dmg: build
    cd app && npm run make

# Quality gates
test:
    cd crates/inference-core && cargo nextest run
    cd app && npm test

lint:
    cd crates/inference-core && cargo clippy --all-targets -- -D warnings
    cd app && npm run lint
    just check-license

format:
    cd crates/inference-core && cargo fmt
    cd app && npm run format

clean:
    cd crates/inference-core && cargo clean
    cd app && rm -rf out node_modules/.vite

setup:
    cd app && npm install
    cd crates/inference-core && cargo fetch

check-license:
    @find crates app/src -type f \( -name '*.rs' -o -name '*.ts' -o -name '*.svelte' \) \
      -exec grep -L 'SPDX-License-Identifier: Apache-2.0' {} \; | \
      tee /tmp/missing-license.txt; \
    [ ! -s /tmp/missing-license.txt ]
```

---

## 3. CI: GitHub Actions

### Workflow file

`.github/workflows/build-mac.yml`:

```yaml
name: build-mac

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
  workflow_dispatch:

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - runner: macos-15
            arch: arm64
            rust_target: aarch64-apple-darwin
          - runner: macos-13
            arch: x64
            rust_target: x86_64-apple-darwin

    runs-on: ${{ matrix.runner }}
    timeout-minutes: 30

    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        # rust-toolchain.toml is auto-detected

      - name: Cache Cargo
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: crates/inference-core

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: npm
          cache-dependency-path: app/package-lock.json

      - name: Install just
        run: brew install just

      - name: Install npm deps
        working-directory: app
        run: npm ci

      - name: Lint
        run: just lint

      - name: Test
        run: just test

      - name: Build sidecar (release)
        working-directory: crates/inference-core
        run: cargo build --release --target ${{ matrix.rust_target }}

      - name: Copy sidecar into app resources
        run: |
          mkdir -p app/resources
          cp crates/inference-core/target/${{ matrix.rust_target }}/release/inference-core \
             app/resources/inference-core

      - name: Build DMG (unsigned in M0)
        working-directory: app
        env:
          SKIP_NOTARIZATION: "true"
        run: npm run make -- --arch ${{ matrix.arch }}

      - name: Upload DMG artifact
        uses: actions/upload-artifact@v4
        with:
          name: local-dictation-app-${{ matrix.arch }}-unsigned
          path: app/out/make/**/*.dmg
          retention-days: 30
          if-no-files-found: error
```

### Forge config (M0 essentials)

`app/forge.config.ts`:

```typescript
import type { ForgeConfig } from '@electron-forge/shared-types';
import { MakerDMG } from '@electron-forge/maker-dmg';
import { VitePlugin } from '@electron-forge/plugin-vite';

const config: ForgeConfig = {
  packagerConfig: {
    name: 'local-dictation-app',
    appBundleId: 'app.localdictation.dev',  // placeholder, finalize in M0.5
    extraResource: ['./resources/inference-core'],
    // signing config commented for M0 (relaxed acceptance):
    // osxSign: { /* M0.5 */ },
    // osxNotarize: { /* M0.5 */ },
  },
  makers: [
    new MakerDMG({
      name: 'local-dictation-app',
      format: 'ULFO',
    }),
  ],
  plugins: [
    new VitePlugin({
      build: [
        { entry: 'src/main.ts', config: 'vite.main.config.ts' },
        { entry: 'src/preload.ts', config: 'vite.preload.config.ts' },
      ],
      renderer: [
        { name: 'main_window', config: 'vite.renderer.config.ts' },
      ],
    }),
  ],
};

export default config;
```

### Branch protection on main

Configured manually on GitHub after repo creation:
- Require pull request before merging (1 approving review)
- Require status check `build-mac / build (macos-15, arm64)` to pass
- Require status check `build-mac / build (macos-13, x64)` to pass
- Restrict who can push to matching branches: nobody (force PRs)

---

## 4. License and Apache-2.0 housekeeping

### Files at repo root

- **`LICENSE`** - Apache License 2.0 full text, downloaded from https://www.apache.org/licenses/LICENSE-2.0.txt, unmodified.
- **`NOTICE`** - required by Apache-2.0 for redistribution. Initial content:

```
local-dictation-app
Copyright 2026 Lorenzo Fiore

This product includes software developed by:
- whisper-rs / whisper.cpp (MIT)
- llama-cpp-2 / llama.cpp (MIT)
- cpal (Apache-2.0 OR MIT)
- silero-vad (MIT)
- Electron (MIT)
- Svelte (MIT)
- mlx-lm / mlx-whisper (MIT)

Full third-party license texts are available in the THIRD_PARTY_LICENSES file
in distributed binaries (auto-generated at build time).
```

NOTICE will be regenerated by `tools/generate-notice.sh` starting M1. In M0 the file is hand-written.

### Source file headers (SPDX short form)

Every `.rs`, `.ts`, and `.svelte` file starts with:

```rust
// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Lorenzo Fiore
```

(TypeScript identical with `//`; Svelte uses `<!-- -->` comment block at file top.)

Rationale for SPDX over the full Apache header (12 lines):
- 2 lines vs 12, less noise per file
- Machine-readable, REUSE-compliant
- npm and cargo license detectors recognize it
- Apache Software Foundation accepts both forms
- Standard in modern projects (Linux kernel, Rust stdlib, kubectl)

### Enforcement

`just check-license` (part of `just lint`) fails if any source file under `crates/` or `app/src/` lacks the SPDX line.

### Third-party license aggregation (M1+, not M0)

A future script `tools/generate-third-party-licenses.sh` will:
- Run `cargo about generate` for Rust crates
- Run `license-checker-rseidelsohn` for npm packages
- Concatenate into `THIRD_PARTY_LICENSES.txt`
- Forge bundles this via `extraResource` into the `.app`

M0 does not include this step. The static NOTICE at repo root is sufficient for the M0 CI run.

### README license badge

```markdown
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
```

---

## 5. Acceptance criteria and Definition of Done

### Deliverables at end of M0

| Deliverable | Path | Verification |
|---|---|---|
| Public GitHub repo | `github.com/fiorelorenzo/local-dictation-app` | URL opens, README rendered, GitHub auto-detects LICENSE as Apache-2.0 |
| Working Cargo workspace | `Cargo.toml` + `crates/inference-core/` | `cargo build --release` produces a binary |
| Electron Forge app | `app/` | `cd app && npm start` opens a dev window |
| Justfile orchestrator | `Justfile` | `just dev`, `just build`, `just dmg`, `just test`, `just lint` all work |
| CI workflow | `.github/workflows/build-mac.yml` | Push to main triggers build, matrix arm64+x64, both green |
| DMG arm64 unsigned | CI artifact | Download, mount, drag to Applications, Right-click > Open, app launches |
| DMG x64 unsigned | CI artifact | Same as above |
| Sidecar hello-world | runtime | Menu bar shows "Sidecar: connected, v0.0.1" |
| Apache-2.0 compliance | `LICENSE` + `NOTICE` + SPDX headers | `just check-license` passes |

### Definition of Done checklist

```
☐ Public repo created under fiorelorenzo/
☐ Branch protection rules: require PR review + CI pass on main
☐ rust-toolchain.toml pinning Rust 1.84
☐ Cargo workspace with inference-core stub (~50 lines of Rust)
☐ inference-core responds to GET /healthz with JSON {status, version, uptime_ms}
☐ inference-core handles SIGTERM with graceful shutdown (socket cleanup)
☐ Electron Forge scaffold initialized from vite-typescript template
☐ Svelte 5 wired via vite.renderer.config.ts
☐ TypeScript strict mode on
☐ SidecarSupervisor in src/sidecar.ts: spawn, healthz poll, status events
☐ Main process menu bar status indicator showing sidecar state
☐ Apache-2.0 LICENSE + NOTICE + SPDX headers in all source files
☐ Justfile with commands: dev, sidecar-dev, app-dev, build, dmg, test, lint, format, clean, setup, check-license
☐ GitHub Actions workflow build-mac.yml green on matrix arm64+x64
☐ DMG artifact downloadable from a CI run
☐ README updated with license badge + status + 5-line getting started
☐ CHANGELOG.md initialized with entry v0.0.1 "M0 Foundation"
☐ Git tag v0.0.1 on the final M0 commit
```

### Explicitly out of scope for M0

- Real inference (whisper-rs, llama-cpp-2 not imported yet - M1)
- Settings UI, Setup wizard, Model manager (M3)
- Streaming, audio capture, hotkey (M2+)
- Code signing + notarization (M0.5, after Apple Dev enrollment)
- Auto-update (M3)
- Tests beyond the smoke "/healthz responds" (M1+)
- Cross-OS Linux / Windows builds (v2)
- MLX sidecar bundling (M7)

### Expected duration

- 2 weeks @ 1 engineer
- Rough breakdown:
  - Days 1-2: repo init + Cargo workspace + Electron Forge scaffold + Svelte
  - Days 3-5: inference-core stub (Tokio + axum + UNIX socket + healthz)
  - Days 6-7: SidecarSupervisor in Electron main + menu bar status
  - Days 8-9: Justfile + LICENSE + headers
  - Day 10: CI workflow draft + first runs
  - Days 11-14: CI iteration until green + DMG artifact verified + manual smoke test

### Gate review at end of M0 (decisions before M1)

1. Is CI total time acceptable? (target: < 15 min for the full matrix)
2. Does the Forge scaffold behave as expected, or are there frictions to fix before growing?
3. Apple Dev cert: incoming or still deferred to M0.5?
4. Final product name: chosen or still `local-dictation-app` placeholder?

### Risk register specific to M0

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Electron Forge + Svelte 5 setup has undocumented quirks | Medium | Low | Start from official `vite-typescript` template, add Svelte as a tested Vite plugin |
| macos-13 (x86_64) runner deprecated mid-M0 | Low | Medium | Fallback to macos-15 with `--arch x64` cross-compile (Apple Silicon can build x86_64) |
| Gatekeeper blocks unsigned DMG even with Right-click | Low | Low | Document in README: "Right-click > Open the first time", or `xattr -d com.apple.quarantine <App>.app` |
| `cargo nextest` not installed by default on GH runners | Low | Low | Add `cargo install cargo-nextest --locked` step to CI, cached |
| UNIX socket permission issues under sandboxed Electron | Medium | Medium | Use `app.getPath('userData')` which is always writable from the sandbox |

---

## 6. Followups (not blocking M0 acceptance)

These items should land in M0.5 or M1 specs and are listed here so they are not lost:

- **M0.5 - Signing and notarization:** uncomment `osxSign` / `osxNotarize` in `forge.config.ts`, add GitHub Secrets for cert .p12 base64 + password + Apple ID + App-Specific Password + Team ID, replace placeholder `appBundleId`.
- **M1 - Third-party license aggregation script:** `tools/generate-third-party-licenses.sh` invoked in CI before `npm run make`.
- **M1 - NOTICE auto-regeneration:** `tools/generate-notice.sh` invoked in CI to keep NOTICE in sync.
- **Naming:** the placeholder `local-dictation-app` propagates to `package.json` `name`, Forge `name`, `appBundleId`, repo URL, README headline. When the final name is chosen, a rename script must touch all these locations consistently.
