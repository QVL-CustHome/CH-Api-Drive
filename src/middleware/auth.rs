use crate::error::AppError;
use crate::services::jwt::Claims;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;

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
        let token = extract_token(parts, &state.cookie_name).ok_or(AppError::InvalidToken)?;
        let claims = state
            .jwt
            .validate(&token)
            .map_err(|_| AppError::InvalidToken)?;
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
        let token = extract_token(parts, &state.cookie_name).ok_or(AppError::InvalidToken)?;
        let claims = state
            .jwt
            .validate(&token)
            .map_err(|_| AppError::InvalidToken)?;
        if !claims.roles.iter().any(|r| r == "drive_admin") {
            return Err(AppError::Forbidden("Rôle administrateur Drive requis."));
        }
        Ok(DriveAdmin(claims))
    }
}

fn extract_token(parts: &Parts, cookie_name: &str) -> Option<String> {
    if let Some(value) = parts.headers.get(header::AUTHORIZATION) {
        let token = value.to_str().ok()?.strip_prefix("Bearer ")?.trim();
        return (!token.is_empty()).then(|| token.to_string());
    }

    let cookies = parts.headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        let value = value.trim();
        (name == cookie_name && !value.is_empty()).then(|| value.to_string())
    })
}
