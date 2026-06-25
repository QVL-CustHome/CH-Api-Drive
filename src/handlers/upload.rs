use crate::domain::MediaType;
use crate::error::AppError;
use crate::middleware::auth::DriveUser;
use crate::repository::drive_users::{self, DriveUser as DriveUserRow};
use crate::repository::nodes::{self, NodeDto};
use crate::repository::upload_sessions::{self, NewUploadSession, UploadSession, UploadState};
use crate::services::storage::FsStorage;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_CHUNK_BYTES: i64 = 16 * 1024 * 1024;
const MAX_DECLARED_SIZE_BYTES: i64 = 10 * 1024 * 1024 * 1024;
const SESSION_TTL_HOURS: i64 = 24;

#[derive(Deserialize)]
pub struct OpenUploadBody {
    parent_id: Option<Uuid>,
    file_name: String,
    declared_size: i64,
    chunk_size: i32,
    declared_mime: Option<String>,
    checksum: Option<String>,
}

#[derive(Serialize)]
pub struct UploadSessionResponse {
    session_id: Uuid,
    parent_id: Uuid,
    file_name: String,
    declared_size: i64,
    reserved_bytes: i64,
    chunk_size: i32,
    chunk_count: i32,
    state: UploadState,
    received_bytes: i64,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Serialize)]
pub struct PutChunkResponse {
    session_id: Uuid,
    chunk_index: i32,
    received_bytes: i64,
    declared_size: i64,
}

impl UploadSessionResponse {
    fn from_session(session: &UploadSession) -> Self {
        Self {
            session_id: session.id,
            parent_id: session.parent_id,
            file_name: session.file_name.clone(),
            declared_size: session.declared_size,
            reserved_bytes: session.reserved_bytes,
            chunk_size: session.chunk_size,
            chunk_count: session.chunk_count,
            state: session.state,
            received_bytes: session.received_bytes,
            expires_at: session.expires_at,
        }
    }
}

async fn current_user(state: &AppState, user: &DriveUser) -> Result<DriveUserRow, AppError> {
    drive_users::ensure_user(&state.db, user.id(), state.default_quota_bytes)
        .await
        .map_err(AppError::from_db)
}

async fn resolve_parent(
    state: &AppState,
    owner: &str,
    du: &DriveUserRow,
    parent: Option<Uuid>,
) -> Result<Uuid, AppError> {
    match parent {
        None => Ok(du.root_node_id),
        Some(pid) if pid == du.root_node_id => Ok(pid),
        Some(pid) => {
            let node = nodes::get(&state.db, owner, pid)
                .await?
                .ok_or(AppError::NotFound("Dossier parent introuvable."))?;
            if !node.is_folder() {
                return Err(AppError::Validation(
                    "L'élément parent n'est pas un dossier.".to_string(),
                ));
            }
            if node.trashed_at.is_some() {
                return Err(AppError::Validation(
                    "Le dossier parent est dans la corbeille.".to_string(),
                ));
            }
            Ok(pid)
        }
    }
}

