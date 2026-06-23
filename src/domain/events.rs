use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const FILE_UPLOADED_EVENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUploadedEvent {
    pub event_version: u32,
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub owner_id: String,
    pub node_id: Uuid,
    pub parent_id: Uuid,
    pub size_bytes: i64,
}

impl FileUploadedEvent {
    pub fn new(owner_id: String, node_id: Uuid, parent_id: Uuid, size_bytes: i64) -> Self {
        Self {
            event_version: FILE_UPLOADED_EVENT_VERSION,
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            owner_id,
            node_id,
            parent_id,
            size_bytes,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("échec de sérialisation de l'événement : {0}")]
    Serialization(String),
    #[error("topic de publication invalide : {0}")]
    InvalidTopic(String),
    #[error("échec de connexion au bus d'événements : {0}")]
    Connection(String),
    #[error("échec de publication sur le bus d'événements : {0}")]
    Transport(String),
}

#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish_file_uploaded(&self, event: &FileUploadedEvent) -> Result<(), PublishError>;
}
