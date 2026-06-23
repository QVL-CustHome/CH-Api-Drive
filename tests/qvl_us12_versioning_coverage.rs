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
        .connect_lazy("postgres://drive:drive@127.0.0.1:5432/drive_us12_coverage")
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

async fn status(method: &str, path: &str) -> StatusCode {
    router(test_state())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

const PUBLIC_ROUTES: &[&str] = &[
    "/me/storage",
    "/files",
    "/gallery",
    "/search",
    "/duplicates",
    "/folders",
    "/trash",
    "/admin/users",
];

#[tokio::test]
async fn toutes_les_routes_publiques_sont_montees_sous_v1() {
    for route in PUBLIC_ROUTES {
        let versioned = status("GET", &format!("{API_VERSION_PREFIX}{route}")).await;
        assert_ne!(
            versioned,
            StatusCode::NOT_FOUND,
            "la route publique {route} doit exister sous /v1"
        );
    }
}

#[tokio::test]
async fn chaque_route_publique_se_comporte_pareil_en_v1_et_en_legacy() {
    for route in PUBLIC_ROUTES {
        let versioned = status("GET", &format!("{API_VERSION_PREFIX}{route}")).await;
        let legacy = status("GET", route).await;
        assert_eq!(
            versioned, legacy,
            "la transition double-exposition doit donner le meme statut pour {route}"
        );
    }
}

#[tokio::test]
async fn route_inexistante_sous_v1_renvoie_404() {
    let absente = status("GET", &format!("{API_VERSION_PREFIX}/route-qui-nexiste-pas")).await;
    assert_eq!(absente, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn prefixe_v1_seul_nest_pas_un_catch_all() {
    let racine_v1 = status("GET", API_VERSION_PREFIX).await;
    assert_eq!(racine_v1, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn verbe_non_supporte_sur_route_versionnee_renvoie_405_pas_404() {
    let methode_invalide = status("DELETE", &format!("{API_VERSION_PREFIX}/me/storage")).await;
    assert_eq!(methode_invalide, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn sante_versionnee_absente_mais_legacy_presente() {
    assert_eq!(status("GET", "/health").await, StatusCode::OK);
    assert_eq!(
        status("GET", &format!("{API_VERSION_PREFIX}/health")).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn aucune_route_interne_exposee_ni_en_v1_ni_en_legacy() {
    assert_eq!(
        status("POST", &format!("{API_VERSION_PREFIX}/internal/users/resolve")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status("POST", "/internal/users/resolve").await,
        StatusCode::NOT_FOUND
    );
}
