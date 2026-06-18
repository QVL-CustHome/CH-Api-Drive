use crate::error::AppError;
use crate::middleware::auth::DriveUser;
use crate::repository::drive_users;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;

#[derive(Serialize)]
pub struct StorageResponse {
    pub quota_bytes: i64,
    pub used_bytes: i64,
}

pub async fn me_storage(
    State(state): State<AppState>,
    user: DriveUser,
) -> Result<Json<StorageResponse>, AppError> {
    let drive_user =
        drive_users::ensure_user(&state.db, user.id(), state.default_quota_bytes).await?;
    Ok(Json(StorageResponse {
        quota_bytes: drive_user.quota_bytes,
        used_bytes: drive_user.used_bytes,
    }))
}
