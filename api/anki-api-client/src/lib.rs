//! Rust SDK for `anki.api.v1`.
//!
//! Compatibility and version semantics:
//! - `api_version`: protocol major/minor contract (for example `v1`).
//! - `server_version`: `anki-api` server implementation version.
//! - `anki_version`: host Anki application version, when provided by the server.
//!
//! Error contract:
//! - gRPC status codes remain the base transport contract.
//! - `ErrorDetail.code` values in [`error_codes`] are stable SDK-level markers.
//!
//! Capability contract:
//! - Clients should check [`ApiClient::capabilities`] before using optional RPCs.
//! - Unknown capability strings are ignored for forward compatibility.

pub use anki_api_proto::anki::api::v1;
pub use anki_api_proto::anki::api::v1::create_note_request;

use std::collections::HashSet;
use std::pin::Pin;
use std::str::FromStr;
use std::task::Context;
use std::task::Poll;

use futures::Stream;
use prost14::Message;
use thiserror::Error;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::Code;
use tonic::Request;
use tonic::Streaming;

/// Raw tonic client for `HealthService`.
///
/// Prefer [`ApiClient`] for auth injection, capability bootstrap, and typed errors.
pub type HealthClient = v1::health_service_client::HealthServiceClient<tonic::transport::Channel>;
/// Raw tonic client for `DecksService`.
///
/// Prefer [`ApiClient`] for auth injection, capability bootstrap, and typed errors.
pub type DecksClient = v1::decks_service_client::DecksServiceClient<tonic::transport::Channel>;
/// Raw tonic client for `SystemService`.
///
/// Prefer [`ApiClient`] for auth injection, capability bootstrap, and typed errors.
pub type SystemClient = v1::system_service_client::SystemServiceClient<tonic::transport::Channel>;
/// Raw tonic client for `NotesService`.
///
/// Prefer [`ApiClient`] for auth injection, capability bootstrap, and typed errors.
pub type NotesClient = v1::notes_service_client::NotesServiceClient<tonic::transport::Channel>;
/// Raw tonic client for `NotetypesService`.
///
/// Prefer [`ApiClient`] for auth injection, capability bootstrap, and typed errors.
pub type NotetypesClient =
    v1::notetypes_service_client::NotetypesServiceClient<tonic::transport::Channel>;

const AUTH_HEADER: &str = "authorization";
const BEARER_PREFIX: &str = "Bearer ";

/// Stable SDK error detail codes serialized in `ErrorDetail.code`.
pub mod error_codes {
    /// Optimistic concurrency precondition failed (`expected_usn` mismatch).
    pub const VERSION_CONFLICT: &str = "VERSION_CONFLICT";
}

/// Server-advertised feature flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    HealthCheck,
    SystemServerInfo,
    DecksListRefs,
    DecksGetIdByName,
    NotesGet,
    NotesGetBatch,
    NotesCreate,
    NotesCreateBatch,
    NotesDelete,
    NotesListRefsStream,
    NotesListStream,
    NotesUpdateFields,
    NotesUpdateFieldsBatch,
    NotesChanges,
    NotesCount,
    NotetypesGet,
    NotetypesGetBatch,
    NotetypesGetIdByName,
    NotetypesListRefs,
    NotetypesList,
    NotetypesUpdateContent,
    NotetypesUpdateTemplates,
    NotetypesUpdateTemplatesBatch,
    NotetypesUpdateCss,
    NotetypesUpdateCssBatch,
    NotetypesChanges,
    NotetypesCount,
    AuthApiKey,
}

