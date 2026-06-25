use ch_api_drive::domain::events::PublishError;
use ch_api_drive::services::event_publisher_mqtt::{
    MqttEventPublisher, MqttEventPublisherConfig,
};

fn config_with_url(broker_url: &str) -> MqttEventPublisherConfig {
    MqttEventPublisherConfig {
        broker_url: broker_url.to_string(),
        client_id: "svc-drive".to_string(),
        token: "service-scoped-token".to_string(),
        topic_template: "drive/{owner_id}/files/{file_id}/uploaded".to_string(),
    }
}

#[test]
fn ac_rejects_mqtts_scheme_with_connection_error() {
    let result = MqttEventPublisher::new(config_with_url("mqtts://127.0.0.1:8883"));
    assert!(
        matches!(result, Err(PublishError::Connection(_))),
        "mqtts:// doit être rejeté avec PublishError::Connection, obtenu : {:?}",
        result.as_ref().err()
    );
}

#[test]
fn ac_rejects_ssl_scheme_with_connection_error() {
    let result = MqttEventPublisher::new(config_with_url("ssl://127.0.0.1:8883"));
    assert!(
        matches!(result, Err(PublishError::Connection(_))),
        "ssl:// doit être rejeté avec PublishError::Connection, obtenu : {:?}",
        result.as_ref().err()
    );
}

#[test]
fn ac_rejects_tls_scheme_with_connection_error() {
    let result = MqttEventPublisher::new(config_with_url("tls://127.0.0.1:8883"));
    assert!(
        matches!(result, Err(PublishError::Connection(_))),
        "tls:// doit être rejeté avec PublishError::Connection, obtenu : {:?}",
        result.as_ref().err()
    );
}

#[tokio::test]
async fn ac_accepts_mqtt_plaintext_scheme() {
    let result = MqttEventPublisher::new(config_with_url("mqtt://127.0.0.1:1883"));
    assert!(
        result.is_ok(),
        "mqtt:// doit être accepté (plaintext loopback), obtenu : {:?}",
        result.as_ref().err()
    );
}

#[tokio::test]
async fn ac_accepts_tcp_plaintext_scheme() {
    let result = MqttEventPublisher::new(config_with_url("tcp://127.0.0.1:1883"));
    assert!(
        result.is_ok(),
        "tcp:// doit être accepté (plaintext loopback) selon l'AC, obtenu : {:?}",
        result.as_ref().err()
    );
}
