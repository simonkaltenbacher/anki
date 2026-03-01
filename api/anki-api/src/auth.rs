use std::net::SocketAddr;

use thiserror::Error;
use tonic::Request;
use tonic::Status;
use tonic::metadata::MetadataMap;

use crate::config::ServerConfig;

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
    auth_disabled: bool,
    allow_loopback_unauthenticated_health_check: bool,
}

impl ApiKeyAuthenticator {
    pub fn new(config: &ServerConfig) -> Self {
        Self {
            api_key: config.api_key.clone(),
            auth_disabled: config.auth_disabled,
            allow_loopback_unauthenticated_health_check: config
                .allow_loopback_unauthenticated_health_check,
        }
    }

    pub fn authenticate(&self, request: &Request<()>, is_health_check: bool) -> Result<(), Status> {
        if self.auth_disabled {
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
                tracing::warn!(error = %err, "authentication failed");
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