impl Capability {
    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "health.check" => Some(Self::HealthCheck),
            "system.server_info" => Some(Self::SystemServerInfo),
            "decks.list_refs" => Some(Self::DecksListRefs),
            "decks.get_id_by_name" => Some(Self::DecksGetIdByName),
            "notes.get" => Some(Self::NotesGet),
            "notes.get.batch" => Some(Self::NotesGetBatch),
            "notes.create" => Some(Self::NotesCreate),
            "notes.create.batch" => Some(Self::NotesCreateBatch),
            "notes.delete" => Some(Self::NotesDelete),
            "notes.list_refs.stream" => Some(Self::NotesListRefsStream),
            "notes.list.stream" => Some(Self::NotesListStream),
            "notes.update_fields" => Some(Self::NotesUpdateFields),
            "notes.update_fields.batch" => Some(Self::NotesUpdateFieldsBatch),
            "notes.changes" => Some(Self::NotesChanges),
            "notes.count" => Some(Self::NotesCount),
            "notetypes.get" => Some(Self::NotetypesGet),
            "notetypes.get.batch" => Some(Self::NotetypesGetBatch),
            "notetypes.get_id_by_name" => Some(Self::NotetypesGetIdByName),
            "notetypes.list_refs" => Some(Self::NotetypesListRefs),
            "notetypes.list" => Some(Self::NotetypesList),
            "notetypes.update_content" => Some(Self::NotetypesUpdateContent),
            "notetypes.update_templates" => Some(Self::NotetypesUpdateTemplates),
            "notetypes.update_templates.batch" => Some(Self::NotetypesUpdateTemplatesBatch),
            "notetypes.update_css" => Some(Self::NotetypesUpdateCss),
            "notetypes.update_css.batch" => Some(Self::NotetypesUpdateCssBatch),
            "notetypes.changes" => Some(Self::NotetypesChanges),
            "notetypes.count" => Some(Self::NotetypesCount),
            "auth.api_key" => Some(Self::AuthApiKey),
            _ => None,
        }
    }
}

/// Parsed capability set from `GetServerInfoResponse.capabilities`.
#[derive(Clone, Default)]
pub struct CapabilitySet {
    known: HashSet<Capability>,
}

impl CapabilitySet {
    /// Returns `true` when the server explicitly advertises the capability.
    ///
    /// Prefer this check before calling optional RPCs to support mixed server versions.
    pub fn has(&self, capability: Capability) -> bool {
        self.known.contains(&capability)
    }
}

impl std::fmt::Debug for CapabilitySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapabilitySet")
            .field("known_count", &self.known.len())
            .finish()
    }
}

/// Connection settings for [`ApiClient::connect`].
#[derive(Clone)]
pub struct ConnectionConfig {
    /// gRPC endpoint URI, for example `http://127.0.0.1:50051`.
    pub endpoint: String,
    /// Optional API key used as `Authorization: Bearer <key>`.
    pub api_key: Option<String>,
}

impl ConnectionConfig {
    /// Creates a config with no API key.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key: None,
        }
    }

    /// Sets the API key used for bearer authentication.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }
}

/// SDK-level error type.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Endpoint URI is invalid and could not be parsed.
    #[error("invalid endpoint URI: {0}")]
    InvalidEndpoint(String),
    /// API key could not be encoded as gRPC metadata.
    #[error("invalid api key metadata value")]
    InvalidApiKeyMetadata,
    /// Transport setup/connectivity error.
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
    /// Optimistic concurrency mismatch reported by server.
    #[error("version conflict (retryable={retryable}): {message}")]
    VersionConflict { retryable: bool, message: String },
    /// Any other RPC failure.
    #[error("rpc failed: {0}")]
    Rpc(#[from] tonic::Status),
}

// Manual Debug impls intentionally redact secrets so API keys/tokens are never
// exposed if client configuration/state is logged.
impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("endpoint", &self.endpoint)
            .field(
                "api_key",
                &self
                    .api_key
                    .as_ref()
                    .map(|_| "<redacted>")
                    .unwrap_or("<none>"),
            )
            .finish()
    }
}

/// High-level Anki API client with auth injection and capability bootstrap.
#[derive(Clone)]
pub struct ApiClient {
    channel: tonic::transport::Channel,
    authorization: Option<MetadataValue<Ascii>>,
    server_info: v1::GetServerInfoResponse,
    capabilities: CapabilitySet,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("channel", &"<tonic::transport::Channel>")
            .field("api_version", &self.server_info.api_version)
            .field("server_version", &self.server_info.server_version)
            .field("anki_version", &self.server_info.anki_version)
            .field("capability_count", &self.server_info.capabilities.len())
            .field(
                "authorization",
                &self
                    .authorization
                    .as_ref()
                    .map(|_| "<redacted>")
                    .unwrap_or("<none>"),
            )
            .finish()
    }
}

