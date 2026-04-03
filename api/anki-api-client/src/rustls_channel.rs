use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::header::USER_AGENT;
use http::{HeaderValue, Request, Response, Uri};
use hyper::client::conn::http2::Builder;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector as RustlsConnector;
use tonic::body::Body;
use tower::buffer::{future::ResponseFuture as BufferResponseFuture, Buffer};
use tower::Service;

const DEFAULT_BUFFER_SIZE: usize = 1024;
const TONIC_USER_AGENT: &str = concat!("tonic/", env!("CARGO_PKG_VERSION"));

pub(crate) type BoxError = Box<dyn StdError + Send + Sync + 'static>;

pub(crate) async fn connect(
    uri: Uri,
    connect_timeout: Option<Duration>,
    tls_config: tokio_rustls::rustls::ClientConfig,
    server_name: ServerName<'static>,
) -> Result<Channel, Error> {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_nodelay(true);
    http.set_connect_timeout(connect_timeout);

    let io = http.call(uri.clone()).await.map_err(Error::from_source)?;
    let io = io.into_inner();
    let io = TlsConnector::new(tls_config, server_name)
        .connect(io)
        .await
        .map_err(Error::from_source)?;

    let mut builder = Builder::new(TokioExecutor::new());
    builder.timer(TokioTimer::new());
    let (send_request, connection) = builder
        .handshake::<_, Body>(io)
        .await
        .map_err(Error::from_source)?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::debug!(error = ?error, "SPIFFE channel connection task failed");
        }
    });

    let service = SendRequest {
        inner: send_request,
    };
    let service = AddOrigin::new(service, uri);
    let service = UserAgent::new(service);
    let (svc, worker) = Buffer::pair(service, DEFAULT_BUFFER_SIZE);
    tokio::spawn(worker);

    Ok(Channel { svc })
}

pub struct Channel {
    svc: Buffer<Request<Body>, BoxFuture<'static, Result<Response<Body>, BoxError>>>,
}

impl Clone for Channel {
    fn clone(&self) -> Self {
        Self {
            svc: self.svc.clone(),
        }
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpiffeChannel").finish()
    }
}

pub struct ResponseFuture {
    inner: BufferResponseFuture<BoxFuture<'static, Result<Response<Body>, BoxError>>>,
}

impl fmt::Debug for ResponseFuture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseFuture").finish()
    }
}

impl Service<Request<Body>> for Channel {
    type Response = Response<Body>;
    type Error = BoxError;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::poll_ready(&mut self.svc, cx).map_err(Into::into)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        ResponseFuture {
            inner: Service::call(&mut self.svc, request),
        }
    }
}

impl Future for ResponseFuture {
    type Output = Result<Response<Body>, BoxError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx).map_err(Into::into)
    }
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct Error {
    source: Option<BoxError>,
}

impl Error {
    pub fn from_source(source: impl Into<BoxError>) -> Self {
        Self {
            source: Some(source.into()),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut tuple = f.debug_tuple("rustls_channel::Error");
        if let Some(source) = &self.source {
            tuple.field(source);
        }
        tuple.finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("transport error")
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|source| &**source as &(dyn StdError + 'static))
    }
}

struct SendRequest {
    inner: hyper::client::conn::http2::SendRequest<Body>,
}

impl Service<Request<Body>> for SendRequest {
    type Response = Response<Body>;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let future = self.inner.send_request(req);
        Box::pin(async move {
            future
                .await
                .map_err(Into::into)
                .map(|response| response.map(Body::new))
        })
    }
}

#[derive(Debug)]
struct AddOrigin<T> {
    inner: T,
    scheme: Option<http::uri::Scheme>,
    authority: Option<http::uri::Authority>,
}

impl<T> AddOrigin<T> {
    fn new(inner: T, origin: Uri) -> Self {
        let http::uri::Parts {
            scheme, authority, ..
        } = origin.into_parts();

        Self {
            inner,
            scheme,
            authority,
        }
    }
}

impl<T, ReqBody> Service<Request<ReqBody>> for AddOrigin<T>
where
    T: Service<Request<ReqBody>>,
    T::Future: Send + 'static,
    T::Error: Into<BoxError>,
{
    type Response = T::Response;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        if self.scheme.is_none() || self.authority.is_none() {
            return Box::pin(async move {
                Err::<Self::Response, _>(
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid uri").into(),
                )
            });
        }

        let (mut head, body) = req.into_parts();
        head.uri = {
            let mut uri: http::uri::Parts = head.uri.into();
            uri.scheme = self.scheme.clone();
            uri.authority = self.authority.clone();
            Uri::from_parts(uri).expect("valid uri")
        };

        let request = Request::from_parts(head, body);
        let future = self.inner.call(request);
        Box::pin(async move { future.await.map_err(Into::into) })
    }
}

#[derive(Debug)]
struct UserAgent<T> {
    inner: T,
    user_agent: HeaderValue,
}

impl<T> UserAgent<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            user_agent: HeaderValue::from_static(TONIC_USER_AGENT),
        }
    }
}

impl<T, ReqBody> Service<Request<ReqBody>> for UserAgent<T>
where
    T: Service<Request<ReqBody>>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        req.headers_mut()
            .insert(USER_AGENT, self.user_agent.clone());
        self.inner.call(req)
    }
}

struct TlsConnector {
    config: Arc<tokio_rustls::rustls::ClientConfig>,
    server_name: ServerName<'static>,
}

impl TlsConnector {
    fn new(
        mut config: tokio_rustls::rustls::ClientConfig,
        server_name: ServerName<'static>,
    ) -> Self {
        if !config
            .alpn_protocols
            .iter()
            .any(|protocol| protocol == b"h2")
        {
            config.alpn_protocols.push(b"h2".to_vec());
        }

        Self {
            config: Arc::new(config),
            server_name,
        }
    }

    async fn connect<I>(
        &self,
        io: I,
    ) -> Result<TokioIo<tokio_rustls::client::TlsStream<I>>, BoxError>
    where
        I: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let io = RustlsConnector::from(self.config.clone())
            .connect(self.server_name.clone(), io)
            .await?;

        let (_, session) = io.get_ref();
        if session.alpn_protocol() != Some(b"h2") {
            return Err("HTTP/2 was not negotiated".into());
        }

        Ok(TokioIo::new(io))
    }
}
