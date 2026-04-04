use thiserror::Error;
use tonic::Request;
use tonic::Status;
use tonic::metadata::MetadataMap;

use crate::config::ServerConfig;
use crate::config::ServerConnectionMode;
use crate::config::TlsAuthMode;

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
}

impl ApiKeyAuthenticator {
    pub fn new(config: &ServerConfig) -> Self {
        Self {
            api_key: match &config.connection_mode {
                ServerConnectionMode::Tls {
                    auth: TlsAuthMode::ApiKey(api_key),
                    ..
                } => Some(api_key.clone()),
                _ => None,
            },
        }
    }

    pub fn authenticate(
        &self,
        request: &Request<()>,
        _is_health_check: bool,
    ) -> Result<(), Status> {
        if self.api_key.is_none() {
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

#[cfg(test)]
mod tests {
    use super::ApiKeyAuthenticator;
    use crate::config::ServerConfig;
    use crate::config::ServerConnectionMode;
    use crate::config::TlsAuthMode;
    use crate::config::TlsTransportConfig;
    use tonic::Request;

    fn plaintext_config() -> ServerConfig {
        ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 50051,
            anki_version: None,
            allow_non_local: false,
            connection_mode: ServerConnectionMode::Plaintext,
        }
    }

    fn tls_config() -> ServerConfig {
        ServerConfig {
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
        let config = ServerConfig {
            connection_mode: ServerConnectionMode::Tls {
                tls: TlsTransportConfig {
                    cert_path: "/tmp/server.pem".to_owned(),
                    key_path: "/tmp/server.key".to_owned(),
                },
                auth: TlsAuthMode::Disabled,
            },
            ..plaintext_config()
        };
        let auth = ApiKeyAuthenticator::new(&config);
        let request = Request::new(());
        auth.authenticate(&request, false)
            .expect("auth_disabled should bypass");
    }
}
