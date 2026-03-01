use std::collections::HashMap;

use anki_api_proto::anki::api::v1::CountNotetypesRequest;
use anki_api_proto::anki::api::v1::CountNotetypesResponse;
use anki_api_proto::anki::api::v1::GetNotetypeChangesRequest;
use anki_api_proto::anki::api::v1::GetNotetypeChangesResponse;
use anki_api_proto::anki::api::v1::GetNotetypeIdByNameRequest;
use anki_api_proto::anki::api::v1::GetNotetypeIdByNameResponse;
use anki_api_proto::anki::api::v1::GetNotetypeRequest;
use anki_api_proto::anki::api::v1::GetNotetypeResponse;
use anki_api_proto::anki::api::v1::GetNotetypesRequest;
use anki_api_proto::anki::api::v1::GetNotetypesResponse;
use anki_api_proto::anki::api::v1::ListNotetypeRefsRequest;
use anki_api_proto::anki::api::v1::ListNotetypeRefsResponse;
use anki_api_proto::anki::api::v1::ListNotetypesRequest;
use anki_api_proto::anki::api::v1::ListNotetypesResponse;
use anki_api_proto::anki::api::v1::NotetypeChange;
use anki_api_proto::anki::api::v1::NotetypeRef;
use anki_api_proto::anki::api::v1::NotetypeWriteMetadata;
use anki_api_proto::anki::api::v1::UpdateCssBatchRequest;
use anki_api_proto::anki::api::v1::UpdateCssBatchResponse;
use anki_api_proto::anki::api::v1::UpdateCssRequest;
use anki_api_proto::anki::api::v1::UpdateCssResponse;
use anki_api_proto::anki::api::v1::UpdateNotetypeContentRequest;
use anki_api_proto::anki::api::v1::UpdateNotetypeContentResponse;
use anki_api_proto::anki::api::v1::UpdateTemplatesBatchRequest;
use anki_api_proto::anki::api::v1::UpdateTemplatesBatchResponse;
use anki_api_proto::anki::api::v1::UpdateTemplatesRequest;
use anki_api_proto::anki::api::v1::UpdateTemplatesResponse;
use anki_api_proto::anki::api::v1::notetypes_service_server::NotetypesService;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::adapter;
use crate::service::common;
use crate::store::SharedStore;

#[derive(Clone)]
pub struct NotetypesApi {
    store: SharedStore,
}

