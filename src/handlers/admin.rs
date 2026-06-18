use crate::error::AppError;
use crate::middleware::auth::DriveAdmin;
use crate::repository::drive_users::{self, DriveUserAdmin};
use crate::state::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
struct ResolvedUser {
    user_id: String,
    name: String,
    email: String,
}

#[derive(Serialize)]
pub struct AdminUserView {
    user_id: String,
    quota_bytes: i64,
    used_bytes: i64,
    created_at: DateTime<Utc>,
    name: Option<String>,
    email: Option<String>,
}

async fn resolve_identities(state: &AppState, ids: &[String]) -> HashMap<String, (String, String)> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let url = format!("{}/internal/users/resolve", state.auth_internal_url);
    let result = reqwest::Client::new()
        .post(&url)
        .header("x-internal-secret", &state.internal_secret)
        .json(&serde_json::json!({ "ids": ids }))
        .send()
        .await
        .and_then(|r| r.error_for_status());

    let resolved: Vec<ResolvedUser> = match result {
        Ok(resp) => resp.json().await.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "résolution des identités impossible");
            Vec::new()
        }
    };
    resolved
        .into_iter()
        .map(|u| (u.user_id, (u.name, u.email)))
        .collect()
}

pub async fn list_users(
    State(state): State<AppState>,
    _admin: DriveAdmin,
) -> Result<Json<Vec<AdminUserView>>, AppError> {
    let users = drive_users::list_all(&state.db)
        .await
        .map_err(AppError::from_db)?;
    let ids: Vec<String> = users.iter().map(|u| u.user_id.clone()).collect();
    let identities = resolve_identities(&state, &ids).await;

    let view: Vec<AdminUserView> = users
        .into_iter()
        .filter_map(|u| {
            let (name, email) = identities.get(&u.user_id)?;
            Some(AdminUserView {
                user_id: u.user_id,
                quota_bytes: u.quota_bytes,
                used_bytes: u.used_bytes,
                created_at: u.created_at,
                name: Some(name.clone()),
                email: Some(email.clone()),
            })
        })
        .collect();
    Ok(Json(view))
}

#[derive(Deserialize)]
pub struct SetQuotaBody {
    quota_bytes: i64,
}

pub async fn set_quota(
    State(state): State<AppState>,
    _admin: DriveAdmin,
    Path(id): Path<String>,
    body: Result<Json<SetQuotaBody>, JsonRejection>,
) -> Result<Json<DriveUserAdmin>, AppError> {
    let Json(body) = body?;
    if body.quota_bytes <= 0 {
        return Err(AppError::Validation(
            "Le quota doit être strictement positif.".to_string(),
        ));
    }
    let updated = drive_users::set_quota(&state.db, &id, body.quota_bytes)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound("Utilisateur Drive introuvable."))?;
    Ok(Json(updated))
}

pub async fn recompute_used(
    State(state): State<AppState>,
    _admin: DriveAdmin,
    Path(id): Path<String>,
) -> Result<Json<DriveUserAdmin>, AppError> {
    let updated = drive_users::recompute_used(&state.db, &id)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound("Utilisateur Drive introuvable."))?;
    Ok(Json(updated))
}
