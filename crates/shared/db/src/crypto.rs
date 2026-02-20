//! Re-exports from the standalone `uptrakit-crypto` crate.
//!
//! Preserves backward compatibility for downstream crates that import
//! `uptrakit_shared_db::crypto::*`.

pub use uptrakit_crypto::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn ensure_test_key() {
        let key = zeroize::Zeroizing::new([0x42u8; 32]);
        let _ = init_master_key(key);
    }

    #[tokio::test]
    async fn test_nullable_encrypted_string_decodes_to_none() {
        {
            let _lock = TEST_LOCK.lock().unwrap();
            ensure_test_key();
        }

        use crate::entity::mqtt_client;
        use crate::entity::prelude::MqttClient;
        use sea_orm::{DbBackend, EntityTrait, MockDatabase};
        use time::OffsetDateTime;
        use uptrakit_shared_types::{MqttClientConnectionStatus, MqttTransport};
        use uuid::Uuid;

        let model = mqtt_client::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            enabled: true,
            transport: MqttTransport::Tcp,
            host: "broker".to_string(),
            port: 1883,
            client_id: "uptrakit-controller".to_string(),
            username: None,
            password: None,
            ca_cert_pem: None,
            topic_prefix: "uptrakit".to_string(),
            connection_status: MqttClientConnectionStatus::Offline,
            status_updated_at: OffsetDateTime::now_utc(),
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        };

        let db = MockDatabase::new(DbBackend::Postgres)
            .append_query_results([[model.clone()]])
            .into_connection();

        let loaded = MqttClient::find_by_id(model.id).one(&db).await;
        let password_none = matches!(loaded, Ok(Some(ref found)) if found.password.is_none());
        assert!(password_none);
    }
}
