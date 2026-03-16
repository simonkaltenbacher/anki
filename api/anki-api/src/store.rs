use std::path::PathBuf;
use std::sync::Arc;

use anki::backend::Backend;
use anki::backend::init_backend;
use anki::services::BackendCollectionService;
use anki_proto::backend::BackendError;
use anki_proto::backend::BackendInit;
use anki_proto::backend::backend_error::Kind as BackendErrorKind;
use anki_proto::collection::OpChanges;
use anki_proto::collection::OpenCollectionRequest;
use anki_proto::decks::DeckId;
use anki_proto::decks::DeckNames;
use anki_proto::decks::GetDeckNamesRequest;
use anki_proto::generic::String as GenericString;
use anki_proto::notes::AddNoteRequest;
use anki_proto::notes::AddNoteResponse;
use anki_proto::notes::GetNoteChangesPageRequest;
use anki_proto::notes::GetNoteChangesPageResponse;
use anki_proto::notes::Note;
use anki_proto::notes::NoteId;
use anki_proto::notes::RemoveNotesRequest;
use anki_proto::notes::UpdateNotesRequest;
#[cfg(test)]
use anki_proto::notes::{DeckAndNotetype, DefaultsForAddingRequest};
use anki_proto::notetypes::GetNotetypeChangesPageRequest;
use anki_proto::notetypes::GetNotetypeChangesPageResponse;
use anki_proto::notetypes::Notetype;
use anki_proto::notetypes::NotetypeId;
use anki_proto::notetypes::NotetypeNames;
use anki_proto::search::SearchRequest;
use anki_proto::search::SearchResponse;
use anki_proto::search::SortOrder;
use prost::Message;
use thiserror::Error;
use tonic::Status;

// Service indices are derived from `backend.proto` service declaration order
// and consumed by `Backend::run_service_method(service, method, ...)`.
// Keep these in sync with `proto/anki/backend.proto` + generated dispatch in
// `anki_proto::backend::BackendService`.
const SERVICE_NOTETYPES: u32 = 23;
const SERVICE_NOTES: u32 = 25;
const SERVICE_SEARCH: u32 = 29;
const SERVICE_DECKS: u32 = 7;