impl ApiClient {
    /// Connects to the server and eagerly fetches `GetServerInfo`.
    ///
    /// This method fails fast if the server is unreachable or auth is invalid.
    pub async fn connect(config: ConnectionConfig) -> Result<Self, ClientError> {
        let channel = connect_channel(&config.endpoint).await?;
        let authorization = if let Some(api_key) = config.api_key {
            let value = format!("{BEARER_PREFIX}{api_key}");
            let metadata =
                MetadataValue::from_str(&value).map_err(|_| ClientError::InvalidApiKeyMetadata)?;
            Some(metadata)
        } else {
            None
        };

        let mut client = Self {
            channel,
            authorization,
            server_info: v1::GetServerInfoResponse {
                api_version: String::new(),
                server_version: String::new(),
                capabilities: Vec::new(),
                anki_version: String::new(),
            },
            capabilities: CapabilitySet::default(),
        };
        client.server_info = client.fetch_server_info().await?;
        client.capabilities = parse_capabilities(&client.server_info.capabilities);
        Ok(client)
    }

    /// Calls custom `HealthService.Check`.
    pub async fn health_check(&self) -> Result<v1::HealthCheckResponse, ClientError> {
        let mut client = HealthClient::new(self.channel.clone());
        let request = self.request(v1::HealthCheckRequest {})?;
        let response = client
            .check(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Returns cached server info fetched during [`ApiClient::connect`].
    pub async fn get_server_info(&self) -> Result<v1::GetServerInfoResponse, ClientError> {
        Ok(self.server_info.clone())
    }

    /// Lists lightweight deck references (ID + name).
    pub async fn list_deck_refs(&self) -> Result<v1::ListDeckRefsResponse, ClientError> {
        let mut client = DecksClient::new(self.channel.clone());
        let request = self.request(v1::ListDeckRefsRequest {})?;
        let response = client
            .list_deck_refs(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Resolves one exact human deck name to a stable deck ID.
    pub async fn get_deck_id_by_name(&self, name: impl Into<String>) -> Result<i64, ClientError> {
        let mut client = DecksClient::new(self.channel.clone());
        let request = self.request(v1::GetDeckIdByNameRequest { name: name.into() })?;
        let response = client
            .get_deck_id_by_name(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response.deck_id)
    }

    /// Returns parsed typed capabilities advertised by server.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Returns raw capability strings as reported by server.
    pub fn capability_strings(&self) -> &[String] {
        &self.server_info.capabilities
    }

    /// Gets a note by ID.
    pub async fn get_note(&self, note_id: i64) -> Result<v1::GetNoteResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::GetNoteRequest { note_id })?;
        let response = client
            .get_note(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Gets multiple notes by ID in request order.
    pub async fn get_notes(&self, note_ids: Vec<i64>) -> Result<v1::GetNotesResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::GetNotesRequest { note_ids })?;
        let response = client
            .get_notes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Creates one note in the provided deck using a complete named field payload.
    pub async fn create_note(
        &self,
        notetype_id: i64,
        deck: v1::create_note_request::Deck,
        fields: Vec<v1::NoteFieldUpdate>,
    ) -> Result<v1::CreateNoteResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::CreateNoteRequest {
            notetype_id,
            deck: Some(deck),
            fields,
        })?;
        let response = client
            .create_note(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Creates multiple notes atomically in request order.
    pub async fn create_notes(
        &self,
        requests: Vec<v1::CreateNoteRequest>,
    ) -> Result<v1::CreateNotesResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::CreateNotesRequest { requests })?;
        let response = client
            .create_notes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Creates one note addressed by human-readable deck name.
    pub async fn create_note_in_deck_name(
        &self,
        notetype_id: i64,
        deck_name: impl Into<String>,
        fields: Vec<v1::NoteFieldUpdate>,
    ) -> Result<v1::CreateNoteResponse, ClientError> {
        self.create_note(
            notetype_id,
            v1::create_note_request::Deck::DeckName(deck_name.into()),
            fields,
        )
        .await
    }

    /// Creates one note addressed by internal deck ID.
    pub async fn create_note_in_deck_id(
        &self,
        notetype_id: i64,
        deck_id: i64,
        fields: Vec<v1::NoteFieldUpdate>,
    ) -> Result<v1::CreateNoteResponse, ClientError> {
        self.create_note(
            notetype_id,
            v1::create_note_request::Deck::DeckId(deck_id),
            fields,
        )
        .await
    }

    /// Deletes multiple notes by ID.
    pub async fn delete_notes(&self, note_ids: Vec<i64>) -> Result<u64, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::DeleteNotesRequest { note_ids })?;
        let response = client
            .delete_notes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response.deleted_count)
    }

    /// Streams note references, optionally filtered by backend search query.
    pub async fn list_note_refs(
        &self,
        query: Option<String>,
        offset: Option<u64>,
        limit: Option<u64>,
        order_by: Vec<v1::NoteOrdering>,
    ) -> Result<NoteRefsStream, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::ListNoteRefsRequest {
            query: query.unwrap_or_default(),
            offset: offset.unwrap_or(0),
            limit: limit.unwrap_or(0),
            order_by,
        })?;
        let response = client
            .list_note_refs(request)
            .await
            .map_err(Self::map_status)?;
        Ok(ResponseStream::new(response.into_inner()))
    }

    /// Streams notes, optionally filtered by backend search query.
    pub async fn list_notes(
        &self,
        query: Option<String>,
        offset: Option<u64>,
        limit: Option<u64>,
        order_by: Vec<v1::NoteOrdering>,
    ) -> Result<NotesStream, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::ListNotesRequest {
            query: query.unwrap_or_default(),
            offset: offset.unwrap_or(0),
            limit: limit.unwrap_or(0),
            order_by,
        })?;
        let response = client.list_notes(request).await.map_err(Self::map_status)?;
        Ok(ResponseStream::new(response.into_inner()))
    }

    /// Updates one note's fields.
    pub async fn update_note_fields(
        &self,
        note_id: i64,
        fields: Vec<v1::NoteFieldUpdate>,
        expected_usn: Option<i64>,
    ) -> Result<v1::UpdateNoteFieldsResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateNoteFieldsRequest {
            note_id,
            fields,
            expected_usn,
        })?;
        let response = client
            .update_note_fields(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Updates multiple notes.
    ///
    /// Server semantics: non-atomic ordered execution with stop-on-first-error.
    /// Successful earlier updates are not rolled back.
    pub async fn update_note_fields_batch(
        &self,
        updates: Vec<v1::UpdateNoteFieldsRequest>,
    ) -> Result<v1::UpdateNoteFieldsBatchResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateNoteFieldsBatchRequest { updates })?;
        let response = client
            .update_note_fields_batch(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Returns note changes since cursor.
    pub async fn get_note_changes(
        &self,
        cursor: impl Into<String>,
        limit: u32,
    ) -> Result<v1::GetNoteChangesResponse, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::GetNoteChangesRequest {
            cursor: cursor.into(),
            limit,
        })?;
        let response = client
            .get_note_changes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Returns the total number of notes matching a query.
    pub async fn count_notes(&self, query: Option<String>) -> Result<u64, ClientError> {
        let mut client = NotesClient::new(self.channel.clone());
        let request = self.request(v1::CountNotesRequest {
            query: query.unwrap_or_default(),
        })?;
        let response = client
            .count_notes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response.count)
    }

    /// Lists lightweight notetype references (ID + name only).
    pub async fn list_notetype_refs(&self) -> Result<v1::ListNotetypeRefsResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::ListNotetypeRefsRequest {})?;
        let response = client
            .list_notetype_refs(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Lists all notetypes.
    pub async fn list_notetypes(&self) -> Result<v1::ListNotetypesResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::ListNotetypesRequest {})?;
        let response = client
            .list_notetypes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Gets one notetype by ID.
    pub async fn get_notetype(
        &self,
        notetype_id: i64,
    ) -> Result<v1::GetNotetypeResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::GetNotetypeRequest { notetype_id })?;
        let response = client
            .get_notetype(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Gets multiple notetypes by ID in request order.
    pub async fn get_notetypes(
        &self,
        notetype_ids: Vec<i64>,
    ) -> Result<v1::GetNotetypesResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::GetNotetypesRequest { notetype_ids })?;
        let response = client
            .get_notetypes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Resolves a notetype ID from an exact notetype name.
    pub async fn get_notetype_id_by_name(
        &self,
        name: impl Into<String>,
    ) -> Result<i64, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::GetNotetypeIdByNameRequest { name: name.into() })?;
        let response = client
            .get_notetype_id_by_name(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response.notetype_id)
    }

    /// Updates template content for one notetype.
    pub async fn update_templates(
        &self,
        notetype_id: i64,
        templates: Vec<v1::NotetypeTemplate>,
        expected_usn: Option<i64>,
    ) -> Result<v1::UpdateTemplatesResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateTemplatesRequest {
            notetype_id,
            templates,
            expected_usn,
        })?;
        let response = client
            .update_templates(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Updates template content for multiple notetypes.
    ///
    /// Server semantics: non-atomic ordered execution with stop-on-first-error.
    /// Successful earlier updates are not rolled back.
    pub async fn update_templates_batch(
        &self,
        updates: Vec<v1::UpdateTemplatesRequest>,
    ) -> Result<v1::UpdateTemplatesBatchResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateTemplatesBatchRequest { updates })?;
        let response = client
            .update_templates_batch(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Updates CSS for one notetype.
    pub async fn update_css(
        &self,
        notetype_id: i64,
        css: impl Into<String>,
        expected_usn: Option<i64>,
    ) -> Result<v1::UpdateCssResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateCssRequest {
            notetype_id,
            css: css.into(),
            expected_usn,
        })?;
        let response = client
            .update_css(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Atomically updates CSS + templates for one notetype.
    pub async fn update_notetype_content(
        &self,
        notetype_id: i64,
        templates: Vec<v1::NotetypeTemplate>,
        css: impl Into<String>,
        expected_usn: Option<i64>,
    ) -> Result<v1::UpdateNotetypeContentResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateNotetypeContentRequest {
            notetype_id,
            templates,
            css: css.into(),
            expected_usn,
        })?;
        let response = client
            .update_notetype_content(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Updates CSS for multiple notetypes.
    ///
    /// Server semantics: non-atomic ordered execution with stop-on-first-error.
    /// Successful earlier updates are not rolled back.
    pub async fn update_css_batch(
        &self,
        updates: Vec<v1::UpdateCssRequest>,
    ) -> Result<v1::UpdateCssBatchResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::UpdateCssBatchRequest { updates })?;
        let response = client
            .update_css_batch(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Returns notetype changes since cursor.
    pub async fn get_notetype_changes(
        &self,
        cursor: impl Into<String>,
        limit: u32,
    ) -> Result<v1::GetNotetypeChangesResponse, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::GetNotetypeChangesRequest {
            cursor: cursor.into(),
            limit,
        })?;
        let response = client
            .get_notetype_changes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    /// Returns the total number of notetypes.
    pub async fn count_notetypes(&self) -> Result<u64, ClientError> {
        let mut client = NotetypesClient::new(self.channel.clone());
        let request = self.request(v1::CountNotetypesRequest {})?;
        let response = client
            .count_notetypes(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response.count)
    }

    fn request<T>(&self, message: T) -> Result<Request<T>, ClientError> {
        let mut request = Request::new(message);
        if let Some(value) = &self.authorization {
            request.metadata_mut().insert(AUTH_HEADER, value.clone());
        }
        Ok(request)
    }

    async fn fetch_server_info(&self) -> Result<v1::GetServerInfoResponse, ClientError> {
        let mut client = SystemClient::new(self.channel.clone());
        let request = self.request(v1::GetServerInfoRequest {})?;
        let response = client
            .get_server_info(request)
            .await
            .map_err(Self::map_status)?
            .into_inner();
        Ok(response)
    }

    fn map_status(status: tonic::Status) -> ClientError {
        if status.code() == Code::Aborted {
            if let Ok(detail) = v1::ErrorDetail::decode(status.details()) {
                if detail.code == error_codes::VERSION_CONFLICT {
                    return ClientError::VersionConflict {
                        retryable: detail.retryable,
                        message: status.message().to_owned(),
                    };
                }
            }
        }
        ClientError::Rpc(status)
    }
}

