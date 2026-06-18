use crate::config::Settings;
use crate::db::Db;
use crate::services::jwt::JwtService;
use crate::services::storage::FsStorage;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub jwt: Arc<JwtService>,
    pub cookie_name: String,
    pub default_quota_bytes: i64,
    pub storage: FsStorage,
    pub auth_internal_url: String,
    pub internal_secret: String,
}

impl AppState {
    pub fn new(settings: &Settings, db: Db) -> Self {
        Self {
            db,
            jwt: Arc::new(JwtService::new(&settings.secrets.jwt_secret)),
            cookie_name: settings.config.token.cookie_name.clone(),
            default_quota_bytes: settings.config.storage.default_quota_bytes,
            storage: FsStorage::new(PathBuf::from(&settings.config.storage.root)),
            auth_internal_url: settings.config.auth_internal_url.clone(),
            internal_secret: settings.secrets.jwt_secret.clone(),
        }
    }
}
