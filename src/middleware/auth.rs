use crate::error::AppError;
use crate::services::storage::is_object_id;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use ch_auth_jwt::{Claims, extract_token};

pub struct DriveUser(pub Claims);

impl DriveUser {
    pub fn id(&self) -> &str {
        &self.0.sub
    }
}

impl FromRequestParts<AppState> for DriveUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let claims = authenticate(parts, state)?;
        Ok(DriveUser(claims))
    }
}

pub struct DriveAdmin(pub Claims);

impl FromRequestParts<AppState> for DriveAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let claims = authenticate(parts, state)?;
        if !claims.roles.iter().any(|r| r == "drive_admin") {
            return Err(AppError::Forbidden("Rôle administrateur Drive requis."));
        }
        Ok(DriveAdmin(claims))
    }
}

fn authenticate(parts: &Parts, state: &AppState) -> Result<Claims, AppError> {
    let authorization = header_value(parts, header::AUTHORIZATION);
    let cookie = header_value(parts, header::COOKIE);
    let token = extract_token(authorization, cookie, &state.cookie_name).ok_or(AppError::InvalidToken)?;
    let claims = state.jwt.decode(&token).map_err(|_| AppError::InvalidToken)?;
    if !is_object_id(&claims.sub) {
        return Err(AppError::InvalidToken);
    }
    Ok(claims)
}

fn header_value(parts: &Parts, name: header::HeaderName) -> Option<&str> {
    parts.headers.get(name)?.to_str().ok()
}
