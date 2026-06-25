mod common;

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use ch_api_drive::db::Db;
use ch_api_drive::routes::router;
use ch_api_drive::services::jwt::JwtService;
use ch_api_drive::services::storage::FsStorage;
use ch_api_drive::state::AppState;
use common::{seed_drive_user, DisposableDb, NoopEventPublisher};
use http_body_util::BodyExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const JWT_SECRET: &str = "secret-de-test-suffisamment-long-pour-32-octets!!";
const JWT_ISSUER: &str = "ch-api-authenticator";
const JWT_AUDIENCE: &str = "ch-api-drive";

macro_rules! require_db {
    () => {
        match DisposableDb::create().await {
            Some(db) => db,
            None => {
                eprintln!(
                    "SCRUM-188 C3 ignoré : variable {} absente (Postgres jetable requis)",
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

impl TestApp {
    fn build(pool: Db) -> Self {
        let mut storage_root = std::env::temp_dir();
        storage_root.push(format!("scrum188_{}_{}", std::process::id(), Uuid::new_v4()));
        std::fs::create_dir_all(&storage_root).unwrap();

        let jwt = Arc::new(JwtService::from_secret(JWT_SECRET, JWT_ISSUER, JWT_AUDIENCE));
        let state = AppState {
            db: pool,
            jwt,
            cookie_name: "ch_token".to_string(),
            default_quota_bytes: 50_000_000_000,
            storage: FsStorage::new(storage_root.clone()),
            event_publisher: Arc::new(NoopEventPublisher),
            auth_internal_url: "http://localhost:8181".to_string(),
            internal_secret: "x".repeat(32),
            http_client: reqwest::Client::new(),
        };
        Self {
            state,
            storage_root,
        }
    }

    fn token(&self, owner: &str) -> String {
        self.state
            .jwt
            .issue(owner, Vec::new(), None, Duration::from_secs(3600))
            .unwrap()
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = router(self.state.clone()).oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, json)
    }

    fn cleanup(self) {
        let _ = std::fs::remove_dir_all(&self.storage_root);
    }
}

fn json_request(method: &str, uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn bare_request(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn chunk_request(uri: &str, token: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(Bytes::from(bytes)))
        .unwrap()
}

async fn open_session(
    app: &TestApp,
    token: &str,
    declared_size: i64,
    chunk_size: i32,
    file_name: &str,
) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "file_name": file_name,
        "declared_size": declared_size,
        "chunk_size": chunk_size,
        "declared_mime": "application/octet-stream"
    });
    app.send(json_request("POST", "/uploads", token, body)).await
}

mod open_endpoint {
    use super::*;

    #[tokio::test]
    async fn open_cree_session_ouverte_avec_reservation_et_chunk_count() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188001";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (status, body) = open_session(&app, &token, 30_000, 10_000, "doc.bin").await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["state"], "open");
        assert_eq!(body["reserved_bytes"], 30_000);
        assert_eq!(body["declared_size"], 30_000);
        assert_eq!(body["chunk_count"], 3);
        assert_eq!(body["received_bytes"], 0);
        assert!(body["session_id"].is_string());
        assert!(body["expires_at"].is_string());

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_sans_authentification_renvoie_401() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/uploads")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({"file_name":"x.bin","declared_size":10,"chunk_size":10})
                    .to_string(),
            ))
            .unwrap();
        let (status, body) = app.send(req).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_chunk_size_zero_renvoie_400() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188002";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (status, body) = open_session(&app, &token, 100, 0, "bad.bin").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "bad_request");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_chunk_size_au_dessus_de_16_mio_renvoie_400() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188003";
        seed_drive_user(&db.pool, owner, 50_000_000_000).await;
        let token = app.token(owner);

        let (status, _) = open_session(&app, &token, 100, 16 * 1024 * 1024 + 1, "big.bin").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_declared_size_au_dessus_de_10_gio_renvoie_400() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188004";
        seed_drive_user(&db.pool, owner, i64::MAX / 2).await;
        let token = app.token(owner);

        let (status, _) =
            open_session(&app, &token, 10 * 1024 * 1024 * 1024 + 1, 1024, "huge.bin").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_nom_vide_renvoie_400() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188005";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (status, _) = open_session(&app, &token, 100, 50, "   ").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_parent_introuvable_renvoie_404() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188006";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let body = serde_json::json!({
            "parent_id": Uuid::new_v4(),
            "file_name": "x.bin",
            "declared_size": 100,
            "chunk_size": 50
        });
        let (status, resp) = app.send(json_request("POST", "/uploads", &token, body)).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(resp["error"], "not_found");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn open_au_dela_du_quota_renvoie_413_quota_exceeded() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188007";
        seed_drive_user(&db.pool, owner, 1_000).await;
        let token = app.token(owner);

        let (status, body) = open_session(&app, &token, 5_000, 1_000, "trop.bin").await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"], "quota_exceeded");

        app.cleanup();
        db.destroy().await;
    }
}

