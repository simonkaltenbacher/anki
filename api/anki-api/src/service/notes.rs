use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use anki_api_proto::anki::api::v1::CountNotesRequest;
use anki_api_proto::anki::api::v1::CountNotesResponse;
use anki_api_proto::anki::api::v1::CreateNoteRequest;
use anki_api_proto::anki::api::v1::CreateNoteResponse;
use anki_api_proto::anki::api::v1::CreateNotesRequest;
use anki_api_proto::anki::api::v1::CreateNotesResponse;
use anki_api_proto::anki::api::v1::DeleteNotesRequest;
use anki_api_proto::anki::api::v1::DeleteNotesResponse;
use anki_api_proto::anki::api::v1::GetNoteChangesRequest;
use anki_api_proto::anki::api::v1::GetNoteChangesResponse;
use anki_api_proto::anki::api::v1::GetNoteRequest;
use anki_api_proto::anki::api::v1::GetNoteResponse;
use anki_api_proto::anki::api::v1::GetNotesRequest;
use anki_api_proto::anki::api::v1::GetNotesResponse;
use anki_api_proto::anki::api::v1::ListNoteRefsRequest;
use anki_api_proto::anki::api::v1::ListNoteRefsResponse;
use anki_api_proto::anki::api::v1::ListNotesRequest;
use anki_api_proto::anki::api::v1::ListNotesResponse;
use anki_api_proto::anki::api::v1::NoteChange;
use anki_api_proto::anki::api::v1::NoteOrdering;
use anki_api_proto::anki::api::v1::NoteRef;
use anki_api_proto::anki::api::v1::NoteSortColumn;
use anki_api_proto::anki::api::v1::NoteWriteMetadata;
use anki_api_proto::anki::api::v1::SortDirection;
use anki_api_proto::anki::api::v1::UpdateNoteFieldsBatchRequest;
use anki_api_proto::anki::api::v1::UpdateNoteFieldsBatchResponse;
use anki_api_proto::anki::api::v1::UpdateNoteFieldsRequest;
use anki_api_proto::anki::api::v1::UpdateNoteFieldsResponse;
use anki_api_proto::anki::api::v1::create_note_request::Deck;
use anki_api_proto::anki::api::v1::notes_service_server::NotesService;
use anki_proto::notes::AddNoteRequest as BackendAddNoteRequest;
use anki_proto::search::SortOrder as BackendSortOrder;
use anki_proto::search::sort_order::Value as BackendSortOrderValue;
use futures::Stream;
use tokio::sync::mpsc;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::adapter;
use crate::service::common;
use crate::store::BackendStore;

#[derive(Clone)]
pub struct NotesApi {
    store: BackendStore,
}

type NotetypeCache = HashMap<i64, anki_proto::notetypes::Notetype>;
type DeckCache = HashMap<String, i64>;
pub struct ListNotesStream {
    receiver: mpsc::Receiver<Result<ListNotesResponse, Status>>,
}

impl Stream for ListNotesStream {
    type Item = Result<ListNotesResponse, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().receiver.poll_recv(cx)
    }
}

pub struct ListNoteRefsStream {
    receiver: mpsc::Receiver<Result<ListNoteRefsResponse, Status>>,
}

impl Stream for ListNoteRefsStream {
    type Item = Result<ListNoteRefsResponse, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().receiver.poll_recv(cx)
    }
}

