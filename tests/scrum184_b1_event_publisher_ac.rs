use ch_api_drive::domain::events::{
    EventPublisher, FileUploadedEvent, PublishError, FILE_UPLOADED_EVENT_VERSION,
};
use ch_api_drive::services::event_publisher_mqtt::{
    MqttConfigError, MqttEventPublisher, MqttEventPublisherConfig,
};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use std::sync::Mutex;
use uuid::Uuid;

static ENV_GUARD: Mutex<()> = Mutex::new(());

const SERVICE_ENV_VARS: [&str; 4] = [
    "RELAY_MQTT_URL",
    "RELAY_SERVICE_IDENTITY",
    "RELAY_SERVICE_TOKEN",
    "RELAY_UPLOAD_TOPIC_TEMPLATE",
];

const USER_AND_INTERNAL_VARS: [&str; 3] = ["JWT_SECRET", "INTERNAL_API_SECRET", "PORT"];

fn set_env(key: &str, value: &str) {
    unsafe { std::env::set_var(key, value) };
}

fn remove_env(key: &str) {
    unsafe { std::env::remove_var(key) };
}

fn clear_all_relevant_vars() {
    for key in SERVICE_ENV_VARS.iter().chain(USER_AND_INTERNAL_VARS.iter()) {
        remove_env(key);
    }
}

fn sample_event() -> FileUploadedEvent {
    FileUploadedEvent {
        event_version: FILE_UPLOADED_EVENT_VERSION,
        event_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
        occurred_at: Utc.with_ymd_and_hms(2026, 6, 23, 10, 0, 0).unwrap(),
        owner_id: "owner-opaque-1".to_string(),
        node_id: Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
        parent_id: Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap(),
        size_bytes: 10_737_418_240,
    }
}

fn publisher_with_template(template: &str) -> MqttEventPublisher {
    MqttEventPublisher::for_topic_resolution(template)
}

#[test]
fn ac1_config_reads_dedicated_service_identity_and_token() {
    let _lock = ENV_GUARD.lock().unwrap();
    clear_all_relevant_vars();

    set_env("RELAY_MQTT_URL", "mqtt://relay:1883");
    set_env("RELAY_SERVICE_IDENTITY", "svc-drive-identity");
    set_env("RELAY_SERVICE_TOKEN", "service-scoped-token");
    set_env(
        "RELAY_UPLOAD_TOPIC_TEMPLATE",
        "drive/{owner_id}/{file_id}/uploaded",
    );
    set_env("JWT_SECRET", "user-jwt-secret-value");
    set_env("INTERNAL_API_SECRET", "internal-api-secret-value-32-octets!!");

    let config = MqttEventPublisherConfig::from_env().expect("config from env");

    assert_eq!(config.client_id, "svc-drive-identity");
    assert_eq!(config.token, "service-scoped-token");

    assert_ne!(config.token, std::env::var("JWT_SECRET").unwrap());
    assert_ne!(config.token, std::env::var("INTERNAL_API_SECRET").unwrap());
    assert_ne!(config.client_id, std::env::var("JWT_SECRET").unwrap());
    assert_ne!(config.client_id, std::env::var("INTERNAL_API_SECRET").unwrap());

    clear_all_relevant_vars();
}

#[test]
fn ac1_config_does_not_fall_back_to_user_jwt_or_internal_secret() {
    let _lock = ENV_GUARD.lock().unwrap();
    clear_all_relevant_vars();

    set_env("JWT_SECRET", "user-jwt-secret-value");
    set_env("INTERNAL_API_SECRET", "internal-api-secret-value-32-octets!!");

    let result = MqttEventPublisherConfig::from_env();

    match result {
        Err(MqttConfigError::MissingVar(name)) => {
            assert!(SERVICE_ENV_VARS.contains(&name));
        }
        Ok(config) => panic!(
            "la config ne devrait pas se construire sans variables de service dédiées, obtenu : {config:?}"
        ),
    }

    clear_all_relevant_vars();
}

#[test]
fn ac1_config_requires_each_service_var_non_empty() {
    let _lock = ENV_GUARD.lock().unwrap();

    for missing in SERVICE_ENV_VARS {
        clear_all_relevant_vars();
        for key in SERVICE_ENV_VARS {
            if key != missing {
                set_env(key, "valeur");
            }
        }
        if missing != "RELAY_UPLOAD_TOPIC_TEMPLATE" {
            set_env("RELAY_UPLOAD_TOPIC_TEMPLATE", "drive/{owner_id}/{file_id}");
        }

        let result = MqttEventPublisherConfig::from_env();
        assert!(
            matches!(result, Err(MqttConfigError::MissingVar(name)) if name == missing),
            "variable manquante {missing} non détectée : {result:?}"
        );
    }

    clear_all_relevant_vars();
}

