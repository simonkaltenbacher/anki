## Error Handling Style

- Prefer `?` over `.map_err(...)` when a suitable `From` implementation exists and no custom variant/context mapping is needed.
- Use explicit error variants when adding contextual information materially improves debugging.
- Avoid panics in normal control flow; return typed errors instead.
- Use `thiserror` for defining structured error types.

## Repository Overview

- This working directory is a fork of Anki (desktop flashcard app), with a mixed Rust + Python + TypeScript/Svelte codebase.
- Top-level build entry points:
  - `./run`: build + run app in dev mode.
  - `./ninja check`: run full checks.
  - `./ninja format`: format code.
  - `./ninja fix`: autofix lint/header issues.

## Key Directories

- `rslib/`: Rust core logic and backend services (scheduling, storage, syncing, stats, etc.).
- `pylib/anki/`: Python library layer.
- `pylib/rsbridge/`: Rust/Python bridge crate used by Python layer.
- `qt/aqt/`: Qt/Python desktop UI.
- `ts/`: TypeScript/Svelte frontend code used in webviews/editor/reviewer/routes.
- `proto/anki/`: protobuf definitions (internal APIs and contracts).
- `build/`: Rust build orchestration crates (`configure`, `ninja_gen`, `runner`).
- `tools/`: developer scripts (`build`, `runopt`, `install-n2`, etc.).
- `docs/`: contributor and architecture docs (`development.md`, `language_bridge.md`, etc.).

## Workspace/Build Notes

- Rust workspace members are declared in `Cargo.toml`; main crate dependency alias is `anki = { path = "rslib" }`.
- Rust toolchain is pinned in `rust-toolchain.toml`.
- Proto generation and language-bridge behavior are documented in `docs/protobuf.md` and `docs/language_bridge.md`.
- `.cargo/config.toml` pins `PROTOC` to `out/extracted/protoc/bin/protoc` for reproducible builds.
- `cargo run -p configure` only generates `out/build.ninja`; it does not execute build targets.
- To materialize pinned tool binaries (including `protoc`), ensure `n2` is installed and run Ninja targets.
  - Install: `bash tools/install-n2`
  - Provision protoc: `./ninja extract_protoc_bin` (or `./ninja protoc_binary`)

## Architecture Notes (Practical)

- Core business logic should live in Rust (`rslib`) whenever possible.
- For this gRPC effort, implement in `api/anki-api`/`api/anki-api-client` and treat `rslib` as a dependency without modifying it.
- Python/Qt layers should stay thin and delegate to Rust for heavy data-path logic.
- TypeScript code powers webview/editor/reviewer UX and should align with backend contracts.

## Recommended Workflow For Changes

1. Identify affected layer(s): `rslib`, `pylib/qt`, `ts`, `proto`.
2. Make minimal, layered changes at the lowest responsible layer.
3. Run targeted checks first, then broader checks (`./ninja check` when feasible).
4. Keep protobuf/interface changes synchronized with call sites and generated bindings.

## Build Bootstrap Quickstart

1. `bash tools/install-n2`
2. `cargo run -p configure`
3. `./ninja extract_protoc_bin`
4. Run Cargo commands normally (for example `cargo check`).

## Current Context

- `PLAN.md` contains an in-repo implementation plan for an in-process gRPC API (`anki.api.v1`) and related crates (`api/anki-api`, `api/anki-api-proto`, `api/anki-api-client`).
- Related external consumer project is expected at `../anki-edit`.
- `api/anki-api` intentionally uses `edition = "2024"` and `rust-version = "1.92"` (workspace defaults differ).
- gRPC server currently exposes both:
  - custom `anki.api.v1.HealthService`
  - standard `grpc.health.v1.Health` via `tonic-health`
- `ANKI_PUBLIC_API_KEY=""` is treated as explicit misconfiguration and returns a typed config error (`EmptyValue`) instead of silently disabling auth.
