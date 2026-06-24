mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ch_api_drive::config::{Config, Secrets, ServerConfig, Settings, StorageConfig, TokenConfig};
use ch_api_drive::db::Db;
use ch_api_drive::routes;
use ch_api_drive::services::jwt::JwtService;
use ch_api_drive::services::storage::FsStorage;
use ch_api_drive::state::AppState;
use common::{DisposableDb, NoopEventPublisher};
use http_body_util::BodyExt;
use sqlx::Pool;
use sqlx::Postgres;
use tower::ServiceExt;
use uuid::Uuid;

const JWT_SECRET: &str = "secret-de-test-suffisamment-long-pour-drive-d3";
const ISSUER: &str = "ch-api-authenticator";
const AUDIENCE: &str = "ch-api-drive";

macro_rules! require_db {
    () => {
        match DisposableDb::create().await {
            Some(db) => db,
            None => {
                eprintln!(
                    "D3 ignoré : variable {} absente (Postgres jetable requis)",
                    common::ENV_ADMIN_URL
                );
                return;
            }
        }
    };
}

struct TestApp {
    state: AppState,
    storage_root: PathBuf,
}

fn temp_storage_root() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("d3_store_{}_{}", std::process::id(), Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn build_app(pool: Pool<Postgres>) -> TestApp {
    let storage_root = temp_storage_root();
    let settings = Settings {
        config: Config {
            server: ServerConfig {
                port: 0,
                log_level: "INFO".to_string(),
            },
            storage: StorageConfig {
                root: storage_root.to_string_lossy().to_string(),
                default_quota_bytes: 16_106_127_360,
            },
            token: TokenConfig {
                cookie_name: "ch_token".to_string(),
                issuer: ISSUER.to_string(),
                audience: AUDIENCE.to_string(),
            },
            upload_gc: Default::default(),
            auth_internal_url: "http://localhost:8181".to_string(),
        },
        secrets: Secrets {
            database_url: String::new(),
            jwt_secret: JWT_SECRET.to_string(),
            internal_api_secret: "interne-de-test-suffisamment-long-pour-d3".to_string(),
        },
    };
    let state = AppState::new(&settings, pool as Db, Arc::new(NoopEventPublisher));
    TestApp {
        state,
        storage_root,
    }
}

fn token_for(owner: &str) -> String {
    let jwt = JwtService::from_secret(JWT_SECRET, ISSUER, AUDIENCE);
    jwt.issue(owner, vec![], None, Duration::from_secs(3600))
        .expect("génération du jeton de test")
}

async fn seed_user(pool: &Pool<Postgres>, owner: &str) -> Uuid {
    common::seed_drive_user(pool, owner, 16_106_127_360).await
}

async fn write_blob(app: &TestApp, owner: &str, node_id: Uuid, bytes: &[u8]) -> String {
    let storage = FsStorage::new(app.storage_root.clone());
    let key = FsStorage::build_key(owner, node_id).expect("clé de stockage");
    storage.write_bytes(&key, bytes).await.expect("écriture blob");
    key
}

async fn insert_file_node(
    pool: &Pool<Postgres>,
    node_id: Uuid,
    owner: &str,
    parent: Uuid,
    name: &str,
    mime: &str,
    storage_key: &str,
    size: i64,
    trashed: bool,
) {
    let trashed_at = if trashed { Some(chrono::Utc::now()) } else { None };
    sqlx::query(
        "INSERT INTO nodes (id, owner_id, parent_id, kind, name, mime, size_bytes, storage_key, trashed_at) \
         VALUES ($1, $2, $3, 'file', $4, $5, $6, $7, $8)",
    )
    .bind(node_id)
    .bind(owner)
    .bind(parent)
    .bind(name)
    .bind(mime)
    .bind(size)
    .bind(storage_key)
    .bind(trashed_at)
    .execute(pool)
    .await
    .expect("insertion nœud fichier");
}

async fn download_request(app: &TestApp, owner: &str, id: Uuid, range: Option<&str>) -> axum::response::Response {
    let router = routes::router(app.state.clone());
    let mut builder = Request::builder()
        .uri(format!("/files/{id}/content"))
        .header(header::AUTHORIZATION, format!("Bearer {}", token_for(owner)));
    if let Some(spec) = range {
        builder = builder.header(header::RANGE, spec);
    }
    let request = builder.body(Body::empty()).unwrap();
    router.oneshot(request).await.unwrap()
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    response.into_body().collect().await.unwrap().to_bytes().to_vec()
}

fn cleanup(app: TestApp) {
    let _ = std::fs::remove_dir_all(&app.storage_root);
}

mod ac1_corbeille_non_telechargeable {
    use super::*;

    #[tokio::test]
    async fn fichier_en_corbeille_renvoie_404() {
        let db = require_db!();
        let owner = "0123456789abcdef0000a001";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let key = write_blob(&app, owner, node_id, b"contenu corbeille").await;
        insert_file_node(&db.pool, node_id, owner, root, "secret.txt", "text/plain", &key, 17, true).await;

        let resp = download_request(&app, owner, node_id, None).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn fichier_actif_reste_telechargeable_en_200() {
        let db = require_db!();
        let owner = "0123456789abcdef0000a002";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let payload = b"contenu actif lisible";
        let key = write_blob(&app, owner, node_id, payload).await;
        insert_file_node(&db.pool, node_id, owner, root, "actif.txt", "text/plain", &key, payload.len() as i64, false).await;

        let resp = download_request(&app, owner, node_id, None).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, payload);

        cleanup(app);
        db.destroy().await;
    }
}

mod ac2_range_rfc7233 {
    use super::*;

    async fn fixture(db: &DisposableDb, owner: &str) -> (TestApp, Uuid, Vec<u8>) {
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());
        let node_id = Uuid::new_v4();
        let payload: Vec<u8> = (0..1000u32).map(|i| (i % 256) as u8).collect();
        let key = write_blob(&app, owner, node_id, &payload).await;
        insert_file_node(&db.pool, node_id, owner, root, "data.bin", "application/octet-stream", &key, payload.len() as i64, false).await;
        (app, node_id, payload)
    }

    #[tokio::test]
    async fn range_satisfiable_renvoie_206_avec_contenu_partiel() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b001";
        let (app, node_id, payload) = fixture(&db, owner).await;

        let resp = download_request(&app, owner, node_id, Some("bytes=0-499")).await;

        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            resp.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 0-499/1000"
        );
        assert_eq!(resp.headers().get(header::CONTENT_LENGTH).unwrap(), "500");
        assert_eq!(resp.headers().get(header::ACCEPT_RANGES).unwrap(), "bytes");
        assert_eq!(body_bytes(resp).await, payload[0..500].to_vec());

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn range_suffixe_renvoie_206_avec_queue() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b002";
        let (app, node_id, payload) = fixture(&db, owner).await;

        let resp = download_request(&app, owner, node_id, Some("bytes=-100")).await;

        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            resp.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 900-999/1000"
        );
        assert_eq!(body_bytes(resp).await, payload[900..1000].to_vec());

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn range_start_au_dela_de_la_taille_renvoie_416() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b003";
        let (app, node_id, _payload) = fixture(&db, owner).await;

        let resp = download_request(&app, owner, node_id, Some("bytes=2000-3000")).await;

        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            resp.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes */1000"
        );
        assert_eq!(resp.headers().get(header::ACCEPT_RANGES).unwrap(), "bytes");
        assert!(body_bytes(resp).await.is_empty());

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn suffixe_zero_renvoie_416() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b004";
        let (app, node_id, _payload) = fixture(&db, owner).await;

        let resp = download_request(&app, owner, node_id, Some("bytes=-0")).await;

        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            resp.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes */1000"
        );

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn fichier_vide_avec_range_renvoie_416() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b005";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());
        let node_id = Uuid::new_v4();
        let key = write_blob(&app, owner, node_id, b"").await;
        insert_file_node(&db.pool, node_id, owner, root, "vide.bin", "application/octet-stream", &key, 0, false).await;

        let resp = download_request(&app, owner, node_id, Some("bytes=0-10")).await;

        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            resp.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes */0"
        );

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn range_malforme_est_ignore_et_renvoie_200_complet() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b006";
        let (app, node_id, payload) = fixture(&db, owner).await;

        let resp = download_request(&app, owner, node_id, Some("octets=0-499")).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_LENGTH).unwrap(), "1000");
        assert!(resp.headers().get(header::CONTENT_RANGE).is_none());
        assert_eq!(body_bytes(resp).await, payload);

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn range_sans_bornes_numeriques_est_ignore_et_renvoie_200() {
        let db = require_db!();
        let owner = "0123456789abcdef0000b007";
        let (app, node_id, payload) = fixture(&db, owner).await;

        let resp = download_request(&app, owner, node_id, Some("bytes=abc-def")).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, payload);

        cleanup(app);
        db.destroy().await;
    }
}

