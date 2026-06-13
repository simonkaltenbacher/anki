use anki_api_proto::anki::api::v1::AddDeckRequest;
use anki_api_proto::anki::api::v1::AddDeckResponse;
use anki_api_proto::anki::api::v1::DeckRef;
use anki_api_proto::anki::api::v1::GetDeckIdByNameRequest;
use anki_api_proto::anki::api::v1::GetDeckIdByNameResponse;
use anki_api_proto::anki::api::v1::ListDeckRefsRequest;
use anki_api_proto::anki::api::v1::ListDeckRefsResponse;
use anki_api_proto::anki::api::v1::RemoveDeckRequest;
use anki_api_proto::anki::api::v1::RemoveDeckResponse;
use anki_api_proto::anki::api::v1::decks_service_server::DecksService;
use tonic::Code;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::store::BackendStore;

#[derive(Clone)]
pub struct DecksApi {
    store: BackendStore,
}

impl DecksApi {
    pub fn new(store: BackendStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl DecksService for DecksApi {
    async fn list_deck_refs(
        &self,
        _request: Request<ListDeckRefsRequest>,
    ) -> Result<Response<ListDeckRefsResponse>, Status> {
        let decks = self
            .store
            .list_deck_refs()?
            .into_iter()
            .map(|(deck_id, name)| DeckRef { deck_id, name })
            .collect();
        Ok(Response::new(ListDeckRefsResponse { decks }))
    }

    async fn get_deck_id_by_name(
        &self,
        request: Request<GetDeckIdByNameRequest>,
    ) -> Result<Response<GetDeckIdByNameResponse>, Status> {
        let name = request.into_inner().name;
        if name.trim().is_empty() {
            return Err(Status::invalid_argument("name must not be empty"));
        }
        let deck_id = self.store.get_deck_id_by_name(&name)?;
        Ok(Response::new(GetDeckIdByNameResponse { deck_id }))
    }

    async fn add_deck(
        &self,
        request: Request<AddDeckRequest>,
    ) -> Result<Response<AddDeckResponse>, Status> {
        let name = request.into_inner().name;
        let name = name.trim();
        if name.is_empty() {
            return Err(Status::invalid_argument("name must not be empty"));
        }
        // Get-or-create: resolve first so `created` reliably reflects whether a
        // new deck was made, and adding an existing name is idempotent.
        match self.store.get_deck_id_by_name(name) {
            Ok(deck_id) => Ok(Response::new(AddDeckResponse {
                deck_id,
                created: false,
            })),
            Err(status) if status.code() == Code::NotFound => {
                let deck_id = self.store.add_deck(name)?;
                Ok(Response::new(AddDeckResponse {
                    deck_id,
                    created: true,
                }))
            }
            Err(status) => Err(status),
        }
    }

    async fn remove_deck(
        &self,
        request: Request<RemoveDeckRequest>,
    ) -> Result<Response<RemoveDeckResponse>, Status> {
        let request = request.into_inner();
        if request.deck_id <= 0 {
            return Err(Status::invalid_argument("deck_id must be positive"));
        }
        let (card_count, removed) = self.store.remove_deck(request.deck_id, request.force)?;
        Ok(Response::new(RemoveDeckResponse {
            card_count,
            removed,
        }))
    }
}

#[cfg(test)]
mod tests {
    use tonic::Code;
    use tonic::Request;

    use super::*;
    use crate::service::common::TestStore;

    #[tokio::test]
    async fn list_deck_refs_returns_default_deck() {
        let fixture = TestStore::new("decks-list-refs");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let response =
            <DecksApi as DecksService>::list_deck_refs(&api, Request::new(ListDeckRefsRequest {}))
                .await
                .expect("list deck refs")
                .into_inner();

        assert!(
            response
                .decks
                .iter()
                .any(|deck| deck.name == "Default" && deck.deck_id > 0),
            "expected Default deck in response: {:?}",
            response.decks
        );
    }

    #[tokio::test]
    async fn get_deck_id_by_name_resolves_exact_name() {
        let fixture = TestStore::new("decks-get-id-by-name");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let response = <DecksApi as DecksService>::get_deck_id_by_name(
            &api,
            Request::new(GetDeckIdByNameRequest {
                name: "Default".to_owned(),
            }),
        )
        .await
        .expect("get deck id by name")
        .into_inner();

        assert!(response.deck_id > 0);
    }

    #[tokio::test]
    async fn get_deck_id_by_name_rejects_empty_name() {
        let fixture = TestStore::new("decks-get-id-by-name-empty");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let status = <DecksApi as DecksService>::get_deck_id_by_name(
            &api,
            Request::new(GetDeckIdByNameRequest {
                name: "   ".to_owned(),
            }),
        )
        .await
        .expect_err("empty name should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn get_deck_id_by_name_surfaces_not_found() {
        let fixture = TestStore::new("decks-get-id-by-name-missing");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let status = <DecksApi as DecksService>::get_deck_id_by_name(
            &api,
            Request::new(GetDeckIdByNameRequest {
                name: "Does Not Exist".to_owned(),
            }),
        )
        .await
        .expect_err("missing deck should fail");

        assert_eq!(status.code(), Code::NotFound);
        assert!(status.message().contains("Does Not Exist"));
    }

    #[tokio::test]
    async fn add_deck_creates_new_deck() {
        let fixture = TestStore::new("decks-add-new");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let response = <DecksApi as DecksService>::add_deck(
            &api,
            Request::new(AddDeckRequest {
                name: "Plan Test Deck".to_owned(),
            }),
        )
        .await
        .expect("add deck")
        .into_inner();

        assert!(response.created);
        assert!(response.deck_id > 0);
    }

    #[tokio::test]
    async fn add_deck_returns_existing_without_creating() {
        let fixture = TestStore::new("decks-add-existing");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let response = <DecksApi as DecksService>::add_deck(
            &api,
            Request::new(AddDeckRequest {
                name: "Default".to_owned(),
            }),
        )
        .await
        .expect("add deck")
        .into_inner();

        assert!(!response.created);
        assert!(response.deck_id > 0);
    }

    #[tokio::test]
    async fn add_deck_trims_name_before_create() {
        let fixture = TestStore::new("decks-add-trim");
        let store = fixture.store();
        let api = DecksApi::new(store.clone());

        let created = <DecksApi as DecksService>::add_deck(
            &api,
            Request::new(AddDeckRequest {
                name: "  Trimmed Deck  ".to_owned(),
            }),
        )
        .await
        .expect("add deck")
        .into_inner();
        assert!(created.created);

        // The trimmed name resolves; the padded name was not what got stored.
        let resolved = store
            .get_deck_id_by_name("Trimmed Deck")
            .expect("trimmed name resolves");
        assert_eq!(resolved, created.deck_id);
    }

    #[tokio::test]
    async fn add_deck_rejects_empty_name() {
        let fixture = TestStore::new("decks-add-empty");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let status = <DecksApi as DecksService>::add_deck(
            &api,
            Request::new(AddDeckRequest {
                name: "   ".to_owned(),
            }),
        )
        .await
        .expect_err("empty name should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn remove_deck_with_force_removes_cards_and_reports_count() {
        let fixture = TestStore::new("decks-remove-force");
        let store = fixture.store();
        // Seed a card (lands in Default), then move it into the target deck so
        // removing the target actually removes a card.
        let _ = store.create_test_note().expect("seed note");
        let card_ids = store
            .search_card_ids_with_query("api-test-front")
            .expect("search cards");
        let deck_id = store.add_deck("Removal Target").expect("create deck");
        store
            .set_card_deck(card_ids.clone(), deck_id)
            .expect("move card into target deck");
        let api = DecksApi::new(store.clone());

        let response = <DecksApi as DecksService>::remove_deck(
            &api,
            Request::new(RemoveDeckRequest {
                deck_id,
                force: true,
            }),
        )
        .await
        .expect("remove deck")
        .into_inner();

        assert!(response.removed);
        assert_eq!(response.card_count, card_ids.len() as u64);

        let status = store
            .get_deck_id_by_name("Removal Target")
            .expect_err("removed deck should no longer resolve");
        assert_eq!(status.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn remove_deck_without_force_refuses_non_empty_deck() {
        let fixture = TestStore::new("decks-remove-blocked");
        let store = fixture.store();
        let _ = store.create_test_note().expect("seed note");
        let card_ids = store
            .search_card_ids_with_query("api-test-front")
            .expect("search cards");
        let deck_id = store.add_deck("Blocked Target").expect("create deck");
        store
            .set_card_deck(card_ids.clone(), deck_id)
            .expect("move card into target deck");
        let api = DecksApi::new(store.clone());

        let response = <DecksApi as DecksService>::remove_deck(
            &api,
            Request::new(RemoveDeckRequest {
                deck_id,
                force: false,
            }),
        )
        .await
        .expect("remove deck")
        .into_inner();

        assert!(!response.removed);
        assert_eq!(response.card_count, card_ids.len() as u64);

        // The deck still exists because removal was refused.
        let resolved = store
            .get_deck_id_by_name("Blocked Target")
            .expect("non-empty deck should still exist");
        assert_eq!(resolved, deck_id);
    }

    #[tokio::test]
    async fn remove_deck_without_force_removes_empty_deck() {
        let fixture = TestStore::new("decks-remove-empty");
        let store = fixture.store();
        let deck_id = store.add_deck("Empty Target").expect("create deck");
        let api = DecksApi::new(store.clone());

        let response = <DecksApi as DecksService>::remove_deck(
            &api,
            Request::new(RemoveDeckRequest {
                deck_id,
                force: false,
            }),
        )
        .await
        .expect("remove deck")
        .into_inner();

        assert!(response.removed);
        assert_eq!(response.card_count, 0);

        let status = store
            .get_deck_id_by_name("Empty Target")
            .expect_err("removed deck should no longer resolve");
        assert_eq!(status.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn remove_deck_reports_not_found_for_unknown_deck_even_with_force() {
        let fixture = TestStore::new("decks-remove-unknown");
        let store = fixture.store();
        let api = DecksApi::new(store);

        // Native removal silently no-ops unknown IDs; the API must surface a
        // NotFound rather than reporting a phantom removal.
        let status = <DecksApi as DecksService>::remove_deck(
            &api,
            Request::new(RemoveDeckRequest {
                deck_id: 999_999_999,
                force: true,
            }),
        )
        .await
        .expect_err("unknown deck should fail");

        assert_eq!(status.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn remove_deck_rejects_default_deck_even_with_force() {
        let fixture = TestStore::new("decks-remove-default");
        let store = fixture.store();
        let default_id = store.get_deck_id_by_name("Default").expect("default deck");
        let api = DecksApi::new(store.clone());

        // Native removal resets the default deck rather than deleting it, so the
        // API rejects it instead of reporting a phantom removal.
        let status = <DecksApi as DecksService>::remove_deck(
            &api,
            Request::new(RemoveDeckRequest {
                deck_id: default_id,
                force: true,
            }),
        )
        .await
        .expect_err("default deck should be rejected");

        assert_eq!(status.code(), Code::FailedPrecondition);

        // The default deck still resolves because removal was refused.
        assert_eq!(
            store.get_deck_id_by_name("Default").expect("default deck"),
            default_id
        );
    }

    #[tokio::test]
    async fn remove_deck_rejects_non_positive_id() {
        let fixture = TestStore::new("decks-remove-bad-id");
        let store = fixture.store();
        let api = DecksApi::new(store);

        let status = <DecksApi as DecksService>::remove_deck(
            &api,
            Request::new(RemoveDeckRequest {
                deck_id: 0,
                force: false,
            }),
        )
        .await
        .expect_err("non-positive id should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }
}