impl NotetypesApi {
    pub fn new(store: SharedStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl NotetypesService for NotetypesApi {
    async fn list_notetype_refs(
        &self,
        _request: Request<ListNotetypeRefsRequest>,
    ) -> Result<Response<ListNotetypeRefsResponse>, Status> {
        let notetypes = self
            .store
            .list_notetype_refs()?
            .into_iter()
            .map(|(notetype_id, name)| NotetypeRef { notetype_id, name })
            .collect();
        Ok(Response::new(ListNotetypeRefsResponse { notetypes }))
    }

    async fn list_notetypes(
        &self,
        _request: Request<ListNotetypesRequest>,
    ) -> Result<Response<ListNotetypesResponse>, Status> {
        let notetype_ids = self.store.list_notetype_ids()?;
        let mut notetypes = Vec::with_capacity(notetype_ids.len());
        for notetype_id in notetype_ids {
            let notetype = self.store.get_notetype(notetype_id)?;
            notetypes.push(adapter::map_notetype(&notetype));
        }
        Ok(Response::new(ListNotetypesResponse { notetypes }))
    }

    async fn get_notetype(
        &self,
        request: Request<GetNotetypeRequest>,
    ) -> Result<Response<GetNotetypeResponse>, Status> {
        let notetype_id = request.into_inner().notetype_id;
        if notetype_id <= 0 {
            return Err(Status::invalid_argument("notetype_id must be > 0"));
        }

        let notetype = self.store.get_notetype(notetype_id)?;

        Ok(Response::new(GetNotetypeResponse {
            notetype: Some(adapter::map_notetype(&notetype)),
        }))
    }

    async fn get_notetypes(
        &self,
        request: Request<GetNotetypesRequest>,
    ) -> Result<Response<GetNotetypesResponse>, Status> {
        let notetype_ids = request.into_inner().notetype_ids;
        let mut notetypes = Vec::with_capacity(notetype_ids.len());
        for notetype_id in notetype_ids {
            if notetype_id <= 0 {
                return Err(Status::invalid_argument("notetype_id must be > 0"));
            }
            let notetype = self.store.get_notetype(notetype_id)?;
            notetypes.push(adapter::map_notetype(&notetype));
        }
        Ok(Response::new(GetNotetypesResponse { notetypes }))
    }

    async fn get_notetype_id_by_name(
        &self,
        request: Request<GetNotetypeIdByNameRequest>,
    ) -> Result<Response<GetNotetypeIdByNameResponse>, Status> {
        let name = request.into_inner().name;
        if name.is_empty() {
            return Err(Status::invalid_argument("name must not be empty"));
        }
        let notetype_id = self.store.get_notetype_id_by_name(&name)?;
        Ok(Response::new(GetNotetypeIdByNameResponse { notetype_id }))
    }

    async fn update_templates(
        &self,
        request: Request<UpdateTemplatesRequest>,
    ) -> Result<Response<UpdateTemplatesResponse>, Status> {
        let response = update_templates_inner(&self.store, request.into_inner())?;
        Ok(Response::new(response))
    }

    async fn update_templates_batch(
        &self,
        request: Request<UpdateTemplatesBatchRequest>,
    ) -> Result<Response<UpdateTemplatesBatchResponse>, Status> {
        let updates = request.into_inner().updates;
        let results = common::execute_batch(updates, "update_templates_batch", |update| {
            let response = update_templates_inner(&self.store, update)?;
            let notetype = response
                .notetype
                .ok_or_else(|| Status::internal("batch templates response missing notetype"))?;
            Ok(NotetypeWriteMetadata {
                notetype_id: notetype.notetype_id,
                usn: notetype.usn,
            })
        })?;
        Ok(Response::new(UpdateTemplatesBatchResponse { results }))
    }

    async fn update_css(
        &self,
        request: Request<UpdateCssRequest>,
    ) -> Result<Response<UpdateCssResponse>, Status> {
        let response = update_css_inner(&self.store, request.into_inner())?;
        Ok(Response::new(response))
    }

    async fn update_notetype_content(
        &self,
        request: Request<UpdateNotetypeContentRequest>,
    ) -> Result<Response<UpdateNotetypeContentResponse>, Status> {
        let response = update_notetype_content_inner(&self.store, request.into_inner())?;
        Ok(Response::new(response))
    }

    async fn update_css_batch(
        &self,
        request: Request<UpdateCssBatchRequest>,
    ) -> Result<Response<UpdateCssBatchResponse>, Status> {
        let updates = request.into_inner().updates;
        let results = common::execute_batch(updates, "update_css_batch", |update| {
            let response = update_css_inner(&self.store, update)?;
            let notetype = response
                .notetype
                .ok_or_else(|| Status::internal("batch css response missing notetype"))?;
            Ok(NotetypeWriteMetadata {
                notetype_id: notetype.notetype_id,
                usn: notetype.usn,
            })
        })?;
        Ok(Response::new(UpdateCssBatchResponse { results }))
    }

    async fn get_notetype_changes(
        &self,
        request: Request<GetNotetypeChangesRequest>,
    ) -> Result<Response<GetNotetypeChangesResponse>, Status> {
        let request = request.into_inner();
        let (rows, next_cursor) =
            common::get_changes_page(&request.cursor, request.limit, |cursor, limit| {
                self.store
                    .get_notetype_changes_page(cursor.0, cursor.1, limit)
            })?;

        let changes: Vec<NotetypeChange> = rows
            .iter()
            .map(|(usn, notetype_id, mtime_secs)| NotetypeChange {
                notetype_id: *notetype_id,
                modified_at: Some(common::timestamp_from_secs(*mtime_secs)),
                usn: *usn,
            })
            .collect();

        Ok(Response::new(GetNotetypeChangesResponse {
            changes,
            next_cursor,
        }))
    }

    async fn count_notetypes(
        &self,
        _request: Request<CountNotetypesRequest>,
    ) -> Result<Response<CountNotetypesResponse>, Status> {
        let refs = self.store.list_notetype_refs()?;
        Ok(Response::new(CountNotetypesResponse {
            count: refs.len() as u64,
        }))
    }
}

fn update_templates_inner(
    store: &SharedStore,
    request: UpdateTemplatesRequest,
) -> Result<UpdateTemplatesResponse, Status> {
    let updated = update_notetype_content(
        store,
        request.notetype_id,
        request.templates,
        None,
        request.expected_usn,
    )?;
    Ok(UpdateTemplatesResponse {
        notetype: Some(adapter::map_notetype(&updated)),
    })
}

fn update_css_inner(
    store: &SharedStore,
    request: UpdateCssRequest,
) -> Result<UpdateCssResponse, Status> {
    let updated = update_notetype_content(
        store,
        request.notetype_id,
        Vec::new(),
        Some(request.css),
        request.expected_usn,
    )?;

    Ok(UpdateCssResponse {
        notetype: Some(adapter::map_notetype(&updated)),
    })
}

fn update_notetype_content_inner(
    store: &SharedStore,
    request: UpdateNotetypeContentRequest,
) -> Result<UpdateNotetypeContentResponse, Status> {
    let updated = update_notetype_content(
        store,
        request.notetype_id,
        request.templates,
        Some(request.css),
        request.expected_usn,
    )?;
    Ok(UpdateNotetypeContentResponse {
        notetype: Some(adapter::map_notetype(&updated)),
    })
}

fn update_notetype_content(
    store: &SharedStore,
    notetype_id: i64,
    templates: Vec<anki_api_proto::anki::api::v1::NotetypeTemplate>,
    css: Option<String>,
    expected_usn: Option<i64>,
) -> Result<anki_proto::notetypes::Notetype, Status> {
    if notetype_id <= 0 {
        return Err(Status::invalid_argument("notetype_id must be > 0"));
    }

    let mut notetype = store.get_notetype(notetype_id)?;
    common::enforce_expected_usn(
        expected_usn,
        i64::from(notetype.usn),
        "notetype",
        notetype.id,
    )?;

    if !templates.is_empty() {
        let ord_to_index: HashMap<u32, usize> = notetype
            .templates
            .iter()
            .enumerate()
            .map(|(index, template)| {
                (
                    template
                        .ord
                        .as_ref()
                        .map(|ord| ord.val)
                        .unwrap_or(index as u32),
                    index,
                )
            })
            .collect();

        for template in templates {
            let Some(index) = ord_to_index.get(&template.ordinal) else {
                return Err(Status::invalid_argument(format!(
                    "unknown template ordinal: {}",
                    template.ordinal
                )));
            };
            let backend_template = &mut notetype.templates[*index];
            backend_template.name = template.name;
            let config = backend_template.config.get_or_insert_with(Default::default);
            config.q_format = template.front;
            config.a_format = template.back;
        }
    }

    if let Some(css) = css {
        let config = notetype.config.get_or_insert_with(Default::default);
        config.css = css;
    }

    store.update_notetype(notetype)?;
    store.get_notetype(notetype_id)
}

#[cfg(test)]
mod tests {
    use anki_api_proto::anki::api::v1::ErrorDetail;
    use prost14::Message;
    use tonic::Code;
    use tonic::Request;

    use super::*;

    fn sorted_notetype_refs(mut refs: Vec<NotetypeRef>) -> Vec<NotetypeRef> {
        refs.sort_by_key(|entry| entry.notetype_id);
        refs
    }

    #[tokio::test]
    async fn update_css_reports_version_conflict_details() {
        let fixture = crate::service::common::TestStore::new("notetypes-conflict");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");
        let notetype = store.get_notetype(notetype_id).expect("notetype");

        let updated = <NotetypesApi as NotetypesService>::update_css(
            &api,
            Request::new(UpdateCssRequest {
                notetype_id,
                css: ".card { color: red; }".to_owned(),
                expected_usn: Some(i64::from(notetype.usn)),
            }),
        )
        .await
        .expect("update with expected_usn")
        .into_inner()
        .notetype
        .expect("notetype response");
        assert!(updated.css.contains("color: red"));

        let stale = <NotetypesApi as NotetypesService>::update_css(
            &api,
            Request::new(UpdateCssRequest {
                notetype_id,
                css: ".card { color: blue; }".to_owned(),
                expected_usn: Some(i64::from(notetype.usn)),
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
    async fn update_templates_applies_changes() {
        let fixture = crate::service::common::TestStore::new("notetypes-templates-success");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");
        let notetype = store.get_notetype(notetype_id).expect("notetype");
        let template = notetype.templates.first().expect("at least one template");
        let ordinal = template.ord.as_ref().map(|ord| ord.val).unwrap_or(0);

        let response = <NotetypesApi as NotetypesService>::update_templates(
            &api,
            Request::new(UpdateTemplatesRequest {
                notetype_id,
                templates: vec![anki_api_proto::anki::api::v1::NotetypeTemplate {
                    ordinal,
                    name: "Card 1 Updated".to_owned(),
                    front: "{{Front}}<hr/>updated".to_owned(),
                    back: "{{Back}}<hr/>updated".to_owned(),
                }],
                expected_usn: Some(i64::from(notetype.usn)),
            }),
        )
        .await
        .expect("update templates")
        .into_inner()
        .notetype
        .expect("notetype response");

        let updated = response
            .templates
            .iter()
            .find(|template| template.ordinal == ordinal)
            .expect("updated template exists");
        assert_eq!(updated.name, "Card 1 Updated");
        assert_eq!(updated.front, "{{Front}}<hr/>updated");
        assert_eq!(updated.back, "{{Back}}<hr/>updated");
    }

    #[tokio::test]
    async fn update_notetype_content_applies_templates_and_css() {
        let fixture = crate::service::common::TestStore::new("notetypes-content-success");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");
        let notetype = store.get_notetype(notetype_id).expect("notetype");
        let template = notetype.templates.first().expect("at least one template");
        let ordinal = template.ord.as_ref().map(|ord| ord.val).unwrap_or(0);

        let response = <NotetypesApi as NotetypesService>::update_notetype_content(
            &api,
            Request::new(UpdateNotetypeContentRequest {
                notetype_id,
                templates: vec![anki_api_proto::anki::api::v1::NotetypeTemplate {
                    ordinal,
                    name: "Card 1 Content".to_owned(),
                    front: "{{Front}}<hr/>content".to_owned(),
                    back: "{{Back}}<hr/>content".to_owned(),
                }],
                css: ".card { color: green; }".to_owned(),
                expected_usn: Some(i64::from(notetype.usn)),
            }),
        )
        .await
        .expect("update content")
        .into_inner()
        .notetype
        .expect("notetype response");

        let updated = response
            .templates
            .iter()
            .find(|template| template.ordinal == ordinal)
            .expect("updated template exists");
        assert_eq!(updated.name, "Card 1 Content");
        assert_eq!(updated.front, "{{Front}}<hr/>content");
        assert_eq!(updated.back, "{{Back}}<hr/>content");
        assert!(response.css.contains("color: green"));
    }

    #[tokio::test]
    async fn update_templates_batch_returns_write_metadata_and_stops_on_error() {
        let fixture = crate::service::common::TestStore::new("notetypes-templates-batch");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");
        let notetype = store.get_notetype(notetype_id).expect("notetype");
        let template = notetype.templates.first().expect("at least one template");
        let ordinal = template.ord.as_ref().map(|ord| ord.val).unwrap_or(0);

        let err = <NotetypesApi as NotetypesService>::update_templates_batch(
            &api,
            Request::new(UpdateTemplatesBatchRequest {
                updates: vec![
                    UpdateTemplatesRequest {
                        notetype_id,
                        templates: vec![anki_api_proto::anki::api::v1::NotetypeTemplate {
                            ordinal,
                            name: "Card 1 Batch".to_owned(),
                            front: "{{Front}}<hr/>batch".to_owned(),
                            back: "{{Back}}<hr/>batch".to_owned(),
                        }],
                        expected_usn: None,
                    },
                    UpdateTemplatesRequest {
                        notetype_id,
                        templates: vec![anki_api_proto::anki::api::v1::NotetypeTemplate {
                            ordinal: u32::MAX,
                            name: "Invalid".to_owned(),
                            front: "x".to_owned(),
                            back: "y".to_owned(),
                        }],
                        expected_usn: None,
                    },
                ],
            }),
        )
        .await
        .expect_err("second update should fail");
        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err.message().contains("batch index 1"));
    }

    #[tokio::test]
    async fn update_templates_batch_returns_write_metadata() {
        let fixture = crate::service::common::TestStore::new("notetypes-templates-batch-metadata");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");
        let notetype = store.get_notetype(notetype_id).expect("notetype");
        let template = notetype.templates.first().expect("at least one template");
        let ordinal = template.ord.as_ref().map(|ord| ord.val).unwrap_or(0);

        let response = <NotetypesApi as NotetypesService>::update_templates_batch(
            &api,
            Request::new(UpdateTemplatesBatchRequest {
                updates: vec![UpdateTemplatesRequest {
                    notetype_id,
                    templates: vec![anki_api_proto::anki::api::v1::NotetypeTemplate {
                        ordinal,
                        name: "Card 1 Batch Meta".to_owned(),
                        front: "{{Front}}<hr/>batch-meta".to_owned(),
                        back: "{{Back}}<hr/>batch-meta".to_owned(),
                    }],
                    expected_usn: None,
                }],
            }),
        )
        .await
        .expect("batch templates")
        .into_inner();

        assert_eq!(response.results.len(), 1);
        let metadata = &response.results[0];
        assert_eq!(metadata.notetype_id, notetype_id);
        let current = store.get_notetype(notetype_id).expect("current notetype");
        assert_eq!(metadata.usn, i64::from(current.usn));
    }

    #[tokio::test]
    async fn update_css_batch_returns_write_metadata() {
        let fixture = crate::service::common::TestStore::new("notetypes-css-batch");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");

        let response = <NotetypesApi as NotetypesService>::update_css_batch(
            &api,
            Request::new(UpdateCssBatchRequest {
                updates: vec![UpdateCssRequest {
                    notetype_id,
                    css: ".card { color: orange; }".to_owned(),
                    expected_usn: None,
                }],
            }),
        )
        .await
        .expect("batch css")
        .into_inner();

        assert_eq!(response.results.len(), 1);
        let metadata = &response.results[0];
        assert_eq!(metadata.notetype_id, notetype_id);
        let current = store.get_notetype(notetype_id).expect("current notetype");
        assert_eq!(metadata.usn, i64::from(current.usn));
    }

    #[tokio::test]
    async fn get_notetype_changes_returns_entries() {
        let fixture = crate::service::common::TestStore::new("notetypes-changes");
        let store = fixture.store();
        let api = NotetypesApi::new(store);

        let response = <NotetypesApi as NotetypesService>::get_notetype_changes(
            &api,
            Request::new(GetNotetypeChangesRequest {
                cursor: String::new(),
                limit: 10,
            }),
        )
        .await
        .expect("get changes")
        .into_inner();

        assert!(!response.changes.is_empty());
    }

    #[tokio::test]
    async fn get_notetype_id_by_name_resolves_existing_name() {
        let fixture = crate::service::common::TestStore::new("notetypes-id-by-name-existing");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let notetype_id = store
            .list_notetype_ids()
            .expect("notetype ids")
            .into_iter()
            .next()
            .expect("at least one notetype");
        let notetype = store.get_notetype(notetype_id).expect("notetype");

        let response = <NotetypesApi as NotetypesService>::get_notetype_id_by_name(
            &api,
            Request::new(GetNotetypeIdByNameRequest {
                name: notetype.name,
            }),
        )
        .await
        .expect("lookup by name")
        .into_inner();

        assert_eq!(response.notetype_id, notetype_id);
    }

    #[tokio::test]
    async fn get_notetype_id_by_name_rejects_empty_name() {
        let fixture = crate::service::common::TestStore::new("notetypes-id-by-name-empty");
        let api = NotetypesApi::new(fixture.store());

        let err = <NotetypesApi as NotetypesService>::get_notetype_id_by_name(
            &api,
            Request::new(GetNotetypeIdByNameRequest {
                name: String::new(),
            }),
        )
        .await
        .expect_err("empty name should fail");
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn get_notetype_id_by_name_returns_not_found_for_missing_name() {
        let fixture = crate::service::common::TestStore::new("notetypes-id-by-name-missing");
        let api = NotetypesApi::new(fixture.store());

        let err = <NotetypesApi as NotetypesService>::get_notetype_id_by_name(
            &api,
            Request::new(GetNotetypeIdByNameRequest {
                name: "does-not-exist".to_owned(),
            }),
        )
        .await
        .expect_err("missing name should fail");
        assert_eq!(err.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn get_notetypes_returns_results_in_request_order() {
        let fixture = crate::service::common::TestStore::new("notetypes-get-batch-order");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());
        let mut ids = store.list_notetype_ids().expect("notetype ids");
        assert!(ids.len() >= 2);

        let request_ids = vec![ids.remove(1), ids.remove(0)];
        let response = <NotetypesApi as NotetypesService>::get_notetypes(
            &api,
            Request::new(GetNotetypesRequest {
                notetype_ids: request_ids.clone(),
            }),
        )
        .await
        .expect("get notetypes")
        .into_inner();

        let response_ids = response
            .notetypes
            .iter()
            .map(|notetype| notetype.notetype_id)
            .collect::<Vec<_>>();
        assert_eq!(response_ids, request_ids);
    }

    #[tokio::test]
    async fn list_notetype_refs_returns_ids_and_names() {
        let fixture = crate::service::common::TestStore::new("notetype-refs-shape");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());

        let response = <NotetypesApi as NotetypesService>::list_notetype_refs(
            &api,
            Request::new(ListNotetypeRefsRequest {}),
        )
        .await
        .expect("list refs")
        .into_inner();

        assert!(!response.notetypes.is_empty());
        let first = response.notetypes.first().expect("first ref");
        assert!(first.notetype_id > 0);
        assert!(!first.name.is_empty());
    }

    #[tokio::test]
    async fn count_notetypes_matches_list_notetype_refs() {
        let fixture = crate::service::common::TestStore::new("notetypes-count");
        let store = fixture.store();
        let api = NotetypesApi::new(store.clone());

        let refs = <NotetypesApi as NotetypesService>::list_notetype_refs(
            &api,
            Request::new(ListNotetypeRefsRequest {}),
        )
        .await
        .expect("list refs")
        .into_inner();

        let count = <NotetypesApi as NotetypesService>::count_notetypes(
            &api,
            Request::new(CountNotetypesRequest {}),
        )
        .await
        .expect("count notetypes")
        .into_inner();

        assert_eq!(count.count, refs.notetypes.len() as u64);
    }

    #[tokio::test]
    async fn list_notetype_refs_are_sorted_by_notetype_id() {
        let fixture = crate::service::common::TestStore::new("notetype-refs-order");
        let api = NotetypesApi::new(fixture.store());

        let response = <NotetypesApi as NotetypesService>::list_notetype_refs(
            &api,
            Request::new(ListNotetypeRefsRequest {}),
        )
        .await
        .expect("list refs")
        .into_inner();

        assert_eq!(
            response.notetypes,
            sorted_notetype_refs(response.notetypes.clone())
        );
    }
}
