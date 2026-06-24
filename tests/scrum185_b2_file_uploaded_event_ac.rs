mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ch_api_drive::services::jwt::JwtService;
use ch_api_drive::services::storage::FsStorage;
use ch_api_drive::state::AppState;
use common::{DisposableDb, RecordingEventPublisher, seed_drive_user};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::{Pool, Postgres};
use tower::ServiceExt;
use uuid::Uuid;

const ISSUER: &str = "ch-api-authenticator";
const AUDIENCE: &str = "ch-api-drive";
const JWT_SECRET: &str = "un-secret-suffisamment-long-pour-32-octets!!";
const MULTIPART_BOUNDARY: &str = "scrum185boundary";

macro_rules! require_db {
    () => {
        match DisposableDb::create().await {
            Some(db) => db,
            None => {
                eprintln!(
                    "SCRUM-185 ignoré : variable {} absente (Postgres jetable requis)",
                    common::ENV_ADMIN_URL
                );
                return;
            }
        }
    };
}

fn jwt_service() -> Arc<JwtService> {
    Arc::new(JwtService::from_secret(JWT_SECRET, ISSUER, AUDIENCE))
}

fn temp_storage_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("scrum185-storage-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_state(
    pool: Pool<Postgres>,
    jwt: Arc<JwtService>,
    publisher: Arc<RecordingEventPublisher>,
) -> AppState {
    AppState {
        db: pool,
        jwt,
        cookie_name: "ch_token".to_string(),
        default_quota_bytes: 1_000_000_000,
        storage: FsStorage::new(temp_storage_root()),
        event_publisher: publisher,
        auth_internal_url: "http://localhost:8181".to_string(),
        internal_secret: "internal-api-secret-value-32-octets!!".to_string(),
        http_client: reqwest::Client::new(),
    }
}

fn bearer_for(jwt: &JwtService, owner: &str) -> String {
    let token = jwt
        .issue(
            owner.to_string(),
            Vec::new(),
            None,
            Duration::from_secs(300),
        )
        .unwrap();
    format!("Bearer {token}")
}

fn multipart_file_body(file_name: &str, content: &[u8]) -> Body {
    let mut bytes = Vec::new();
    let head = format!(
        "--{MULTIPART_BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    );
    bytes.extend_from_slice(head.as_bytes());
    bytes.extend_from_slice(content);
    let tail = format!("\r\n--{MULTIPART_BOUNDARY}--\r\n");
    bytes.extend_from_slice(tail.as_bytes());
    Body::from(bytes)
}

fn upload_request(bearer: &str, file_name: &str, content: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/files")
        .header(header::AUTHORIZATION, bearer)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}"),
        )
        .body(multipart_file_body(file_name, content))
        .unwrap()
}

async fn node_exists(pool: &Pool<Postgres>, owner: &str, node_id: Uuid) -> bool {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM nodes WHERE id = $1 AND owner_id = $2")
            .bind(node_id)
            .bind(owner)
            .fetch_one(pool)
            .await
            .unwrap();
    count == 1
}

#[tokio::test]
async fn ac1_upload_reussi_publie_un_unique_event_avec_owner_et_node_du_node_cree() {
    let db = require_db!();
    let owner = "0123456789abcdef00000101";
    seed_drive_user(&db.pool, owner, 1_000_000).await;

    let jwt = jwt_service();
    let publisher = Arc::new(RecordingEventPublisher::succeeding());
    let state = build_state(db.pool.clone(), jwt.clone(), publisher.clone());
    let app = ch_api_drive::routes::router(state);

    let content = b"contenu-fichier-ac1";
    let response = app
        .oneshot(upload_request(
            &bearer_for(&jwt, owner),
            "rapport.bin",
            content,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let dto: Value = serde_json::from_slice(&body).unwrap();
    let created_node_id = dto["id"].as_str().unwrap().parse::<Uuid>().unwrap();
    let resolved_parent_id = dto["parent_id"].as_str().unwrap().parse::<Uuid>().unwrap();

    assert_eq!(publisher.call_count(), 1);
    let event = publisher.captured().into_iter().next().unwrap();
    assert_eq!(event.owner_id, owner);
    assert_eq!(event.node_id, created_node_id);
    assert_eq!(event.parent_id, resolved_parent_id);
    assert_eq!(event.size_bytes, content.len() as i64);

    db.destroy().await;
}

#[tokio::test]
async fn ac2_owner_id_de_l_event_est_le_proprietaire_authentifie_reel() {
    let db = require_db!();
    let owner_a = "0123456789abcdef00000201";
    let owner_b = "0123456789abcdef00000202";
    seed_drive_user(&db.pool, owner_a, 1_000_000).await;
    seed_drive_user(&db.pool, owner_b, 1_000_000).await;

    let jwt = jwt_service();
    let publisher = Arc::new(RecordingEventPublisher::succeeding());
    let state = build_state(db.pool.clone(), jwt.clone(), publisher.clone());
    let app = ch_api_drive::routes::router(state);

    let response = app
        .oneshot(upload_request(
            &bearer_for(&jwt, owner_a),
            "a.bin",
            b"depuis-A",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let event = publisher.captured().into_iter().next().unwrap();
    assert_eq!(event.owner_id, owner_a);
    assert_ne!(event.owner_id, owner_b);

    assert!(node_exists(&db.pool, owner_a, event.node_id).await);
    assert!(!node_exists(&db.pool, owner_b, event.node_id).await);

    db.destroy().await;
}

#[tokio::test]
async fn ac3_echec_de_publication_n_altere_pas_la_reponse_201_et_l_upload_reste_persiste() {
    let db = require_db!();
    let owner = "0123456789abcdef00000301";
    seed_drive_user(&db.pool, owner, 1_000_000).await;

    let jwt = jwt_service();
    let publisher = Arc::new(RecordingEventPublisher::failing());
    let state = build_state(db.pool.clone(), jwt.clone(), publisher.clone());
    let app = ch_api_drive::routes::router(state);

    let content = b"contenu-malgre-bus-down";
    let response = app
        .oneshot(upload_request(
            &bearer_for(&jwt, owner),
            "resilient.bin",
            content,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let dto: Value = serde_json::from_slice(&body).unwrap();
    let created_node_id = dto["id"].as_str().unwrap().parse::<Uuid>().unwrap();

    assert_eq!(publisher.call_count(), 1);
    assert!(node_exists(&db.pool, owner, created_node_id).await);

    let stored_size: i64 = sqlx::query_scalar("SELECT size_bytes FROM nodes WHERE id = $1")
        .bind(created_node_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stored_size, content.len() as i64);

    db.destroy().await;
}
