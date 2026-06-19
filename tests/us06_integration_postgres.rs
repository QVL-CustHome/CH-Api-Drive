mod common;

use ch_api_drive::repository::{drive_users, nodes};
use common::{seed_drive_user, DisposableDb};
use uuid::Uuid;

macro_rules! require_db {
    () => {
        match DisposableDb::create().await {
            Some(db) => db,
            None => {
                eprintln!(
                    "US-06 ignoré : variable {} absente (Postgres jetable requis)",
                    common::ENV_ADMIN_URL
                );
                return;
            }
        }
    };
}

async fn insert_sized_file(
    pool: &sqlx::Pool<sqlx::Postgres>,
    owner: &str,
    parent: Uuid,
    name: &str,
    size: i64,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let node = nodes::insert_file(
        &mut tx,
        owner,
        parent,
        name,
        Some("application/octet-stream"),
        size,
        &format!("{owner}/{}", Uuid::new_v4()),
        "deadbeef",
        false,
        None,
    )
    .await
    .unwrap();
    drive_users::add_used_bytes(&mut tx, owner, size)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    node.id
}

#[tokio::test]
async fn ac2_quota_respecte_transactionnellement() {
    let db = require_db!();
    let owner = "0123456789abcdef00000001";
    let root = seed_drive_user(&db.pool, owner, 1_000).await;

    insert_sized_file(&db.pool, owner, root, "a.bin", 600).await;

    let mut tx = db.pool.begin().await.unwrap();
    let (quota, used) = drive_users::quota_used_for_update(&mut tx, owner)
        .await
        .unwrap();
    assert_eq!(quota, 1_000);
    assert_eq!(used, 600);
    let over_quota = used + 600 > quota;
    assert!(over_quota, "le second upload doit dépasser le quota");
    tx.rollback().await.unwrap();

    let after = drive_users::find(&db.pool, owner).await.unwrap().unwrap();
    assert_eq!(after.used_bytes, 600, "used_bytes ne doit pas dériver");

    db.destroy().await;
}

#[tokio::test]
async fn ac3_purge_recursive_supprime_tous_les_descendants() {
    let db = require_db!();
    let owner = "0123456789abcdef00000002";
    let root = seed_drive_user(&db.pool, owner, 1_000_000).await;

    let folder = nodes::create_folder(&db.pool, owner, root, "dossier")
        .await
        .unwrap();
    let sub = nodes::create_folder(&db.pool, owner, folder.id, "sous-dossier")
        .await
        .unwrap();
    insert_sized_file(&db.pool, owner, folder.id, "f1.bin", 100).await;
    insert_sized_file(&db.pool, owner, sub.id, "f2.bin", 200).await;

    let (blobs, freed) = nodes::purge_node(&db.pool, owner, folder.id).await.unwrap();
    assert_eq!(freed, 300, "les octets des deux fichiers doivent être libérés");
    assert_eq!(blobs.len(), 2, "les deux blobs doivent être retournés");

    assert!(nodes::get(&db.pool, owner, folder.id).await.unwrap().is_none());
    assert!(nodes::get(&db.pool, owner, sub.id).await.unwrap().is_none());

    let children = nodes::list_children(&db.pool, owner, root, true)
        .await
        .unwrap();
    assert!(children.is_empty(), "aucun descendant ne doit subsister");

    let user = drive_users::find(&db.pool, owner).await.unwrap().unwrap();
    assert_eq!(user.used_bytes, 0);

    db.destroy().await;
}

#[tokio::test]
async fn ac4_blobs_coherents_sans_orphelin() {
    let db = require_db!();
    let owner = "0123456789abcdef00000003";
    let root = seed_drive_user(&db.pool, owner, 1_000_000).await;

    insert_sized_file(&db.pool, owner, root, "keep.bin", 50).await;
    let trashed = insert_sized_file(&db.pool, owner, root, "trash.bin", 70).await;

    nodes::trash_subtree(&db.pool, owner, trashed).await.unwrap();

    let (blobs, freed) = nodes::purge_trash(&db.pool, owner).await.unwrap();
    assert_eq!(freed, 70);
    assert_eq!(blobs.len(), 1, "seul le fichier en corbeille est purgé");
    assert!(blobs.iter().all(|b| b.storage_key.is_some()));

    let remaining = nodes::list_children(&db.pool, owner, root, true)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "keep.bin");

    let user = drive_users::find(&db.pool, owner).await.unwrap().unwrap();
    assert_eq!(user.used_bytes, 50);

    db.destroy().await;
}

#[tokio::test]
async fn ac1_base_jetable_migree() {
    let db = require_db!();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables \
         WHERE table_name IN ('nodes', 'drive_users')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(count, 2, "les tables migrées doivent exister");
    db.destroy().await;
}