impl NotesApi {
    pub fn new(store: BackendStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl NotesService for NotesApi {
    type ListNoteRefsStream = ListNoteRefsStream;
    type ListNotesStream = ListNotesStream;

    async fn get_note(
        &self,
        request: Request<GetNoteRequest>,
    ) -> Result<Response<GetNoteResponse>, Status> {
        let note_id = request.into_inner().note_id;
        if note_id <= 0 {
            return Err(Status::invalid_argument("note_id must be > 0"));
        }

        let mut notetype_cache = NotetypeCache::default();
        let note = self.store.get_note(note_id)?;
        let notetype = notetype_cache.get_or_load(&self.store, note.notetype_id)?;
        let api_note = adapter::map_note(&note, &notetype)?;

        Ok(Response::new(GetNoteResponse {
            note: Some(api_note),
        }))
    }

    async fn get_notes(
        &self,
        request: Request<GetNotesRequest>,
    ) -> Result<Response<GetNotesResponse>, Status> {
        let mut notetype_cache = NotetypeCache::default();
        let note_ids = request.into_inner().note_ids;
        let mut notes = Vec::with_capacity(note_ids.len());

        for note_id in note_ids {
            if note_id <= 0 {
                return Err(Status::invalid_argument("note_id must be > 0"));
            }
            let note = self.store.get_note(note_id)?;
            let notetype = notetype_cache.get_or_load(&self.store, note.notetype_id)?;
            notes.push(adapter::map_note(&note, &notetype)?);
        }

        Ok(Response::new(GetNotesResponse { notes }))
    }

    async fn create_note(
        &self,
        request: Request<CreateNoteRequest>,
    ) -> Result<Response<CreateNoteResponse>, Status> {
        let mut notetype_cache = NotetypeCache::default();
        let mut deck_cache = DeckCache::default();
        let response = create_note_inner(
            &self.store,
            request.into_inner(),
            &mut notetype_cache,
            &mut deck_cache,
        )?;
        Ok(Response::new(response))
    }

    async fn create_notes(
        &self,
        request: Request<CreateNotesRequest>,
    ) -> Result<Response<CreateNotesResponse>, Status> {
        let requests = request.into_inner().requests;
        if requests.is_empty() {
            return Err(Status::invalid_argument("requests must not be empty"));
        }

        let mut notetype_cache = NotetypeCache::default();
        let mut deck_cache = DeckCache::default();
        let mut add_requests = Vec::with_capacity(requests.len());

        for (index, request) in requests.into_iter().enumerate() {
            let (fields, deck_id, notetype_id) = prepare_create_note_fields_and_deck(
                &self.store,
                request,
                &mut notetype_cache,
                &mut deck_cache,
            )
            .map_err(|status| common::annotate_batch_status(status, "create_notes", index))?;
            let mut note = self
                .store
                .new_note(notetype_id)
                .map_err(|status| common::annotate_batch_status(status, "create_notes", index))?;
            note.fields = fields;
            add_requests.push(BackendAddNoteRequest {
                note: Some(note),
                deck_id,
            });
        }

        let note_ids = self.store.add_notes(add_requests)?;
        let mut notes = Vec::with_capacity(note_ids.len());
        for note_id in note_ids {
            let note = self.store.get_note(note_id)?;
            let notetype = notetype_cache.get_or_load(&self.store, note.notetype_id)?;
            notes.push(adapter::map_note(&note, &notetype)?);
        }

        Ok(Response::new(CreateNotesResponse { notes }))
    }

    async fn delete_notes(
        &self,
        request: Request<DeleteNotesRequest>,
    ) -> Result<Response<DeleteNotesResponse>, Status> {
        let note_ids = request.into_inner().note_ids;
        if note_ids.is_empty() {
            return Err(Status::invalid_argument("note_ids must not be empty"));
        }
        if note_ids.iter().any(|note_id| *note_id <= 0) {
            return Err(Status::invalid_argument("note_ids must all be > 0"));
        }

        let deleted_count = self.store.delete_notes(note_ids)?;
        Ok(Response::new(DeleteNotesResponse {
            deleted_count: u64::from(deleted_count),
        }))
    }

    async fn list_note_refs(
        &self,
        request: Request<ListNoteRefsRequest>,
    ) -> Result<Response<Self::ListNoteRefsStream>, Status> {
        let req = request.into_inner();
        let order = note_order_by_to_backend_sort(req.order_by)?;
        Ok(Response::new(self.stream_note_refs_for_query(
            req.query, req.offset, req.limit, order,
        )))
    }

    async fn list_notes(
        &self,
        request: Request<ListNotesRequest>,
    ) -> Result<Response<Self::ListNotesStream>, Status> {
        let req = request.into_inner();
        let order = note_order_by_to_backend_sort(req.order_by)?;
        Ok(Response::new(self.stream_notes_for_query(
            req.query, req.offset, req.limit, order,
        )))
    }

    async fn update_note_fields(
        &self,
        request: Request<UpdateNoteFieldsRequest>,
    ) -> Result<Response<UpdateNoteFieldsResponse>, Status> {
        let mut notetype_cache = NotetypeCache::default();
        let response =
            update_note_fields_inner(&self.store, request.into_inner(), &mut notetype_cache)?;
        Ok(Response::new(response))
    }

    async fn update_note_fields_batch(
        &self,
        request: Request<UpdateNoteFieldsBatchRequest>,
    ) -> Result<Response<UpdateNoteFieldsBatchResponse>, Status> {
        let mut notetype_cache = NotetypeCache::default();
        let updates = request.into_inner().updates;
        let results = common::execute_batch(updates, "update_note_fields_batch", |update| {
            let response = update_note_fields_inner(&self.store, update, &mut notetype_cache)?;
            let note = response
                .note
                .ok_or_else(|| Status::internal("batch update response missing note payload"))?;
            Ok(NoteWriteMetadata {
                note_id: note.note_id,
                usn: note.usn,
                sort_field: note.sort_field,
            })
        })?;
        Ok(Response::new(UpdateNoteFieldsBatchResponse { results }))
    }

    async fn get_note_changes(
        &self,
        request: Request<GetNoteChangesRequest>,
    ) -> Result<Response<GetNoteChangesResponse>, Status> {
        let request = request.into_inner();
        let (rows, next_cursor) =
            common::get_changes_page(&request.cursor, request.limit, |cursor, limit| {
                self.store.get_note_changes_page(cursor.0, cursor.1, limit)
            })?;

        let changes: Vec<NoteChange> = rows
            .iter()
            .map(|(usn, note_id, mtime_secs)| NoteChange {
                note_id: *note_id,
                modified_at: Some(common::timestamp_from_secs(*mtime_secs)),
                usn: *usn,
            })
            .collect();

        Ok(Response::new(GetNoteChangesResponse {
            changes,
            next_cursor,
        }))
    }

    async fn count_notes(
        &self,
        request: Request<CountNotesRequest>,
    ) -> Result<Response<CountNotesResponse>, Status> {
        let query = request.into_inner().query;
        let ids = self.store.search_note_ids_with_query(&query, None)?;
        Ok(Response::new(CountNotesResponse {
            count: ids.len() as u64,
        }))
    }
}

/// Returns the `start..end` range to slice after applying offset/limit to a result set.
fn paginate_range(len: usize, offset: u64, limit: u64) -> std::ops::Range<usize> {
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(len);
    let end = if limit > 0 {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        start.saturating_add(limit).min(len)
    } else {
        len
    };
    start..end
}

fn note_order_by_to_backend_sort(
    order_by: Vec<NoteOrdering>,
) -> Result<Option<BackendSortOrder>, Status> {
    if order_by.is_empty() {
        return Ok(None);
    }

    let mut clauses = Vec::with_capacity(order_by.len());
    for ordering in order_by {
        let expr = match NoteSortColumn::try_from(ordering.column) {
            Ok(NoteSortColumn::CreatedAt) => "n.id",
            Ok(NoteSortColumn::ModifiedAt) => "n.mod",
            Ok(NoteSortColumn::SortField) => "n.sfld collate nocase",
            Ok(NoteSortColumn::Tags) => "n.tags",
            Ok(NoteSortColumn::Unspecified) => {
                return Err(Status::invalid_argument(
                    "order_by column must not be unspecified",
                ));
            }
            Err(_) => {
                return Err(Status::invalid_argument(format!(
                    "unsupported note sort column: {}",
                    ordering.column
                )));
            }
        };
        let direction = match SortDirection::try_from(ordering.direction) {
            Ok(SortDirection::Ascending) => "ASC",
            Ok(SortDirection::Descending) => "DESC",
            Err(_) => {
                return Err(Status::invalid_argument(format!(
                    "unsupported sort direction: {}",
                    ordering.direction
                )));
            }
        };
        clauses.push(format!("{expr} {direction}"));
    }

    Ok(Some(BackendSortOrder {
        value: Some(BackendSortOrderValue::Custom(clauses.join(", "))),
    }))
}

impl NotesApi {
    fn stream_note_refs_for_query(
        &self,
        query: String,
        offset: u64,
        limit: u64,
        order: Option<BackendSortOrder>,
    ) -> ListNoteRefsStream {
        let (tx, receiver) = mpsc::channel(64);
        let store = self.store.clone();

        tokio::task::spawn_blocking(move || {
            let note_ids = match store.search_note_ids_with_query(&query, order) {
                Ok(ids) => ids,
                Err(err) => {
                    let _ = tx.blocking_send(Err(err));
                    return;
                }
            };

            let range = paginate_range(note_ids.len(), offset, limit);
            let mut notetype_cache = NotetypeCache::default();
            for note_id in &note_ids[range] {
                let item = map_list_note_ref(&store, *note_id, &mut notetype_cache);
                if tx.blocking_send(item).is_err() {
                    return;
                }
            }
        });

        ListNoteRefsStream { receiver }
    }

    fn stream_notes_for_query(
        &self,
        query: String,
        offset: u64,
        limit: u64,
        order: Option<BackendSortOrder>,
    ) -> ListNotesStream {
        let (tx, receiver) = mpsc::channel(32);
        let store = self.store.clone();

        tokio::task::spawn_blocking(move || {
            let note_ids = match store.search_note_ids_with_query(&query, order) {
                Ok(ids) => ids,
                Err(err) => {
                    let _ = tx.blocking_send(Err(err));
                    return;
                }
            };

            let range = paginate_range(note_ids.len(), offset, limit);
            let mut notetype_cache = NotetypeCache::default();
            for note_id in &note_ids[range] {
                let item = map_list_note(&store, *note_id, &mut notetype_cache);
                if tx.blocking_send(item).is_err() {
                    return;
                }
            }
        });

        ListNotesStream { receiver }
    }
}

fn map_list_note(
    store: &BackendStore,
    note_id: i64,
    notetype_cache: &mut NotetypeCache,
) -> Result<ListNotesResponse, Status> {
    let note = store.get_note(note_id)?;
    let notetype = notetype_cache.get_or_load(store, note.notetype_id)?;

    Ok(ListNotesResponse {
        note: Some(adapter::map_note(&note, &notetype)?),
    })
}

fn map_list_note_ref(
    store: &BackendStore,
    note_id: i64,
    notetype_cache: &mut NotetypeCache,
) -> Result<ListNoteRefsResponse, Status> {
    let note = store.get_note(note_id)?;
    let notetype = notetype_cache.get_or_load(store, note.notetype_id)?;
    let sort_field = adapter::map_sort_field(&note, &notetype)?;

    Ok(ListNoteRefsResponse {
        note_ref: Some(NoteRef {
            note_id: note.id,
            sort_field: Some(sort_field),
        }),
    })
}

fn update_note_fields_inner(
    store: &BackendStore,
    request: UpdateNoteFieldsRequest,
    notetype_cache: &mut NotetypeCache,
) -> Result<UpdateNoteFieldsResponse, Status> {
    if request.note_id <= 0 {
        return Err(Status::invalid_argument("note_id must be > 0"));
    }
    if request.fields.is_empty() {
        return Err(Status::invalid_argument("fields must not be empty"));
    }

    let current_note = store.get_note(request.note_id)?;
    common::enforce_expected_usn(
        request.expected_usn,
        i64::from(current_note.usn),
        "note",
        current_note.id,
    )?;

    let notetype = notetype_cache.get_or_load(store, current_note.notetype_id)?;
    let ordered_fields = adapter::ordered_notetype_fields(&notetype, current_note.fields.len())?;
    let name_to_ordinal: HashMap<&str, usize> = ordered_fields
        .iter()
        .map(|field| (field.name.as_str(), field.ordinal))
        .collect();

    let mut merged_fields = current_note.fields.clone();
    for update in request.fields {
        let ordinal = name_to_ordinal.get(update.name.as_str()).ok_or_else(|| {
            Status::invalid_argument(format!(
                "field '{}' not found in notetype_id={}",
                update.name, current_note.notetype_id
            ))
        })?;
        merged_fields[*ordinal] = update.value;
    }

    let note = store.update_note_fields(current_note, merged_fields)?;
    let updated_notetype = notetype_cache.get_or_load(store, note.notetype_id)?;

    Ok(UpdateNoteFieldsResponse {
        note: Some(adapter::map_note(&note, &updated_notetype)?),
    })
}

fn create_note_inner(
    store: &BackendStore,
    request: CreateNoteRequest,
    notetype_cache: &mut NotetypeCache,
    deck_cache: &mut DeckCache,
) -> Result<CreateNoteResponse, Status> {
    let (fields, deck_id, notetype_id) =
        prepare_create_note_fields_and_deck(store, request, notetype_cache, deck_cache)?;
    let note = store.create_note(notetype_id, deck_id, fields)?;
    let updated_notetype = notetype_cache.get_or_load(store, note.notetype_id)?;

    Ok(CreateNoteResponse {
        note: Some(adapter::map_note(&note, &updated_notetype)?),
    })
}

fn prepare_create_note_fields_and_deck(
    store: &BackendStore,
    request: CreateNoteRequest,
    notetype_cache: &mut NotetypeCache,
    deck_cache: &mut DeckCache,
) -> Result<(Vec<String>, i64, i64), Status> {
    if request.notetype_id <= 0 {
        return Err(Status::invalid_argument("notetype_id must be > 0"));
    }

    let notetype = notetype_cache.get_or_load(store, request.notetype_id)?;
    let ordered_fields = adapter::ordered_notetype_fields(&notetype, notetype.fields.len())?;
    let expected_field_count = ordered_fields.len();
    if request.fields.len() != expected_field_count {
        return Err(Status::invalid_argument(format!(
            "fields must contain exactly {expected_field_count} entries for notetype_id={}",
            request.notetype_id
        )));
    }

    let name_to_ordinal: HashMap<&str, usize> = ordered_fields
        .iter()
        .map(|field| (field.name.as_str(), field.ordinal))
        .collect();
    let mut fields = vec![None; expected_field_count];
    for field in request.fields {
        let ordinal = name_to_ordinal.get(field.name.as_str()).ok_or_else(|| {
            Status::invalid_argument(format!(
                "field '{}' not found in notetype_id={}",
                field.name, request.notetype_id
            ))
        })?;
        fields[*ordinal] = Some(field.value);
    }

    let missing_fields = ordered_fields
        .iter()
        .filter(|field| fields[field.ordinal].is_none())
        .map(|field| field.name.clone())
        .collect::<Vec<_>>();
    if !missing_fields.is_empty() {
        return Err(Status::invalid_argument(format!(
            "missing field values for notetype_id={}: {}",
            request.notetype_id,
            missing_fields.join(", ")
        )));
    }

    let deck_id = resolve_create_note_deck_id(store, request.deck, deck_cache)?;
    Ok((
        fields
            .into_iter()
            .map(|value| value.expect("missing fields rejected above"))
            .collect(),
        deck_id,
        request.notetype_id,
    ))
}

fn resolve_create_note_deck_id(
    store: &BackendStore,
    deck: Option<Deck>,
    deck_cache: &mut DeckCache,
) -> Result<i64, Status> {
    match deck {
        Some(Deck::DeckId(deck_id)) => {
            if deck_id <= 0 {
                return Err(Status::invalid_argument("deck_id must be > 0"));
            }
            Ok(deck_id)
        }
        Some(Deck::DeckName(deck_name)) => {
            if deck_name.trim().is_empty() {
                return Err(Status::invalid_argument("deck_name must not be empty"));
            }
            if let Some(deck_id) = deck_cache.get(&deck_name) {
                return Ok(*deck_id);
            }
            let deck_id = store.get_deck_id_by_name(&deck_name)?;
            deck_cache.insert(deck_name, deck_id);
            Ok(deck_id)
        }
        None => Err(Status::invalid_argument(
            "exactly one deck selector must be provided",
        )),
    }
}

trait NotetypeLookup {
    fn get_or_load(
        &mut self,
        store: &BackendStore,
        notetype_id: i64,
    ) -> Result<anki_proto::notetypes::Notetype, Status>;
}

impl NotetypeLookup for NotetypeCache {
    fn get_or_load(
        &mut self,
        store: &BackendStore,
        notetype_id: i64,
    ) -> Result<anki_proto::notetypes::Notetype, Status> {
        match self.entry(notetype_id) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                let notetype = store.get_notetype(notetype_id)?;
                entry.insert(notetype.clone());
                Ok(notetype)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use anki_api_proto::anki::api::v1::ErrorDetail;
    use anki_api_proto::anki::api::v1::NoteFieldUpdate;
    use futures::StreamExt;
    use prost14::Message;
    use tonic::Code;
    use tonic::Request;

    use super::*;
    use crate::service::common::TestStore;

    fn ordering(column: NoteSortColumn, direction: SortDirection) -> NoteOrdering {
        NoteOrdering {
            column: column as i32,
            direction: direction as i32,
        }
    }

    fn basic_create_request(front: &str, back: &str) -> CreateNoteRequest {
        CreateNoteRequest {
            notetype_id: 0,
            deck: Some(Deck::DeckName("Default".to_owned())),
            fields: vec![
                NoteFieldUpdate {
                    name: "Front".to_owned(),
                    value: front.to_owned(),
                },
                NoteFieldUpdate {
                    name: "Back".to_owned(),
                    value: back.to_owned(),
                },
            ],
        }
    }

    async fn collect_note_ref_ids(mut stream: ListNoteRefsStream) -> Vec<i64> {
        let mut ids = Vec::new();
        while let Some(item) = stream.next().await {
            ids.push(item.expect("stream item").note_ref.expect("ref").note_id);
        }
        ids
    }

    async fn collect_note_ids(mut stream: ListNotesStream) -> Vec<i64> {
        let mut ids = Vec::new();
        while let Some(item) = stream.next().await {
            ids.push(item.expect("stream item").note.expect("note").note_id);
        }
        ids
    }

    #[tokio::test]
    async fn update_note_fields_reports_version_conflict_details() {
        let fixture = TestStore::new("notes-conflict");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");
        let names = ordered_names(&notetype);

        let updated = <NotesApi as NotesService>::update_note_fields(
            &api,
            Request::new(UpdateNoteFieldsRequest {
                note_id: note.id,
                fields: vec![NoteFieldUpdate {
                    name: names[0].clone(),
                    value: "first-update".to_owned(),
                }],
                expected_usn: Some(i64::from(note.usn)),
            }),
        )
        .await
        .expect("update with expected_usn")
        .into_inner()
        .note
        .expect("note response");
        assert_eq!(updated.fields[0].value, "first-update");

        let stale = <NotesApi as NotesService>::update_note_fields(
            &api,
            Request::new(UpdateNoteFieldsRequest {
                note_id: note.id,
                fields: vec![NoteFieldUpdate {
                    name: names[0].clone(),
                    value: "stale-update".to_owned(),
                }],
                expected_usn: Some(i64::from(note.usn) + 1),
            }),
        )
        .await
        .expect_err("stale expected_usn should fail");
        assert_eq!(stale.code(), Code::Aborted);

        let detail = ErrorDetail::decode(stale.details()).expect("decode details");
        assert_eq!(detail.code, "VERSION_CONFLICT");
        assert!(detail.retryable);
    }

    #[tokio::test]
    async fn update_note_fields_applies_changes_without_precondition() {
        let fixture = TestStore::new("notes-update-success");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let original_back = note.fields.get(1).cloned().expect("back field");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");
        let names = ordered_names(&notetype);

        let response = <NotesApi as NotesService>::update_note_fields(
            &api,
            Request::new(UpdateNoteFieldsRequest {
                note_id: note.id,
                fields: vec![NoteFieldUpdate {
                    name: names[0].clone(),
                    value: "updated-front".to_owned(),
                }],
                expected_usn: None,
            }),
        )
        .await
        .expect("update note")
        .into_inner()
        .note
        .expect("note response");

        assert_eq!(response.fields.len(), 2);
        assert_eq!(response.fields[0].value, "updated-front");
        assert_eq!(response.fields[1].value, original_back);
    }

    #[tokio::test]
    async fn update_note_fields_batch_returns_sort_field_and_usn() {
        let fixture = TestStore::new("notes-batch-write-metadata");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");
        let names = ordered_names(&notetype);

        let response = <NotesApi as NotesService>::update_note_fields_batch(
            &api,
            Request::new(UpdateNoteFieldsBatchRequest {
                updates: vec![UpdateNoteFieldsRequest {
                    note_id: note.id,
                    fields: vec![NoteFieldUpdate {
                        name: names[0].clone(),
                        value: "updated-front".to_owned(),
                    }],
                    expected_usn: None,
                }],
            }),
        )
        .await
        .expect("batch update")
        .into_inner();

        assert_eq!(response.results.len(), 1);
        let metadata = &response.results[0];
        assert_eq!(metadata.note_id, note.id);
        assert!(metadata.usn >= i64::from(note.usn));
        assert!(metadata.sort_field.is_some());
        let sort_field = metadata.sort_field.as_ref().expect("sort field");
        assert_eq!(sort_field.value, "updated-front");
    }

    #[tokio::test]
    async fn update_note_fields_rejects_unknown_name() {
        let fixture = TestStore::new("notes-update-unknown-field");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let status = <NotesApi as NotesService>::update_note_fields(
            &api,
            Request::new(UpdateNoteFieldsRequest {
                note_id: note.id,
                fields: vec![NoteFieldUpdate {
                    name: "DoesNotExist".to_owned(),
                    value: "x".to_owned(),
                }],
                expected_usn: None,
            }),
        )
        .await
        .expect_err("unknown field should fail");
        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("not found"));
    }

    #[tokio::test]
    async fn update_note_fields_rejects_empty_fields() {
        let fixture = TestStore::new("notes-update-empty");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");

        let status = <NotesApi as NotesService>::update_note_fields(
            &api,
            Request::new(UpdateNoteFieldsRequest {
                note_id: note.id,
                fields: Vec::new(),
                expected_usn: None,
            }),
        )
        .await
        .expect_err("empty fields should fail");
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn update_note_fields_allows_duplicate_names_last_wins() {
        let fixture = TestStore::new("notes-update-duplicate");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");
        let names = ordered_names(&notetype);

        let response = <NotesApi as NotesService>::update_note_fields(
            &api,
            Request::new(UpdateNoteFieldsRequest {
                note_id: note.id,
                fields: vec![
                    NoteFieldUpdate {
                        name: names[0].clone(),
                        value: "first".to_owned(),
                    },
                    NoteFieldUpdate {
                        name: names[0].clone(),
                        value: "second".to_owned(),
                    },
                ],
                expected_usn: None,
            }),
        )
        .await
        .expect("patch note")
        .into_inner()
        .note
        .expect("note response");
        assert_eq!(response.fields[0].value, "second");
    }

    #[tokio::test]
    async fn update_note_fields_batch_allows_partial_named_updates() {
        let fixture = TestStore::new("notes-update-batch-write-metadata");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");
        let names = ordered_names(&notetype);

        let response = <NotesApi as NotesService>::update_note_fields_batch(
            &api,
            Request::new(UpdateNoteFieldsBatchRequest {
                updates: vec![UpdateNoteFieldsRequest {
                    note_id: note.id,
                    fields: vec![NoteFieldUpdate {
                        name: names[0].clone(),
                        value: "patched-front".to_owned(),
                    }],
                    expected_usn: None,
                }],
            }),
        )
        .await
        .expect("batch update")
        .into_inner();

        assert_eq!(response.results.len(), 1);
        let metadata = &response.results[0];
        assert_eq!(metadata.note_id, note.id);
        assert!(metadata.usn >= i64::from(note.usn));
        let sort_field = metadata.sort_field.as_ref().expect("sort field");
        assert_eq!(sort_field.value, "patched-front");
    }

    #[tokio::test]
    async fn create_note_returns_created_note_with_sort_field_and_usn() {
        let fixture = TestStore::new("notes-create-success");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let response = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: Some(Deck::DeckName("Default".to_owned())),
                fields: vec![
                    NoteFieldUpdate {
                        name: "Front".to_owned(),
                        value: "created-front".to_owned(),
                    },
                    NoteFieldUpdate {
                        name: "Back".to_owned(),
                        value: "created-back".to_owned(),
                    },
                ],
            }),
        )
        .await
        .expect("create note")
        .into_inner()
        .note
        .expect("note response");

        assert!(response.note_id > 0);
        assert_eq!(response.notetype_id, notetype_id);
        assert_eq!(response.fields.len(), 2);
        assert_eq!(response.fields[0].name, "Front");
        assert_eq!(response.fields[0].value, "created-front");
        assert_eq!(response.fields[1].name, "Back");
        assert_eq!(response.fields[1].value, "created-back");
        let sort_field = response.sort_field.as_ref().expect("sort field");
        assert_eq!(sort_field.name, "Front");
        assert_eq!(sort_field.value, "created-front");
    }

