use std::env;
use std::net::IpAddr;

use thiserror::Error;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 50051;

#[derive(Clone, Debug, Default)]
pub struct RuntimeOverrides {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub api_key: Option<String>,
    pub anki_version: Option<String>,
    pub auth_disabled: Option<bool>,
    pub allow_non_local: Option<bool>,
    pub allow_loopback_unauthenticated_health_check: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct ProfileConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub api_key: Option<String>,
    pub anki_version: Option<String>,
    pub auth_disabled: Option<bool>,
    pub allow_non_local: Option<bool>,
    pub allow_loopback_unauthenticated_health_check: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub anki_version: Option<String>,
    pub auth_disabled: bool,
    pub allow_non_local: bool,
    pub allow_loopback_unauthenticated_health_check: bool,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid ANKI_PUBLIC_API_PORT value: {0}")]
    InvalidPort(String),
    #[error("invalid {key} value: {value}")]
    InvalidBoolean { key: &'static str, value: String },
    #[error("{key} cannot be empty")]
    EmptyValue { key: &'static str },
    #[error("api_key is required when auth is enabled")]
    MissingApiKey,
    #[error("non-local bind requires allow_non_local=true")]
    NonLocalBindNotAllowed,
}

impl ServerConfig {
    pub fn resolve(runtime: RuntimeOverrides, profile: ProfileConfig) -> Result<Self, ConfigError> {
        let host = pick_value(
            runtime.host,
            env::var("ANKI_PUBLIC_API_HOST").ok(),
            profile.host,
            DEFAULT_HOST.to_owned(),
        );
        let port = pick_value(runtime.port, env_port()?, profile.port, DEFAULT_PORT);
        let api_key = pick_value(
            runtime.api_key,
            env_api_key()?,
            profile.api_key,
            String::new(),
        );
        let anki_version = pick_value(
            runtime.anki_version,
            env::var("ANKI_PUBLIC_API_ANKI_VERSION").ok(),
            profile.anki_version,
            String::new(),
        );
        let auth_disabled = pick_value(
            runtime.auth_disabled,
            env_bool("ANKI_PUBLIC_API_AUTH_DISABLED")?,
            profile.auth_disabled,
            false,
        );
        let allow_non_local = pick_value(
            runtime.allow_non_local,
            env_bool("ANKI_PUBLIC_API_ALLOW_NON_LOCAL")?,
            profile.allow_non_local,
            false,
        );
        let allow_loopback_unauthenticated_health_check = pick_value(
            runtime.allow_loopback_unauthenticated_health_check,
            env_bool("ANKI_PUBLIC_API_ALLOW_LOOPBACK_HEALTH_WITHOUT_AUTH")?,
            profile.allow_loopback_unauthenticated_health_check,
            false,
        );

        let config = Self {
            host,
            port,
            api_key: if api_key.is_empty() {
                None
            } else {
                Some(api_key)
            },
            anki_version: if anki_version.is_empty() {
                None
            } else {
                Some(anki_version)
            },
            auth_disabled,
            allow_non_local,
            allow_loopback_unauthenticated_health_check,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if !self.auth_disabled && self.api_key.is_none() {
            return Err(ConfigError::MissingApiKey);
        }
        if !self.allow_non_local && !is_local_host(&self.host) {
            return Err(ConfigError::NonLocalBindNotAllowed);
        }
        Ok(())
    }
}

fn env_port() -> Result<Option<u16>, ConfigError> {
    match env::var("ANKI_PUBLIC_API_PORT") {
        Ok(value) => value
            .parse::<u16>()
            .map(Some)
            .map_err(|_| ConfigError::InvalidPort(value)),
        Err(_) => Ok(None),
    }
}

fn env_bool(key: &'static str) -> Result<Option<bool>, ConfigError> {
    match env::var(key) {
        Ok(value) => match value.as_str() {
            "1" | "true" | "TRUE" | "True" => Ok(Some(true)),
            "0" | "false" | "FALSE" | "False" => Ok(Some(false)),
            _ => Err(ConfigError::InvalidBoolean { key, value }),
        },
        Err(_) => Ok(None),
    }
}

fn env_api_key() -> Result<Option<String>, ConfigError> {
    match env::var("ANKI_PUBLIC_API_KEY") {
        Ok(value) if value.is_empty() => Err(ConfigError::EmptyValue {
            key: "ANKI_PUBLIC_API_KEY",
        }),
        Ok(value) => Ok(Some(value)),
        Err(_) => Ok(None),
    }
}

fn pick_value<T>(runtime: Option<T>, env: Option<T>, profile: Option<T>, default: T) -> T {
    runtime.or(env).or(profile).unwrap_or(default)
}

fn is_local_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}