fn validate_file_name(raw: &str) -> Result<String, AppError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::Validation("Le nom est obligatoire.".to_string()));
    }
    if name.len() > 255 {
        return Err(AppError::Validation("Le nom est trop long.".to_string()));
    }
    if name == "." || name == ".." {
        return Err(AppError::Validation("Nom réservé.".to_string()));
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '\\') || c.is_control())
    {
        return Err(AppError::Validation(
            "Le nom contient des caractères interdits.".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn expected_chunk_count(declared_size: i64, chunk_size: i32) -> i32 {
    let chunk = chunk_size as i64;
    let count = (declared_size + chunk - 1) / chunk;
    count.max(1) as i32
}

fn media_type_of(mime: Option<&str>) -> Option<MediaType> {
    mime.and_then(MediaType::from_mime)
}

pub async fn open(
    State(state): State<AppState>,
    user: DriveUser,
    body: Result<Json<OpenUploadBody>, JsonRejection>,
) -> Result<Response, AppError> {
    let Json(body) = body?;
    let du = current_user(&state, &user).await?;
    let parent_id = resolve_parent(&state, user.id(), &du, body.parent_id).await?;
    let file_name = validate_file_name(&body.file_name)?;

    if body.declared_size < 0 {
        return Err(AppError::Validation(
            "La taille déclarée est invalide.".to_string(),
        ));
    }
    if body.declared_size > MAX_DECLARED_SIZE_BYTES {
        return Err(AppError::Validation(
            "La taille déclarée dépasse la limite autorisée.".to_string(),
        ));
    }
    if body.chunk_size <= 0 || body.chunk_size as i64 > MAX_CHUNK_BYTES {
        return Err(AppError::Validation(
            "La taille de chunk est invalide.".to_string(),
        ));
    }

    let node_id = Uuid::new_v4();
    let storage_key = FsStorage::build_key(user.id(), node_id)
        .map_err(|_| AppError::Forbidden("Identité de stockage invalide."))?;
    let tmp_key = format!("{storage_key}.part");
    let chunk_count = expected_chunk_count(body.declared_size, body.chunk_size);
    let expires_at = Utc::now() + Duration::hours(SESSION_TTL_HOURS);

    let mut tx = state.db.begin().await.map_err(AppError::from_db)?;
    let (quota, used) = drive_users::quota_used_for_update(&mut tx, user.id())
        .await
        .map_err(AppError::from_db)?;
    let reserved = upload_sessions::sum_reserved_active(&mut tx, user.id()).await?;
    let remaining = (quota - used - reserved).max(0);
    if body.declared_size > remaining {
        tx.rollback().await.ok();
        return Err(AppError::QuotaExceeded);
    }

    let session = upload_sessions::create_tx(
        &mut tx,
        NewUploadSession {
            owner_id: user.id(),
            parent_id,
            file_name: &file_name,
            declared_mime: body.declared_mime.as_deref(),
            declared_size: body.declared_size,
            reserved_bytes: body.declared_size,
            chunk_size: body.chunk_size,
            chunk_count,
            checksum: body.checksum.as_deref(),
            storage_key: &storage_key,
            tmp_key: &tmp_key,
            expires_at,
        },
    )
    .await?;
    tx.commit().await.map_err(AppError::from_db)?;

    Ok((
        StatusCode::CREATED,
        Json(UploadSessionResponse::from_session(&session)),
    )
        .into_response())
}

async fn active_session(
    state: &AppState,
    owner: &str,
    session_id: Uuid,
) -> Result<UploadSession, AppError> {
    let session = upload_sessions::get(&state.db, owner, session_id)
        .await?
        .ok_or(AppError::NotFound("Session d'upload introuvable."))?;
    if session.state != UploadState::Open {
        return Err(AppError::Conflict(
            "La session d'upload n'est plus ouverte.",
        ));
    }
    if session.expires_at <= Utc::now() {
        return Err(AppError::Conflict("La session d'upload a expiré."));
    }
    Ok(session)
}

async fn completable_session(
    state: &AppState,
    owner: &str,
    session_id: Uuid,
) -> Result<UploadSession, AppError> {
    let session = upload_sessions::get(&state.db, owner, session_id)
        .await?
        .ok_or(AppError::NotFound("Session d'upload introuvable."))?;
    match session.state {
        UploadState::Open if session.expires_at <= Utc::now() => {
            Err(AppError::Conflict("La session d'upload a expiré."))
        }
        UploadState::Open | UploadState::Completing => Ok(session),
        UploadState::Completed => Err(AppError::Conflict(
            "La session d'upload est déjà finalisée.",
        )),
        UploadState::Aborted => Err(AppError::Conflict("La session d'upload a été annulée.")),
    }
}

async fn claim_for_completion(
    state: &AppState,
    owner: &str,
    session: &UploadSession,
) -> Result<UploadSession, AppError> {
    if session.state == UploadState::Completing {
        return Ok(session.clone());
    }
    upload_sessions::transition_state(
        &state.db,
        owner,
        session.id,
        UploadState::Open,
        UploadState::Completing,
    )
    .await?
    .ok_or(AppError::Conflict(
        "La session d'upload n'est plus ouverte.",
    ))
}

fn chunk_offset(session: &UploadSession, chunk_index: i32) -> u64 {
    chunk_index as u64 * session.chunk_size as u64
}

pub async fn put_chunk(
    State(state): State<AppState>,
    user: DriveUser,
    Path((session_id, chunk_index)): Path<(Uuid, i32)>,
    body: Bytes,
) -> Result<Json<PutChunkResponse>, AppError> {
    let session = active_session(&state, user.id(), session_id).await?;

    if chunk_index < 0 || chunk_index >= session.chunk_count {
        return Err(AppError::Validation(
            "Index de chunk hors limites.".to_string(),
        ));
    }
    let incoming = body.len() as i64;
    if incoming == 0 {
        return Err(AppError::Validation("Chunk vide.".to_string()));
    }
    if incoming > session.chunk_size as i64 {
        return Err(AppError::PayloadTooLarge(
            "Le chunk dépasse la taille déclarée à l'ouverture de la session.",
        ));
    }

    let previous = upload_sessions::chunk_size(&state.db, session_id, chunk_index)
        .await?
        .unwrap_or(0);
    let projected = session.received_bytes - previous + incoming;
    if projected > session.declared_size {
        return Err(AppError::QuotaExceeded);
    }

    let offset = chunk_offset(&session, chunk_index);
    state
        .storage
        .write_at(&session.tmp_key, offset, &body)
        .await
        .map_err(|_| AppError::Internal)?;

    upload_sessions::record_chunk(&state.db, session_id, chunk_index, incoming as i32, None)
        .await?;

    let delta = incoming - previous;
    let received_bytes =
        upload_sessions::add_received_bytes(&state.db, user.id(), session_id, delta)
            .await?
            .ok_or(AppError::NotFound("Session d'upload introuvable."))?;

    Ok(Json(PutChunkResponse {
        session_id,
        chunk_index,
        received_bytes,
        declared_size: session.declared_size,
    }))
}

pub async fn status(
    State(state): State<AppState>,
    user: DriveUser,
    Path(session_id): Path<Uuid>,
) -> Result<Json<UploadSessionResponse>, AppError> {
    let session = upload_sessions::get(&state.db, user.id(), session_id)
        .await?
        .ok_or(AppError::NotFound("Session d'upload introuvable."))?;
    Ok(Json(UploadSessionResponse::from_session(&session)))
}

pub async fn complete(
    State(state): State<AppState>,
    user: DriveUser,
    Path(session_id): Path<Uuid>,
) -> Result<Response, AppError> {
    let session = completable_session(&state, user.id(), session_id).await?;

    if session.received_bytes != session.declared_size {
        return Err(AppError::Conflict(
            "Tous les octets déclarés n'ont pas été reçus.",
        ));
    }
    let chunk_count = upload_sessions::count_chunks(&state.db, session_id).await?;
    if chunk_count != session.chunk_count as i64 {
        return Err(AppError::Conflict(
            "Tous les chunks attendus n'ont pas été reçus.",
        ));
    }

    let claimed = claim_for_completion(&state, user.id(), &session).await?;

    match materialize(&state, user.id(), &claimed).await {
        Ok(node) => {
            upload_sessions::transition_state(
                &state.db,
                user.id(),
                session_id,
                UploadState::Completing,
                UploadState::Completed,
            )
            .await?;
            Ok((StatusCode::CREATED, Json(node)).into_response())
        }
        Err(error) => {
            upload_sessions::transition_state(
                &state.db,
                user.id(),
                session_id,
                UploadState::Completing,
                UploadState::Open,
            )
            .await
            .ok();
            Err(error)
        }
    }
}

async fn materialize(
    state: &AppState,
    owner: &str,
    session: &UploadSession,
) -> Result<NodeDto, AppError> {
    if let Some(node) = existing_node(state, owner, session).await? {
        return Ok(node);
    }
    state
        .storage
        .finalize(&session.tmp_key, &session.storage_key)
        .await
        .map_err(|_| AppError::Internal)?;
    persist_node(state, owner, session).await
}

async fn existing_node(
    state: &AppState,
    owner: &str,
    session: &UploadSession,
) -> Result<Option<NodeDto>, AppError> {
    let Some(node_id) = session.node_id else {
        return Ok(None);
    };
    Ok(nodes::get(&state.db, owner, node_id)
        .await?
        .map(|node| node.to_dto()))
}

async fn persist_node(
    state: &AppState,
    owner: &str,
    session: &UploadSession,
) -> Result<NodeDto, AppError> {
    let name = nodes::free_name(&state.db, owner, session.parent_id, &session.file_name).await?;
    let media_type = media_type_of(session.declared_mime.as_deref());
    let is_media = media_type.is_some();

    let mut tx = state.db.begin().await.map_err(AppError::from_db)?;
    let (quota, used) = drive_users::quota_used_for_update(&mut tx, owner)
        .await
        .map_err(AppError::from_db)?;
    if used + session.declared_size > quota {
        tx.rollback().await.ok();
        return Err(AppError::QuotaExceeded);
    }
    let node = nodes::insert_file(
        &mut tx,
        owner,
        session.parent_id,
        &name,
        session.declared_mime.as_deref(),
        session.declared_size,
        &session.storage_key,
        session.checksum.as_deref().unwrap_or(""),
        is_media,
        media_type,
    )
    .await?;
    drive_users::add_used_bytes(&mut tx, owner, session.declared_size)
        .await
        .map_err(AppError::from_db)?;
    upload_sessions::set_node_tx(&mut tx, owner, session.id, node.id).await?;
    tx.commit().await.map_err(AppError::from_db)?;

    Ok(node.to_dto())
}

pub async fn abort(
    State(state): State<AppState>,
    user: DriveUser,
    Path(session_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let session = upload_sessions::get(&state.db, user.id(), session_id)
        .await?
        .ok_or(AppError::NotFound("Session d'upload introuvable."))?;

    if session.state == UploadState::Completed {
        return Err(AppError::Conflict(
            "Une session terminée ne peut pas être annulée.",
        ));
    }

    upload_sessions::transition_state(
        &state.db,
        user.id(),
        session_id,
        session.state,
        UploadState::Aborted,
    )
    .await?;

    state.storage.delete(&session.tmp_key).await.ok();

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_chunk_count_rounds_up() {
        assert_eq!(expected_chunk_count(0, 1024), 1);
        assert_eq!(expected_chunk_count(1, 1024), 1);
        assert_eq!(expected_chunk_count(1024, 1024), 1);
        assert_eq!(expected_chunk_count(1025, 1024), 2);
        assert_eq!(expected_chunk_count(4096, 1024), 4);
    }

    #[test]
    fn validate_file_name_trims_and_accepts() {
        assert_eq!(validate_file_name("  video.mp4  ").unwrap(), "video.mp4");
    }

    #[test]
    fn validate_file_name_rejects_separators() {
        assert!(matches!(
            validate_file_name("a/b"),
            Err(AppError::Validation(_))
        ));
    }
}
