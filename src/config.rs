use figment::Figment;
use figment::providers::{Env, Format, Toml};
use serde::Deserialize;

pub const MIN_JWT_SECRET_BYTES: usize = 32;
pub const MIN_INTERNAL_API_SECRET_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("fichier de configuration invalide : {0}")]
    File(Box<figment::Error>),
    #[error("variable d'environnement requise manquante ou vide : {0}")]
    MissingSecret(&'static str),
    #[error("JWT_SECRET trop court : {0} octets (minimum {MIN_JWT_SECRET_BYTES})")]
    WeakJwtSecret(usize),
    #[error(
        "INTERNAL_API_SECRET trop court : {0} octets (minimum {MIN_INTERNAL_API_SECRET_BYTES})"
    )]
    WeakInternalApiSecret(usize),
}

impl From<figment::Error> for ConfigError {
    fn from(e: figment::Error) -> Self {
        ConfigError::File(Box::new(e))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub token: TokenConfig,
    #[serde(default = "default_auth_internal_url")]
    pub auth_internal_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub root: String,
    pub default_quota_bytes: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenConfig {
    #[serde(default = "default_cookie_name")]
    pub cookie_name: String,
}

#[derive(Clone)]
pub struct Secrets {
    pub database_url: String,
    pub jwt_secret: String,
    pub internal_api_secret: String,
}

pub struct Settings {
    pub config: Config,
    pub secrets: Secrets,
}

pub fn load(path: &str) -> Result<Settings, ConfigError> {
    let mut config: Config = Figment::new()
        .merge(Toml::file(path))
        .merge(Env::prefixed("CH__").split("__"))
        .extract()?;

    if let Some(port) = optional("PORT").and_then(|p| p.parse::<u16>().ok()) {
        config.server.port = port;
    }

    if let Some(url) = optional("AUTH_INTERNAL_URL") {
        config.auth_internal_url = url;
    }

    let secrets = Secrets {
        database_url: require("DATABASE_URL")?,
        jwt_secret: require("JWT_SECRET")?,
        internal_api_secret: require("INTERNAL_API_SECRET")?,
    };
    validate_secrets(&secrets)?;

    Ok(Settings { config, secrets })
}

fn validate_secrets(secrets: &Secrets) -> Result<(), ConfigError> {
    let jwt_len = secrets.jwt_secret.len();
    if jwt_len < MIN_JWT_SECRET_BYTES {
        return Err(ConfigError::WeakJwtSecret(jwt_len));
    }
    let internal_len = secrets.internal_api_secret.len();
    if internal_len < MIN_INTERNAL_API_SECRET_BYTES {
        return Err(ConfigError::WeakInternalApiSecret(internal_len));
    }
    Ok(())
}

fn require(name: &'static str) -> Result<String, ConfigError> {
    optional(name).ok_or(ConfigError::MissingSecret(name))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn default_log_level() -> String {
    "INFO".to_string()
}

fn default_cookie_name() -> String {
    "ch_token".to_string()
}

fn default_auth_internal_url() -> String {
    "http://localhost:8181".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(jwt_secret: &str, internal_api_secret: &str) -> Secrets {
        Secrets {
            database_url: "postgres://localhost/test".to_string(),
            jwt_secret: jwt_secret.to_string(),
            internal_api_secret: internal_api_secret.to_string(),
        }
    }

    const STRONG: &str = "un-secret-suffisamment-long-pour-32-octets!!";

    #[test]
    fn secrets_valides_acceptes() {
        assert!(validate_secrets(&secrets(STRONG, STRONG)).is_ok());
    }

    #[test]
    fn jwt_secret_trop_court_rejete() {
        let err = validate_secrets(&secrets("trop-court", STRONG)).unwrap_err();
        assert!(matches!(err, ConfigError::WeakJwtSecret(10)));
    }

    #[test]
    fn internal_api_secret_trop_court_rejete() {
        let err = validate_secrets(&secrets(STRONG, "trop-court")).unwrap_err();
        assert!(matches!(err, ConfigError::WeakInternalApiSecret(10)));
    }

    #[test]
    fn internal_api_secret_exactement_32_octets_accepte() {
        let exact = "a".repeat(MIN_INTERNAL_API_SECRET_BYTES);
        assert!(validate_secrets(&secrets(STRONG, &exact)).is_ok());
    }
}
