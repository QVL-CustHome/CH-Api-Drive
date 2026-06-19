use crate::db::Db;
use crate::domain::{MediaType, NodeKind};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Postgres;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct Node {
    pub id: Uuid,
    pub owner_id: String,
    pub parent_id: Option<Uuid>,
    pub kind: NodeKind,
    pub name: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub storage_key: Option<String>,
    pub content_hash: Option<String>,
    pub is_media: bool,
    pub media_type: Option<MediaType>,
    pub taken_at: Option<DateTime<Utc>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_ms: Option<i32>,
    pub has_thumbnail: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct NodeDto {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub kind: NodeKind,
    pub name: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub is_media: bool,
    pub media_type: Option<MediaType>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_ms: Option<i32>,
    pub has_thumbnail: bool,
    pub taken_at: Option<DateTime<Utc>>,
    pub trashed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Node {
    pub fn to_dto(&self) -> NodeDto {
        NodeDto {
            id: self.id,
            parent_id: self.parent_id,
            kind: self.kind,
            name: self.name.clone(),
            mime: self.mime.clone(),
            size_bytes: self.size_bytes,
            is_media: self.is_media,
            media_type: self.media_type,
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            has_thumbnail: self.has_thumbnail,
            taken_at: self.taken_at,
            trashed: self.trashed_at.is_some(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    pub fn is_folder(&self) -> bool {
        self.kind.is_folder()
    }
}

const NODE_COLS: &str = "id, owner_id, parent_id, kind, name, mime, size_bytes, storage_key, \
    content_hash, is_media, media_type, taken_at, width, height, duration_ms, has_thumbnail, \
    trashed_at, created_at, updated_at";

pub async fn get(pool: &Db, owner_id: &str, id: Uuid) -> Result<Option<Node>, AppError> {
    let sql = format!("SELECT {NODE_COLS} FROM nodes WHERE id = $1 AND owner_id = $2");
    sqlx::query_as::<_, Node>(&sql)
        .bind(id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await
        .map_err(AppError::from_db)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Crumb {
    pub id: Uuid,
    pub name: String,
}

pub async fn ancestors(pool: &Db, owner_id: &str, id: Uuid) -> Result<Vec<Crumb>, AppError> {
    sqlx::query_as::<_, Crumb>(
        "WITH RECURSIVE up AS (
            SELECT id, parent_id, name, 0 AS depth FROM nodes WHERE id = $1 AND owner_id = $2
            UNION ALL
            SELECT n.id, n.parent_id, n.name, up.depth + 1 FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT id, name FROM up ORDER BY depth DESC",
    )
    .bind(id)
    .bind(owner_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::from_db)
}

pub async fn list_trashed(pool: &Db, owner_id: &str) -> Result<Vec<Node>, AppError> {
    let sql = format!(
        "SELECT {NODE_COLS} FROM nodes \
         WHERE owner_id = $1 AND trashed_at IS NOT NULL \
         ORDER BY trashed_at DESC, lower(name) ASC"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(owner_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn list_children(
    pool: &Db,
    owner_id: &str,
    parent_id: Uuid,
    include_trashed: bool,
) -> Result<Vec<Node>, AppError> {
    let trash_clause = if include_trashed {
        ""
    } else {
        "AND trashed_at IS NULL"
    };
    let sql = format!(
        "SELECT {NODE_COLS} FROM nodes \
         WHERE owner_id = $1 AND parent_id = $2 {trash_clause} \
         ORDER BY (kind = 'folder') DESC, lower(name) ASC"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(owner_id)
        .bind(parent_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn name_exists(
    pool: &Db,
    owner_id: &str,
    parent_id: Uuid,
    name: &str,
) -> Result<bool, AppError> {
    let exists: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM nodes \
         WHERE owner_id = $1 AND parent_id = $2 AND name = $3 AND trashed_at IS NULL",
    )
    .bind(owner_id)
    .bind(parent_id)
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(exists.is_some())
}

pub async fn free_name(
    pool: &Db,
    owner_id: &str,
    parent_id: Uuid,
    desired: &str,
) -> Result<String, AppError> {
    if !name_exists(pool, owner_id, parent_id, desired).await? {
        return Ok(desired.to_string());
    }
    let (stem, ext) = split_ext(desired);
    for n in 1..10_000 {
        let candidate = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        if !name_exists(pool, owner_id, parent_id, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(AppError::Conflict(
        "Impossible de générer un nom libre pour cet élément.",
    ))
}

fn split_ext(name: &str) -> (String, Option<String>) {
    match name.rfind('.') {
        Some(idx) if idx > 0 && idx < name.len() - 1 => {
            (name[..idx].to_string(), Some(name[idx + 1..].to_string()))
        }
        _ => (name.to_string(), None),
    }
}

pub async fn create_folder(
    pool: &Db,
    owner_id: &str,
    parent_id: Uuid,
    name: &str,
) -> Result<Node, AppError> {
    let id = Uuid::new_v4();
    let sql = format!(
        "INSERT INTO nodes (id, owner_id, parent_id, kind, name) \
         VALUES ($1, $2, $3, 'folder', $4) RETURNING {NODE_COLS}"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(id)
        .bind(owner_id)
        .bind(parent_id)
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(AppError::from_db)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_file(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    owner_id: &str,
    parent_id: Uuid,
    name: &str,
    mime: Option<&str>,
    size_bytes: i64,
    storage_key: &str,
    content_hash: &str,
    is_media: bool,
    media_type: Option<MediaType>,
) -> Result<Node, AppError> {
    let id = Uuid::new_v4();
    let sql = format!(
        "INSERT INTO nodes \
         (id, owner_id, parent_id, kind, name, mime, size_bytes, storage_key, content_hash, is_media, media_type) \
         VALUES ($1, $2, $3, 'file', $4, $5, $6, $7, $8, $9, $10) RETURNING {NODE_COLS}"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(id)
        .bind(owner_id)
        .bind(parent_id)
        .bind(name)
        .bind(mime)
        .bind(size_bytes)
        .bind(storage_key)
        .bind(content_hash)
        .bind(is_media)
        .bind(media_type)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::from_db)
}

pub async fn rename(pool: &Db, owner_id: &str, id: Uuid, new_name: &str) -> Result<Node, AppError> {
    let sql = format!(
        "UPDATE nodes SET name = $3, updated_at = now() \
         WHERE id = $1 AND owner_id = $2 RETURNING {NODE_COLS}"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(id)
        .bind(owner_id)
        .bind(new_name)
        .fetch_one(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn move_node(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
    new_parent: Uuid,
) -> Result<Node, AppError> {
    let sql = format!(
        "UPDATE nodes SET parent_id = $3, updated_at = now() \
         WHERE id = $1 AND owner_id = $2 RETURNING {NODE_COLS}"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(id)
        .bind(owner_id)
        .bind(new_parent)
        .fetch_one(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn is_descendant_or_self(
    pool: &Db,
    owner_id: &str,
    ancestor: Uuid,
    candidate: Uuid,
) -> Result<bool, AppError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE subtree AS (
            SELECT id FROM nodes WHERE id = $1 AND owner_id = $2
            UNION ALL
            SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
         )
         SELECT id FROM subtree WHERE id = $3",
    )
    .bind(ancestor)
    .bind(owner_id)
    .bind(candidate)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(found.is_some())
}

pub async fn trash_subtree(pool: &Db, owner_id: &str, id: Uuid) -> Result<u64, AppError> {
    let result = sqlx::query(
        "WITH RECURSIVE subtree AS (
            SELECT id FROM nodes WHERE id = $1 AND owner_id = $2
            UNION ALL
            SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
         )
         UPDATE nodes SET trashed_at = now(), updated_at = now()
         WHERE id IN (SELECT id FROM subtree) AND trashed_at IS NULL",
    )
    .bind(id)
    .bind(owner_id)
    .execute(pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(result.rows_affected())
}

pub async fn restore_subtree(pool: &Db, owner_id: &str, id: Uuid) -> Result<u64, AppError> {
    let result = sqlx::query(
        "WITH RECURSIVE subtree AS (
            SELECT id FROM nodes WHERE id = $1 AND owner_id = $2
            UNION ALL
            SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
         )
         UPDATE nodes SET trashed_at = NULL, updated_at = now()
         WHERE id IN (SELECT id FROM subtree) AND trashed_at IS NOT NULL",
    )
    .bind(id)
    .bind(owner_id)
    .execute(pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(result.rows_affected())
}

pub async fn set_media_meta(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
    width: i32,
    height: i32,
    taken_at: Option<DateTime<Utc>>,
    has_thumbnail: bool,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE nodes SET width = $3, height = $4, taken_at = $5, has_thumbnail = $6 \
         WHERE id = $1 AND owner_id = $2",
    )
    .bind(id)
    .bind(owner_id)
    .bind(width)
    .bind(height)
    .bind(taken_at)
    .bind(has_thumbnail)
    .execute(pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}

pub async fn search_by_name(
    pool: &Db,
    owner_id: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<Node>, AppError> {
    let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
    let sql = format!(
        "SELECT {NODE_COLS} FROM nodes \
         WHERE owner_id = $1 AND trashed_at IS NULL AND name ILIKE $2 \
         ORDER BY (kind = 'folder') DESC, lower(name) ASC LIMIT $3"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(owner_id)
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn find_duplicates(pool: &Db, owner_id: &str) -> Result<Vec<Node>, AppError> {
    let sql = format!(
        "SELECT {NODE_COLS} FROM nodes \
         WHERE owner_id = $1 AND kind = 'file' AND trashed_at IS NULL AND content_hash IS NOT NULL \
         AND content_hash IN ( \
            SELECT content_hash FROM nodes \
            WHERE owner_id = $1 AND kind = 'file' AND trashed_at IS NULL AND content_hash IS NOT NULL \
            GROUP BY content_hash HAVING count(*) > 1 \
         ) \
         ORDER BY content_hash ASC, created_at ASC"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(owner_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}

#[derive(Debug, sqlx::FromRow)]
pub struct PurgedBlob {
    pub storage_key: Option<String>,
    pub has_thumbnail: bool,
}

pub async fn purge_node(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
) -> Result<(Vec<PurgedBlob>, i64), AppError> {
    let mut tx = pool.begin().await.map_err(AppError::from_db)?;

    let blobs = sqlx::query_as::<_, PurgedBlob>(
        "WITH RECURSIVE subtree AS (
            SELECT id FROM nodes WHERE id = $1 AND owner_id = $2
            UNION ALL
            SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
         )
         SELECT storage_key, has_thumbnail FROM nodes \
         WHERE id IN (SELECT id FROM subtree) AND kind = 'file' AND storage_key IS NOT NULL",
    )
    .bind(id)
    .bind(owner_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::from_db)?;

    let freed: i64 = sqlx::query_scalar(
        "WITH RECURSIVE subtree AS (
            SELECT id FROM nodes WHERE id = $1 AND owner_id = $2
            UNION ALL
            SELECT n.id FROM nodes n JOIN subtree s ON n.parent_id = s.id
         )
         SELECT COALESCE(SUM(size_bytes), 0)::BIGINT FROM nodes \
         WHERE id IN (SELECT id FROM subtree) AND kind = 'file'",
    )
    .bind(id)
    .bind(owner_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(AppError::from_db)?;

    sqlx::query("DELETE FROM nodes WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::from_db)?;

    release_quota(&mut tx, owner_id, freed).await?;

    tx.commit().await.map_err(AppError::from_db)?;
    Ok((blobs, freed))
}

async fn release_quota(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    owner_id: &str,
    freed: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE drive_users SET used_bytes = GREATEST(used_bytes - $2, 0) WHERE user_id = $1",
    )
    .bind(owner_id)
    .bind(freed)
    .execute(&mut **tx)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}

pub async fn purge_trash(pool: &Db, owner_id: &str) -> Result<(Vec<PurgedBlob>, i64), AppError> {
    let mut tx = pool.begin().await.map_err(AppError::from_db)?;

    let blobs = sqlx::query_as::<_, PurgedBlob>(
        "SELECT storage_key, has_thumbnail FROM nodes \
         WHERE owner_id = $1 AND trashed_at IS NOT NULL AND kind = 'file' AND storage_key IS NOT NULL",
    )
    .bind(owner_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::from_db)?;

    let freed: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(size_bytes), 0)::BIGINT FROM nodes \
         WHERE owner_id = $1 AND trashed_at IS NOT NULL AND kind = 'file'",
    )
    .bind(owner_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(AppError::from_db)?;

    sqlx::query("DELETE FROM nodes WHERE owner_id = $1 AND trashed_at IS NOT NULL")
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::from_db)?;

    release_quota(&mut tx, owner_id, freed).await?;

    tx.commit().await.map_err(AppError::from_db)?;
    Ok((blobs, freed))
}

pub async fn list_gallery(pool: &Db, owner_id: &str) -> Result<Vec<Node>, AppError> {
    let sql = format!(
        "SELECT {NODE_COLS} FROM nodes \
         WHERE owner_id = $1 AND is_media = TRUE AND kind = 'file' AND trashed_at IS NULL \
         ORDER BY COALESCE(taken_at, created_at) DESC, lower(name) ASC"
    );
    sqlx::query_as::<_, Node>(&sql)
        .bind(owner_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}
