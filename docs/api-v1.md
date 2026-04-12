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

## Transports & Authentication

`anki-api` supports three connection modes. The mode is selected via the
`transport_mode` config key (`ANKI_PUBLIC_API_TRANSPORT_MODE` env var, or
runtime override). Each mode has a different authentication model and is
incompatible with credentials from another mode (for example, `api_key` is
only valid in `tls` mode).

### `plaintext` (default)

1. Cleartext gRPC over TCP. No transport security.
2. No authentication enforced at the API layer.
3. Loopback-only by default (`allow_non_local=false`); non-loopback binds
   require `allow_non_local=true`.
4. `api_key` is rejected in this mode (`ApiKeyRequiresTls` config error).
5. Intended for local development and intra-host integrations on a trusted
   loopback interface.

### `tls`

1. Server-side TLS. Requires `tls_cert_path` and `tls_key_path`.
2. Authentication is API key by default
   (header: `authorization: Bearer <api_key>`).
   - Missing/invalid key returns `UNAUTHENTICATED`.
   - `api_key` is required unless `auth_disabled=true`.
3. With `auth_disabled=true`, the server logs a startup warning that requests
   will not require an API key.
4. Capability `auth.api_key` is advertised when API-key auth is active.

### `spiffe_mtls`

1. Mutual TLS with SPIFFE X.509 SVIDs on both sides, bootstrapped via the
   SPIFFE Workload API.
2. The server fetches its own SVID from the Workload API at startup. Bootstrap
   has a 10-second timeout. If the SPIRE agent is not reachable on the
   configured socket, or no workload entry is registered for the server
   process, startup fails fast with a structured error and the gRPC server
   does not come up.
3. Peers are authorized by exact match against `spiffe_allowed_client_id` (a
   single SPIFFE ID). Peers presenting a different SPIFFE ID — or no SVID at
   all — are rejected at the TLS layer before any gRPC handler runs.
4. `api_key` is rejected in this mode; SPIFFE peer identity is the sole
   credential.
5. The Workload API socket defaults to the SPIFFE-standard endpoint
   (`/tmp/spire-agent/public/api.sock` on macOS/Linux). It can be overridden
   per-process via `spiffe_workload_api_socket`.
6. Capability `auth.spiffe_mtls` is advertised when SPIFFE mTLS is active.
7. Reference client: the `anki-edit` CLI uses `anki-api-client`'s
   `TransportConfig::SpiffeMtls` mode and authorizes the server by SPIFFE ID
   in the same way.

## Configuration

### Desktop Anki (`aqt`) configuration sources

The in-process API server reads configuration from:

1. Runtime overrides passed from the desktop layer.
2. Environment variables (`ANKI_PUBLIC_API_*`).
3. Native API config file (`public-api.toml`).
4. Local profile config keys (`anki_public_api_*` in `prefs21.db`).
5. Built-in defaults.

Effective precedence for server settings is:

1. runtime overrides
2. environment variables
3. file config
4. profile config
5. defaults

Startup enable/disable is determined in the desktop layer before launching the
gRPC thread:

1. `ANKI_PUBLIC_API_ENABLED` overrides profile enable.
2. `anki_public_api_enabled` is used when env enable is unset.
3. Explicit `false` disables startup.

### Supported environment variables

1. `ANKI_PUBLIC_API_ENABLED` (`true/false`, `1/0`)
2. `ANKI_PUBLIC_API_HOST`
3. `ANKI_PUBLIC_API_PORT`
4. `ANKI_PUBLIC_API_KEY`
5. `ANKI_PUBLIC_API_AUTH_DISABLED` (`true/false`, `1/0`)
6. `ANKI_PUBLIC_API_ALLOW_NON_LOCAL` (`true/false`, `1/0`)
7. `ANKI_PUBLIC_API_TRANSPORT_MODE` (`plaintext`, `tls`, `spiffe`; case-insensitive)
8. `ANKI_PUBLIC_API_TLS_CERT_PATH` (PEM file; required for `tls`)
9. `ANKI_PUBLIC_API_TLS_KEY_PATH` (PEM file; required for `tls`)
10. `ANKI_PUBLIC_API_SPIFFE_ALLOWED_CLIENT_ID` (SPIFFE ID; required for `spiffe`)
11. `ANKI_PUBLIC_API_SPIFFE_WORKLOAD_API_SOCKET` (Workload API socket path; optional, defaults to the SPIFFE standard endpoint)

### Supported profile config keys

1. `anki_public_api_enabled` (`bool`)
2. `anki_public_api_host` (`str`)
3. `anki_public_api_port` (`int`)
4. `anki_public_api_auth_disabled` (`bool`)
5. `anki_public_api_allow_non_local` (`bool`)
6. `anki_public_api_allow_loopback_unauthenticated_health_check` (`bool`)

