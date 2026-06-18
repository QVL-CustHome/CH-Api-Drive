use ch_api_drive::config;
use ch_api_drive::db;
use ch_api_drive::routes;
use ch_api_drive::state::AppState;

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

    let port = settings.config.server.port;
    let state = AppState::new(&settings, pool);
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

fn init_tracing(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_new(level.to_lowercase())
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
}
