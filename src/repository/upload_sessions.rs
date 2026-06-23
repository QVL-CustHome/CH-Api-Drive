use crate::db::Db;
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Postgres;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UploadState {
    Open,
    Completing,
    Completed,
    Aborted,
}

impl UploadState {
    pub fn as_str(self) -> &'static str {
        match self {
            UploadState::Open => "open",
            UploadState::Completing => "completing",
            UploadState::Completed => "completed",
            UploadState::Aborted => "aborted",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("état de session d'upload invalide : {0}")]
pub struct ParseUploadStateError(String);

impl std::str::FromStr for UploadState {
    type Err = ParseUploadStateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "open" => Ok(UploadState::Open),
            "completing" => Ok(UploadState::Completing),
            "completed" => Ok(UploadState::Completed),
            "aborted" => Ok(UploadState::Aborted),
            other => Err(ParseUploadStateError(other.to_string())),
        }
    }
}

impl sqlx::Type<Postgres> for UploadState {
    fn type_info() -> PgTypeInfo {
        <&str as sqlx::Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <&str as sqlx::Type<Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, Postgres> for UploadState {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let raw = <&str as sqlx::Decode<Postgres>>::decode(value)?;
        raw.parse().map_err(Into::into)
    }
}

impl<'q> sqlx::Encode<'q, Postgres> for UploadState {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <&str as sqlx::Encode<Postgres>>::encode(self.as_str(), buf)
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct UploadSession {
    pub id: Uuid,
    pub owner_id: String,
    pub parent_id: Uuid,
    pub file_name: String,
    pub declared_mime: Option<String>,
    pub declared_size: i64,
    pub reserved_bytes: i64,
    pub chunk_size: i32,
    pub chunk_count: i32,
    pub checksum: Option<String>,
    pub storage_key: String,
    pub tmp_key: String,
    pub state: UploadState,
    pub received_bytes: i64,
    pub node_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct UploadChunk {
    pub session_id: Uuid,
    pub chunk_index: i32,
    pub size_bytes: i32,
    pub chunk_sha256: Option<String>,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct NewUploadSession<'a> {
    pub owner_id: &'a str,
    pub parent_id: Uuid,
    pub file_name: &'a str,
    pub declared_mime: Option<&'a str>,
    pub declared_size: i64,
    pub reserved_bytes: i64,
    pub chunk_size: i32,
    pub chunk_count: i32,
    pub checksum: Option<&'a str>,
    pub storage_key: &'a str,
    pub tmp_key: &'a str,
    pub expires_at: DateTime<Utc>,
}

const SESSION_COLS: &str = "id, owner_id, parent_id, file_name, declared_mime, declared_size, \
    reserved_bytes, chunk_size, chunk_count, checksum, storage_key, tmp_key, state, \
    received_bytes, node_id, expires_at, created_at, updated_at";

const CHUNK_COLS: &str = "session_id, chunk_index, size_bytes, chunk_sha256, received_at";

pub async fn create(pool: &Db, new: NewUploadSession<'_>) -> Result<UploadSession, AppError> {
    let id = Uuid::new_v4();
    let sql = format!(
        "INSERT INTO upload_sessions \
         (id, owner_id, parent_id, file_name, declared_mime, declared_size, reserved_bytes, \
          chunk_size, chunk_count, checksum, storage_key, tmp_key, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING {SESSION_COLS}"
    );
    sqlx::query_as::<_, UploadSession>(&sql)
        .bind(id)
        .bind(new.owner_id)
        .bind(new.parent_id)
        .bind(new.file_name)
        .bind(new.declared_mime)
        .bind(new.declared_size)
        .bind(new.reserved_bytes)
        .bind(new.chunk_size)
        .bind(new.chunk_count)
        .bind(new.checksum)
        .bind(new.storage_key)
        .bind(new.tmp_key)
        .bind(new.expires_at)
        .fetch_one(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn get(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
) -> Result<Option<UploadSession>, AppError> {
    let sql =
        format!("SELECT {SESSION_COLS} FROM upload_sessions WHERE id = $1 AND owner_id = $2");
    sqlx::query_as::<_, UploadSession>(&sql)
        .bind(id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn list_by_owner(pool: &Db, owner_id: &str) -> Result<Vec<UploadSession>, AppError> {
    let sql = format!(
        "SELECT {SESSION_COLS} FROM upload_sessions \
         WHERE owner_id = $1 ORDER BY created_at DESC"
    );
    sqlx::query_as::<_, UploadSession>(&sql)
        .bind(owner_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn transition_state(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
    from: UploadState,
    to: UploadState,
) -> Result<Option<UploadSession>, AppError> {
    let sql = format!(
        "UPDATE upload_sessions SET state = $4, updated_at = now() \
         WHERE id = $1 AND owner_id = $2 AND state = $3 RETURNING {SESSION_COLS}"
    );
    sqlx::query_as::<_, UploadSession>(&sql)
        .bind(id)
        .bind(owner_id)
        .bind(from)
        .bind(to)
        .fetch_optional(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn set_node(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
    node_id: Uuid,
) -> Result<Option<UploadSession>, AppError> {
    let sql = format!(
        "UPDATE upload_sessions SET node_id = $3, updated_at = now() \
         WHERE id = $1 AND owner_id = $2 RETURNING {SESSION_COLS}"
    );
    sqlx::query_as::<_, UploadSession>(&sql)
        .bind(id)
        .bind(owner_id)
        .bind(node_id)
        .fetch_optional(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn add_received_bytes(
    pool: &Db,
    owner_id: &str,
    id: Uuid,
    delta: i64,
) -> Result<Option<i64>, AppError> {
    sqlx::query_scalar(
        "UPDATE upload_sessions SET received_bytes = received_bytes + $3, updated_at = now() \
         WHERE id = $1 AND owner_id = $2 RETURNING received_bytes",
    )
    .bind(id)
    .bind(owner_id)
    .bind(delta)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from_db)
}

pub async fn record_chunk(
    pool: &Db,
    session_id: Uuid,
    chunk_index: i32,
    size_bytes: i32,
    chunk_sha256: Option<&str>,
) -> Result<UploadChunk, AppError> {
    let sql = format!(
        "INSERT INTO upload_chunks (session_id, chunk_index, size_bytes, chunk_sha256) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (session_id, chunk_index) DO UPDATE \
         SET size_bytes = EXCLUDED.size_bytes, \
             chunk_sha256 = EXCLUDED.chunk_sha256, \
             received_at = now() \
         RETURNING {CHUNK_COLS}"
    );
    sqlx::query_as::<_, UploadChunk>(&sql)
        .bind(session_id)
        .bind(chunk_index)
        .bind(size_bytes)
        .bind(chunk_sha256)
        .fetch_one(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn list_chunks(pool: &Db, session_id: Uuid) -> Result<Vec<UploadChunk>, AppError> {
    let sql = format!(
        "SELECT {CHUNK_COLS} FROM upload_chunks \
         WHERE session_id = $1 ORDER BY chunk_index ASC"
    );
    sqlx::query_as::<_, UploadChunk>(&sql)
        .bind(session_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from_db)
}

pub async fn count_chunks(pool: &Db, session_id: Uuid) -> Result<i64, AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM upload_chunks WHERE session_id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .map_err(AppError::from_db)?;
    Ok(count)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ExpiredSession {
    pub id: Uuid,
    pub owner_id: String,
    pub tmp_key: String,
    pub reserved_bytes: i64,
    pub state: UploadState,
}

pub async fn find_expired(pool: &Db, limit: i64) -> Result<Vec<ExpiredSession>, AppError> {
    sqlx::query_as::<_, ExpiredSession>(
        "SELECT id, owner_id, tmp_key, reserved_bytes, state \
         FROM upload_sessions \
         WHERE state IN ('open', 'aborted') AND expires_at < now() \
         ORDER BY expires_at ASC \
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(AppError::from_db)
}

pub async fn delete_if_expired(pool: &Db, id: Uuid) -> Result<bool, AppError> {
    let result = sqlx::query(
        "DELETE FROM upload_sessions \
         WHERE id = $1 AND state IN ('open', 'aborted') AND expires_at < now()",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(result.rows_affected() == 1)
}
