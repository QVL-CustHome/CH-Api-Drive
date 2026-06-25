use async_trait::async_trait;
use rumqttc::{AsyncClient, ClientError, MqttOptions, QoS};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

use crate::domain::events::{EventPublisher, FileUploadedEvent, PublishError};

const OWNER_PLACEHOLDER: &str = "{owner_id}";
const FILE_PLACEHOLDER: &str = "{file_id}";
const CLIENT_CHANNEL_CAPACITY: usize = 64;
const KEEP_ALIVE: Duration = Duration::from_secs(30);
const DEFAULT_PORT: u16 = 1883;
const PLAINTEXT_SCHEMES: [&str; 2] = ["mqtt", "tcp"];
const ENCRYPTED_SCHEMES: [&str; 3] = ["mqtts", "ssl", "tls"];

#[derive(Debug, Clone)]
pub struct MqttEventPublisherConfig {
    pub broker_url: String,
    pub client_id: String,
    pub token: String,
    pub topic_template: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MqttConfigError {
    #[error("variable d'environnement requise manquante ou vide : {0}")]
    MissingVar(&'static str),
}

impl MqttEventPublisherConfig {
    pub fn from_env() -> Result<Self, MqttConfigError> {
        Ok(Self {
            broker_url: require("RELAY_MQTT_URL")?,
            client_id: require("RELAY_SERVICE_IDENTITY")?,
            token: require("RELAY_SERVICE_TOKEN")?,
            topic_template: require("RELAY_UPLOAD_TOPIC_TEMPLATE")?,
        })
    }
}

fn require(name: &'static str) -> Result<String, MqttConfigError> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or(MqttConfigError::MissingVar(name))
}

#[derive(Clone)]
pub struct MqttEventPublisher {
    topic_template: Arc<str>,
    client: AsyncClient,
}

impl MqttEventPublisher {
    pub fn new(config: MqttEventPublisherConfig) -> Result<Self, PublishError> {
        let options = build_mqtt_options(&config)?;
        let (client, mut eventloop) = AsyncClient::new(options, CLIENT_CHANNEL_CAPACITY);

        tokio::spawn(async move {
            loop {
                if let Err(error) = eventloop.poll().await {
                    tracing::debug!(target: "drive::events::mqtt", %error, "boucle MQTT interrompue, nouvelle tentative");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });

        Ok(Self {
            topic_template: Arc::from(config.topic_template.as_str()),
            client,
        })
    }

    #[doc(hidden)]
    pub fn for_topic_resolution(topic_template: &str) -> Self {
        let options = MqttOptions::new("svc-drive-test", "localhost", 1883);
        let (client, _eventloop) = AsyncClient::new(options, CLIENT_CHANNEL_CAPACITY);
        Self {
            topic_template: Arc::from(topic_template),
            client,
        }
    }

    pub fn resolve_topic(&self, event: &FileUploadedEvent) -> Result<String, PublishError> {
        let owner_id = &event.owner_id;
        let file_id = event.node_id;

        if !self.topic_template.contains(OWNER_PLACEHOLDER)
            || !self.topic_template.contains(FILE_PLACEHOLDER)
        {
            return Err(PublishError::InvalidTopic(self.topic_template.to_string()));
        }

        Ok(self
            .topic_template
            .replace(OWNER_PLACEHOLDER, owner_id)
            .replace(FILE_PLACEHOLDER, &file_id.to_string()))
    }
}

fn ensure_plaintext_scheme(scheme: &str) -> Result<(), PublishError> {
    if PLAINTEXT_SCHEMES.contains(&scheme) {
        return Ok(());
    }
    let supported = PLAINTEXT_SCHEMES.join("', '");
    if ENCRYPTED_SCHEMES.contains(&scheme) {
        return Err(PublishError::Connection(format!(
            "transport chiffré '{scheme}' non supporté : la topologie est mono-hôte loopback en clair, utilisez l'un des schemes '{supported}'"
        )));
    }
    Err(PublishError::Connection(format!(
        "scheme d'URL du broker non supporté : '{scheme}', attendu l'un de '{supported}'"
    )))
}

fn build_mqtt_options(config: &MqttEventPublisherConfig) -> Result<MqttOptions, PublishError> {
    let url = Url::parse(&config.broker_url)
        .map_err(|e| PublishError::Connection(e.to_string()))?;
    ensure_plaintext_scheme(url.scheme())?;
    let host = url
        .host_str()
        .ok_or_else(|| PublishError::Connection("hôte du broker absent".to_string()))?
        .to_string();
    let port = url.port().unwrap_or(DEFAULT_PORT);

    let mut options = MqttOptions::new(config.client_id.clone(), host, port);
    options.set_keep_alive(KEEP_ALIVE);
    options.set_credentials(config.client_id.clone(), config.token.clone());
    Ok(options)
}

#[async_trait]
impl EventPublisher for MqttEventPublisher {
    async fn publish_file_uploaded(&self, event: &FileUploadedEvent) -> Result<(), PublishError> {
        let topic = self.resolve_topic(event)?;
        let payload = serde_json::to_vec(event)
            .map_err(|e| PublishError::Serialization(e.to_string()))?;

        match self.client.try_publish(topic, QoS::AtLeastOnce, false, payload) {
            Ok(()) => Ok(()),
            Err(ClientError::TryRequest(_)) => {
                tracing::warn!(target: "drive::events::mqtt", event_id = %event.event_id, "publication MQTT abandonnée, file d'envoi pleine, bus indisponible");
                Ok(())
            }
            Err(error) => {
                tracing::warn!(target: "drive::events::mqtt", event_id = %event.event_id, %error, "publication MQTT abandonnée");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn sample_event() -> FileUploadedEvent {
        FileUploadedEvent {
            event_version: 1,
            event_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            occurred_at: Utc.with_ymd_and_hms(2026, 6, 23, 10, 0, 0).unwrap(),
            owner_id: "user-42".to_string(),
            node_id: Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
            parent_id: Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap(),
            size_bytes: 10_737_418_240,
        }
    }

    fn publisher(template: &str) -> MqttEventPublisher {
        MqttEventPublisher::for_topic_resolution(template)
    }

    #[test]
    fn resolve_topic_substitutes_owner_and_node() {
        let p = publisher("users/{owner_id}/files/{file_id}/uploaded");
        let topic = p.resolve_topic(&sample_event()).unwrap();
        assert_eq!(
            topic,
            "users/user-42/files/22222222-2222-2222-2222-222222222222/uploaded"
        );
    }

    #[test]
    fn resolve_topic_rejects_template_without_placeholders() {
        let p = publisher("users/files/uploaded");
        assert!(matches!(
            p.resolve_topic(&sample_event()),
            Err(PublishError::InvalidTopic(_))
        ));
    }

    #[test]
    fn payload_serializes_with_opaque_ids_only() {
        let payload = serde_json::to_value(sample_event()).unwrap();
        let object = payload.as_object().unwrap();

        assert_eq!(object["event_version"], 1);
        assert_eq!(object["event_id"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(object["occurred_at"], "2026-06-23T10:00:00Z");
        assert_eq!(object["owner_id"], "user-42");
        assert_eq!(object["node_id"], "22222222-2222-2222-2222-222222222222");
        assert_eq!(object["parent_id"], "33333333-3333-3333-3333-333333333333");
        assert_eq!(object["size_bytes"], 10_737_418_240i64);

        for forbidden in ["file_name", "mime", "content_hash", "path"] {
            assert!(!object.contains_key(forbidden));
        }
        assert_eq!(object.len(), 7);
    }
}