mod nominal_flow {
    use super::*;

    #[tokio::test]
    async fn open_put_chunks_complete_cree_le_node_fichier() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188100";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 25, 10, "complet.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();
        assert_eq!(opened["chunk_count"], 3);

        let payloads: [&[u8]; 3] = [b"AAAAAAAAAA", b"BBBBBBBBBB", b"CCCCC"];
        for (index, payload) in payloads.iter().enumerate() {
            let uri = format!("/uploads/{session_id}/chunks/{index}");
            let (status, body) = app.send(chunk_request(&uri, &token, payload.to_vec())).await;
            assert_eq!(status, StatusCode::OK, "put-chunk {index}");
            assert_eq!(body["chunk_index"], index as i64);
        }

        let (status, status_body) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(status_body["received_bytes"], 25);
        assert_eq!(status_body["state"], "open");

        let (status, node) = app
            .send(bare_request(
                "POST",
                &format!("/uploads/{session_id}/complete"),
                &token,
            ))
            .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(node["name"], "complet.bin");
        assert_eq!(node["size_bytes"], 25);
        assert_eq!(node["kind"], "file");

        let (status, after) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(after["state"], "completed");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn status_reflete_received_bytes_courants() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188101";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "suivi.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        app.send(chunk_request(
            &format!("/uploads/{session_id}/chunks/0"),
            &token,
            b"0123456789".to_vec(),
        ))
        .await;

        let (status, body) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["received_bytes"], 10);

        app.cleanup();
        db.destroy().await;
    }
}

mod put_chunk_errors {
    use super::*;

    #[tokio::test]
    async fn put_chunk_index_hors_bornes_renvoie_400() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188200";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "borne.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let (status, _) = app
            .send(chunk_request(
                &format!("/uploads/{session_id}/chunks/99"),
                &token,
                b"data".to_vec(),
            ))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_vide_renvoie_400() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188201";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "vide.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/uploads/{session_id}/chunks/0"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::empty())
            .unwrap();
        let (status, _) = app.send(req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_plus_gros_que_chunk_size_renvoie_413_payload_too_large() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188202";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 100, 10, "gros.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let (status, body) = app
            .send(chunk_request(
                &format!("/uploads/{session_id}/chunks/0"),
                &token,
                vec![b'X'; 11],
            ))
            .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"], "payload_too_large");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_corps_au_dessus_de_17_mio_est_rejete() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188203";
        seed_drive_user(&db.pool, owner, 50_000_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 16 * 1024 * 1024, 16 * 1024 * 1024, "max.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let oversized = vec![b'Z'; 17 * 1024 * 1024 + 1];
        let (status, _) = app
            .send(chunk_request(
                &format!("/uploads/{session_id}/chunks/0"),
                &token,
                oversized,
            ))
            .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_sur_session_inexistante_renvoie_404() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188204";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (status, _) = app
            .send(chunk_request(
                &format!("/uploads/{}/chunks/0", Uuid::new_v4()),
                &token,
                b"data".to_vec(),
            ))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_idempotent_ne_double_pas_received_bytes() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188205";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "idem.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();
        let uri = format!("/uploads/{session_id}/chunks/0");

        let (_, first) = app.send(chunk_request(&uri, &token, b"0123456789".to_vec())).await;
        assert_eq!(first["received_bytes"], 10);

        let (status, second) = app.send(chunk_request(&uri, &token, b"0123456789".to_vec())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second["received_bytes"], 10);

        let (_, body) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(body["received_bytes"], 10);

        app.cleanup();
        db.destroy().await;
    }
}

mod tenant_isolation {
    use super::*;

    #[tokio::test]
    async fn status_d_un_autre_tenant_renvoie_404() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner_a = "0123456789abcdef00188300";
        let owner_b = "0123456789abcdef00188301";
        seed_drive_user(&db.pool, owner_a, 1_000_000).await;
        seed_drive_user(&db.pool, owner_b, 1_000_000).await;

        let token_a = app.token(owner_a);
        let token_b = app.token(owner_b);

