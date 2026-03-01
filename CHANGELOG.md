# Changelog (Local Fork)

This file tracks local changes made to the upstream Anki repository for the
`anki-api`/gRPC integration work, to simplify future upstream merges.

## Unreleased

### `api/`
- Added API workspace crates:
  - `api/anki-api-proto`
  - `api/anki-api`
  - `api/anki-api-client`
- Added gRPC API server wiring with auth, health, system, notes, and notetypes
  services.
- Added/updated capability handling and API docs alignment for implemented RPCs.
- `api/anki-api-client`:
  - high-level `ApiClient` with auth injection
  - capability bootstrap/parsing
  - typed error mapping (including version conflicts)
  - streaming wrappers
  - notetype lookup/ref methods:
    - `get_notetype_id_by_name(...)`
    - `list_notetype_refs()`

### `proto/anki/api/v1/`
- Added and evolved public API v1 contracts:
  - Notes:
    - streaming list/search endpoints
    - streaming ID list/search endpoints
    - named note field representation
    - note write/update endpoints (single + batch)
    - note change feed with cursor pagination
  - Notetypes:
    - list/get endpoints
    - `GetNotetypeIdByName`
    - lightweight refs endpoint: `ListNotetypeRefs` (`notetype_id`, `name`)
    - update endpoints (content/templates/css, single + batch)
    - notetype change feed with cursor pagination
  - Common/system/health:
    - structured error details for version conflicts
    - server info/capabilities contract
    - custom and standard gRPC health services

### `proto/anki/` (internal backend proto)
- Added paged change-feed RPCs:
  - `proto/anki/notes.proto`:
    - `NotesService.GetNoteChangesPage`
    - `GetNoteChangesPageRequest`
    - `NoteChangeEntry`
    - `GetNoteChangesPageResponse`
  - `proto/anki/notetypes.proto`:
    - `NotetypesService.GetNotetypeChangesPage`
    - `GetNotetypeChangesPageRequest`
    - `NotetypeChangeEntry`
    - `GetNotetypeChangesPageResponse`
- Aligned internal change-entry types for consistency/safety:
  - `usn`: `sint32` for notes and notetypes
  - `mtime_secs`: `int64` for notes and notetypes

### `pylib/`
- Added in-process API server lifecycle hooks:
  - `pylib/rsbridge/lib.rs`:
    - backend-bound `start_api_server(...)`
    - backend-bound `stop_api_server(...)`
  - `pylib/anki/_backend.py`:
    - Python wrapper methods for server lifecycle

### `qt/`
- Wired API server lifecycle to profile lifecycle:
  - `qt/aqt/main.py`: start on profile open, stop on profile close/switch.

### `rslib/`
- Added storage-layer paged change queries:
  - `rslib/src/storage/note/get_changes_page.sql`
  - `rslib/src/storage/notetype/get_changes_page.sql`
  - `SqliteStorage::get_note_changes_page(...)`
  - `SqliteStorage::get_notetype_changes_page(...)`
- Added backend service implementations for paged changes:
  - `rslib/src/notes/service.rs`: `get_note_changes_page(...)`
  - `rslib/src/notetype/service.rs`: `get_notetype_changes_page(...)`
- Changed notes change mapping to remove truncating cast:
  - `rslib/src/notes/service.rs`: `mtime_secs: mtime_secs.0`
- Added defensive cursor conversion:
  - clamp `after_usn` (`int64`) to `i32` range before constructing `Usn`
  - files: `rslib/src/notes/service.rs`, `rslib/src/notetype/service.rs`

### Follow-up Notes
- TODOs added in rslib SQL change-page queries for possible composite indexes:
  - `notes(usn, id)`
  - `notetypes(usn, id)`
  - files:
    - `rslib/src/storage/note/get_changes_page.sql`
    - `rslib/src/storage/notetype/get_changes_page.sql`
