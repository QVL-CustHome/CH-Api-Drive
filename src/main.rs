use ch_api_drive::config;
use ch_api_drive::db;
use ch_api_drive::domain::events::EventPublisher;
use ch_api_drive::jobs::upload_gc::{self, GcConfig};
use ch_api_drive::routes;
use ch_api_drive::services::event_publisher_mqtt::{
    MqttEventPublisher, MqttEventPublisherConfig,
};
use ch_api_drive::state::AppState;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let settings = match config::load("config.toml") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Démarrage impossible — configuration invalide : {e}");
            std::process::exit(1);
        }
    };

    init_tracing(&settings.config.server.log_level);

    let pool = match db::connect(&settings.secrets.database_url).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "PostgreSQL injoignable");
            eprintln!("Démarrage impossible — PostgreSQL injoignable : {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = db::migrate(&pool).await {
        tracing::error!(error = %e, "Migrations en échec");
        eprintln!("Démarrage impossible — migrations en échec : {e}");
        std::process::exit(1);
    }

    let event_publisher = match build_event_publisher() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "Configuration du bus d'événements MQTT invalide");
            eprintln!(
                "Démarrage impossible — configuration du bus d'événements MQTT invalide : {e}"
            );
            std::process::exit(1);
        }
    };

    let port = settings.config.server.port;
    let gc_config = GcConfig::new(Duration::from_secs(settings.config.upload_gc.interval_secs))
        .with_batch_size(settings.config.upload_gc.batch_size);
    let state = AppState::new(&settings, pool, event_publisher);

    let _gc_handle = upload_gc::spawn_gc(state.db.clone(), state.storage.clone(), gc_config);

    let app = routes::router(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Démarrage impossible — écoute sur {addr} refusée : {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(%addr, version = env!("CARGO_PKG_VERSION"), "CH-Api-Drive démarré");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("Erreur serveur : {e}");
        std::process::exit(1);
    }
}

fn build_event_publisher() -> Result<Arc<dyn EventPublisher>, String> {
    let config = MqttEventPublisherConfig::from_env().map_err(|e| e.to_string())?;
    let publisher = MqttEventPublisher::new(config).map_err(|e| e.to_string())?;
    Ok(Arc::new(publisher))
}

fn init_tracing(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_new(level.to_lowercase())
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
}
