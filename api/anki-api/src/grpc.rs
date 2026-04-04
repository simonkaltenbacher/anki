use std::future::Future;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use anki_api_proto::anki::api::v1::decks_service_server::DecksServiceServer;
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
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Identity;
use tonic::transport::Server;
use tonic::transport::ServerTlsConfig;
use tonic::transport::server::TcpConnectInfo;
use tonic_health::ServingStatus;
use tower_http::classify::GrpcFailureClass;
use tower_http::trace::DefaultOnBodyChunk;
use tower_http::trace::DefaultOnEos;
use tower_http::trace::GrpcMakeClassifier;
use tower_http::trace::TraceLayer;
use tracing::Span;

use crate::auth::ApiKeyAuthenticator;
use crate::config::ServerConfig;
use crate::config::ServerConnectionMode;
use crate::config::TlsAuthMode;
use crate::service::decks::DecksApi;
use crate::service::health::HealthApi;
use crate::service::notes::NotesApi;
use crate::service::notetypes::NotetypesApi;
use crate::service::system::SystemApi;
use crate::store;
use crate::transport;
use crate::transport::SpiffeConnectInfo;

type GrpcTraceLayer = TraceLayer<
    GrpcMakeClassifier,
    fn(&HttpRequest<Body>) -> Span,
    (),
    fn(&HttpResponse<Body>, Duration, &Span),
    DefaultOnBodyChunk,
    DefaultOnEos,
    fn(GrpcFailureClass, Duration, &Span),
>;

type StartupStatusTx = Sender<Result<(), String>>;

pub struct GrpcServer<F> {
    config: ServerConfig,
    store: store::BackendStore,
    shutdown_signal: F,
    startup_status_tx: Option<StartupStatusTx>,
}

