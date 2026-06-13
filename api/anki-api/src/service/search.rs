use anki_api_proto::anki::api::v1::SearchCardsRequest;
use anki_api_proto::anki::api::v1::SearchCardsResponse;
use anki_api_proto::anki::api::v1::search_service_server::SearchService;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::store::BackendStore;

#[derive(Clone)]
pub struct SearchApi {
    store: BackendStore,
}

impl SearchApi {
    pub fn new(store: BackendStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl SearchService for SearchApi {
    async fn search_cards(
        &self,
        request: Request<SearchCardsRequest>,
    ) -> Result<Response<SearchCardsResponse>, Status> {
        let query = request.into_inner().query;
        if query.trim().is_empty() {
            return Err(Status::invalid_argument("query must not be empty"));
        }

        let card_ids = self.store.search_card_ids_with_query(&query)?;
        Ok(Response::new(SearchCardsResponse { card_ids }))
    }
}

#[cfg(test)]
mod tests {
    use tonic::Code;
    use tonic::Request;

    use super::*;
    use crate::service::common::TestStore;

    #[tokio::test]
    async fn search_cards_returns_matching_card_ids() {
        let fixture = TestStore::new("search-cards");
        let store = fixture.store();
        let _ = store
            .create_test_note_with_fields("search-card-front", Some("search-card-back"))
            .expect("seed note");
        let api = SearchApi::new(store);

        let response = <SearchApi as SearchService>::search_cards(
            &api,
            Request::new(SearchCardsRequest {
                query: "search-card-front".to_owned(),
            }),
        )
        .await
        .expect("search cards")
        .into_inner();

        assert_eq!(response.card_ids.len(), 1);
        assert!(response.card_ids[0] > 0);
    }

    #[tokio::test]
    async fn search_cards_returns_empty_for_no_matches() {
        let fixture = TestStore::new("search-cards-no-matches");
        let store = fixture.store();
        let _ = store.create_test_note().expect("seed note");
        let api = SearchApi::new(store);

        let response = <SearchApi as SearchService>::search_cards(
            &api,
            Request::new(SearchCardsRequest {
                query: "definitely-no-matching-card".to_owned(),
            }),
        )
        .await
        .expect("search cards")
        .into_inner();

        assert!(response.card_ids.is_empty());
    }

    #[tokio::test]
    async fn search_cards_rejects_empty_query() {
        let fixture = TestStore::new("search-cards-empty-query");
        let store = fixture.store();
        let api = SearchApi::new(store);

        let status = <SearchApi as SearchService>::search_cards(
            &api,
            Request::new(SearchCardsRequest {
                query: "   ".to_owned(),
            }),
        )
        .await
        .expect_err("empty query should fail");

        assert_eq!(status.code(), Code::InvalidArgument);
    }
}