/// Streaming wrapper that maps all mid-stream errors into [`ClientError`].
pub struct ResponseStream<T> {
    inner: Streaming<T>,
}

impl<T> ResponseStream<T> {
    fn new(inner: Streaming<T>) -> Self {
        Self { inner }
    }

    /// Returns the underlying tonic stream for advanced use-cases.
    pub fn into_inner(self) -> Streaming<T> {
        self.inner
    }
}

impl<T> Stream for ResponseStream<T> {
    type Item = Result<T, ClientError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.get_mut().inner).poll_next(cx) {
            Poll::Ready(Some(Ok(message))) => Poll::Ready(Some(Ok(message))),
            Poll::Ready(Some(Err(status))) => Poll::Ready(Some(Err(ApiClient::map_status(status)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Stream of `ListNotesResponse` messages.
pub type NotesStream = ResponseStream<v1::ListNotesResponse>;
/// Stream of `ListNoteRefsResponse` messages.
pub type NoteRefsStream = ResponseStream<v1::ListNoteRefsResponse>;

/// Connects a raw tonic channel to an Anki API endpoint.
pub async fn connect_channel(
    endpoint: impl AsRef<str>,
) -> Result<tonic::transport::Channel, ClientError> {
    let endpoint_str = endpoint.as_ref().to_owned();
    let endpoint = tonic::transport::Endpoint::from_shared(endpoint_str.clone())
        .map_err(|_| ClientError::InvalidEndpoint(endpoint_str))?;
    endpoint.connect().await.map_err(Into::into)
}

fn parse_capabilities(values: &[String]) -> CapabilitySet {
    let known = values
        .iter()
        .filter_map(|value| Capability::from_wire(value))
        .collect();
    CapabilitySet { known }
}

#[cfg(test)]
mod tests {
    use tonic::Code;

    use super::*;

    #[test]
    fn parses_notetype_get_id_by_name_capability() {
        let caps = parse_capabilities(&["notetypes.get_id_by_name".to_owned()]);
        assert!(caps.has(Capability::NotetypesGetIdByName));
    }

    #[test]
    fn parses_deck_capabilities() {
        let caps = parse_capabilities(&[
            "decks.list_refs".to_owned(),
            "decks.get_id_by_name".to_owned(),
        ]);
        assert!(caps.has(Capability::DecksListRefs));
        assert!(caps.has(Capability::DecksGetIdByName));
    }

    #[test]
    fn parses_notetype_list_refs_capability() {
        let caps = parse_capabilities(&["notetypes.list_refs".to_owned()]);
        assert!(caps.has(Capability::NotetypesListRefs));
    }

    #[test]
    fn parses_get_batch_capabilities() {
        let caps = parse_capabilities(&[
            "notes.get.batch".to_owned(),
            "notetypes.get.batch".to_owned(),
        ]);
        assert!(caps.has(Capability::NotesGetBatch));
        assert!(caps.has(Capability::NotetypesGetBatch));
    }

    #[test]
    fn parses_notes_create_capability() {
        let caps = parse_capabilities(&["notes.create".to_owned()]);
        assert!(caps.has(Capability::NotesCreate));
    }

    #[test]
    fn parses_notes_create_batch_capability() {
        let caps = parse_capabilities(&["notes.create.batch".to_owned()]);
        assert!(caps.has(Capability::NotesCreateBatch));
    }

    #[test]
    fn parses_notes_delete_capability() {
        let caps = parse_capabilities(&["notes.delete".to_owned()]);
        assert!(caps.has(Capability::NotesDelete));
    }

    #[test]
    fn maps_not_found_status_to_rpc_error() {
        let error = ApiClient::map_status(tonic::Status::not_found("missing notetype"));
        match error {
            ClientError::Rpc(status) => assert_eq!(status.code(), Code::NotFound),
            other => panic!("expected rpc error, got {other:?}"),
        }
    }
}