    #[tokio::test]
    async fn create_note_rejects_missing_named_fields() {
        let fixture = TestStore::new("notes-create-missing-fields");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let status = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: Some(Deck::DeckName("Default".to_owned())),
                fields: vec![NoteFieldUpdate {
                    name: "Front".to_owned(),
                    value: "created-front".to_owned(),
                }],
            }),
        )
        .await
        .expect_err("missing field should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("exactly 2 entries"));
    }

    #[tokio::test]
    async fn create_note_rejects_unknown_field_name() {
        let fixture = TestStore::new("notes-create-unknown-field");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let status = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: Some(Deck::DeckName("Default".to_owned())),
                fields: vec![
                    NoteFieldUpdate {
                        name: "Front".to_owned(),
                        value: "created-front".to_owned(),
                    },
                    NoteFieldUpdate {
                        name: "Unknown".to_owned(),
                        value: "created-back".to_owned(),
                    },
                ],
            }),
        )
        .await
        .expect_err("unknown field should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("not found"));
    }

    #[tokio::test]
    async fn create_note_surfaces_deck_not_found() {
        let fixture = TestStore::new("notes-create-missing-deck");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let status = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: Some(Deck::DeckName("Does Not Exist".to_owned())),
                fields: vec![
                    NoteFieldUpdate {
                        name: "Front".to_owned(),
                        value: "created-front".to_owned(),
                    },
                    NoteFieldUpdate {
                        name: "Back".to_owned(),
                        value: "created-back".to_owned(),
                    },
                ],
            }),
        )
        .await
        .expect_err("missing deck should fail");

        assert_eq!(status.code(), Code::NotFound);
        assert!(
            status.message().contains("Does Not Exist"),
            "unexpected message: {}",
            status.message()
        );
    }

    #[tokio::test]
    async fn create_note_allows_empty_field_values_when_backend_accepts_them() {
        let fixture = TestStore::new("notes-create-empty-field");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let response = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: Some(Deck::DeckName("Default".to_owned())),
                fields: vec![
                    NoteFieldUpdate {
                        name: "Front".to_owned(),
                        value: String::new(),
                    },
                    NoteFieldUpdate {
                        name: "Back".to_owned(),
                        value: "created-back".to_owned(),
                    },
                ],
            }),
        )
        .await
        .expect("empty field values should pass through when backend accepts them")
        .into_inner()
        .note
        .expect("note response");

        assert_eq!(response.fields[0].name, "Front");
        assert_eq!(response.fields[0].value, "");
        assert_eq!(response.fields[1].name, "Back");
        assert_eq!(response.fields[1].value, "created-back");
    }

    #[tokio::test]
    async fn create_note_accepts_deck_id_selector() {
        let fixture = TestStore::new("notes-create-deck-id");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");
        let deck_id = store.get_deck_id_by_name("Default").expect("default deck");

        let response = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: Some(Deck::DeckId(deck_id)),
                fields: vec![
                    NoteFieldUpdate {
                        name: "Front".to_owned(),
                        value: "created-front".to_owned(),
                    },
                    NoteFieldUpdate {
                        name: "Back".to_owned(),
                        value: "created-back".to_owned(),
                    },
                ],
            }),
        )
        .await
        .expect("create note by deck id")
        .into_inner()
        .note
        .expect("note response");

        assert!(response.note_id > 0);
        assert_eq!(response.notetype_id, notetype_id);
    }

    #[tokio::test]
    async fn create_note_requires_deck_selector() {
        let fixture = TestStore::new("notes-create-missing-deck-selector");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let status = <NotesApi as NotesService>::create_note(
            &api,
            Request::new(CreateNoteRequest {
                notetype_id,
                deck: None,
                fields: vec![
                    NoteFieldUpdate {
                        name: "Front".to_owned(),
                        value: "created-front".to_owned(),
                    },
                    NoteFieldUpdate {
                        name: "Back".to_owned(),
                        value: "created-back".to_owned(),
                    },
                ],
            }),
        )
        .await
        .expect_err("missing deck selector should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("deck selector"));
    }

    #[tokio::test]
    async fn create_notes_returns_created_notes_in_request_order() {
        let fixture = TestStore::new("notes-create-batch-success");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let response = <NotesApi as NotesService>::create_notes(
            &api,
            Request::new(CreateNotesRequest {
                requests: vec![
                    CreateNoteRequest {
                        notetype_id,
                        ..basic_create_request("batch-front-1", "batch-back-1")
                    },
                    CreateNoteRequest {
                        notetype_id,
                        ..basic_create_request("batch-front-2", "batch-back-2")
                    },
                ],
            }),
        )
        .await
        .expect("create notes")
        .into_inner();

        assert_eq!(response.notes.len(), 2);
        assert_eq!(response.notes[0].fields[0].value, "batch-front-1");
        assert_eq!(response.notes[0].fields[1].value, "batch-back-1");
        assert_eq!(response.notes[1].fields[0].value, "batch-front-2");
        assert_eq!(response.notes[1].fields[1].value, "batch-back-2");
        assert!(response.notes[0].sort_field.is_some());
        assert!(response.notes[1].sort_field.is_some());
        assert!(response.notes[0].note_id > 0);
        assert!(response.notes[1].note_id > 0);

        let count = <NotesApi as NotesService>::count_notes(
            &api,
            Request::new(CountNotesRequest {
                query: String::new(),
            }),
        )
        .await
        .expect("count notes")
        .into_inner();
        assert_eq!(count.count, 2);
    }

    #[tokio::test]
    async fn create_notes_rejects_empty_requests() {
        let fixture = TestStore::new("notes-create-batch-empty");
        let api = NotesApi::new(fixture.store());

        let status = <NotesApi as NotesService>::create_notes(
            &api,
            Request::new(CreateNotesRequest {
                requests: Vec::new(),
            }),
        )
        .await
        .expect_err("empty batch should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("requests must not be empty"));
    }

    #[tokio::test]
    async fn create_notes_reports_validation_failure_with_batch_index_and_rolls_back() {
        let fixture = TestStore::new("notes-create-batch-validation-failure");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let notetype_id = store
            .get_notetype_id_by_name("Basic")
            .expect("basic notetype");

        let status = <NotesApi as NotesService>::create_notes(
            &api,
            Request::new(CreateNotesRequest {
                requests: vec![
                    CreateNoteRequest {
                        notetype_id,
                        ..basic_create_request("valid-front", "valid-back")
                    },
                    CreateNoteRequest {
                        notetype_id,
                        deck: Some(Deck::DeckName("Default".to_owned())),
                        fields: vec![NoteFieldUpdate {
                            name: "Front".to_owned(),
                            value: "missing-back".to_owned(),
                        }],
                    },
                ],
            }),
        )
        .await
        .expect_err("invalid batch should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("batch index 1"));

        let count = <NotesApi as NotesService>::count_notes(
            &api,
            Request::new(CountNotesRequest {
                query: String::new(),
            }),
        )
        .await
        .expect("count notes")
        .into_inner();
        assert_eq!(count.count, 0);
    }

    #[tokio::test]
    async fn delete_notes_removes_multiple_notes() {
        let fixture = TestStore::new("notes-delete-multiple");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note_a = store.create_test_note().expect("seed note a");
        let note_b = store.create_test_note().expect("seed note b");

        let response = <NotesApi as NotesService>::delete_notes(
            &api,
            Request::new(DeleteNotesRequest {
                note_ids: vec![note_a.id, note_b.id],
            }),
        )
        .await
        .expect("delete notes")
        .into_inner();

        assert_eq!(response.deleted_count, 2);

        let status = <NotesApi as NotesService>::get_note(
            &api,
            Request::new(GetNoteRequest { note_id: note_a.id }),
        )
        .await
        .expect_err("deleted note should not exist");
        assert_eq!(status.code(), Code::NotFound);

        let remaining = <NotesApi as NotesService>::count_notes(
            &api,
            Request::new(CountNotesRequest {
                query: format!("nid:{} or nid:{}", note_a.id, note_b.id),
            }),
        )
        .await
        .expect("count deleted notes")
        .into_inner();
        assert_eq!(remaining.count, 0);
    }

    #[tokio::test]
    async fn delete_notes_rejects_empty_ids() {
        let fixture = TestStore::new("notes-delete-empty");
        let api = NotesApi::new(fixture.store());

        let status = <NotesApi as NotesService>::delete_notes(
            &api,
            Request::new(DeleteNotesRequest {
                note_ids: Vec::new(),
            }),
        )
        .await
        .expect_err("empty note_ids should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("must not be empty"));
    }

    #[tokio::test]
    async fn delete_notes_rejects_non_positive_ids() {
        let fixture = TestStore::new("notes-delete-invalid-id");
        let api = NotesApi::new(fixture.store());

        let status = <NotesApi as NotesService>::delete_notes(
            &api,
            Request::new(DeleteNotesRequest {
                note_ids: vec![0, 1],
            }),
        )
        .await
        .expect_err("non-positive note_ids should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("must all be > 0"));
    }

    #[tokio::test]
    async fn get_note_returns_named_fields_in_ordinal_order() {
        let fixture = TestStore::new("notes-get-named-fields");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");

        let response = <NotesApi as NotesService>::get_note(
            &api,
            Request::new(GetNoteRequest { note_id: note.id }),
        )
        .await
        .expect("get note")
        .into_inner()
        .note
        .expect("note payload");

        let expected_names = ordered_names(&notetype);
        let actual_names = response
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(actual_names, expected_names);
        assert_eq!(response.fields.len(), note.fields.len());
    }

    #[tokio::test]
    async fn get_notes_returns_results_in_request_order() {
        let fixture = TestStore::new("notes-get-batch-order");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note_a = store.create_test_note().expect("seed note a");
        let note_b = store.create_test_note().expect("seed note b");

        let response = <NotesApi as NotesService>::get_notes(
            &api,
            Request::new(GetNotesRequest {
                note_ids: vec![note_b.id, note_a.id],
            }),
        )
        .await
        .expect("get notes")
        .into_inner();

        let ids = response
            .notes
            .iter()
            .map(|note| note.note_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![note_b.id, note_a.id]);
        assert_eq!(
            response.notes[0]
                .created_at
                .as_ref()
                .expect("created_at")
                .seconds,
            note_b.id / 1000
        );
        assert_eq!(
            response.notes[1]
                .created_at
                .as_ref()
                .expect("created_at")
                .seconds,
            note_a.id / 1000
        );
    }

    #[tokio::test]
    async fn list_notes_returns_named_fields_for_filtered_and_unfiltered_queries() {
        let fixture = TestStore::new("notes-list-filtered-unfiltered-named-fields");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");
        let notetype = store.get_notetype(note.notetype_id).expect("notetype");
        let expected_names = ordered_names(&notetype);

        let mut listed = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: String::new(),
                offset: 0,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("list notes")
        .into_inner();
        let listed_first = listed
            .next()
            .await
            .expect("first stream item")
            .expect("list response")
            .note
            .expect("note payload");
        assert_eq!(
            listed_first
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect::<Vec<_>>(),
            expected_names
        );
        assert_eq!(
            listed_first
                .created_at
                .as_ref()
                .expect("created_at")
                .seconds,
            note.id / 1000
        );

        let mut filtered = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: format!("nid:{}", note.id),
                offset: 0,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("filtered list notes")
        .into_inner();
        let filtered_first = filtered
            .next()
            .await
            .expect("first stream item")
            .expect("filtered response")
            .note
            .expect("note payload");
        assert_eq!(
            filtered_first
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect::<Vec<_>>(),
            expected_names
        );
        assert_eq!(
            filtered_first
                .created_at
                .as_ref()
                .expect("created_at")
                .seconds,
            note.id / 1000
        );
    }

    #[tokio::test]
    async fn list_note_refs_returns_note_id_and_sort_field() {
        let fixture = TestStore::new("notes-list-refs");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");

        let mut stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: format!("nid:{}", note.id),
                offset: 0,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("list note refs")
        .into_inner();

        let item = stream
            .next()
            .await
            .expect("first stream item")
            .expect("stream response")
            .note_ref
            .expect("note ref payload");
        assert_eq!(item.note_id, note.id);
        assert!(item.sort_field.is_some());
    }

    #[tokio::test]
    async fn get_note_changes_supports_cursor_paging() {
        let fixture = TestStore::new("notes-changes-paging");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note 1");
        let _ = store.create_test_note().expect("seed note 2");

        let first_page = <NotesApi as NotesService>::get_note_changes(
            &api,
            Request::new(GetNoteChangesRequest {
                cursor: String::new(),
                limit: 1,
            }),
        )
        .await
        .expect("first page")
        .into_inner();
        assert_eq!(first_page.changes.len(), 1);
        assert!(!first_page.next_cursor.is_empty());
        let first_row = first_page.changes[0];

        let second_page = <NotesApi as NotesService>::get_note_changes(
            &api,
            Request::new(GetNoteChangesRequest {
                cursor: first_page.next_cursor,
                limit: 100,
            }),
        )
        .await
        .expect("second page")
        .into_inner();
        assert!(!second_page.changes.is_empty());
        let second_row = second_page.changes[0];
        assert_ne!(second_row.note_id, first_row.note_id);
        assert!(
            (second_row.usn, second_row.note_id) > (first_row.usn, first_row.note_id),
            "cursor boundary should be exclusive"
        );
    }

    #[tokio::test]
    async fn count_notes_returns_total_for_empty_query() {
        let fixture = TestStore::new("notes-count-empty-query");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note 1");
        let _ = store.create_test_note().expect("seed note 2");

        let response = <NotesApi as NotesService>::count_notes(
            &api,
            Request::new(CountNotesRequest {
                query: String::new(),
            }),
        )
        .await
        .expect("count notes")
        .into_inner();

        assert!(response.count >= 2);
    }

    #[tokio::test]
    async fn count_notes_respects_query_filter() {
        let fixture = TestStore::new("notes-count-filtered");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note = store.create_test_note().expect("seed note");

        let filtered = <NotesApi as NotesService>::count_notes(
            &api,
            Request::new(CountNotesRequest {
                query: format!("nid:{}", note.id),
            }),
        )
        .await
        .expect("count notes filtered")
        .into_inner();

        assert_eq!(filtered.count, 1);
    }

    #[tokio::test]
    async fn list_note_refs_respects_offset() {
        let fixture = TestStore::new("notes-list-refs-offset");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note a");
        let _ = store.create_test_note().expect("seed note b");
        let _ = store.create_test_note().expect("seed note c");

        let stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: String::new(),
                offset: 0,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("list all")
        .into_inner();

        let all_ids = collect_note_ref_ids(stream).await;
        assert!(all_ids.len() >= 3);

        let stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: String::new(),
                offset: 1,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("list with offset")
        .into_inner();

        let offset_ids = collect_note_ref_ids(stream).await;
        assert_eq!(offset_ids, all_ids[1..]);
    }

    #[tokio::test]
    async fn list_note_refs_respects_limit() {
        let fixture = TestStore::new("notes-list-refs-limit");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note a");
        let _ = store.create_test_note().expect("seed note b");
        let _ = store.create_test_note().expect("seed note c");

        let stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: String::new(),
                offset: 0,
                limit: 2,
                order_by: vec![],
            }),
        )
        .await
        .expect("list with limit")
        .into_inner();

        let ids = collect_note_ref_ids(stream).await;
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn list_note_refs_respects_offset_and_limit() {
        let fixture = TestStore::new("notes-list-refs-offset-limit");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note a");
        let _ = store.create_test_note().expect("seed note b");
        let _ = store.create_test_note().expect("seed note c");

        let all_stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: String::new(),
                offset: 0,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("list all")
        .into_inner();

        let all_ids = collect_note_ref_ids(all_stream).await;

        let stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: String::new(),
                offset: 1,
                limit: 1,
                order_by: vec![],
            }),
        )
        .await
        .expect("list with offset+limit")
        .into_inner();

        let ids = collect_note_ref_ids(stream).await;
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], all_ids[1]);
    }

    #[tokio::test]
    async fn list_notes_respects_offset_and_limit() {
        let fixture = TestStore::new("notes-list-notes-offset-limit");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note a");
        let _ = store.create_test_note().expect("seed note b");
        let _ = store.create_test_note().expect("seed note c");

        let all_stream = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: String::new(),
                offset: 0,
                limit: 0,
                order_by: vec![],
            }),
        )
        .await
        .expect("list all")
        .into_inner();

        let all_ids = collect_note_ids(all_stream).await;
        assert!(all_ids.len() >= 3);

        let stream = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: String::new(),
                offset: 1,
                limit: 1,
                order_by: vec![],
            }),
        )
        .await
        .expect("list with offset+limit")
        .into_inner();

        let ids = collect_note_ids(stream).await;
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], all_ids[1]);
    }

    #[tokio::test]
    async fn list_note_refs_supports_created_at_descending() {
        let fixture = TestStore::new("notes-list-refs-created-at-desc");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note_a = store
            .create_test_note_with_fields("sort-a", Some("back-a"))
            .expect("seed note a");
        let note_b = store
            .create_test_note_with_fields("sort-b", Some("back-b"))
            .expect("seed note b");

        let stream = <NotesApi as NotesService>::list_note_refs(
            &api,
            Request::new(ListNoteRefsRequest {
                query: format!("nid:{} or nid:{}", note_a.id, note_b.id),
                offset: 0,
                limit: 0,
                order_by: vec![ordering(
                    NoteSortColumn::CreatedAt,
                    SortDirection::Descending,
                )],
            }),
        )
        .await
        .expect("list note refs")
        .into_inner();

        assert_eq!(
            collect_note_ref_ids(stream).await,
            vec![note_b.id, note_a.id]
        );
    }

    #[tokio::test]
    async fn list_notes_supports_sort_field_ordering() {
        let fixture = TestStore::new("notes-list-notes-sort-field");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note_b = store
            .create_test_note_with_fields("bravo", Some("back-b"))
            .expect("seed note b");
        let note_a = store
            .create_test_note_with_fields("alpha", Some("back-a"))
            .expect("seed note a");

        let stream = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: format!("nid:{} or nid:{}", note_a.id, note_b.id),
                offset: 0,
                limit: 0,
                order_by: vec![ordering(
                    NoteSortColumn::SortField,
                    SortDirection::Ascending,
                )],
            }),
        )
        .await
        .expect("list notes")
        .into_inner();

        assert_eq!(collect_note_ids(stream).await, vec![note_a.id, note_b.id]);
    }

    #[tokio::test]
    async fn list_notes_supports_multi_column_ordering() {
        let fixture = TestStore::new("notes-list-notes-multi-column");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let note_a = store
            .create_test_note_with_fields("same", Some("back-a"))
            .expect("seed note a");
        let note_b = store
            .create_test_note_with_fields("same", Some("back-b"))
            .expect("seed note b");

        let stream = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: format!("nid:{} or nid:{}", note_a.id, note_b.id),
                offset: 0,
                limit: 0,
                order_by: vec![
                    ordering(NoteSortColumn::SortField, SortDirection::Ascending),
                    ordering(NoteSortColumn::CreatedAt, SortDirection::Descending),
                ],
            }),
        )
        .await
        .expect("list notes")
        .into_inner();

        assert_eq!(collect_note_ids(stream).await, vec![note_b.id, note_a.id]);
    }

    #[tokio::test]
    async fn list_notes_rejects_unspecified_order_column() {
        let fixture = TestStore::new("notes-list-notes-invalid-order-column");
        let store = fixture.store();
        let api = NotesApi::new(store.clone());
        let _ = store.create_test_note().expect("seed note");

        let result = <NotesApi as NotesService>::list_notes(
            &api,
            Request::new(ListNotesRequest {
                query: String::new(),
                offset: 0,
                limit: 0,
                order_by: vec![ordering(
                    NoteSortColumn::Unspecified,
                    SortDirection::Ascending,
                )],
            }),
        )
        .await;

        let status = match result {
            Ok(_) => panic!("unspecified order column should fail"),
            Err(status) => status,
        };

        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("must not be unspecified"));
    }

    #[test]
    fn paginate_range_saturates_on_overflow() {
        let range = super::paginate_range(10, u64::MAX, u64::MAX);
        assert!(range.is_empty());

        let range = super::paginate_range(10, 0, u64::MAX);
        assert_eq!(range, 0..10);

        let range = super::paginate_range(10, 5, u64::MAX);
        assert_eq!(range, 5..10);
    }

    fn ordered_names(notetype: &anki_proto::notetypes::Notetype) -> Vec<String> {
        let mut fields = notetype
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                (
                    field
                        .ord
                        .as_ref()
                        .map(|ord| ord.val as usize)
                        .unwrap_or(index),
                    field.name.clone(),
                )
            })
            .collect::<Vec<_>>();
        fields.sort_unstable_by_key(|(ordinal, _)| *ordinal);
        fields.into_iter().map(|(_, name)| name).collect()
    }
}
