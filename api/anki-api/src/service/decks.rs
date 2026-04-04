use anki_api_proto::anki::api::v1::DeckRef;
use anki_api_proto::anki::api::v1::GetDeckIdByNameRequest;
use anki_api_proto::anki::api::v1::GetDeckIdByNameResponse;
use anki_api_proto::anki::api::v1::ListDeckRefsRequest;
use anki_api_proto::anki::api::v1::ListDeckRefsResponse;
use anki_api_proto::anki::api::v1::decks_service_server::DecksService;
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
}