impl<F> GrpcServer<F>
where
    F: Future<Output = ()> + Send + 'static,
{
    pub fn new(config: ServerConfig, store: store::BackendStore, shutdown_signal: F) -> Self {
        Self {
            config,
            store,
            shutdown_signal,
            startup_status_tx: None,
        }
    }

    pub fn with_startup_status(mut self, startup_status_tx: StartupStatusTx) -> Self {
        self.startup_status_tx = Some(startup_status_tx);
        self
    }

    pub async fn serve(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let bind_addr = resolve_bind_addr(&self.config)?;
        let auth = Arc::new(ApiKeyAuthenticator::new(&self.config));

        let health_api = HealthApi::new(self.store.clone());
        let health_service = with_auth(
            HealthServiceServer::new(health_api),
            Arc::clone(&auth),
            true,
        );
        let (standard_health_reporter, standard_health_service) =
            tonic_health::server::health_reporter();
        standard_health_reporter
            .set_service_status("", ServingStatus::Serving)
            .await;

        let standard_health_service = with_auth(standard_health_service, Arc::clone(&auth), true);

        let system_service = with_auth(
            SystemServiceServer::new(SystemApi::new(
                format!("anki-api/{}", env!("CARGO_PKG_VERSION")),
                self.config.anki_version.clone(),
                configured_capabilities(&self.config),
            )),
            Arc::clone(&auth),
            false,
        );

        let notes_api = NotesApi::new(self.store.clone());
        let notes_service = with_auth(NotesServiceServer::new(notes_api), Arc::clone(&auth), false);

        let decks_api = DecksApi::new(self.store.clone());
        let decks_service = with_auth(DecksServiceServer::new(decks_api), Arc::clone(&auth), false);

        let notetypes_api = NotetypesApi::new(self.store.clone());
        let notetypes_service = with_auth(
            NotetypesServiceServer::new(notetypes_api),
            Arc::clone(&auth),
            false,
        );

        self.log_startup(bind_addr);
        let listener = self.bind_listener(bind_addr).await?;
        let add_services = move |server: Server| {
            server
                .layer(make_grpc_trace_layer())
                .add_service(standard_health_service)
                .add_service(health_service)
                .add_service(system_service)
                .add_service(decks_service)
                .add_service(notes_service)
                .add_service(notetypes_service)
        };
        let server = self.configure_server()?;

        if let ServerConnectionMode::Spiffe(spiffe) = &self.config.connection_mode {
            let incoming = transport::build_spiffe_incoming(listener, spiffe).await?;
            self.report_startup(Ok(()));
            add_services(server)
                .serve_with_incoming_shutdown(incoming, self.shutdown_signal)
                .await?;
        } else {
            self.report_startup(Ok(()));
            add_services(server)
                .serve_with_incoming_shutdown(
                    TcpListenerStream::new(listener),
                    self.shutdown_signal,
                )
                .await?;
        }

        Ok(())
    }

    async fn bind_listener(
        &mut self,
        bind_addr: SocketAddr,
    ) -> Result<TcpListener, Box<dyn std::error::Error + Send + Sync>> {
        match TcpListener::bind(bind_addr).await {
            Ok(listener) => Ok(listener),
            Err(err) => {
                self.report_startup(Err(format!(
                    "failed to bind api server on {bind_addr}: {err}"
                )));
                Err(Box::new(err))
            }
        }
    }

    fn configure_server(&self) -> Result<Server, Box<dyn std::error::Error + Send + Sync>> {
        let server = Server::builder();

        match &self.config.connection_mode {
            ServerConnectionMode::Tls { tls, .. } => {
                let identity = load_tls_identity(&tls.cert_path, &tls.key_path)?;
                Ok(server.tls_config(ServerTlsConfig::new().identity(identity))?)
            }
            ServerConnectionMode::Plaintext | ServerConnectionMode::Spiffe(_) => Ok(server),
        }
    }

    fn log_startup(&self, bind_addr: SocketAddr) {
        tracing::info!(
            address = %bind_addr,
            auth_mode = self.auth_mode_name(),
            transport = self.transport_name(),
            "starting anki api grpc server"
        );

        if self.config.allow_non_local
            && matches!(self.config.connection_mode, ServerConnectionMode::Plaintext)
        {
            tracing::warn!("non-local binding enabled; terminate TLS at a reverse proxy");
        }
        if matches!(
            self.config.connection_mode,
            ServerConnectionMode::Tls {
                auth: TlsAuthMode::Disabled,
                ..
            }
        ) {
            tracing::warn!(
                "tls transport is enabled with auth disabled; requests will not require an api key"
            );
        }
    }

    fn report_startup(&mut self, status: Result<(), String>) {
        if let Some(tx) = self.startup_status_tx.take() {
            let _ = tx.send(status);
        }
    }

    fn auth_mode_name(&self) -> &'static str {
        match &self.config.connection_mode {
            ServerConnectionMode::Plaintext => "disabled",
            ServerConnectionMode::Tls {
                auth: TlsAuthMode::Disabled,
                ..
            } => "disabled",
            ServerConnectionMode::Tls {
                auth: TlsAuthMode::ApiKey(_),
                ..
            } => "api_key",
            ServerConnectionMode::Spiffe(_) => "spiffe_mtls",
        }
    }

    fn transport_name(&self) -> &'static str {
        match &self.config.connection_mode {
            ServerConnectionMode::Plaintext => "plaintext",
            ServerConnectionMode::Tls { .. } => "tls",
            ServerConnectionMode::Spiffe(_) => "spiffe",
        }
    }
}

fn with_auth<S>(
    service: S,
    auth: Arc<ApiKeyAuthenticator>,
    is_health_check: bool,
) -> InterceptedService<S, AuthInterceptor> {
    InterceptedService::new(
        service,
        AuthInterceptor {
            auth,
            is_health_check,
        },
    )
}

#[derive(Clone)]
struct AuthInterceptor {
    auth: Arc<ApiKeyAuthenticator>,
    is_health_check: bool,
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, tonic::Status> {
        self.auth.authenticate(&request, self.is_health_check)?;
        Ok(request)
    }
}

fn make_grpc_trace_layer() -> GrpcTraceLayer {
    TraceLayer::new_for_grpc()
        .make_span_with(make_grpc_request_span as fn(&HttpRequest<Body>) -> Span)
        .on_request(())
        .on_response(log_grpc_response as fn(&HttpResponse<Body>, Duration, &Span))
        .on_failure(log_grpc_failure as fn(GrpcFailureClass, Duration, &Span))
}