// Method indices are derived from RPC declaration order in each service proto:
// - SERVICE_NOTETYPES: `proto/anki/notetypes.proto`
// - SERVICE_NOTES: `proto/anki/notes.proto`
// - SERVICE_SEARCH: `proto/anki/search.proto`
const METHOD_NOTETYPES_GET: u32 = 6;
const METHOD_NOTETYPES_GET_NAMES: u32 = 8;
const METHOD_NOTETYPES_GET_ID_BY_NAME: u32 = 10;
const METHOD_NOTETYPES_UPDATE: u32 = 1;
const METHOD_NOTES_NEW: u32 = 0;
const METHOD_NOTES_ADD: u32 = 1;
#[cfg(test)]
const METHOD_NOTES_DEFAULTS_FOR_ADDING: u32 = 3;
const METHOD_NOTES_UPDATE: u32 = 5;
const METHOD_NOTES_GET: u32 = 6;
const METHOD_NOTES_REMOVE: u32 = 7;
const METHOD_NOTES_GET_CHANGES_PAGE: u32 = 14;
const METHOD_DECKS_GET_ID_BY_NAME: u32 = 7;
const METHOD_DECKS_GET_NAMES: u32 = 13;
const METHOD_SEARCH_NOTES: u32 = 2;
const METHOD_NOTETYPES_GET_CHANGES_PAGE: u32 = 19;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to initialize backend: {0}")]
    BackendInit(String),
    #[error("failed to initialize backend collection: {0}")]
    CollectionInit(#[from] anki::error::AnkiError),
    #[error("failed to prepare media directory: {0}")]
    MediaDir(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct BackendStore {
    backend: Backend,
}

pub type SharedStore = Arc<BackendStore>;

impl BackendStore {
    pub fn get_note(&self, note_id: i64) -> Result<Note, Status> {
        self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_GET,
            Some(NoteId { nid: note_id }),
        )
    }

    pub fn search_note_ids_with_query(
        &self,
        query: &str,
        order: Option<SortOrder>,
    ) -> Result<Vec<i64>, Status> {
        let response: SearchResponse = self.run_method(
            SERVICE_SEARCH,
            METHOD_SEARCH_NOTES,
            Some(SearchRequest {
                search: query.to_owned(),
                order,
            }),
        )?;
        Ok(response.ids)
    }

    pub fn get_notetype(&self, notetype_id: i64) -> Result<Notetype, Status> {
        self.run_method(
            SERVICE_NOTETYPES,
            METHOD_NOTETYPES_GET,
            Some(NotetypeId { ntid: notetype_id }),
        )
    }

    pub fn list_notetype_ids(&self) -> Result<Vec<i64>, Status> {
        Ok(self
            .list_notetype_refs()?
            .into_iter()
            .map(|(id, _name)| id)
            .collect())
    }

    pub fn list_notetype_refs(&self) -> Result<Vec<(i64, String)>, Status> {
        // Backend does not expose a batch "get all notetype payloads" RPC,
        // so list_notetypes currently does names/ids + per-id fetches.
        let response: NotetypeNames = self
            .run_method::<anki_proto::generic::Empty, NotetypeNames>(
                SERVICE_NOTETYPES,
                METHOD_NOTETYPES_GET_NAMES,
                None,
            )?;
        let mut refs: Vec<(i64, String)> = response
            .entries
            .into_iter()
            .map(|entry| (entry.id, entry.name))
            .collect();
        refs.sort_by_key(|(id, _name)| *id);
        Ok(refs)
    }

    pub fn get_notetype_id_by_name(&self, name: &str) -> Result<i64, Status> {
        let response: NotetypeId = self.run_method(
            SERVICE_NOTETYPES,
            METHOD_NOTETYPES_GET_ID_BY_NAME,
            Some(GenericString {
                val: name.to_owned(),
            }),
        )?;
        Ok(response.ntid)
    }

    pub fn get_deck_id_by_name(&self, name: &str) -> Result<i64, Status> {
        let response: DeckId = self.run_method(
            SERVICE_DECKS,
            METHOD_DECKS_GET_ID_BY_NAME,
            Some(GenericString {
                val: name.to_owned(),
            }),
        )?;
        Ok(response.did)
    }

    pub fn list_deck_refs(&self) -> Result<Vec<(i64, String)>, Status> {
        let response: DeckNames = self.run_method(
            SERVICE_DECKS,
            METHOD_DECKS_GET_NAMES,
            Some(GetDeckNamesRequest {
                skip_empty_default: false,
                include_filtered: true,
            }),
        )?;
        let mut refs: Vec<(i64, String)> = response
            .entries
            .into_iter()
            .map(|entry| (entry.id, entry.name))
            .collect();
        refs.sort_by_key(|(id, _name)| *id);
        Ok(refs)
    }

    pub fn update_note_fields(&self, mut note: Note, fields: Vec<String>) -> Result<Note, Status> {
        note.fields = fields;
        let note_id = note.id;

        let _: OpChanges = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_UPDATE,
            Some(UpdateNotesRequest {
                notes: vec![note],
                skip_undo_entry: false,
            }),
        )?;

        self.get_note(note_id)
    }

    pub fn create_note(
        &self,
        notetype_id: i64,
        deck_id: i64,
        fields: Vec<String>,
    ) -> Result<Note, Status> {
        let mut note: Note = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_NEW,
            Some(NotetypeId { ntid: notetype_id }),
        )?;
        note.fields = fields;

        let added: AddNoteResponse = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_ADD,
            Some(AddNoteRequest {
                note: Some(note),
                deck_id,
            }),
        )?;

        self.get_note(added.note_id)
    }

    pub fn delete_notes(&self, note_ids: Vec<i64>) -> Result<u32, Status> {
        let removed: anki_proto::collection::OpChangesWithCount = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_REMOVE,
            Some(RemoveNotesRequest {
                note_ids,
                card_ids: Vec::new(),
            }),
        )?;

        Ok(removed.count)
    }

    pub fn update_notetype(&self, notetype: Notetype) -> Result<(), Status> {
        let _: OpChanges =
            self.run_method(SERVICE_NOTETYPES, METHOD_NOTETYPES_UPDATE, Some(notetype))?;
        Ok(())
    }

    pub fn get_note_changes_page(
        &self,
        after_usn: i64,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<(i64, i64, i64)>, Status> {
        let response: GetNoteChangesPageResponse = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_GET_CHANGES_PAGE,
            Some(GetNoteChangesPageRequest {
                after_usn,
                after_id,
                limit,
            }),
        )?;

        Ok(response
            .entries
            .into_iter()
            .map(|entry| {
                (
                    i64::from(entry.usn),
                    entry.note_id,
                    entry.mtime_secs,
                )
            })
            .collect())
    }

    pub fn get_notetype_changes_page(
        &self,
        after_usn: i64,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<(i64, i64, i64)>, Status> {
        let response: GetNotetypeChangesPageResponse = self.run_method(
            SERVICE_NOTETYPES,
            METHOD_NOTETYPES_GET_CHANGES_PAGE,
            Some(GetNotetypeChangesPageRequest {
                after_usn,
                after_id,
                limit,
            }),
        )?;

        Ok(response
            .entries
            .into_iter()
            .map(|entry| (i64::from(entry.usn), entry.notetype_id, entry.mtime_secs))
            .collect())
    }

    #[cfg(test)]
    pub(crate) fn create_test_note(&self) -> Result<Note, Status> {
        self.create_test_note_with_fields("api-test-front", Some("api-test-back"))
    }

    #[cfg(test)]
    pub(crate) fn create_test_note_with_fields(
        &self,
        front: &str,
        back: Option<&str>,
    ) -> Result<Note, Status> {
        let defaults: DeckAndNotetype = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_DEFAULTS_FOR_ADDING,
            Some(DefaultsForAddingRequest {
                home_deck_of_current_review_card: 0,
            }),
        )?;

        let mut note: Note = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_NEW,
            Some(NotetypeId {
                ntid: defaults.notetype_id,
            }),
        )?;
        if let Some(first) = note.fields.first_mut() {
            *first = front.to_owned();
        }
        if let (Some(back), true) = (back, note.fields.len() > 1) {
            note.fields[1] = back.to_owned();
        }

        let added: AddNoteResponse = self.run_method(
            SERVICE_NOTES,
            METHOD_NOTES_ADD,
            Some(AddNoteRequest {
                note: Some(note),
                deck_id: defaults.deck_id,
            }),
        )?;
        self.get_note(added.note_id)
    }

    fn run_method<I, O>(&self, service: u32, method: u32, input: Option<I>) -> Result<O, Status>
    where
        I: Message,
        O: Message + Default,
    {
        let input_bytes = if let Some(input) = input {
            input.encode_to_vec()
        } else {
            Vec::new()
        };

        let output_bytes = self
            .backend
            .run_service_method(service, method, &input_bytes)
            .map_err(status_from_backend_error_bytes)?;

        O::decode(output_bytes.as_slice())
            .map_err(|_| Status::internal("invalid backend response payload"))
    }
}

