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
    #[error("erreur interne")]
    Internal,
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
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
            AppError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let body = json!({ "error": error, "message": self.to_string() });
        (status, Json(body)).into_response()
    }
}
