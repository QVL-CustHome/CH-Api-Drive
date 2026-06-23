mod common;

use ch_api_drive::jobs::upload_gc::{run_once, GcConfig};
use ch_api_drive::repository::upload_sessions::{self, NewUploadSession, UploadState};
use ch_api_drive::services::storage::Storage;
use chrono::{DateTime, Duration, Utc};
use common::{seed_drive_user, DisposableDb};
use sqlx::{Pool, Postgres};
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use uuid::Uuid;

#[test]
fn ac4_gcconfig_interval_zero_est_borne_a_un_minimum_non_nul() {
    let config = GcConfig::new(StdDuration::ZERO);
    assert!(
        config.interval >= StdDuration::from_secs(1),
        "GcConfig::new(ZERO) doit être borné pour éviter la panique tokio::time::interval(ZERO)"
    );
}

#[test]
fn ac4_gcconfig_interval_au_dessus_du_minimum_est_conserve() {
    let config = GcConfig::new(StdDuration::from_secs(30));
    assert_eq!(
        config.interval,
        StdDuration::from_secs(30),
        "un interval déjà au-dessus du plancher doit être conservé tel quel"
    );
}

#[test]
fn ac4_gcconfig_batch_size_par_defaut_est_cent() {
    let config = GcConfig::new(StdDuration::from_secs(60));
    assert_eq!(config.batch_size, 100, "le batch_size par défaut doit rester 100");
}

#[test]
fn ac4_gcconfig_with_batch_size_zero_est_borne_a_un_minimum() {
    let config = GcConfig::new(StdDuration::from_secs(60)).with_batch_size(0);
    assert!(
        config.batch_size >= 1,
        "with_batch_size(0) doit appliquer un plancher de 1"
    );
}

#[test]
fn ac4_gcconfig_with_batch_size_valide_est_applique() {
    let config = GcConfig::new(StdDuration::from_secs(60)).with_batch_size(50);
    assert_eq!(config.batch_size, 50, "un batch_size valide doit être appliqué");
}

macro_rules! require_db {
    () => {
        match DisposableDb::create().await {
            Some(db) => db,
            None => {
                eprintln!(
                    "SCRUM-188 C4 ignoré : variable {} absente (Postgres jetable requis)",
                    common::ENV_ADMIN_URL
                );
                return;
            }
        }
    };
}

#[derive(Clone, Default)]
struct RecordingStorage {
    existing: Arc<Mutex<Vec<String>>>,
    deleted: Arc<Mutex<Vec<String>>>,
}

impl RecordingStorage {
    fn with_file(key: &str) -> Self {
        let storage = Self::default();
        storage.existing.lock().unwrap().push(key.to_string());
        storage
    }

    fn deleted_keys(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
    }

    fn file_present(&self, key: &str) -> bool {
        self.existing.lock().unwrap().iter().any(|k| k == key)
    }
}

impl Storage for RecordingStorage {
    fn write_bytes(&self, _key: &str, _bytes: &[u8]) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    fn write_at(
        &self,
        _key: &str,
        _offset: u64,
        _bytes: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    fn create_writer(&self, _key: &str) -> impl Future<Output = Result<tokio::fs::File>> + Send {
        async { Err(Error::new(ErrorKind::Unsupported, "non utilisé")) }
    }

    fn open(&self, _key: &str) -> impl Future<Output = Result<tokio::fs::File>> + Send {
        async { Err(Error::new(ErrorKind::Unsupported, "non utilisé")) }
    }

    fn finalize(
        &self,
        _tmp_key: &str,
        _storage_key: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        let key = key.to_string();
        let existing = self.existing.clone();
        let deleted = self.deleted.clone();
        async move {
            deleted.lock().unwrap().push(key.clone());
            existing.lock().unwrap().retain(|k| k != &key);
            Ok(())
        }
    }

    fn metadata(&self, _key: &str) -> impl Future<Output = Result<std::fs::Metadata>> + Send {
        async { Err(Error::new(ErrorKind::NotFound, "non utilisé")) }
    }
}

fn session_with_expiry<'a>(
    owner: &'a str,
    parent: Uuid,
    name: &'a str,
    tmp_key: &'a str,
    expires_at: DateTime<Utc>,
) -> NewUploadSession<'a> {
    NewUploadSession {
        owner_id: owner,
        parent_id: parent,
        file_name: name,
        declared_mime: Some("application/octet-stream"),
        declared_size: 1_000_000,
        reserved_bytes: 1_000_000,
        chunk_size: 8_388_608,
        chunk_count: 1,
        checksum: None,
        storage_key: "storage/key",
        tmp_key,
        expires_at,
    }
}

