# Anki API v1

`anki.api.v1` is a local gRPC integration API for applications that need
programmatic access to Anki notes and notetypes.

The protobuf contract lives under `proto/anki/api/v1/`.

## Scope

V1 focuses on note/notetype read and write workflows:

1. Note fetch/list/search/change feeds
2. Notetype fetch/list/change feeds
3. Note field updates
4. Notetype template/CSS/content updates
5. Health and server metadata

Deferred post-v1 resources:

1. Cards
2. Decks
3. Media

## Compatibility

Compatibility is additive within `v1`:

1. Existing field numbers and RPC names are not repurposed.
2. New fields/RPCs are additive.
3. Removed fields must be marked `reserved`.
4. Clients should prefer capability checks over hard-coded version checks.

### Version Fields (`System.GetServerInfo`)

`GetServerInfoResponse` returns three independent version values:

1. `api_version`: wire/protocol contract version (`v1`).
2. `server_version`: `anki-api` server implementation version.
3. `anki_version`: host Anki application version when provided by server startup config.

## Authentication

By default, API key auth is enabled:

1. Header: `authorization: Bearer <api_key>`
2. Missing/invalid key returns `UNAUTHENTICATED`.
3. Server capabilities include `auth.api_key` when auth is enabled.

## Error Contract

### gRPC Status Codes

Common status codes:

1. `INVALID_ARGUMENT`: invalid request values, malformed cursor, unknown template ordinal, unknown note field name.
2. `NOT_FOUND`: note/notetype/resource not found.
3. `ABORTED`: optimistic concurrency precondition mismatch (`expected_usn`).
4. `FAILED_PRECONDITION`: invariant/required state failure.
5. `UNAUTHENTICATED`: missing/invalid API key.
6. `INTERNAL`: unexpected server/backend failure.
7. `UNAVAILABLE`: transient transport/server outage.

### Structured Error Detail

For selected errors, responses include `anki.api.v1.ErrorDetail` in gRPC status details.

Stable detail codes:

1. `VERSION_CONFLICT`: optimistic concurrency check failed (retryable after refetch).

Detail payload fields:

1. `code`: stable machine-readable code.
2. `retryable`: whether caller can retry after reconciliation.
3. `message`: server-provided detail for diagnostics.

## Optimistic Concurrency

Write requests accept `optional int64 expected_usn`:

1. If omitted, writes use last-writer-wins semantics.
2. If provided and stale, server returns `ABORTED` with `ErrorDetail.code=VERSION_CONFLICT`.
3. Use returned entity `usn` values (or change feeds) to advance client state.

## Streaming and Pagination Semantics

1. `Notes.ListNoteRefs` and `Notes.ListNotes` are server-streaming RPCs.
2. Both accept an optional query string; empty query means "all notes".
3. Note ref streams emit `ListNoteRefsResponse` containing one `NoteRef` (`note_id` + `sort_field`) per item.
4. Note streams emit `ListNotesResponse` containing one `Note` per item.
5. `GetNoteChanges` and `GetNotetypeChanges` are unary paged change feeds using `(usn,id)` cursor semantics.

Cursor format:

1. `"<usn>:<id>"` where both components are signed 64-bit integers.
2. Empty cursor starts from beginning.
3. Empty `next_cursor` means no additional page.

## Batch Semantics

Batch write RPCs are non-atomic:

1. Updates are applied in request order.
2. Processing stops at first error.
3. Earlier successful updates are not rolled back.
4. `results.len()` indicates successful prefix length.
5. `Notes.UpdateNoteFieldsBatch.results` returns per-note authoritative write metadata:
   `note_id`, `usn`, and `sort_field`.
6. `Notetypes` batch RPCs return `NotetypeWriteMetadata` with post-write
   `(notetype_id, usn)`.

Batch RPCs:

1. `Notes.UpdateNoteFieldsBatch`
2. `Notetypes.UpdateTemplatesBatch`
3. `Notetypes.UpdateCssBatch`

## Capabilities

Current server capability keys:

1. `health.check`
2. `system.server_info`
3. `notes.get`
4. `notes.get.batch`
5. `notes.list_refs.stream`
6. `notes.list.stream`
7. `notes.update_fields`
8. `notes.update_fields.batch`
9. `notes.changes`
10. `notes.count`
11. `notetypes.get`
12. `notetypes.get.batch`
13. `notetypes.get_id_by_name`
14. `notetypes.list_refs`
15. `notetypes.list`
16. `notetypes.update_content`
17. `notetypes.update_templates`
18. `notetypes.update_templates.batch`
19. `notetypes.update_css`
20. `notetypes.update_css.batch`
21. `notetypes.changes`
22. `notetypes.count`
23. `auth.api_key` (only when auth is enabled)

## RPC Set (Current V1)

1. `HealthService.Check`
2. `SystemService.GetServerInfo`
3. `NotesService.GetNote`
4. `NotesService.GetNotes`
5. `NotesService.ListNoteRefs` (server-streaming)
6. `NotesService.ListNotes` (server-streaming)
7. `NotesService.UpdateNoteFields`
8. `NotesService.UpdateNoteFieldsBatch`
9. `NotesService.GetNoteChanges`
10. `NotesService.CountNotes`
11. `NotetypesService.ListNotetypeRefs`
12. `NotetypesService.ListNotetypes`
13. `NotetypesService.GetNotetype`
14. `NotetypesService.GetNotetypes`
15. `NotetypesService.GetNotetypeIdByName`
16. `NotetypesService.UpdateNotetypeContent`
17. `NotetypesService.UpdateTemplates`
18. `NotetypesService.UpdateTemplatesBatch`
19. `NotetypesService.UpdateCss`
20. `NotetypesService.UpdateCssBatch`
21. `NotetypesService.GetNotetypeChanges`
22. `NotetypesService.CountNotetypes`

## Health Endpoints

Server currently exposes both:

1. custom `anki.api.v1.HealthService`
2. standard `grpc.health.v1.Health`

Health semantics differ slightly:

1. `anki.api.v1.HealthService.Check` performs a backend availability probe and can report `NOT_SERVING`.
2. `grpc.health.v1.Health` is currently process/service liveness and reports serving once the gRPC server is up.

Integrators should choose based on whether they need backend readiness (`anki.api.v1.HealthService`) or generic gRPC liveness tooling (`grpc.health.v1.Health`).
