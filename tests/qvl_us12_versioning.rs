use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ch_api_drive::domain::events::{EventPublisher, FileUploadedEvent, PublishError};
use ch_api_drive::routes::{API_VERSION_PREFIX, router};
use ch_api_drive::services::storage::FsStorage;
use ch_api_drive::state::AppState;
use ch_api_drive::services::jwt::JwtService;
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

struct NoopEventPublisher;

#[async_trait]
impl EventPublisher for NoopEventPublisher {
    async fn publish_file_uploaded(&self, _event: &FileUploadedEvent) -> Result<(), PublishError> {
        Ok(())
    }
}

fn test_state() -> AppState {
    let db = PgPoolOptions::new()
        .connect_lazy("postgres://drive:drive@127.0.0.1:5432/drive_us12_routing")
        .expect("pool paresseux");
    AppState {
        db,
        jwt: Arc::new(JwtService::from_secret(
            "secret-de-test-suffisamment-long-pour-hs256",
            "ch-api-authenticator",
            "ch-api-drive",
        )),
        cookie_name: "drive_token".to_string(),
        default_quota_bytes: 0,
        storage: FsStorage::new(PathBuf::from(std::env::temp_dir())),
        event_publisher: Arc::new(NoopEventPublisher),
        auth_internal_url: "http://127.0.0.1:9".to_string(),
        internal_secret: "secret-interne-de-test".to_string(),
        http_client: reqwest::Client::new(),
    }
}

async fn status_get(path: &str) -> StatusCode {
    let response = router(test_state())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response.status()
}

#[tokio::test]
async fn route_publique_exposee_sous_v1() {
    let status = status_get(&format!("{API_VERSION_PREFIX}/me/storage")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn route_publique_legacy_sans_prefixe_toujours_disponible() {
    let status = status_get("/me/storage").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn route_operationnelle_non_versionnee() {
    let health = status_get("/health").await;
    assert_eq!(health, StatusCode::OK);

    let health_versionnee = status_get(&format!("{API_VERSION_PREFIX}/health")).await;
    assert_eq!(health_versionnee, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn route_interne_non_disponible_sous_v1() {
    let versionnee = status_get(&format!("{API_VERSION_PREFIX}/internal/users/resolve")).await;
    assert_eq!(versionnee, StatusCode::NOT_FOUND);

    let legacy = status_get("/internal/users/resolve").await;
    assert_eq!(legacy, StatusCode::NOT_FOUND);
}
