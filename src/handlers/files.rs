use crate::domain::MediaType;
use crate::error::AppError;
use crate::middleware::auth::DriveUser;
use crate::repository::drive_users::{self, DriveUser as DriveUserRow};
use crate::repository::nodes::{self, Crumb, Node, NodeDto};
use crate::services::storage::FsStorage;
use crate::state::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ParentQuery {
    parent: Option<Uuid>,
}

#[derive(Serialize)]
pub struct ListResponse {
    parent_id: Uuid,
    ancestors: Vec<Crumb>,
    items: Vec<NodeDto>,
}

#[derive(Deserialize)]
pub struct CreateFolderBody {
    parent_id: Option<Uuid>,
    name: String,
}

#[derive(Deserialize)]
pub struct PatchBody {
    name: Option<String>,
    parent_id: Option<Uuid>,
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

fn validate_name(raw: &str) -> Result<String, AppError> {
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
    if name.chars().any(|c| matches!(c, '/' | '\\') || c.is_control()) {
        return Err(AppError::Validation(
            "Le nom contient des caractères interdits.".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn resolve_mime(file_name: &str, declared: Option<&str>) -> Option<String> {
    if let Some(guessed) = mime_guess::from_path(file_name).first_raw() {
        return Some(guessed.to_string());
    }
    declared
        .map(str::trim)
        .filter(|ct| is_well_formed_mime(ct))
        .map(|ct| ct.to_ascii_lowercase())
}

fn is_well_formed_mime(value: &str) -> bool {
    match value.split_once('/') {
        Some((kind, sub)) => {
            !kind.is_empty()
                && !sub.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '+' | '-'))
        }
        None => false,
    }
}

fn classify_media(mime: Option<&str>) -> (bool, Option<MediaType>) {
    match mime.and_then(MediaType::from_mime) {
        Some(media_type) => (true, Some(media_type)),
        None => (false, None),
    }
}

fn content_disposition(name: &str) -> HeaderValue {
    let safe: String = name
        .chars()
        .map(|c| if c == '"' || c.is_control() { '_' } else { c })
        .collect();
    HeaderValue::from_str(&format!("inline; filename=\"{safe}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("inline"))
}

pub async fn list(
    State(state): State<AppState>,
    user: DriveUser,
    Query(q): Query<ParentQuery>,
) -> Result<Json<ListResponse>, AppError> {
    let du = current_user(&state, &user).await?;
    let parent_id = resolve_parent(&state, user.id(), &du, q.parent).await?;
    let children = nodes::list_children(&state.db, user.id(), parent_id, false).await?;
    let ancestors = nodes::ancestors(&state.db, user.id(), parent_id).await?;
    Ok(Json(ListResponse {
        parent_id,
        ancestors,
        items: children.iter().map(|n| n.to_dto()).collect(),
    }))
}

pub async fn list_trash(
    State(state): State<AppState>,
    user: DriveUser,
) -> Result<Json<Vec<NodeDto>>, AppError> {
    current_user(&state, &user).await?;
    let items = nodes::list_trashed(&state.db, user.id()).await?;
    Ok(Json(items.iter().map(|n| n.to_dto()).collect()))
}

pub async fn create_folder(
    State(state): State<AppState>,
    user: DriveUser,
    body: Result<Json<CreateFolderBody>, JsonRejection>,
) -> Result<Response, AppError> {
    let Json(body) = body?;
    let du = current_user(&state, &user).await?;
    let parent_id = resolve_parent(&state, user.id(), &du, body.parent_id).await?;
    let name = validate_name(&body.name)?;
    let node = nodes::create_folder(&state.db, user.id(), parent_id, &name).await?;
    Ok((StatusCode::CREATED, Json(node.to_dto())).into_response())
}

pub async fn upload(
    State(state): State<AppState>,
    user: DriveUser,
    Query(q): Query<ParentQuery>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let du = current_user(&state, &user).await?;
    let parent_id = resolve_parent(&state, user.id(), &du, q.parent).await?;

    let field = loop {
        match multipart
            .next_field()
            .await
            .map_err(|_| AppError::Validation("Requête multipart invalide.".to_string()))?
        {
            Some(f) if f.name() == Some("file") => break f,
            Some(_) => continue,
            None => {
                return Err(AppError::Validation(
                    "Champ « file » manquant dans la requête.".to_string(),
                ));
            }
        }
    };

    let original_name = field
        .file_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "fichier".to_string());
    let field_ct = field.content_type().map(|s| s.to_string());

    let node_id = Uuid::new_v4();
    let key = FsStorage::build_key(user.id(), node_id)
        .map_err(|_| AppError::Forbidden("Identité de stockage invalide."))?;
    let remaining = (du.quota_bytes - du.used_bytes).max(0);

    let mut writer = state
        .storage
        .create_writer(&key)
        .await
        .map_err(|_| AppError::Internal)?;
    let mut hasher = Sha256::new();
    let mut size: i64 = 0;
    let mut over_quota = false;

    let mut field = field;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|_| AppError::Validation("Lecture du flux échouée.".to_string()))?
    {
        size += chunk.len() as i64;
        if size > remaining {
            over_quota = true;
            break;
        }
        hasher.update(&chunk);
        writer
            .write_all(&chunk)
            .await
            .map_err(|_| AppError::Internal)?;
    }
    writer.flush().await.ok();
    drop(writer);

    if over_quota {
        let _ = state.storage.delete(&key).await;
        return Err(AppError::QuotaExceeded);
    }

    let hash = format!("{:x}", hasher.finalize());
    let mime = resolve_mime(&original_name, field_ct.as_deref());
    let (is_media, media_type) = classify_media(mime.as_deref());
    let name = nodes::free_name(&state.db, user.id(), parent_id, &original_name).await?;

    let insert = async {
        let mut tx = state.db.begin().await.map_err(AppError::from_db)?;
        let (quota, used) = drive_users::quota_used_for_update(&mut tx, user.id())
            .await
            .map_err(AppError::from_db)?;
        if used + size > quota {
            tx.rollback().await.ok();
            return Err(AppError::QuotaExceeded);
        }
        let node = nodes::insert_file(
            &mut tx,
            user.id(),
            parent_id,
            &name,
            mime.as_deref(),
            size,
            &key,
            &hash,
            is_media,
            media_type,
        )
        .await?;
        drive_users::add_used_bytes(&mut tx, user.id(), size)
            .await
            .map_err(AppError::from_db)?;
        tx.commit().await.map_err(AppError::from_db)?;
        Ok(node)
    }
    .await;

    match insert {
        Ok(node) => {
            let node = if media_type == Some(MediaType::Image) {
                generate_thumbnail(&state, user.id(), node.id, &key)
                    .await
                    .unwrap_or(node)
            } else {
                node
            };
            Ok((StatusCode::CREATED, Json(node.to_dto())).into_response())
        }
        Err(e) => {
            let _ = state.storage.delete(&key).await;
            Err(e)
        }
    }
}

async fn generate_thumbnail(
    state: &AppState,
    owner: &str,
    node_id: Uuid,
    blob_key: &str,
) -> Option<Node> {
    let mut file = state.storage.open(blob_key).await.ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await.ok()?;
    let meta = tokio::task::spawn_blocking(move || crate::services::media::process_image(&bytes))
        .await
        .ok()??;
    let thumb_key = FsStorage::thumb_key(blob_key);
    state.storage.write_bytes(&thumb_key, &meta.thumbnail).await.ok()?;
    nodes::set_media_meta(
        &state.db,
        owner,
        node_id,
        meta.width,
        meta.height,
        meta.taken_at,
        true,
    )
    .await
    .ok()?;
    nodes::get(&state.db, owner, node_id).await.ok()?
}

pub async fn thumbnail(
    State(state): State<AppState>,
    user: DriveUser,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let node = nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Fichier introuvable."))?;
    if !node.has_thumbnail {
        return Err(AppError::NotFound("Vignette indisponible."));
    }
    let blob_key = node
        .storage_key
        .as_deref()
        .ok_or(AppError::NotFound("Vignette indisponible."))?;
    let thumb_key = FsStorage::thumb_key(blob_key);
    let file = state
        .storage
        .open(&thumb_key)
        .await
        .map_err(|_| AppError::NotFound("Vignette indisponible."))?;
    let stream = ReaderStream::new(file);
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    Ok(resp)
}

pub async fn gallery(
    State(state): State<AppState>,
    user: DriveUser,
) -> Result<Json<Vec<NodeDto>>, AppError> {
    current_user(&state, &user).await?;
    let items = nodes::list_gallery(&state.db, user.id()).await?;
    Ok(Json(items.iter().map(|n| n.to_dto()).collect()))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
}

pub async fn search(
    State(state): State<AppState>,
    user: DriveUser,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<NodeDto>>, AppError> {
    current_user(&state, &user).await?;
    let term = q.q.trim();
    if term.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let items = nodes::search_by_name(&state.db, user.id(), term, 200).await?;
    Ok(Json(items.iter().map(|n| n.to_dto()).collect()))
}

pub async fn duplicates(
    State(state): State<AppState>,
    user: DriveUser,
) -> Result<Json<Vec<NodeDto>>, AppError> {
    current_user(&state, &user).await?;
    let items = nodes::find_duplicates(&state.db, user.id()).await?;
    Ok(Json(items.iter().map(|n| n.to_dto()).collect()))
}

async fn delete_blobs(state: &AppState, blobs: &[nodes::PurgedBlob]) {
    for blob in blobs {
        if let Some(key) = blob.storage_key.as_deref() {
            let _ = state.storage.delete(key).await;
            if blob.has_thumbnail {
                let _ = state.storage.delete(&FsStorage::thumb_key(key)).await;
            }
        }
    }
}

pub async fn purge_node(
    State(state): State<AppState>,
    user: DriveUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let du = current_user(&state, &user).await?;
    if id == du.root_node_id {
        return Err(AppError::Forbidden(
            "Le dossier racine ne peut pas être supprimé.",
        ));
    }
    nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Élément introuvable."))?;
    let (blobs, _freed) = nodes::purge_node(&state.db, user.id(), id).await?;
    delete_blobs(&state, &blobs).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn purge_trash(
    State(state): State<AppState>,
    user: DriveUser,
) -> Result<StatusCode, AppError> {
    current_user(&state, &user).await?;
    let (blobs, _freed) = nodes::purge_trash(&state.db, user.id()).await?;
    delete_blobs(&state, &blobs).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download(
    State(state): State<AppState>,
    user: DriveUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let node = nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Fichier introuvable."))?;
    if node.is_folder() {
        return Err(AppError::Validation(
            "Impossible de télécharger un dossier.".to_string(),
        ));
    }
    let key = node
        .storage_key
        .as_deref()
        .ok_or(AppError::NotFound("Contenu introuvable."))?;
    let meta = state
        .storage
        .metadata(key)
        .await
        .map_err(|_| AppError::NotFound("Contenu introuvable."))?;
    let total = meta.len();
    let mime = node
        .mime
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let mime_header =
        HeaderValue::from_str(&mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range(s, total));

    let mut file = state
        .storage
        .open(key)
        .await
        .map_err(|_| AppError::NotFound("Contenu introuvable."))?;

    match range {
        Some((start, end)) => {
            let len = end - start + 1;
            file.seek(SeekFrom::Start(start))
                .await
                .map_err(|_| AppError::Internal)?;
            let stream = ReaderStream::new(file.take(len));
            let mut resp = Response::new(Body::from_stream(stream));
            *resp.status_mut() = StatusCode::PARTIAL_CONTENT;
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, mime_header);
            h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
            h.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
                    .unwrap_or(HeaderValue::from_static("bytes */0")),
            );
            h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            h.insert(header::CONTENT_DISPOSITION, content_disposition(&node.name));
            Ok(resp)
        }
        None => {
            let stream = ReaderStream::new(file);
            let mut resp = Response::new(Body::from_stream(stream));
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, mime_header);
            h.insert(header::CONTENT_LENGTH, HeaderValue::from(total));
            h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            h.insert(header::CONTENT_DISPOSITION, content_disposition(&node.name));
            Ok(resp)
        }
    }
}

fn parse_range(raw: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = raw.strip_prefix("bytes=")?;
    let (s, e) = spec.split_once('-')?;
    let (start, end) = match (s.trim(), e.trim()) {
        ("", "") => return None,
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            let n = n.min(total);
            (total - n, total - 1)
        }
        (start, "") => (start.parse().ok()?, total - 1),
        (start, end) => {
            let st: u64 = start.parse().ok()?;
            let en: u64 = end.parse().ok()?;
            (st, en.min(total - 1))
        }
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end))
}

pub async fn patch_node(
    State(state): State<AppState>,
    user: DriveUser,
    Path(id): Path<Uuid>,
    body: Result<Json<PatchBody>, JsonRejection>,
) -> Result<Json<NodeDto>, AppError> {
    let Json(body) = body?;
    let du = current_user(&state, &user).await?;

    let mut current = nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Élément introuvable."))?;

    if let Some(new_parent) = body.parent_id {
        let target = resolve_parent(&state, user.id(), &du, Some(new_parent)).await?;
        if target == current.id {
            return Err(AppError::Validation(
                "Un élément ne peut pas être son propre dossier.".to_string(),
            ));
        }
        if current.is_folder()
            && nodes::is_descendant_or_self(&state.db, user.id(), current.id, target).await?
        {
            return Err(AppError::Validation(
                "Impossible de déplacer un dossier dans lui-même ou un de ses sous-dossiers."
                    .to_string(),
            ));
        }
        current = nodes::move_node(&state.db, user.id(), current.id, target).await?;
    }

    if let Some(name) = body.name {
        let name = validate_name(&name)?;
        current = nodes::rename(&state.db, user.id(), current.id, &name).await?;
    }

    Ok(Json(current.to_dto()))
}

pub async fn trash(
    State(state): State<AppState>,
    user: DriveUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let du = current_user(&state, &user).await?;
    if id == du.root_node_id {
        return Err(AppError::Forbidden(
            "Le dossier racine ne peut pas être supprimé.",
        ));
    }
    nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Élément introuvable."))?;
    nodes::trash_subtree(&state.db, user.id(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn restore(
    State(state): State<AppState>,
    user: DriveUser,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeDto>, AppError> {
    current_user(&state, &user).await?;
    nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Élément introuvable."))?;
    nodes::restore_subtree(&state.db, user.id(), id).await?;
    let node = nodes::get(&state.db, user.id(), id)
        .await?
        .ok_or(AppError::NotFound("Élément introuvable."))?;
    Ok(Json(node.to_dto()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_trims_and_accepts() {
        assert_eq!(validate_name("  photo.jpg  ").unwrap(), "photo.jpg");
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(matches!(validate_name("   "), Err(AppError::Validation(_))));
    }

    #[test]
    fn validate_name_rejects_too_long() {
        let long = "a".repeat(256);
        assert!(matches!(validate_name(&long), Err(AppError::Validation(_))));
    }

    #[test]
    fn validate_name_rejects_dot_segments() {
        assert!(matches!(validate_name("."), Err(AppError::Validation(_))));
        assert!(matches!(validate_name(".."), Err(AppError::Validation(_))));
    }

    #[test]
    fn validate_name_rejects_path_separators() {
        assert!(matches!(
            validate_name("a/b"),
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            validate_name("a\\b"),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn validate_name_rejects_control_chars() {
        assert!(matches!(
            validate_name("a\tb"),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn parse_range_full_suffix() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
    }

    #[test]
    fn parse_range_open_ended() {
        assert_eq!(parse_range("bytes=100-", 1000), Some((100, 999)));
    }

    #[test]
    fn parse_range_suffix_length() {
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
    }

    #[test]
    fn parse_range_suffix_larger_than_total() {
        assert_eq!(parse_range("bytes=-5000", 1000), Some((0, 999)));
    }

    #[test]
    fn parse_range_clamps_end_to_total() {
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
    }

    #[test]
    fn parse_range_rejects_invalid() {
        assert_eq!(parse_range("items=0-99", 1000), None);
        assert_eq!(parse_range("bytes=abc", 1000), None);
        assert_eq!(parse_range("bytes=-", 1000), None);
        assert_eq!(parse_range("bytes=0-99", 0), None);
    }

    #[test]
    fn parse_range_rejects_start_beyond_total() {
        assert_eq!(parse_range("bytes=2000-3000", 1000), None);
    }

    #[test]
    fn classify_media_detects_image_and_video() {
        assert_eq!(classify_media(Some("image/png")), (true, Some(MediaType::Image)));
        assert_eq!(classify_media(Some("video/mp4")), (true, Some(MediaType::Video)));
        assert_eq!(classify_media(Some("application/pdf")), (false, None));
        assert_eq!(classify_media(None), (false, None));
    }

    #[test]
    fn resolve_mime_prefers_extension() {
        assert_eq!(
            resolve_mime("photo.png", Some("application/octet-stream")).as_deref(),
            Some("image/png")
        );
    }

    #[test]
    fn resolve_mime_falls_back_to_valid_declared() {
        assert_eq!(
            resolve_mime("blob", Some("Application/JSON")).as_deref(),
            Some("application/json")
        );
    }

    #[test]
    fn resolve_mime_rejects_malformed_declared() {
        assert_eq!(resolve_mime("blob", Some("not-a-mime")), None);
        assert_eq!(resolve_mime("blob", Some("image/<script>")), None);
        assert_eq!(resolve_mime("blob", None), None);
    }

    #[test]
    fn is_well_formed_mime_checks() {
        assert!(is_well_formed_mime("image/png"));
        assert!(is_well_formed_mime("application/vnd.api+json"));
        assert!(!is_well_formed_mime("image"));
        assert!(!is_well_formed_mime("/png"));
        assert!(!is_well_formed_mime("image/"));
        assert!(!is_well_formed_mime("image/png; charset=utf-8"));
    }
}
