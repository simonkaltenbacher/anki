use std::net::SocketAddr;

use thiserror::Error;
use tonic::Request;
use tonic::Status;
use tonic::metadata::MetadataMap;

use crate::config::ServerConfig;
use crate::config::ServerTransportMode;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing authorization header")]
    MissingAuthorizationHeader,
    #[error("invalid authorization scheme")]
    InvalidAuthorizationScheme,
    #[error("invalid api key")]
    InvalidApiKey,
}

#[derive(Clone, Debug)]
pub struct ApiKeyAuthenticator {
    api_key: Option<String>,
    requires_api_key: bool,
    auth_disabled: bool,
    allow_loopback_unauthenticated_health_check: bool,
}

impl ApiKeyAuthenticator {
    pub fn new(config: &ServerConfig) -> Self {
        Self {
            api_key: config.api_key.clone(),
            requires_api_key: matches!(config.transport_mode, ServerTransportMode::Tls(_)),
            auth_disabled: config.auth_disabled,
            allow_loopback_unauthenticated_health_check: config
                .allow_loopback_unauthenticated_health_check,
        }
    }

    pub fn authenticate(&self, request: &Request<()>, is_health_check: bool) -> Result<(), Status> {
        if self.auth_disabled {
            return Ok(());
        }
        if !self.requires_api_key {
            return Ok(());
        }

        if is_health_check
            && self.allow_loopback_unauthenticated_health_check
            && is_loopback_peer(request.remote_addr())
        {
            return Ok(());
        }

        self.validate_authorization_header(request.metadata())
            .map_err(|err| {
                tracing::debug!(error = %err, "authentication failed");
                Status::unauthenticated(err.to_string())
            })
    }

    fn validate_authorization_header(&self, metadata: &MetadataMap) -> Result<(), AuthError> {
        let raw_header = metadata
            .get("authorization")
            .ok_or(AuthError::MissingAuthorizationHeader)?;
        let header = raw_header
            .to_str()
            .map_err(|_| AuthError::InvalidAuthorizationScheme)?;
        let provided_key = header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidAuthorizationScheme)?;

        match self.api_key.as_deref() {
            Some(expected_key) if expected_key == provided_key => Ok(()),
            _ => Err(AuthError::InvalidApiKey),
        }
    }
}

fn is_loopback_peer(peer: Option<SocketAddr>) -> bool {
    peer.map(|addr| addr.ip().is_loopback()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::ApiKeyAuthenticator;
    use crate::config::ServerConfig;
    use crate::config::ServerTransportMode;
    use crate::config::TlsTransportConfig;
    use tonic::Request;
    use tonic::transport::server::TcpConnectInfo;

    fn plaintext_config() -> ServerConfig {
        ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            api_key: None,
            anki_version: None,
            auth_disabled: false,
            allow_non_local: false,
            allow_loopback_unauthenticated_health_check: false,
            transport_mode: ServerTransportMode::Plaintext,
        }
    }

    fn tls_config() -> ServerConfig {
        ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            api_key: Some("test-key".to_owned()),
            anki_version: None,
            auth_disabled: false,
            allow_non_local: false,
            allow_loopback_unauthenticated_health_check: false,
            transport_mode: ServerTransportMode::Tls(TlsTransportConfig {
                cert_path: "/tmp/server.pem".to_owned(),
                key_path: "/tmp/server.key".to_owned(),
            }),
        }
    }

    #[test]
    fn plaintext_mode_does_not_require_api_key() {
        let auth = ApiKeyAuthenticator::new(&plaintext_config());
        let request = Request::new(());
        auth.authenticate(&request, false)
            .expect("plaintext should pass through");
    }

    #[test]
    fn tls_mode_rejects_missing_api_key() {
        let auth = ApiKeyAuthenticator::new(&tls_config());
        let request = Request::new(());
        let error = auth
            .authenticate(&request, false)
            .expect_err("tls should require auth header");
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn auth_disabled_bypasses_tls_api_key_check() {
        let mut config = tls_config();
        config.auth_disabled = true;
        let auth = ApiKeyAuthenticator::new(&config);
        let request = Request::new(());
        auth.authenticate(&request, false)
            .expect("auth_disabled should bypass");
    }

    #[test]
    fn loopback_health_check_bypasses_tls_api_key_check() {
        let mut config = tls_config();
        config.allow_loopback_unauthenticated_health_check = true;
        let auth = ApiKeyAuthenticator::new(&config);
        let mut request = Request::new(());
        request.extensions_mut().insert(TcpConnectInfo {
            local_addr: Some(std::net::SocketAddr::from(([127, 0, 0, 1], 50051))),
            remote_addr: Some(std::net::SocketAddr::from(([127, 0, 0, 1], 12345))),
        });
        auth.authenticate(&request, true)
            .expect("loopback health check should bypass");
    }
}