fn make_grpc_request_span(request: &HttpRequest<Body>) -> Span {
    let user_agent = request
        .headers()
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-");
    let remote_addr = request
        .extensions()
        .get::<SpiffeConnectInfo>()
        .and_then(SpiffeConnectInfo::remote_addr)
        .or_else(|| {
            request
                .extensions()
                .get::<TcpConnectInfo>()
                .and_then(TcpConnectInfo::remote_addr)
        })
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let spiffe_id = request
        .extensions()
        .get::<SpiffeConnectInfo>()
        .and_then(SpiffeConnectInfo::peer_identity)
        .map(|identity| identity.spiffe_id().to_owned())
        .unwrap_or_else(|| "-".to_owned());

    tracing::info_span!(
        "grpc_request",
        method = %request.uri().path(),
        user_agent = %user_agent,
        remote_addr = %remote_addr,
        spiffe_id = %spiffe_id
    )
}

fn log_grpc_response(response: &HttpResponse<Body>, latency: Duration, _span: &Span) {
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
}

fn log_grpc_failure(failure_classification: GrpcFailureClass, latency: Duration, _span: &Span) {
    tracing::warn!(
        latency_ms = latency.as_millis() as u64,
        failure = ?failure_classification,
        "grpc request failed"
    );
}

fn load_tls_identity(
    cert_path: &str,
    key_path: &str,
) -> Result<Identity, Box<dyn std::error::Error + Send + Sync>> {
    let cert = std::fs::read(cert_path)?;
    let key = std::fs::read(key_path)?;
    Ok(Identity::from_pem(cert, key))
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
        "decks.list_refs".to_owned(),
        "decks.get_id_by_name".to_owned(),
        "notes.get".to_owned(),
        "notes.get.batch".to_owned(),
        "notes.create".to_owned(),
        "notes.create.batch".to_owned(),
        "notes.delete".to_owned(),
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
    if matches!(
        config.connection_mode,
        crate::config::ServerConnectionMode::Tls {
            auth: crate::config::TlsAuthMode::ApiKey(_),
            ..
        }
    ) {
        capabilities.push("auth.api_key".to_owned());
    }
    if matches!(
        config.connection_mode,
        crate::config::ServerConnectionMode::Spiffe(_)
    ) {
        capabilities.push("auth.spiffe_mtls".to_owned());
    }
    capabilities
}

#[cfg(test)]
mod tests {
    use super::configured_capabilities;
    use crate::config::ServerConfig;
    use crate::config::ServerConnectionMode;
    use crate::config::SpiffeTransportConfig;
    use crate::config::TlsAuthMode;
    use crate::config::TlsTransportConfig;

    #[test]
    fn configured_capabilities_include_notes_delete() {
        let capabilities = configured_capabilities(&ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            anki_version: None,
            allow_non_local: false,
            connection_mode: ServerConnectionMode::Plaintext,
        });

        assert!(capabilities.iter().any(|cap| cap == "notes.delete"));
    }

    #[test]
    fn configured_capabilities_include_notes_create_batch() {
        let capabilities = configured_capabilities(&ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            anki_version: None,
            allow_non_local: false,
            connection_mode: ServerConnectionMode::Plaintext,
        });

        assert!(capabilities.iter().any(|cap| cap == "notes.create.batch"));
    }

    #[test]
    fn configured_capabilities_include_spiffe_auth_when_enabled() {
        let capabilities = configured_capabilities(&ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            anki_version: None,
            allow_non_local: false,
            connection_mode: ServerConnectionMode::Spiffe(SpiffeTransportConfig {
                allowed_client_id: "spiffe://example.org/anki-edit".to_owned(),
                workload_api_socket: None,
            }),
        });

        assert!(capabilities.iter().any(|cap| cap == "auth.spiffe_mtls"));
    }

    #[test]
    fn configured_capabilities_include_api_key_only_for_tls() {
        let capabilities = configured_capabilities(&ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            anki_version: None,
            allow_non_local: false,
            connection_mode: ServerConnectionMode::Tls {
                tls: TlsTransportConfig {
                    cert_path: "/tmp/server.pem".to_owned(),
                    key_path: "/tmp/server.key".to_owned(),
                },
                auth: TlsAuthMode::ApiKey("test-key".to_owned()),
            },
        });

        assert!(capabilities.iter().any(|cap| cap == "auth.api_key"));
    }
}
