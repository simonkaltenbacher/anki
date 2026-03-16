# Changelog (Local Fork)

This file tracks local changes on top of upstream Anki.

## 2026-03-05 (Unreleased)

### Added

- [api] Added API workspace crates: `api/anki-api-proto`, `api/anki-api`, and `api/anki-api-client`.
- [api] Added gRPC API server wiring for auth, health, system, notes, and notetypes services.
- [api-client] Added high-level `ApiClient` with auth injection, capability bootstrap/parsing, typed error mapping (including version conflicts), and streaming wrappers.
- [api-client] Added notetype lookup/ref methods: `get_notetype_id_by_name(...)` and `list_notetype_refs()`.
- [proto/api-v1] Added and evolved public API v1 contracts for notes and notetypes: list/get/update (single+batch), refs, and change feeds.
- [proto/api-v1] Added common/system/health contracts for structured errors, capabilities/server info, and dual health endpoints.
- [proto/internal] Added internal paged change-feed RPCs: `NotesService.GetNoteChangesPage` and `NotetypesService.GetNotetypeChangesPage` (+ request/response/change entry messages).
- [rslib] Added storage-layer paged change queries and methods:
  `rslib/src/storage/note/get_changes_page.sql`,
  `rslib/src/storage/notetype/get_changes_page.sql`,
  `SqliteStorage::get_note_changes_page(...)`,
  `SqliteStorage::get_notetype_changes_page(...)`.
- [rslib] Added backend service implementations:
  `rslib/src/notes/service.rs:get_note_changes_page(...)` and
  `rslib/src/notetype/service.rs:get_notetype_changes_page(...)`.
- [qt] Wired API server lifecycle into profile lifecycle (`qt/aqt/main.py`: start on profile open, stop on close/switch).
- [qt] Added profile-backed API configuration keys:
  `anki_public_api_enabled`,
  `anki_public_api_host`,
  `anki_public_api_port`,
  `anki_public_api_auth_disabled`,
  `anki_public_api_allow_non_local`,
  `anki_public_api_allow_loopback_unauthenticated_health_check`.
- [launcher] Added in-repo cross-platform locale module `qt/launcher/src/locale.rs` (env-first lookup + platform fallback + normalization + tests).
- [launcher] Added explicit local-install mode for fork builds:
  marker file `Contents/Resources/local-install-mode`,
  bundled wheel source `Contents/Resources/wheels/`,
  non-interactive install/update path for first install and rebuild/update scenarios.
- [launcher] Added strict local wheel resolution in local mode: `UV_FIND_LINKS` + `UV_NO_INDEX=1`.
- [api] Added native API config file loading (`public-api.toml`) in Rust config resolution, shared by both `./run` and packaged launcher startup paths.
- [launcher/mac] Added local build tooling:
  `qt/launcher/mac/build-local.sh` and `qt/launcher/mac/pyproject.local.toml`.

### Changed

- [api] Consolidated server config resolution semantics to runtime > env > file > profile > defaults.
- [qt] Consolidated API startup gating into a resolved `api_server_enabled` flow (`ANKI_PUBLIC_API_ENABLED` over profile key).
- [rslib] Changed notes change mapping to avoid truncating cast (`mtime_secs: mtime_secs.0`).
- [proto/internal] Aligned internal change-entry scalar types for consistency:
  `usn` as `sint32`, `mtime_secs` as `int64` (notes + notetypes).
- [launcher] Removed third-party locale dependency usage from launcher startup path.
- [launcher] Local install mode now treats a missing `.sync_complete` marker as a forced reinstall trigger.
- [launcher] Local install mode now forces `uv sync` to reinstall bundled `anki` and `aqt` wheels even when the package version is unchanged.
- [launcher/mac] Direct terminal invocation now keeps Anki attached after install/update so stdout/stderr logs remain visible; Finder-style launches still detach.

### Fixed

- [launcher] Prevented startup crash on malformed locale data by replacing panic-prone locale detection path.
- [launcher] Added compatibility fallback when Python expects newer rsbridge API config helpers than the loaded backend exposes.
- [rslib] Added defensive cursor conversion by clamping `after_usn` (`int64`) to `i32` range before constructing `Usn` (notes + notetypes).
- [launcher/mac] Local installer now uses `ditto` and `lsregister` for app bundle installation/registration, and clears the installed venv/lock/sync state to force a clean reinstall from bundled wheels.
- [launcher/mac] Suppressed detached-launch messaging when the launcher will exec Anki directly in the current terminal.

### Security

- [api] API keys are no longer read from profile config; API key remains env/runtime supplied (`ANKI_PUBLIC_API_KEY`).

### Docs

- [docs] Expanded `docs/api-v1.md` with configuration documentation:
  source precedence, env/profile key tables, startup enable semantics, API key security note, and macOS launcher usage.

### Follow-up

- [rslib] TODOs added in change-page SQL for possible composite indexes:
  `notes(usn, id)` and `notetypes(usn, id)`.
