use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("requête invalide : {0}")]
    Validation(String),
    #[error("token invalide ou expiré")]
    InvalidToken,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    NotFound(&'static str),
    #[error("quota de stockage dépassé")]
    QuotaExceeded,
    #[error("{0}")]
    PayloadTooLarge(&'static str),
    #[error("erreur interne")]
    Internal,
}

pub const BODY_LIMIT_MESSAGE: &str =
    "La taille du corps de la requête dépasse la limite autorisée.";

fn is_length_limit(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(source) = current {
        if source.is::<axum::extract::rejection::LengthLimitError>() {
            return true;
        }
        current = source.source();
    }
    false
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE || is_length_limit(&rejection) {
            return AppError::PayloadTooLarge(BODY_LIMIT_MESSAGE);
        }
        let message = match rejection {
            JsonRejection::JsonDataError(_) => {
                "Données invalides : un ou plusieurs champs sont incorrects ou manquants."
            }
            JsonRejection::JsonSyntaxError(_) => "Le corps de la requête n'est pas un JSON valide.",
            JsonRejection::MissingJsonContentType(_) => {
                "En-tête « Content-Type: application/json » manquant."
            }
            _ => "Requête invalide.",
        };
        AppError::Validation(message.to_string())
    }
}

impl AppError {
    pub fn from_db(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e {
            if db.code().as_deref() == Some("23505") {
                return AppError::Conflict("Un élément du même nom existe déjà à cet emplacement.");
            }
        }
        tracing::error!(error = %e, "Erreur base de données");
        AppError::Internal
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::from_db(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error) = match &self {
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::InvalidToken => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            AppError::QuotaExceeded => (StatusCode::PAYLOAD_TOO_LARGE, "quota_exceeded"),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            AppError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let body = json!({ "error": error, "message": self.to_string() });
        (status, Json(body)).into_response()
    }
}