async fn force_state(pool: &Pool<Postgres>, id: Uuid, state: UploadState) {
    sqlx::query("UPDATE upload_sessions SET state = $2 WHERE id = $1")
        .bind(id)
        .bind(state)
        .execute(pool)
        .await
        .expect("forçage d'état pour la mise en situation du test");
}

async fn session_exists(pool: &Pool<Postgres>, id: Uuid) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM upload_sessions WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("comptage session")
        > 0
}

#[tokio::test]
async fn ac1_ac2_session_open_expiree_est_supprimee_par_run_once() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4001";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/open-expiree");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "expiree.bin",
            "tmp/open-expiree",
            Utc::now() - Duration::hours(1),
        ),
    )
    .await
    .expect("création session open expirée");

    let report = run_once(&db.pool, &storage, 100).await;

    assert_eq!(report.reclaimed, 1, "la session open expirée doit être récupérée");
    assert_eq!(report.failed, 0, "aucune erreur attendue");
    assert!(
        !session_exists(&db.pool, session.id).await,
        "la ligne upload_sessions de la session expirée doit être supprimée"
    );
    assert_eq!(
        storage.deleted_keys(),
        vec!["tmp/open-expiree".to_string()],
        "le fichier temporaire doit être supprimé après la suppression DB"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac1_ac2_session_aborted_expiree_est_supprimee_par_run_once() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4002";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/aborted-expiree");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "aborted.bin",
            "tmp/aborted-expiree",
            Utc::now() - Duration::minutes(5),
        ),
    )
    .await
    .expect("création session");
    force_state(&db.pool, session.id, UploadState::Aborted).await;

    let report = run_once(&db.pool, &storage, 100).await;

    assert_eq!(report.reclaimed, 1, "une session aborted expirée doit aussi être récupérée");
    assert!(
        !session_exists(&db.pool, session.id).await,
        "la session aborted expirée doit être supprimée"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_session_open_recente_non_expiree_est_conservee() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4003";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/open-recente");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "recente.bin",
            "tmp/open-recente",
            Utc::now() + Duration::hours(24),
        ),
    )
    .await
    .expect("création session open récente");

    let report = run_once(&db.pool, &storage, 100).await;

    assert_eq!(report.reclaimed, 0, "une session open non expirée ne doit jamais être touchée");
    assert!(
        session_exists(&db.pool, session.id).await,
        "la session open récente doit être conservée"
    );
    assert!(
        storage.file_present("tmp/open-recente"),
        "le fichier temporaire d'une session récente doit rester intact"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_session_completed_expiree_est_conservee() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4004";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/completed-expiree");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "completed.bin",
            "tmp/completed-expiree",
            Utc::now() - Duration::days(2),
        ),
    )
    .await
    .expect("création session");
    force_state(&db.pool, session.id, UploadState::Completed).await;

    let report = run_once(&db.pool, &storage, 100).await;

    assert_eq!(
        report.reclaimed, 0,
        "une session completed, même expirée, ne doit jamais être supprimée par le GC"
    );
    assert!(
        session_exists(&db.pool, session.id).await,
        "la session completed expirée doit être conservée intégralement"
    );
    assert!(
        storage.deleted_keys().is_empty(),
        "aucun fichier ne doit être supprimé pour une session completed"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_session_completing_expiree_est_conservee() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4005";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/completing-expiree");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "completing.bin",
            "tmp/completing-expiree",
            Utc::now() - Duration::days(2),
        ),
    )
    .await
    .expect("création session");
    force_state(&db.pool, session.id, UploadState::Completing).await;

    let report = run_once(&db.pool, &storage, 100).await;

    assert_eq!(
        report.reclaimed, 0,
        "une session completing, même expirée, ne doit jamais être supprimée par le GC"
    );
    assert!(
        session_exists(&db.pool, session.id).await,
        "la session completing expirée doit être conservée"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_run_once_rejoue_est_idempotent() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4006";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/idempotent");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "idempotent.bin",
            "tmp/idempotent",
            Utc::now() - Duration::hours(3),
        ),
    )
    .await
    .expect("création session expirée");

    let first = run_once(&db.pool, &storage, 100).await;
    let second = run_once(&db.pool, &storage, 100).await;

    assert_eq!(first.reclaimed, 1, "la première passe récupère la session expirée");
    assert_eq!(second.reclaimed, 0, "la seconde passe ne trouve plus rien à récupérer");
    assert_eq!(second.failed, 0, "rejouer le GC ne doit produire aucune erreur");
    assert!(
        !session_exists(&db.pool, session.id).await,
        "la session reste supprimée après plusieurs passes"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_garde_anti_course_session_reprise_entre_recensement_et_suppression() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4007";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/anti-course");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "course.bin",
            "tmp/anti-course",
            Utc::now() - Duration::hours(1),
        ),
    )
    .await
    .expect("création session expirée recensable");

    let recensees = upload_sessions::find_expired(&db.pool, 100)
        .await
        .expect("recensement des sessions expirées");
    assert!(
        recensees.iter().any(|s| s.id == session.id),
        "la session expirée doit d'abord être recensée par find_expired"
    );

    force_state(&db.pool, session.id, UploadState::Completing).await;

    let supprimee = upload_sessions::delete_if_expired(&db.pool, session.id)
        .await
        .expect("appel delete_if_expired");

    assert!(
        !supprimee,
        "si la session a été reprise (completing) entre recensement et suppression, \
         delete_if_expired ne doit rien supprimer (garde state/expires_at reportée)"
    );
    assert!(
        session_exists(&db.pool, session.id).await,
        "la session reprise doit rester intacte en base"
    );
    assert!(
        storage.file_present("tmp/anti-course"),
        "le fichier temporaire d'une session reprise ne doit pas être supprimé"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac3_quota_reserve_libere_par_la_suppression_de_la_ligne_sans_toucher_used_bytes() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4008";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/quota");

    let used_avant = sqlx::query_scalar::<_, i64>(
        "SELECT used_bytes FROM drive_users WHERE user_id = $1",
    )
    .bind(owner)
    .fetch_one(&db.pool)
    .await
    .expect("lecture used_bytes initial");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "quota.bin",
            "tmp/quota",
            Utc::now() - Duration::hours(1),
        ),
    )
    .await
    .expect("création session expirée avec réservation");
    assert_eq!(session.reserved_bytes, 1_000_000, "réservation posée à l'ouverture");

    let reserve_avant = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT sum(reserved_bytes)::bigint FROM upload_sessions WHERE owner_id = $1",
    )
    .bind(owner)
    .fetch_one(&db.pool)
    .await
    .expect("somme réservée avant GC");
    assert_eq!(reserve_avant, Some(1_000_000), "le quota réservé est porté par la ligne de session");

    run_once(&db.pool, &storage, 100).await;

    let reserve_apres = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT sum(reserved_bytes)::bigint FROM upload_sessions WHERE owner_id = $1",
    )
    .bind(owner)
    .fetch_one(&db.pool)
    .await
    .expect("somme réservée après GC");
    assert_eq!(
        reserve_apres, None,
        "la réservation doit être libérée par la disparition de la ligne upload_sessions"
    );

    let used_apres = sqlx::query_scalar::<_, i64>(
        "SELECT used_bytes FROM drive_users WHERE user_id = $1",
    )
    .bind(owner)
    .fetch_one(&db.pool)
    .await
    .expect("lecture used_bytes après GC");
    assert_eq!(
        used_apres, used_avant,
        "le GC ne doit jamais modifier drive_users.used_bytes"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_cascade_supprime_les_chunks_de_la_session_expiree() {
    let db = require_db!();
    let owner = "0123456789abcdef000c4009";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;
    let storage = RecordingStorage::with_file("tmp/cascade");

    let session = upload_sessions::create(
        &db.pool,
        session_with_expiry(
            owner,
            root,
            "cascade.bin",
            "tmp/cascade",
            Utc::now() - Duration::hours(1),
        ),
    )
    .await
    .expect("création session expirée");

    sqlx::query(
        "INSERT INTO upload_chunks (session_id, chunk_index, size_bytes) VALUES ($1, 0, 8388608)",
    )
    .bind(session.id)
    .execute(&db.pool)
    .await
    .expect("insertion chunk");

    run_once(&db.pool, &storage, 100).await;

    let chunks_restants = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM upload_chunks WHERE session_id = $1",
    )
    .bind(session.id)
    .fetch_one(&db.pool)
    .await
    .expect("comptage chunks après GC");
    assert_eq!(
        chunks_restants, 0,
        "les chunks doivent disparaître via ON DELETE CASCADE lors de la suppression de la session"
    );

    db.destroy().await;
}
