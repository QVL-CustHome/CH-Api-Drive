mod common;

use ch_api_drive::repository::upload_sessions::{
    self, NewUploadSession, UploadSession, UploadState,
};
use chrono::{Duration, Utc};
use common::{seed_drive_user, DisposableDb};
use sqlx::{Pool, Postgres, Row};
use uuid::Uuid;

macro_rules! require_db {
    () => {
        match DisposableDb::create().await {
            Some(db) => db,
            None => {
                eprintln!(
                    "SCRUM-186 C1 ignoré : variable {} absente (Postgres jetable requis)",
                    common::ENV_ADMIN_URL
                );
                return;
            }
        }
    };
}

fn sample_session<'a>(owner: &'a str, parent: Uuid, name: &'a str) -> NewUploadSession<'a> {
    NewUploadSession {
        owner_id: owner,
        parent_id: parent,
        file_name: name,
        declared_mime: Some("application/octet-stream"),
        declared_size: 10_000_000_000,
        reserved_bytes: 10_000_000_000,
        chunk_size: 8_388_608,
        chunk_count: 1192,
        checksum: None,
        storage_key: "storage/key",
        tmp_key: "tmp/key",
        expires_at: Utc::now() + Duration::hours(24),
    }
}

async fn column_exists(pool: &Pool<Postgres>, table: &str, column: &str) -> bool {
    let row = sqlx::query(
        "SELECT count(*) AS n FROM information_schema.columns \
         WHERE table_name = $1 AND column_name = $2",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .expect("introspection information_schema");
    row.get::<i64, _>("n") > 0
}

#[tokio::test]
async fn ac1_reservation_a_l_ouverture_le_support_de_reservation_doit_exister() {
    let db = require_db!();

    let reserved_on_session = column_exists(&db.pool, "upload_sessions", "reserved_bytes").await;
    let reserved_on_user = column_exists(&db.pool, "drive_users", "reserved_bytes").await;
    let declared_on_session =
        column_exists(&db.pool, "upload_sessions", "declared_size").await;
    let quota_on_user = column_exists(&db.pool, "drive_users", "quota_bytes").await;
    let used_on_user = column_exists(&db.pool, "drive_users", "used_bytes").await;

    assert!(
        quota_on_user && used_on_user,
        "la structure de quota (quota_bytes/used_bytes) doit exister pour évaluer la réservation"
    );
    assert!(
        declared_on_session,
        "declared_size doit exister pour dimensionner la réservation à l'ouverture"
    );
    assert!(
        reserved_on_session || reserved_on_user,
        "AUCUNE colonne reserved_bytes trouvée (ni sur upload_sessions ni sur drive_users) \
         alors que l'AC1 et la consigne C1 annoncent reserved_bytes posable à l'ouverture"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac1_cumul_octets_ecrits_incrementable_via_add_received_bytes() {
    let db = require_db!();
    let owner = "0123456789abcdef000c1001";
    let root = seed_drive_user(&db.pool, owner, 20_000_000_000).await;

    let created = upload_sessions::create(&db.pool, sample_session(owner, root, "big.bin"))
        .await
        .expect("création session");
    assert_eq!(created.received_bytes, 0, "received_bytes initial doit être 0");

    let after_first = upload_sessions::add_received_bytes(&db.pool, owner, created.id, 8_388_608)
        .await
        .expect("premier add_received_bytes")
        .expect("session de l'owner présente");
    assert_eq!(after_first, 8_388_608, "le cumul doit refléter le premier delta");

    let after_second = upload_sessions::add_received_bytes(&db.pool, owner, created.id, 8_388_608)
        .await
        .expect("second add_received_bytes")
        .expect("session de l'owner présente");
    assert_eq!(
        after_second, 16_777_216,
        "le cumul d'octets réellement écrits doit s'additionner"
    );

    let reloaded = upload_sessions::get(&db.pool, owner, created.id)
        .await
        .expect("relecture session")
        .expect("session présente");
    assert_eq!(
        reloaded.received_bytes, 16_777_216,
        "le cumul persisté doit correspondre au retour de add_received_bytes"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac1_cumul_recu_confrontable_au_quota_de_l_owner() {
    let db = require_db!();
    let owner = "0123456789abcdef000c1002";
    let quota = 12_000_000i64;
    let root = seed_drive_user(&db.pool, owner, quota).await;

    let mut new = sample_session(owner, root, "huge.bin");
    new.declared_size = 5_000_000;
    new.reserved_bytes = new.declared_size;
    let created = upload_sessions::create(&db.pool, new)
        .await
        .expect("création session");

    let received = upload_sessions::add_received_bytes(&db.pool, owner, created.id, 13_000_000)
        .await
        .expect("add_received_bytes")
        .expect("session de l'owner présente");

    let user_row = sqlx::query("SELECT quota_bytes, used_bytes FROM drive_users WHERE user_id = $1")
        .bind(owner)
        .fetch_one(&db.pool)
        .await
        .expect("lecture quota owner");
    let quota_bytes: i64 = user_row.get("quota_bytes");
    let used_bytes: i64 = user_row.get("used_bytes");

    assert_eq!(quota_bytes, quota);
    assert!(
        used_bytes + received > quota_bytes,
        "les données nécessaires (received_bytes + quota_bytes + used_bytes) doivent permettre \
         à un futur endpoint C3 de détecter un dépassement"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_sessions_paralleles_multiples_par_owner_sont_autorisees() {
    let db = require_db!();
    let owner = "0123456789abcdef000c2001";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;

    let s1 = upload_sessions::create(&db.pool, sample_session(owner, root, "a.bin"))
        .await
        .expect("session 1");
    let s2 = upload_sessions::create(&db.pool, sample_session(owner, root, "b.bin"))
        .await
        .expect("session 2 parallèle même owner");
    let s3 = upload_sessions::create(&db.pool, sample_session(owner, root, "c.bin"))
        .await
        .expect("session 3 parallèle même owner");

    let open: Vec<UploadSession> = upload_sessions::list_by_owner(&db.pool, owner)
        .await
        .expect("liste sessions owner")
        .into_iter()
        .filter(|s| s.state == UploadState::Open)
        .collect();

    assert_eq!(
        open.len(),
        3,
        "trois sessions ouvertes simultanées par owner doivent coexister"
    );
    let ids: std::collections::HashSet<Uuid> = [s1.id, s2.id, s3.id].into_iter().collect();
    assert_eq!(ids.len(), 3, "chaque session doit avoir un identifiant distinct");

    db.destroy().await;
}

#[tokio::test]
async fn ac2_sessions_paralleles_meme_fichier_aucune_unicite_bloquante() {
    let db = require_db!();
    let owner = "0123456789abcdef000c2002";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;

    upload_sessions::create(&db.pool, sample_session(owner, root, "doublon.bin"))
        .await
        .expect("première session sur le fichier");
    let second = upload_sessions::create(&db.pool, sample_session(owner, root, "doublon.bin")).await;

    assert!(
        second.is_ok(),
        "aucune contrainte d'unicité ne doit empêcher deux sessions ouvertes sur le même \
         couple (owner, file_name, parent)"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_transition_state_est_concurrency_safe_via_garde_from() {
    let db = require_db!();
    let owner = "0123456789abcdef000c2003";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;

    let session = upload_sessions::create(&db.pool, sample_session(owner, root, "race.bin"))
        .await
        .expect("création session");

    let first = upload_sessions::transition_state(
        &db.pool,
        owner,
        session.id,
        UploadState::Open,
        UploadState::Completing,
    )
    .await
    .expect("première transition");
    assert!(
        first.is_some(),
        "la transition depuis l'état attendu doit réussir"
    );
    assert_eq!(first.unwrap().state, UploadState::Completing);

    let racing = upload_sessions::transition_state(
        &db.pool,
        owner,
        session.id,
        UploadState::Open,
        UploadState::Aborted,
    )
    .await
    .expect("seconde transition concurrente");
    assert!(
        racing.is_none(),
        "une transition dont l'état source ne correspond plus doit échouer (garde WHERE state=from), \
         garantissant la sûreté en concurrence"
    );

    let reloaded = upload_sessions::get(&db.pool, owner, session.id)
        .await
        .expect("relecture")
        .expect("session présente");
    assert_eq!(
        reloaded.state,
        UploadState::Completing,
        "l'état ne doit pas avoir été écrasé par la transition perdante"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac2_transition_invalide_etat_inexistant_ne_corrompt_pas() {
    let db = require_db!();
    let owner = "0123456789abcdef000c2004";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;

    let session = upload_sessions::create(&db.pool, sample_session(owner, root, "x.bin"))
        .await
        .expect("création");

    let bad = upload_sessions::transition_state(
        &db.pool,
        owner,
        session.id,
        UploadState::Completed,
        UploadState::Aborted,
    )
    .await
    .expect("appel transition");
    assert!(
        bad.is_none(),
        "transitionner depuis un état non courant ne doit rien modifier"
    );

    db.destroy().await;
}

#[tokio::test]
async fn ac3_comptage_sessions_ouvertes_par_owner_pour_appliquer_une_limite() {
    let db = require_db!();
    let owner = "0123456789abcdef000c3001";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;

    for name in ["1.bin", "2.bin", "3.bin", "4.bin"] {
        upload_sessions::create(&db.pool, sample_session(owner, root, name))
            .await
            .expect("création session");
    }

    let open_count = upload_sessions::list_by_owner(&db.pool, owner)
        .await
        .expect("liste owner")
        .into_iter()
        .filter(|s| s.state == UploadState::Open)
        .count();
    assert_eq!(
        open_count, 4,
        "le modèle permet de dénombrer les sessions ouvertes par owner (base d'une limite C3)"
    );

    let owner_index_present = sqlx::query(
        "SELECT count(*) AS n FROM pg_indexes \
         WHERE tablename = 'upload_sessions' AND indexdef ILIKE '%owner_id%'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("introspection index owner")
    .get::<i64, _>("n")
        > 0;
    assert!(
        owner_index_present,
        "un index sur owner_id doit exister pour rendre le comptage par owner efficace"
    );

    db.destroy().await;
}

#[tokio::test]
async fn upload_state_round_trip_sqlx_pour_chaque_variante() {
    let db = require_db!();
    let owner = "0123456789abcdef000c9001";
    let root = seed_drive_user(&db.pool, owner, 50_000_000_000).await;

    let variants = [
        UploadState::Open,
        UploadState::Completing,
        UploadState::Completed,
        UploadState::Aborted,
    ];

    for variant in variants {
        let session = upload_sessions::create(&db.pool, sample_session(owner, root, "rt.bin"))
            .await
            .expect("création session round-trip");

        let updated: UploadState = sqlx::query_scalar(
            "UPDATE upload_sessions SET state = $3 WHERE id = $1 AND owner_id = $2 RETURNING state",
        )
        .bind(session.id)
        .bind(owner)
        .bind(variant)
        .fetch_one(&db.pool)
        .await
        .expect("encode de la variante UploadState");
        assert_eq!(
            updated, variant,
            "le RETURNING immédiat doit redonner la variante encodée"
        );

        let reloaded = upload_sessions::get(&db.pool, owner, session.id)
            .await
            .expect("relecture session")
            .expect("session présente");
        assert_eq!(
            reloaded.state, variant,
            "la relecture via FromRow doit décoder la variante persistée à l'identique"
        );
    }

    db.destroy().await;
}
