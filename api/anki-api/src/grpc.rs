use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::{future::Future, future::pending};

use anki_api_proto::anki::api::v1::health_service_server::HealthServiceServer;
use anki_api_proto::anki::api::v1::notes_service_server::NotesServiceServer;
use anki_api_proto::anki::api::v1::notetypes_service_server::NotetypesServiceServer;
use anki_api_proto::anki::api::v1::system_service_server::SystemServiceServer;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::Request;
use tonic::body::Body;
use tonic::codegen::http::Request as HttpRequest;
use tonic::codegen::http::Response as HttpResponse;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Server;
use tonic_health::ServingStatus;
use tower_http::trace::TraceLayer;
use tracing::Span;

use crate::auth::ApiKeyAuthenticator;
use crate::config::ServerConfig;
use crate::service::health::HealthApi;
use crate::service::notes::NotesApi;
use crate::service::notetypes::NotetypesApi;
use crate::service::system::SystemApi;
use crate::store;

pub async fn serve_with_store(
    config: ServerConfig,
    store: store::SharedStore,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    serve_with_store_and_shutdown(config, store, pending::<()>()).await
}

pub async fn serve_with_store_and_shutdown<F>(
    config: ServerConfig,
    store: store::SharedStore,
    shutdown_signal: F,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: Future<Output = ()> + Send + 'static,
{
    serve_with_store_and_shutdown_and_ready(config, store, shutdown_signal, None).await
}

pub async fn serve_with_store_and_shutdown_and_ready<F>(
    config: ServerConfig,
    store: store::SharedStore,
    shutdown_signal: F,
    ready_tx: Option<Sender<Result<(), String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: Future<Output = ()> + Send + 'static,
{
    let bind_addr = resolve_bind_addr(&config)?;
    let auth = Arc::new(ApiKeyAuthenticator::new(&config));
    let mut ready_tx = ready_tx;

    let health_auth = Arc::clone(&auth);
    let health_api = HealthApi::new(Arc::clone(&store));
    let health_service =
        HealthServiceServer::with_interceptor(health_api, move |request: Request<()>| {
            health_auth.authenticate(&request, true)?;
            Ok(request)
        });
    let (standard_health_reporter, standard_health_service) =
        tonic_health::server::health_reporter();
    standard_health_reporter
        .set_service_status("", ServingStatus::Serving)
        .await;

    let standard_health_auth = Arc::clone(&auth);
    let standard_health_service =
        InterceptedService::new(standard_health_service, move |request: Request<()>| {
            standard_health_auth.authenticate(&request, true)?;
            Ok(request)
        });

    let system_auth = Arc::clone(&auth);
    let system_service = SystemServiceServer::with_interceptor(
        SystemApi::new(
            format!("anki-api/{}", env!("CARGO_PKG_VERSION")),
            config.anki_version.clone(),
            configured_capabilities(&config),
        ),
        move |request: Request<()>| {
            system_auth.authenticate(&request, false)?;
            Ok(request)
        },
    );

    let notes_auth = Arc::clone(&auth);
    let notes_api = NotesApi::new(Arc::clone(&store));
    let notes_service =
        NotesServiceServer::with_interceptor(notes_api, move |request: Request<()>| {
            notes_auth.authenticate(&request, false)?;
            Ok(request)
        });

    let notetypes_auth = Arc::clone(&auth);
    let notetypes_api = NotetypesApi::new(Arc::clone(&store));
    let notetypes_service =
        NotetypesServiceServer::with_interceptor(notetypes_api, move |request: Request<()>| {
            notetypes_auth.authenticate(&request, false)?;
            Ok(request)
        });

    tracing::info!(
        address = %bind_addr,
        auth = if config.auth_disabled { "disabled" } else { "enabled" },
        "starting anki api grpc server"
    );
    if config.allow_non_local {
        tracing::warn!("non-local binding enabled; terminate TLS at a reverse proxy");
    }

    let listener = match TcpListener::bind(bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(Err(format!(
                    "failed to bind api server on {bind_addr}: {err}"
                )));
            }
            return Err(Box::new(err));
        }
    };
    if let Some(tx) = ready_tx.take() {
        let _ = tx.send(Ok(()));
    }

    Server::builder()
        .layer(
            TraceLayer::new_for_grpc()
                .make_span_with(|request: &HttpRequest<Body>| {
                    let user_agent = request
                        .headers()
                        .get("user-agent")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!(
                        "grpc_request",
                        method = %request.uri().path(),
                        user_agent = %user_agent
                    )
                })
                .on_request(())
                .on_response(
                    |response: &HttpResponse<Body>, latency: Duration, _span: &Span| {
                        let grpc_status = response
                            .headers()
                            .get("grpc-status")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("0");
                        tracing::debug!(
                            latency_ms = latency.as_millis() as u64,
                            grpc_status = grpc_status,
                            "grpc request completed"
                        );
                    },
                )
                .on_failure(|failure_classification, latency: Duration, _span: &Span| {
                    tracing::warn!(
                        latency_ms = latency.as_millis() as u64,
                        failure = ?failure_classification,
                        "grpc request failed"
                    );
                }),
        )
        .add_service(standard_health_service)
        .add_service(health_service)
        .add_service(system_service)
        .add_service(notes_service)
        .add_service(notetypes_service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown_signal)
        .await?;

    Ok(())
}

fn resolve_bind_addr(
    config: &ServerConfig,
) -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
    if let Ok(addr) = SocketAddr::from_str(&config.bind_addr()) {
        return Ok(addr);
    }

    (config.host.as_str(), config.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| "failed to resolve bind address".into())
}

fn configured_capabilities(config: &ServerConfig) -> Vec<String> {
    let mut capabilities = vec![
        "health.check".to_owned(),
        "system.server_info".to_owned(),
        "notes.get".to_owned(),
        "notes.get.batch".to_owned(),
        "notes.list_refs.stream".to_owned(),
        "notes.list.stream".to_owned(),
        "notes.update_fields".to_owned(),
        "notes.update_fields.batch".to_owned(),
        "notes.changes".to_owned(),
        "notes.count".to_owned(),
        "notetypes.get".to_owned(),
        "notetypes.get.batch".to_owned(),
        "notetypes.get_id_by_name".to_owned(),
        "notetypes.list_refs".to_owned(),
        "notetypes.list".to_owned(),
        "notetypes.update_content".to_owned(),
        "notetypes.update_templates".to_owned(),
        "notetypes.update_templates.batch".to_owned(),
        "notetypes.update_css".to_owned(),
        "notetypes.update_css.batch".to_owned(),
        "notetypes.changes".to_owned(),
        "notetypes.count".to_owned(),
    ];
    if !config.auth_disabled {
        capabilities.push("auth.api_key".to_owned());
    }
    capabilities
}
