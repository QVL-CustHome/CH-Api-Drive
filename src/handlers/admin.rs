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
    let result = state
        .http_client
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

fn build_admin_user_views(
    users: Vec<DriveUserAdmin>,
    identities: &HashMap<String, (String, String)>,
) -> Vec<AdminUserView> {
    users
        .into_iter()
        .map(|u| {
            let identity = identities.get(&u.user_id);
            AdminUserView {
                user_id: u.user_id,
                quota_bytes: u.quota_bytes,
                used_bytes: u.used_bytes,
                created_at: u.created_at,
                name: identity.map(|(name, _)| name.clone()),
                email: identity.map(|(_, email)| email.clone()),
            }
        })
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
    Ok(Json(build_admin_user_views(users, &identities)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_user(user_id: &str) -> DriveUserAdmin {
        DriveUserAdmin {
            user_id: user_id.to_string(),
            quota_bytes: 1_000,
            used_bytes: 250,
            created_at: DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    fn identity_map(entries: &[(&str, &str, &str)]) -> HashMap<String, (String, String)> {
        entries
            .iter()
            .map(|(id, name, email)| {
                (id.to_string(), (name.to_string(), email.to_string()))
            })
            .collect()
    }

    #[test]
    fn keeps_all_users_when_no_identity_resolved() {
        let users = vec![sample_user("u1"), sample_user("u2")];
        let identities = HashMap::new();

        let views = build_admin_user_views(users, &identities);

        assert_eq!(views.len(), 2);
        assert_eq!(views[0].user_id, "u1");
        assert_eq!(views[1].user_id, "u2");
        assert!(views.iter().all(|v| v.name.is_none() && v.email.is_none()));
    }

    #[test]
    fn fills_identity_fields_when_resolved() {
        let users = vec![sample_user("u1")];
        let identities = identity_map(&[("u1", "Alice", "alice@example.test")]);

        let views = build_admin_user_views(users, &identities);

        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name.as_deref(), Some("Alice"));
        assert_eq!(views[0].email.as_deref(), Some("alice@example.test"));
    }

    #[test]
    fn keeps_unresolved_users_with_null_identity_on_partial_resolution() {
        let users = vec![sample_user("u1"), sample_user("u2"), sample_user("u3")];
        let identities = identity_map(&[("u2", "Bob", "bob@example.test")]);

        let views = build_admin_user_views(users, &identities);

        assert_eq!(views.len(), 3);

        let u1 = views.iter().find(|v| v.user_id == "u1").unwrap();
        assert!(u1.name.is_none() && u1.email.is_none());

        let u2 = views.iter().find(|v| v.user_id == "u2").unwrap();
        assert_eq!(u2.name.as_deref(), Some("Bob"));
        assert_eq!(u2.email.as_deref(), Some("bob@example.test"));

        let u3 = views.iter().find(|v| v.user_id == "u3").unwrap();
        assert!(u3.name.is_none() && u3.email.is_none());
    }

    #[test]
    fn preserves_quota_fields_regardless_of_identity() {
        let users = vec![sample_user("u1")];
        let identities = HashMap::new();

        let views = build_admin_user_views(users, &identities);

        assert_eq!(views[0].quota_bytes, 1_000);
        assert_eq!(views[0].used_bytes, 250);
    }
}