pub fn shared_store_from_backend(backend: Backend) -> SharedStore {
    Arc::new(BackendStore { backend })
}

pub fn initialize_store(collection_path: PathBuf) -> Result<SharedStore, StoreError> {
    initialize_store_with_collection_path(collection_path)
}

#[cfg(test)]
pub(crate) fn initialize_store_for_test(
    collection_path: PathBuf,
) -> Result<SharedStore, StoreError> {
    initialize_store_with_collection_path(collection_path)
}

fn initialize_store_with_collection_path(
    collection_path: PathBuf,
) -> Result<SharedStore, StoreError> {
    let backend = init_backend(
        &BackendInit {
            preferred_langs: vec![],
            locale_folder_path: String::new(),
            server: false,
        }
        .encode_to_vec(),
    )
    .map_err(StoreError::BackendInit)?;
    let (collection_path, media_folder_path, media_db_path) =
        media_paths_from_collection_path(collection_path)?;
    BackendCollectionService::open_collection(
        &backend,
        OpenCollectionRequest {
            collection_path,
            media_folder_path,
            media_db_path,
        },
    )?;

    Ok(Arc::new(BackendStore { backend }))
}

fn media_paths_from_collection_path(
    collection_path: PathBuf,
) -> Result<(String, String, String), std::io::Error> {
    let media_folder = collection_path.with_extension("media");
    std::fs::create_dir_all(&media_folder)?;
    let media_db = collection_path.with_extension("mdb");

    Ok((
        collection_path.to_string_lossy().into_owned(),
        media_folder.to_string_lossy().into_owned(),
        media_db.to_string_lossy().into_owned(),
    ))
}

fn status_from_backend_error_bytes(err_bytes: Vec<u8>) -> Status {
    match BackendError::decode(err_bytes.as_slice()) {
        Ok(err) => status_from_backend_error(err),
        Err(_) => {
            tracing::error!(
                payload_len = err_bytes.len(),
                "failed to decode backend error payload"
            );
            Status::internal("backend call failed")
        }
    }
}

fn status_from_backend_error(err: BackendError) -> Status {
    let kind = BackendErrorKind::try_from(err.kind).ok();

    match kind {
        Some(BackendErrorKind::InvalidInput) => {
            tracing::debug!(message = %err.message, "backend returned invalid input");
            Status::invalid_argument(err.message)
        }
        Some(BackendErrorKind::NotFoundError) => {
            tracing::debug!(message = %err.message, "backend resource not found");
            Status::not_found(err.message)
        }
        Some(BackendErrorKind::Exists) => {
            tracing::debug!(message = %err.message, "backend resource already exists");
            Status::already_exists(err.message)
        }
        Some(BackendErrorKind::UndoEmpty) => {
            tracing::debug!("backend undo stack is empty");
            Status::failed_precondition("undo stack is empty")
        }
        Some(BackendErrorKind::Interrupted) => {
            tracing::debug!("backend operation interrupted");
            Status::cancelled("operation interrupted")
        }
        Some(BackendErrorKind::Deleted) => {
            tracing::debug!("backend resource deleted");
            Status::not_found("resource deleted")
        }
        Some(BackendErrorKind::SchedulerUpgradeRequired) => {
            tracing::debug!("backend scheduler upgrade required");
            Status::failed_precondition("scheduler upgrade required")
        }
        Some(BackendErrorKind::InvalidCertificateFormat) => {
            tracing::debug!("backend invalid certificate format");
            Status::invalid_argument("invalid certificate format")
        }
        _ => {
            tracing::error!(
                kind = ?kind,
                message = %err.message,
                context = %err.context,
                backtrace = %err.backtrace,
                "backend call failed"
            );
            Status::internal("internal server error")
        }
    }
}
