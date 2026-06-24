use crate::db::Db;
use crate::repository::upload_sessions::{self, ExpiredSession};
use crate::services::storage::Storage;
use std::time::Duration;
use tokio::task::JoinHandle;

const DEFAULT_BATCH_SIZE: i64 = 100;
const MIN_INTERVAL: Duration = Duration::from_secs(1);
const MIN_BATCH_SIZE: i64 = 1;

#[derive(Debug, Clone, Copy)]
pub struct GcConfig {
    pub interval: Duration,
    pub batch_size: i64,
}

impl GcConfig {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval: interval.max(MIN_INTERVAL),
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }

    pub fn with_batch_size(mut self, batch_size: i64) -> Self {
        self.batch_size = batch_size.max(MIN_BATCH_SIZE);
        self
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GcReport {
    pub reclaimed: u64,
    pub failed: u64,
}

pub fn spawn_gc<S>(pool: Db, storage: S, config: GcConfig) -> JoinHandle<()>
where
    S: Storage + 'static,
{
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            run_once(&pool, &storage, config.batch_size).await;
        }
    })
}

pub async fn run_once<S>(pool: &Db, storage: &S, batch_size: i64) -> GcReport
where
    S: Storage,
{
    let expired = match upload_sessions::find_expired(pool, batch_size).await {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::error!(error = %error, "GC upload : échec du recensement des sessions expirées");
            return GcReport::default();
        }
    };

    let mut report = GcReport::default();
    for session in expired {
        match reclaim_session(pool, storage, &session).await {
            Ok(true) => report.reclaimed += 1,
            Ok(false) => {}
            Err(()) => report.failed += 1,
        }
    }

    if report.reclaimed > 0 || report.failed > 0 {
        tracing::info!(
            reclaimed = report.reclaimed,
            failed = report.failed,
            "GC upload : passe terminée"
        );
    }
    report
}

async fn reclaim_session<S>(
    pool: &Db,
    storage: &S,
    session: &ExpiredSession,
) -> Result<bool, ()>
where
    S: Storage,
{
    let deleted = upload_sessions::delete_if_expired(pool, session.id)
        .await
        .map_err(|error| {
            tracing::error!(
                session_id = %session.id,
                error = %error,
                "GC upload : échec de la suppression en base"
            );
        })?;

    if !deleted {
        return Ok(false);
    }

    if let Err(error) = storage.delete(&session.tmp_key).await {
        tracing::warn!(
            session_id = %session.id,
            tmp_key = %session.tmp_key,
            error = %error,
            "GC upload : session supprimée en base mais fichier temporaire non nettoyé"
        );
    }

    tracing::debug!(
        session_id = %session.id,
        owner_id = %session.owner_id,
        reserved_bytes = session.reserved_bytes,
        state = session.state.as_str(),
        "GC upload : session expirée récupérée"
    );
    Ok(true)
}
