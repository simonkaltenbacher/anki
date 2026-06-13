use anki_api_proto::anki::api::v1::SetDeckRequest;
use anki_api_proto::anki::api::v1::SetDeckResponse;
use anki_api_proto::anki::api::v1::cards_service_server::CardsService;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::store::BackendStore;

#[derive(Clone)]
pub struct CardsApi {
    store: BackendStore,
}

impl CardsApi {
    pub fn new(store: BackendStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl CardsService for CardsApi {
    async fn set_deck(
        &self,
        request: Request<SetDeckRequest>,
    ) -> Result<Response<SetDeckResponse>, Status> {
        let request = request.into_inner();
        if request.card_ids.is_empty() {
            return Err(Status::invalid_argument("card_ids must not be empty"));
        }
        if request.deck_id <= 0 {
            return Err(Status::invalid_argument("deck_id must be > 0"));
        }

        let changed_count = self
            .store
            .set_card_deck(request.card_ids, request.deck_id)?;
        Ok(Response::new(SetDeckResponse {
            changed_count: u64::from(changed_count),
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
    async fn set_deck_accepts_existing_card_and_deck() {
        let fixture = TestStore::new("cards-set-deck");
        let store = fixture.store();
        let _ = store.create_test_note().expect("seed note");
        let card_ids = store
            .search_card_ids_with_query("api-test-front")
            .expect("search cards");
        let deck_id = store.get_deck_id_by_name("Default").expect("default deck");
        let api = CardsApi::new(store);

        let response = <CardsApi as CardsService>::set_deck(
            &api,
            Request::new(SetDeckRequest { card_ids, deck_id }),
        )
        .await
        .expect("set deck")
        .into_inner();

        assert_eq!(response.changed_count, 0);
    }

    #[tokio::test]
    async fn set_deck_moves_card_to_target_deck() {
        let fixture = TestStore::new("cards-set-deck-moves-card");
        let store = fixture.store();
        let _ = store.create_test_note().expect("seed note");
        let card_ids = store
            .search_card_ids_with_query("api-test-front")
            .expect("search cards");
        let deck_id = store
            .add_deck("Card Move Target")
            .expect("create target deck");
        let api = CardsApi::new(store.clone());

        let response = <CardsApi as CardsService>::set_deck(
            &api,
            Request::new(SetDeckRequest {
                card_ids: card_ids.clone(),
                deck_id,
            }),
        )
        .await
        .expect("set deck")
        .into_inner();

        assert_eq!(response.changed_count, 1);

        let moved_card_ids = store
            .search_card_ids_with_query(r#"deck:"Card Move Target" api-test-front"#)
            .expect("search moved cards");
        assert_eq!(moved_card_ids, card_ids);
    }

    #[tokio::test]
    async fn set_deck_rejects_empty_card_ids() {
        let fixture = TestStore::new("cards-set-deck-empty-card-ids");
        let store = fixture.store();
        let deck_id = store.get_deck_id_by_name("Default").expect("default deck");
        let api = CardsApi::new(store);

        let status = <CardsApi as CardsService>::set_deck(
            &api,
            Request::new(SetDeckRequest {
                card_ids: Vec::new(),
                deck_id,
            }),
        )
        .await
        .expect_err("empty card ids should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn set_deck_rejects_invalid_deck_id() {
        let fixture = TestStore::new("cards-set-deck-invalid-deck-id");
        let store = fixture.store();
        let _ = store.create_test_note().expect("seed note");
        let card_ids = store
            .search_card_ids_with_query("api-test-front")
            .expect("search cards");
        let api = CardsApi::new(store);

        let status = <CardsApi as CardsService>::set_deck(
            &api,
            Request::new(SetDeckRequest {
                card_ids,
                deck_id: 0,
            }),
        )
        .await
        .expect_err("invalid deck id should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }
}