        let (_, opened) = open_session(&app, &token_a, 20, 10, "prive.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let (status, _) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token_b))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_sur_session_d_un_autre_tenant_renvoie_404() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner_a = "0123456789abcdef00188302";
        let owner_b = "0123456789abcdef00188303";
        seed_drive_user(&db.pool, owner_a, 1_000_000).await;
        seed_drive_user(&db.pool, owner_b, 1_000_000).await;

        let token_a = app.token(owner_a);
        let token_b = app.token(owner_b);

        let (_, opened) = open_session(&app, &token_a, 20, 10, "intrus.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let (status, _) = app
            .send(chunk_request(
                &format!("/uploads/{session_id}/chunks/0"),
                &token_b,
                b"intrusion!".to_vec(),
            ))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn abort_d_un_autre_tenant_renvoie_404() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner_a = "0123456789abcdef00188304";
        let owner_b = "0123456789abcdef00188305";
        seed_drive_user(&db.pool, owner_a, 1_000_000).await;
        seed_drive_user(&db.pool, owner_b, 1_000_000).await;

        let token_a = app.token(owner_a);
        let token_b = app.token(owner_b);

        let (_, opened) = open_session(&app, &token_a, 20, 10, "secret.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let (status, _) = app
            .send(bare_request("DELETE", &format!("/uploads/{session_id}"), &token_b))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        app.cleanup();
        db.destroy().await;
    }
}

mod complete_and_abort {
    use super::*;

    #[tokio::test]
    async fn complete_incomplet_renvoie_409() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188400";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 30, 10, "incomplet.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        app.send(chunk_request(
            &format!("/uploads/{session_id}/chunks/0"),
            &token,
            b"0123456789".to_vec(),
        ))
        .await;

        let (status, body) = app
            .send(bare_request(
                "POST",
                &format!("/uploads/{session_id}/complete"),
                &token,
            ))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn abort_passe_la_session_en_aborted_et_renvoie_204() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188401";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "annule.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        let (status, _) = app
            .send(bare_request("DELETE", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "aborted");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn put_chunk_sur_session_abandonnee_renvoie_409() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188402";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "ferme.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        app.send(bare_request("DELETE", &format!("/uploads/{session_id}"), &token))
            .await;

        let (status, _) = app
            .send(chunk_request(
                &format!("/uploads/{session_id}/chunks/0"),
                &token,
                b"trop tard!".to_vec(),
            ))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn abort_sur_session_completee_renvoie_409() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188403";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 10, 10, "fini.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        app.send(chunk_request(
            &format!("/uploads/{session_id}/chunks/0"),
            &token,
            b"0123456789".to_vec(),
        ))
        .await;
        app.send(bare_request(
            "POST",
            &format!("/uploads/{session_id}/complete"),
            &token,
        ))
        .await;

        let (status, body) = app
            .send(bare_request("DELETE", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");

        app.cleanup();
        db.destroy().await;
    }
}

mod concurrent_reservation {
    use super::*;

    async fn complete_session(app: &TestApp, token: &str, session_id: &str, size: usize) {
        app.send(chunk_request(
            &format!("/uploads/{session_id}/chunks/0"),
            token,
            vec![b'X'; size],
        ))
        .await;
        app.send(bare_request(
            "POST",
            &format!("/uploads/{session_id}/complete"),
            token,
        ))
        .await;
    }

    #[tokio::test]
    async fn deux_open_dont_le_cumul_depasse_le_quota_la_seconde_renvoie_413() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188600";
        seed_drive_user(&db.pool, owner, 1_500).await;
        let token = app.token(owner);

        let (first_status, first) = open_session(&app, &token, 1_000, 1_000, "premier.bin").await;
        assert_eq!(first_status, StatusCode::CREATED);
        assert_eq!(first["state"], "open");

        let (second_status, second) = open_session(&app, &token, 1_000, 1_000, "second.bin").await;
        assert_eq!(second_status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(second["error"], "quota_exceeded");

        let first_id = first["session_id"].as_str().unwrap().to_string();
        let (status, still_open) = app
            .send(bare_request("GET", &format!("/uploads/{first_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(still_open["state"], "open");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn abort_de_la_premiere_libere_la_reservation_pour_une_nouvelle_open() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188601";
        seed_drive_user(&db.pool, owner, 1_500).await;
        let token = app.token(owner);

        let (_, first) = open_session(&app, &token, 1_000, 1_000, "a.bin").await;
        let first_id = first["session_id"].as_str().unwrap().to_string();

        let (blocked_status, _) = open_session(&app, &token, 1_000, 1_000, "b.bin").await;
        assert_eq!(blocked_status, StatusCode::PAYLOAD_TOO_LARGE);

        let (abort_status, _) = app
            .send(bare_request("DELETE", &format!("/uploads/{first_id}"), &token))
            .await;
        assert_eq!(abort_status, StatusCode::NO_CONTENT);

        let (freed_status, freed) = open_session(&app, &token, 1_000, 1_000, "c.bin").await;
        assert_eq!(freed_status, StatusCode::CREATED);
        assert_eq!(freed["state"], "open");

        app.cleanup();
        db.destroy().await;
    }

    #[tokio::test]
    async fn complete_de_la_premiere_consomme_le_quota_et_libere_la_reservation_active() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188602";
        seed_drive_user(&db.pool, owner, 1_500).await;
        let token = app.token(owner);

        let (_, first) = open_session(&app, &token, 1_000, 1_000, "a.bin").await;
        let first_id = first["session_id"].as_str().unwrap().to_string();

        let (blocked_status, _) = open_session(&app, &token, 1_000, 1_000, "b.bin").await;
        assert_eq!(blocked_status, StatusCode::PAYLOAD_TOO_LARGE);

        complete_session(&app, &token, &first_id, 1_000).await;

        let (completed_status, completed) = app
            .send(bare_request("GET", &format!("/uploads/{first_id}"), &token))
            .await;
        assert_eq!(completed_status, StatusCode::OK);
        assert_eq!(completed["state"], "completed");

        let (after_complete_status, _) = open_session(&app, &token, 1_000, 1_000, "c.bin").await;
        assert_eq!(after_complete_status, StatusCode::PAYLOAD_TOO_LARGE);

        let (within_remaining_status, within) =
            open_session(&app, &token, 400, 400, "d.bin").await;
        assert_eq!(within_remaining_status, StatusCode::CREATED);
        assert_eq!(within["state"], "open");

        app.cleanup();
        db.destroy().await;
    }
}

mod replay_idempotency {
    use super::*;

    #[tokio::test]
    async fn complete_rejoue_apres_succes_ne_renvoie_jamais_500() {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188700";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 10, 10, "rejeu.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        app.send(chunk_request(
            &format!("/uploads/{session_id}/chunks/0"),
            &token,
            b"0123456789".to_vec(),
        ))
        .await;

        let (first_status, _) = app
            .send(bare_request(
                "POST",
                &format!("/uploads/{session_id}/complete"),
                &token,
            ))
            .await;
        assert_eq!(first_status, StatusCode::CREATED);

        let (second_status, _) = app
            .send(bare_request(
                "POST",
                &format!("/uploads/{session_id}/complete"),
                &token,
            ))
            .await;
        assert_ne!(second_status, StatusCode::INTERNAL_SERVER_ERROR);

        let (status, completed) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(completed["state"], "completed");

        app.cleanup();
        db.destroy().await;
    }
}

mod degraded_persist_node_failure {
    use super::*;

    #[tokio::test]
    async fn complete_qui_echoue_en_persistance_repasse_open_puis_second_complete_materialise_le_node(
    ) {
        let db = require_db!();
        let app = TestApp::build(db.pool.clone());
        let owner = "0123456789abcdef00188500";
        seed_drive_user(&db.pool, owner, 1_000_000).await;
        let token = app.token(owner);

        let (_, opened) = open_session(&app, &token, 20, 10, "degrade.bin").await;
        let session_id = opened["session_id"].as_str().unwrap().to_string();

        for index in 0..2 {
            app.send(chunk_request(
                &format!("/uploads/{session_id}/chunks/{index}"),
                &token,
                b"0123456789".to_vec(),
            ))
            .await;
        }

        sqlx::query("UPDATE drive_users SET quota_bytes = 5 WHERE user_id = $1")
            .bind(owner)
            .execute(&db.pool)
            .await
            .unwrap();

        let (first_status, first_body) = app
            .send(bare_request(
                "POST",
                &format!("/uploads/{session_id}/complete"),
                &token,
            ))
            .await;
        assert_eq!(first_status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(first_body["error"], "quota_exceeded");

        let (status, reopened) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reopened["state"], "open");

        sqlx::query("UPDATE drive_users SET quota_bytes = 1000000 WHERE user_id = $1")
            .bind(owner)
            .execute(&db.pool)
            .await
            .unwrap();

        let (second_status, node) = app
            .send(bare_request(
                "POST",
                &format!("/uploads/{session_id}/complete"),
                &token,
            ))
            .await;
        assert_eq!(second_status, StatusCode::CREATED);
        assert_eq!(node["name"], "degrade.bin");
        assert_eq!(node["size_bytes"], 20);
        assert_eq!(node["kind"], "file");

        let (status, completed) = app
            .send(bare_request("GET", &format!("/uploads/{session_id}"), &token))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(completed["state"], "completed");

        app.cleanup();
        db.destroy().await;
    }
}