#[test]
fn ac2_topic_template_is_a_publication_target_with_required_placeholders() {
    let publisher = publisher_with_template("drive/{owner_id}/files/{file_id}/uploaded");
    let topic = publisher.resolve_topic(&sample_event()).unwrap();

    assert!(!topic.contains('+'));
    assert!(!topic.contains('#'));
    assert_eq!(
        topic,
        "drive/owner-opaque-1/files/22222222-2222-2222-2222-222222222222/uploaded"
    );
}

#[test]
fn ac2_publisher_trait_exposes_only_publish_capability() {
    fn assert_is_publisher<T: EventPublisher>() {}
    assert_is_publisher::<MqttEventPublisher>();
}

#[test]
fn ac2_resolve_topic_rejects_template_missing_owner_placeholder() {
    let publisher = publisher_with_template("drive/files/{file_id}/uploaded");
    assert!(matches!(
        publisher.resolve_topic(&sample_event()),
        Err(PublishError::InvalidTopic(_))
    ));
}

#[test]
fn ac2_resolve_topic_rejects_template_missing_file_placeholder() {
    let publisher = publisher_with_template("drive/{owner_id}/uploaded");
    assert!(matches!(
        publisher.resolve_topic(&sample_event()),
        Err(PublishError::InvalidTopic(_))
    ));
}

#[test]
fn ac2_resolve_topic_substitutes_node_id_for_file_placeholder() {
    let publisher = publisher_with_template("{owner_id}/{file_id}");
    let event = sample_event();
    let topic = publisher.resolve_topic(&event).unwrap();
    assert!(topic.contains(&event.node_id.to_string()));
    assert!(topic.contains(&event.owner_id));
}

#[test]
fn ac3_payload_contains_only_opaque_identifiers() {
    let payload: Value = serde_json::to_value(sample_event()).unwrap();
    let object = payload.as_object().unwrap();

    let allowed_keys = [
        "event_version",
        "event_id",
        "occurred_at",
        "owner_id",
        "node_id",
        "parent_id",
        "size_bytes",
    ];

    for key in object.keys() {
        assert!(
            allowed_keys.contains(&key.as_str()),
            "clé inattendue dans le payload : {key}"
        );
    }
    assert_eq!(object.len(), allowed_keys.len());
}

#[test]
fn ac3_payload_excludes_sensitive_fields() {
    let payload: Value = serde_json::to_value(sample_event()).unwrap();
    let object = payload.as_object().unwrap();

    let forbidden = [
        "file_name",
        "filename",
        "name",
        "path",
        "mime",
        "mime_type",
        "content_type",
        "content_hash",
        "sha256",
        "checksum",
        "url",
        "storage_key",
        "blob",
        "data",
        "content",
        "token",
        "secret",
        "jwt",
    ];

    for key in forbidden {
        assert!(
            !object.contains_key(key),
            "le payload ne doit pas exposer le champ sensible : {key}"
        );
    }
}

#[test]
fn ac3_payload_values_carry_no_filename_or_freeform_text() {
    let event = sample_event();
    let payload: Value = serde_json::to_value(&event).unwrap();
    let object = payload.as_object().unwrap();

    assert_eq!(object["owner_id"], Value::from(event.owner_id.clone()));
    assert_eq!(object["node_id"], Value::from(event.node_id.to_string()));
    assert_eq!(object["parent_id"], Value::from(event.parent_id.to_string()));
    assert!(object["node_id"].as_str().unwrap().parse::<Uuid>().is_ok());
    assert!(object["parent_id"].as_str().unwrap().parse::<Uuid>().is_ok());
    assert!(object["event_id"].as_str().unwrap().parse::<Uuid>().is_ok());
    assert_eq!(object["event_version"], Value::from(1u32));
    assert!(object["size_bytes"].is_number());
}

#[test]
fn ac3_payload_serialized_bytes_do_not_leak_known_filename() {
    let mut event = sample_event();
    event.owner_id = "owner-opaque-1".to_string();
    let bytes = serde_json::to_vec(&event).unwrap();
    let text = String::from_utf8(bytes).unwrap();

    assert!(!text.contains(".jpg"));
    assert!(!text.contains(".png"));
    assert!(!text.contains("vacances"));
    assert!(!text.to_lowercase().contains("filename"));
}