Security note:

1. API keys are intentionally not read from profile config.
2. Provide API keys via `ANKI_PUBLIC_API_KEY` (or runtime override).

### Native API config file

The API server can load file-based defaults from:

1. macOS: `~/Library/Application Support/Anki2/public-api.toml`
2. Linux: `~/.local/share/Anki2/public-api.toml`
3. Windows: `%APPDATA%\\Anki2\\public-api.toml`

Examples, one per transport mode:

```toml
# plaintext: loopback-only, no authentication
[anki_public_api]
enabled = true
host = "127.0.0.1"
port = 50051
transport_mode = "plaintext"
```

```toml
# tls: server cert + API-key authentication
[anki_public_api]
enabled = true
host = "127.0.0.1"
port = 50051
transport_mode = "tls"
tls_cert_path = "/path/to/server.pem"
tls_key_path = "/path/to/server.key"
api_key = "replace-with-strong-random-key"
auth_disabled = false
```

```toml
# spiffe_mtls: SPIFFE workload identity, no API key
[anki_public_api]
enabled = true
host = "127.0.0.1"
port = 50051
transport_mode = "spiffe"
spiffe_allowed_client_id = "spiffe://localhost/anki-edit"
# spiffe_workload_api_socket is optional; defaults to the SPIFFE standard endpoint
```

Process environment variables still override file values.

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

Batch create semantics are different:

1. `Notes.CreateNotes` is atomic and all-or-nothing.
2. Request items are prepared and inserted in request order.
3. If any item fails, the RPC returns an error and no notes from the batch are created.
4. On success, `CreateNotesResponse.notes` returns full created note payloads in request order.
5. Validation failures include a batch index when available.
6. Backend insertion failures may not identify the specific item that caused the batch to fail.

## Capabilities

Current server capability keys:

1. `health.check`
2. `system.server_info`
3. `decks.list_refs`
4. `decks.get_id_by_name`
5. `notes.get`
6. `notes.get.batch`
7. `notes.create`
8. `notes.create.batch`
9. `notes.delete`
10. `notes.list_refs.stream`
11. `notes.list.stream`
12. `notes.update_fields`
13. `notes.update_fields.batch`
14. `notes.changes`
15. `notes.count`
16. `notetypes.get`
17. `notetypes.get.batch`
18. `notetypes.get_id_by_name`
19. `notetypes.list_refs`
20. `notetypes.list`
21. `notetypes.update_content`
22. `notetypes.update_templates`
23. `notetypes.update_templates.batch`
24. `notetypes.update_css`
25. `notetypes.update_css.batch`
26. `notetypes.changes`
27. `notetypes.count`
28. `auth.api_key` (only when transport is `tls` and API-key auth is enabled)
29. `auth.spiffe_mtls` (only when transport is `spiffe`)

## RPC Set (Current V1)

1. `HealthService.Check`
2. `SystemService.GetServerInfo`
3. `DecksService.ListDeckRefs`
4. `DecksService.GetDeckIdByName`
5. `NotesService.GetNote`
6. `NotesService.GetNotes`
7. `NotesService.CreateNote`
8. `NotesService.CreateNotes`
9. `NotesService.DeleteNotes`
10. `NotesService.ListNoteRefs` (server-streaming)
11. `NotesService.ListNotes` (server-streaming)
12. `NotesService.UpdateNoteFields`
13. `NotesService.UpdateNoteFieldsBatch`
14. `NotesService.GetNoteChanges`
15. `NotesService.CountNotes`
16. `NotetypesService.ListNotetypeRefs`
17. `NotetypesService.ListNotetypes`
18. `NotetypesService.GetNotetype`
19. `NotetypesService.GetNotetypes`
20. `NotetypesService.GetNotetypeIdByName`
21. `NotetypesService.UpdateNotetypeContent`
22. `NotetypesService.UpdateTemplates`
23. `NotetypesService.UpdateTemplatesBatch`
24. `NotetypesService.UpdateCss`
25. `NotetypesService.UpdateCssBatch`
26. `NotetypesService.GetNotetypeChanges`
27. `NotetypesService.CountNotetypes`

## Health Endpoints

Server currently exposes both:

1. custom `anki.api.v1.HealthService`
2. standard `grpc.health.v1.Health`

Health semantics differ slightly:

1. `anki.api.v1.HealthService.Check` performs a backend availability probe and can report `NOT_SERVING`.
2. `grpc.health.v1.Health` is currently process/service liveness and reports serving once the gRPC server is up.

Integrators should choose based on whether they need backend readiness (`anki.api.v1.HealthService`) or generic gRPC liveness tooling (`grpc.health.v1.Health`).
