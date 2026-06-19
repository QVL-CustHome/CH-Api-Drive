use crate::config::Settings;
use crate::db::Db;
use crate::services::storage::FsStorage;
use ch_auth_jwt::JwtCodec;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub jwt: Arc<JwtCodec>,
    pub cookie_name: String,
    pub default_quota_bytes: i64,
    pub storage: FsStorage,
    pub auth_internal_url: String,
    pub internal_secret: String,
    pub http_client: reqwest::Client,
}

impl AppState {
    pub fn new(settings: &Settings, db: Db) -> Self {
        Self {
            db,
            jwt: Arc::new(JwtCodec::from_secret(&settings.secrets.jwt_secret)),
            cookie_name: settings.config.token.cookie_name.clone(),
            default_quota_bytes: settings.config.storage.default_quota_bytes,
            storage: FsStorage::new(PathBuf::from(&settings.config.storage.root)),
            auth_internal_url: settings.config.auth_internal_url.clone(),
            internal_secret: settings.secrets.internal_api_secret.clone(),
            http_client: build_http_client(),
        }
    }
}

fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}