mod ac4_isolation_tenant {
    use super::*;

    #[tokio::test]
    async fn un_user_ne_telecharge_pas_le_fichier_d_un_autre() {
        let db = require_db!();
        let owner = "0123456789abcdef0000c001";
        let intrus = "0123456789abcdef0000c002";
        let root_owner = seed_user(&db.pool, owner).await;
        seed_user(&db.pool, intrus).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let key = write_blob(&app, owner, node_id, b"prive").await;
        insert_file_node(&db.pool, node_id, owner, root_owner, "prive.txt", "text/plain", &key, 5, false).await;

        let resp = download_request(&app, intrus, node_id, None).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        cleanup(app);
        db.destroy().await;
    }
}

mod ac5_entetes_securite {
    use super::*;

    #[tokio::test]
    async fn type_actif_est_neutralise_en_octet_stream() {
        let db = require_db!();
        let owner = "0123456789abcdef0000d001";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let key = write_blob(&app, owner, node_id, b"<svg/>").await;
        insert_file_node(&db.pool, node_id, owner, root, "evil.svg", "image/svg+xml", &key, 6, false).await;

        let resp = download_request(&app, owner, node_id, None).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn type_inerte_est_conserve() {
        let db = require_db!();
        let owner = "0123456789abcdef0000d002";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let key = write_blob(&app, owner, node_id, b"data").await;
        insert_file_node(&db.pool, node_id, owner, root, "photo.png", "image/png", &key, 4, false).await;

        let resp = download_request(&app, owner, node_id, None).await;

        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "image/png");

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn entetes_nosniff_disposition_et_metadonnees_sont_presents() {
        let db = require_db!();
        let owner = "0123456789abcdef0000d003";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let payload = b"abcdefghij";
        let key = write_blob(&app, owner, node_id, payload).await;
        insert_file_node(&db.pool, node_id, owner, root, "fichier.txt", "text/plain", &key, payload.len() as i64, false).await;

        let resp = download_request(&app, owner, node_id, None).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get(header::ACCEPT_RANGES).unwrap(), "bytes");
        assert_eq!(resp.headers().get(header::CONTENT_LENGTH).unwrap(), "10");
        let disposition = resp
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(disposition.contains("attachment"));
        assert!(disposition.contains("fichier.txt"));

        cleanup(app);
        db.destroy().await;
    }

    #[tokio::test]
    async fn nom_de_fichier_avec_guillemet_est_assaini() {
        let db = require_db!();
        let owner = "0123456789abcdef0000d004";
        let root = seed_user(&db.pool, owner).await;
        let app = build_app(db.pool.clone());

        let node_id = Uuid::new_v4();
        let key = write_blob(&app, owner, node_id, b"x").await;
        insert_file_node(&db.pool, node_id, owner, root, "a\"b.txt", "text/plain", &key, 1, false).await;

        let resp = download_request(&app, owner, node_id, None).await;

        let disposition = resp
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(!disposition.contains("a\"b.txt"));
        assert!(disposition.contains("a_b.txt"));

        cleanup(app);
        db.destroy().await;
    }
}
