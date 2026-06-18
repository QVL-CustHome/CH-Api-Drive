use crate::db::Db;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Postgres;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct DriveUser {
    pub user_id: String,
    pub quota_bytes: i64,
    pub used_bytes: i64,
    pub root_node_id: Uuid,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DriveUserAdmin {
    pub user_id: String,
    pub quota_bytes: i64,
    pub used_bytes: i64,
    pub created_at: DateTime<Utc>,
}

pub async fn list_all(pool: &Db) -> Result<Vec<DriveUserAdmin>, sqlx::Error> {
    sqlx::query_as::<_, DriveUserAdmin>(
        "SELECT user_id, quota_bytes, used_bytes, created_at FROM drive_users \
         ORDER BY used_bytes DESC, user_id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn recompute_used(
    pool: &Db,
    user_id: &str,
) -> Result<Option<DriveUserAdmin>, sqlx::Error> {
    sqlx::query_as::<_, DriveUserAdmin>(
        "UPDATE drive_users SET used_bytes = COALESCE( \
            (SELECT SUM(size_bytes) FROM nodes \
             WHERE owner_id = $1 AND kind = 'file' AND storage_key IS NOT NULL), 0) \
         WHERE user_id = $1 \
         RETURNING user_id, quota_bytes, used_bytes, created_at",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn set_quota(
    pool: &Db,
    user_id: &str,
    quota_bytes: i64,
) -> Result<Option<DriveUserAdmin>, sqlx::Error> {
    sqlx::query_as::<_, DriveUserAdmin>(
        "UPDATE drive_users SET quota_bytes = $2 WHERE user_id = $1 \
         RETURNING user_id, quota_bytes, used_bytes, created_at",
    )
    .bind(user_id)
    .bind(quota_bytes)
    .fetch_optional(pool)
    .await
}

pub async fn ensure_user(
    pool: &Db,
    user_id: &str,
    default_quota_bytes: i64,
) -> Result<DriveUser, sqlx::Error> {
    if let Some(existing) = find(pool, user_id).await? {
        return Ok(existing);
    }

    let mut tx = pool.begin().await?;
    let root_id = Uuid::new_v4();

    let inserted = sqlx::query(
        "INSERT INTO drive_users (user_id, quota_bytes, used_bytes, root_node_id)
         VALUES ($1, $2, 0, $3)
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(default_quota_bytes)
    .bind(root_id)
    .execute(&mut *tx)
    .await?;

    if inserted.rows_affected() == 1 {
        sqlx::query(
            "INSERT INTO nodes (id, owner_id, parent_id, kind, name)
             VALUES ($1, $2, NULL, 'folder', 'root')",
        )
        .bind(root_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    let user = find(pool, user_id)
        .await?
        .expect("drive_user présent après upsert");
    Ok(user)
}

pub async fn find(pool: &Db, user_id: &str) -> Result<Option<DriveUser>, sqlx::Error> {
    sqlx::query_as::<_, DriveUser>(
        "SELECT user_id, quota_bytes, used_bytes, root_node_id FROM drive_users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn quota_used_for_update(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<(i64, i64), sqlx::Error> {
    let row: (i64, i64) =
        sqlx::query_as("SELECT quota_bytes, used_bytes FROM drive_users WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await?;
    Ok(row)
}

pub async fn add_used_bytes(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    user_id: &str,
    delta: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE drive_users SET used_bytes = used_bytes + $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(delta)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
