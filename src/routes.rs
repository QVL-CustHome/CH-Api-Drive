use crate::handlers;
use crate::state::AppState;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post, put};

pub const API_VERSION_PREFIX: &str = "/v1";

pub const UPLOAD_BODY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
pub const API_BODY_LIMIT_BYTES: usize = 1024 * 1024;
pub const CHUNK_BODY_LIMIT_BYTES: usize = 17 * 1024 * 1024;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(operational_routes())
        .nest(API_VERSION_PREFIX, public_routes())
        .merge(public_routes())
        .with_state(state)
}

fn operational_routes() -> Router<AppState> {
    Router::new().route("/health", get(handlers::health::health))
}

fn public_routes() -> Router<AppState> {
    upload_routes().merge(api_routes())
}

fn upload_routes() -> Router<AppState> {
    let multipart = Router::new()
        .route("/files", post(handlers::files::upload))
        .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT_BYTES));

    let chunk = Router::new()
        .route(
            "/uploads/{id}/chunks/{index}",
            put(handlers::upload::put_chunk),
        )
        .layer(DefaultBodyLimit::max(CHUNK_BODY_LIMIT_BYTES));

    let sessions = Router::new()
        .route("/uploads", post(handlers::upload::open))
        .route(
            "/uploads/{id}",
            get(handlers::upload::status).delete(handlers::upload::abort),
        )
        .route("/uploads/{id}/complete", post(handlers::upload::complete))
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES));

    multipart.merge(chunk).merge(sessions)
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/me/storage", get(handlers::storage::me_storage))
        .route("/files", get(handlers::files::list))
        .route("/files/{id}/content", get(handlers::files::download))
        .route("/files/{id}/thumbnail", get(handlers::files::thumbnail))
        .route("/gallery", get(handlers::files::gallery))
        .route("/search", get(handlers::files::search))
        .route("/duplicates", get(handlers::files::duplicates))
        .route("/admin/users", get(handlers::admin::list_users))
        .route("/admin/users/{id}", patch(handlers::admin::set_quota))
        .route(
            "/admin/users/{id}/recompute",
            post(handlers::admin::recompute_used),
        )
        .route("/folders", post(handlers::files::create_folder))
        .route("/trash", get(handlers::files::list_trash))
        .route("/trash/purge", post(handlers::files::purge_trash))
        .route(
            "/nodes/{id}",
            patch(handlers::files::patch_node).delete(handlers::files::purge_node),
        )
        .route("/nodes/{id}/trash", post(handlers::files::trash))
        .route("/nodes/{id}/restore", post(handlers::files::restore))
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES))
}
