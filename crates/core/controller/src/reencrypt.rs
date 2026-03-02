//! Background re-encryption of legacy plaintext values.
//!
//! When the controller starts with a master key, this module scans all
//! encrypted columns for values that are still stored as plaintext (i.e. they
//! lack the `ENC:v1:` prefix). Such values are re-encrypted in place using the
//! current master key.
//!
//! The routine is:
//! - **Idempotent**: already-encrypted values are skipped (prefix check).
//! - **HA-safe**: concurrent controllers may race on the same row; the last
//!   writer wins, which is fine because the result is always a correctly
//!   encrypted value under the same master key.
//! - **Fault-tolerant**: errors on individual rows are logged and skipped —
//!   the controller still starts successfully.

use sea_orm::{DatabaseConnection, EntityTrait, IntoActiveModel};
use uptrakit_crypto::EncryptedString;

/// Re-encrypt all legacy plaintext values across all encrypted columns.
///
/// Should be called once at startup after the master key is initialised and
/// verified. Skips entirely when no master key is configured (dev mode).
pub(crate) async fn reencrypt_legacy_plaintext(db: &DatabaseConnection) {
    if !uptrakit_crypto::master_key_available() {
        return;
    }

    let mut total = 0u64;

    total += reencrypt_ca_certificate_keys(db).await;
    total += reencrypt_oidc_client_secrets(db).await;
    total += reencrypt_mqtt_passwords(db).await;
    total += reencrypt_mqtt_ca_certs(db).await;
    total += reencrypt_oidc_flow_pkce_verifiers(db).await;

    if total > 0 {
        tracing::info!(
            count = total,
            "re-encrypted legacy plaintext values in database"
        );
    }
}

// ── Per-table helpers ────────────────────────────────────────────────

async fn reencrypt_ca_certificate_keys(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::CaCertificate;

    let rows = match CaCertificate::find().all(db).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query ca_certificates for re-encryption");
            return 0;
        }
    };

    let mut count = 0u64;
    for row in rows {
        if row.key_pem.is_db_value_encrypted() {
            continue;
        }
        let plaintext = row.key_pem.expose_secret().to_string();
        let fingerprint = row.fingerprint.clone();
        match EncryptedString::new(plaintext) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.key_pem = sea_orm::Set(encrypted);
                if let Err(e) = am.update(db).await {
                    tracing::warn!(
                        fingerprint = %fingerprint,
                        error = %e,
                        "failed to re-encrypt ca_certificates.key_pem"
                    );
                } else {
                    count += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    fingerprint = %fingerprint,
                    error = %e,
                    "failed to encrypt ca_certificates.key_pem value"
                );
            }
        }
    }
    if count > 0 {
        tracing::info!(
            table = "ca_certificates",
            column = "key_pem",
            count,
            "re-encrypted"
        );
    }
    count
}

async fn reencrypt_oidc_client_secrets(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::OidcProvider;

    let rows = match OidcProvider::find().all(db).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query oidc_providers for re-encryption");
            return 0;
        }
    };

    let mut count = 0u64;
    for row in rows {
        if row.client_secret.is_db_value_encrypted() {
            continue;
        }
        let plaintext = row.client_secret.expose_secret().to_string();
        let id = row.id;
        match EncryptedString::new(plaintext) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.client_secret = sea_orm::Set(encrypted);
                if let Err(e) = am.update(db).await {
                    tracing::warn!(
                        id = %id,
                        error = %e,
                        "failed to re-encrypt oidc_providers.client_secret"
                    );
                } else {
                    count += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    id = %id,
                    error = %e,
                    "failed to encrypt oidc_providers.client_secret value"
                );
            }
        }
    }
    if count > 0 {
        tracing::info!(
            table = "oidc_providers",
            column = "client_secret",
            count,
            "re-encrypted"
        );
    }
    count
}

async fn reencrypt_mqtt_passwords(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::MqttClient;

    let rows = match MqttClient::find().all(db).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query mqtt_clients for re-encryption");
            return 0;
        }
    };

    let mut count = 0u64;
    for row in rows {
        let Some(ref password) = row.password else {
            continue;
        };
        if password.is_db_value_encrypted() {
            continue;
        }
        let plaintext = password.expose_secret().to_string();
        let id = row.id;
        match EncryptedString::new(plaintext) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.password = sea_orm::Set(Some(encrypted));
                if let Err(e) = am.update(db).await {
                    tracing::warn!(
                        id = %id,
                        error = %e,
                        "failed to re-encrypt mqtt_clients.password"
                    );
                } else {
                    count += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    id = %id,
                    error = %e,
                    "failed to encrypt mqtt_clients.password value"
                );
            }
        }
    }
    if count > 0 {
        tracing::info!(
            table = "mqtt_clients",
            column = "password",
            count,
            "re-encrypted"
        );
    }
    count
}

async fn reencrypt_mqtt_ca_certs(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::MqttClient;

    let rows = match MqttClient::find().all(db).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query mqtt_clients for re-encryption (ca_cert_pem)");
            return 0;
        }
    };

    let mut count = 0u64;
    for row in rows {
        let Some(ref ca_cert) = row.ca_cert_pem else {
            continue;
        };
        if ca_cert.is_db_value_encrypted() {
            continue;
        }
        let plaintext = ca_cert.expose_secret().to_string();
        let id = row.id;
        match EncryptedString::new(plaintext) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.ca_cert_pem = sea_orm::Set(Some(encrypted));
                if let Err(e) = am.update(db).await {
                    tracing::warn!(
                        id = %id,
                        error = %e,
                        "failed to re-encrypt mqtt_clients.ca_cert_pem"
                    );
                } else {
                    count += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    id = %id,
                    error = %e,
                    "failed to encrypt mqtt_clients.ca_cert_pem value"
                );
            }
        }
    }
    if count > 0 {
        tracing::info!(
            table = "mqtt_clients",
            column = "ca_cert_pem",
            count,
            "re-encrypted"
        );
    }
    count
}

async fn reencrypt_oidc_flow_pkce_verifiers(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::PendingOidcFlow;

    let rows = match PendingOidcFlow::find().all(db).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query pending_oidc_flows for re-encryption");
            return 0;
        }
    };

    let mut count = 0u64;
    for row in rows {
        if row.pkce_verifier.is_db_value_encrypted() {
            continue;
        }
        let plaintext = row.pkce_verifier.expose_secret().to_string();
        let csrf_state = row.csrf_state.clone();
        match EncryptedString::new(plaintext) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.pkce_verifier = sea_orm::Set(encrypted);
                if let Err(e) = am.update(db).await {
                    tracing::warn!(
                        csrf_state = %csrf_state,
                        error = %e,
                        "failed to re-encrypt pending_oidc_flows.pkce_verifier"
                    );
                } else {
                    count += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    csrf_state = %csrf_state,
                    error = %e,
                    "failed to encrypt pending_oidc_flows.pkce_verifier value"
                );
            }
        }
    }
    if count > 0 {
        tracing::info!(
            table = "pending_oidc_flows",
            column = "pkce_verifier",
            count,
            "re-encrypted"
        );
    }
    count
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectOptions, Database, EntityTrait, QueryFilter, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        ca_certificate, mqtt_client, oidc_provider, pending_oidc_flow, tenant,
    };
    use uptrakit_shared_types::{MqttClientConnectionStatus, MqttTransport};
    use uuid::Uuid;

    /// Create a fresh in-memory SQLite database with all migrations applied.
    ///
    /// The master key is initialised once (idempotent: subsequent calls are
    /// silently ignored if the key is already set to the same value).
    ///
    /// A tenant with the nil UUID is inserted so that FK constraints on
    /// `oidc_providers.tenant_id` and `mqtt_clients.tenant_id` are satisfied.
    async fn test_db() -> DatabaseConnection {
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("connect to test db");
        crate::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        // Insert a tenant with the nil UUID so FK constraints on
        // oidc_providers.tenant_id and mqtt_clients.tenant_id are satisfied.
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(Uuid::nil()),
            name: Set("Test Tenant".to_string()),
            slug: Set("test-tenant".to_string()),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert nil tenant for FK satisfaction");
        db
    }

    // ── ca_certificates.key_pem ───────────────────────────────────────────────

    #[tokio::test]
    async fn ca_cert_plaintext_gets_reencrypted() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        // Insert a row (sea-orm auto-encrypts key_pem via EncryptedString::new).
        let am = ca_certificate::ActiveModel {
            fingerprint: Set("fp1".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("secret_key".to_string()).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        };
        am.insert(&db).await.expect("insert ca_certificate");

        // Simulate legacy: overwrite key_pem with plaintext (no ENC:v1: prefix).
        {
            let model = uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id(
                "fp1".to_string(),
            )
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
            let mut am: ca_certificate::ActiveModel = model.into();
            am.key_pem = Set(EncryptedString::plaintext_for_test("secret_key".to_string()));
            am.update(&db).await.expect("set plaintext key_pem");
        }

        let count = reencrypt_ca_certificate_keys(&db).await;
        assert_eq!(count, 1, "exactly one row should be re-encrypted");

        // Verify the column now carries the encrypted prefix.
        let row = uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("fp1".to_string())
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.key_pem.is_db_value_encrypted(),
            "key_pem must have ENC:v1: prefix after re-encryption"
        );
    }

    #[tokio::test]
    async fn ca_cert_already_encrypted_is_skipped() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        let am = ca_certificate::ActiveModel {
            fingerprint: Set("fp2".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("secret_key".to_string()).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        };
        am.insert(&db).await.expect("insert ca_certificate");

        // No raw SQL overwrite — key_pem stays encrypted.
        let count = reencrypt_ca_certificate_keys(&db).await;
        assert_eq!(count, 0, "already-encrypted row must not be counted");
    }

    // ── oidc_providers.client_secret ─────────────────────────────────────────

    fn oidc_provider_am(id: Uuid, now: OffsetDateTime) -> oidc_provider::ActiveModel {
        use uptrakit_shared_db::entity::oidc_provider::RoleMapping;
        oidc_provider::ActiveModel {
            id: Set(id),
            tenant_id: Set(Uuid::nil()),
            name: Set("Test IdP".to_string()),
            slug: Set(format!("test-{id}")),
            logo_url: Set(None),
            issuer_url: Set("https://idp.example.com".to_string()),
            client_id: Set("client-id".to_string()),
            client_secret: Set(EncryptedString::new("client-secret-val".to_string()).unwrap()),
            scopes: Set("openid email".to_string()),
            auto_create_users: Set(false),
            role_claim_path: Set(None),
            role_mapping: Set(RoleMapping::default()),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
    }

    #[tokio::test]
    async fn oidc_client_secret_plaintext_gets_reencrypted() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        oidc_provider_am(id, now)
            .insert(&db)
            .await
            .expect("insert oidc_provider");

        // Overwrite client_secret with a plaintext value (no ENC:v1: prefix)
        // to simulate a legacy row written before encryption was added.
        {
            let model = oidc_provider::Entity::find()
                .filter(oidc_provider::Column::Slug.eq(format!("test-{id}")))
                .one(&db)
                .await
                .expect("query")
                .expect("row exists");
            let mut am: oidc_provider::ActiveModel = model.into();
            am.client_secret =
                Set(EncryptedString::plaintext_for_test("client-secret-val".to_string()));
            am.update(&db).await.expect("set plaintext client_secret");
        }

        let count = reencrypt_oidc_client_secrets(&db).await;
        assert_eq!(count, 1, "exactly one row should be re-encrypted");

        let row = uptrakit_shared_db::entity::prelude::OidcProvider::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.client_secret.is_db_value_encrypted(),
            "client_secret must have ENC:v1: prefix after re-encryption"
        );
    }

    #[tokio::test]
    async fn oidc_client_secret_already_encrypted_is_skipped() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        oidc_provider_am(id, now)
            .insert(&db)
            .await
            .expect("insert oidc_provider");

        let count = reencrypt_oidc_client_secrets(&db).await;
        assert_eq!(count, 0, "already-encrypted row must not be counted");
    }

    // ── mqtt_clients.password ─────────────────────────────────────────────────

    fn mqtt_client_am(id: Uuid, now: OffsetDateTime) -> mqtt_client::ActiveModel {
        mqtt_client::ActiveModel {
            id: Set(id),
            tenant_id: Set(Uuid::nil()),
            enabled: Set(true),
            transport: Set(MqttTransport::Tcp),
            host: Set("mqtt.example.com".to_string()),
            port: Set(1883),
            client_id: Set(format!("client-{id}")),
            username: Set(Some("user".to_string())),
            password: Set(Some(EncryptedString::new("mqtt-pass".to_string()).unwrap())),
            ca_cert_pem: Set(None),
            topic_prefix: Set("uptrakit".to_string()),
            connection_status: Set(MqttClientConnectionStatus::Offline),
            status_updated_at: Set(now),
            ha_discovery: Set(false),
            ha_discovery_prefix: Set("homeassistant".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
    }

    #[tokio::test]
    async fn mqtt_password_plaintext_gets_reencrypted() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        mqtt_client_am(id, now)
            .insert(&db)
            .await
            .expect("insert mqtt_client");

        // Overwrite password with a plaintext value (no ENC:v1: prefix)
        // to simulate a legacy row written before encryption was added.
        {
            let model = mqtt_client::Entity::find()
                .filter(mqtt_client::Column::ClientId.eq(format!("client-{id}")))
                .one(&db)
                .await
                .expect("query")
                .expect("row exists");
            let mut am: mqtt_client::ActiveModel = model.into();
            am.password =
                Set(Some(EncryptedString::plaintext_for_test("mqtt-pass".to_string())));
            am.update(&db).await.expect("set plaintext password");
        }

        let count = reencrypt_mqtt_passwords(&db).await;
        assert_eq!(count, 1, "exactly one row should be re-encrypted");

        let row = uptrakit_shared_db::entity::prelude::MqttClient::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        let password = row.password.as_ref().expect("password present");
        assert!(
            password.is_db_value_encrypted(),
            "password must have ENC:v1: prefix after re-encryption"
        );
    }

    #[tokio::test]
    async fn mqtt_password_null_is_skipped() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        let mut am = mqtt_client_am(id, now);
        am.password = Set(None);
        am.insert(&db).await.expect("insert mqtt_client");

        let count = reencrypt_mqtt_passwords(&db).await;
        assert_eq!(count, 0, "NULL password must not be counted");
    }

    #[tokio::test]
    async fn mqtt_password_already_encrypted_is_skipped() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        mqtt_client_am(id, now)
            .insert(&db)
            .await
            .expect("insert mqtt_client");

        let count = reencrypt_mqtt_passwords(&db).await;
        assert_eq!(count, 0, "already-encrypted password must not be counted");
    }

    // ── mqtt_clients.ca_cert_pem ──────────────────────────────────────────────

    #[tokio::test]
    async fn mqtt_ca_cert_plaintext_gets_reencrypted() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        let mut am = mqtt_client_am(id, now);
        am.ca_cert_pem = Set(Some(
            EncryptedString::new("-----BEGIN CERTIFICATE-----".to_string()).unwrap(),
        ));
        am.insert(&db).await.expect("insert mqtt_client");

        // Overwrite ca_cert_pem with a plaintext value (no ENC:v1: prefix)
        // to simulate a legacy row written before encryption was added.
        {
            let model = mqtt_client::Entity::find()
                .filter(mqtt_client::Column::ClientId.eq(format!("client-{id}")))
                .one(&db)
                .await
                .expect("query")
                .expect("row exists");
            let mut am: mqtt_client::ActiveModel = model.into();
            am.ca_cert_pem = Set(Some(EncryptedString::plaintext_for_test(
                "-----BEGIN CERTIFICATE-----".to_string(),
            )));
            am.update(&db).await.expect("set plaintext ca_cert_pem");
        }

        let count = reencrypt_mqtt_ca_certs(&db).await;
        assert_eq!(count, 1, "exactly one row should be re-encrypted");

        let row = uptrakit_shared_db::entity::prelude::MqttClient::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        let ca_cert = row.ca_cert_pem.as_ref().expect("ca_cert_pem present");
        assert!(
            ca_cert.is_db_value_encrypted(),
            "ca_cert_pem must have ENC:v1: prefix after re-encryption"
        );
    }

    #[tokio::test]
    async fn mqtt_ca_cert_null_is_skipped() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        mqtt_client_am(id, now)
            .insert(&db)
            .await
            .expect("insert mqtt_client");

        let count = reencrypt_mqtt_ca_certs(&db).await;
        assert_eq!(count, 0, "NULL ca_cert_pem must not be counted");
    }

    // ── pending_oidc_flows.pkce_verifier ──────────────────────────────────────

    #[tokio::test]
    async fn oidc_flow_pkce_verifier_plaintext_gets_reencrypted() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let csrf = "csrf_state_test_1";

        let am = pending_oidc_flow::ActiveModel {
            csrf_state: Set(csrf.to_string()),
            provider_id: Set(Uuid::nil()),
            pkce_verifier: Set(EncryptedString::new("pkce_secret".to_string()).unwrap()),
            nonce: Set("nonce_value".to_string()),
            created_at: Set(now),
            expires_at: Set(now),
        };
        am.insert(&db).await.expect("insert pending_oidc_flow");

        // Overwrite pkce_verifier with a plaintext value (no ENC:v1: prefix)
        // to simulate a legacy row written before encryption was added.
        {
            let model =
                uptrakit_shared_db::entity::prelude::PendingOidcFlow::find_by_id(csrf.to_string())
                    .one(&db)
                    .await
                    .expect("query")
                    .expect("row exists");
            let mut am: pending_oidc_flow::ActiveModel = model.into();
            am.pkce_verifier =
                Set(EncryptedString::plaintext_for_test("pkce_secret".to_string()));
            am.update(&db).await.expect("set plaintext pkce_verifier");
        }

        let count = reencrypt_oidc_flow_pkce_verifiers(&db).await;
        assert_eq!(count, 1, "exactly one row should be re-encrypted");

        let row = uptrakit_shared_db::entity::prelude::PendingOidcFlow::find_by_id(csrf.to_string())
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.pkce_verifier.is_db_value_encrypted(),
            "pkce_verifier must have ENC:v1: prefix after re-encryption"
        );
    }

    #[tokio::test]
    async fn oidc_flow_pkce_verifier_already_encrypted_is_skipped() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        let am = pending_oidc_flow::ActiveModel {
            csrf_state: Set("csrf_state_test_2".to_string()),
            provider_id: Set(Uuid::nil()),
            pkce_verifier: Set(EncryptedString::new("pkce_secret".to_string()).unwrap()),
            nonce: Set("nonce_value".to_string()),
            created_at: Set(now),
            expires_at: Set(now),
        };
        am.insert(&db).await.expect("insert pending_oidc_flow");

        let count = reencrypt_oidc_flow_pkce_verifiers(&db).await;
        assert_eq!(count, 0, "already-encrypted row must not be counted");
    }

    // ── reencrypt_legacy_plaintext (top-level) ────────────────────────────────

    #[tokio::test]
    async fn reencrypt_legacy_plaintext_processes_all_tables() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        // Insert one plaintext row in each table.
        let ca_am = ca_certificate::ActiveModel {
            fingerprint: Set("fp_all".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("ca_key".to_string()).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        };
        ca_am.insert(&db).await.expect("insert ca_certificate");
        // Simulate legacy: overwrite key_pem with plaintext (no ENC:v1: prefix).
        {
            let model =
                uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("fp_all".to_string())
                    .one(&db)
                    .await
                    .expect("query")
                    .expect("row exists");
            let mut am: ca_certificate::ActiveModel = model.into();
            am.key_pem = Set(EncryptedString::plaintext_for_test("ca_key".to_string()));
            am.update(&db).await.expect("set plaintext key_pem");
        }

        let oidc_id = Uuid::now_v7();
        oidc_provider_am(oidc_id, now)
            .insert(&db)
            .await
            .expect("insert oidc_provider");
        // Simulate legacy: overwrite client_secret with plaintext (no ENC:v1: prefix).
        {
            let model = oidc_provider::Entity::find()
                .filter(oidc_provider::Column::Slug.eq(format!("test-{oidc_id}")))
                .one(&db)
                .await
                .expect("query")
                .expect("row exists");
            let mut am: oidc_provider::ActiveModel = model.into();
            am.client_secret =
                Set(EncryptedString::plaintext_for_test("oidc_secret".to_string()));
            am.update(&db).await.expect("set plaintext client_secret");
        }

        let mqtt_id = Uuid::now_v7();
        mqtt_client_am(mqtt_id, now)
            .insert(&db)
            .await
            .expect("insert mqtt_client");
        // Simulate legacy: overwrite password with plaintext (no ENC:v1: prefix).
        {
            let model = mqtt_client::Entity::find()
                .filter(mqtt_client::Column::ClientId.eq(format!("client-{mqtt_id}")))
                .one(&db)
                .await
                .expect("query")
                .expect("row exists");
            let mut am: mqtt_client::ActiveModel = model.into();
            am.password =
                Set(Some(EncryptedString::plaintext_for_test("mqtt_pass".to_string())));
            am.update(&db).await.expect("set plaintext password");
        }

        let csrf = "csrf_all_tables";
        let flow_am = pending_oidc_flow::ActiveModel {
            csrf_state: Set(csrf.to_string()),
            provider_id: Set(Uuid::nil()),
            pkce_verifier: Set(EncryptedString::new("pkce_val".to_string()).unwrap()),
            nonce: Set("nonce".to_string()),
            created_at: Set(now),
            expires_at: Set(now),
        };
        flow_am.insert(&db).await.expect("insert pending_oidc_flow");
        // Simulate legacy: overwrite pkce_verifier with plaintext (no ENC:v1: prefix).
        {
            let model =
                uptrakit_shared_db::entity::prelude::PendingOidcFlow::find_by_id(csrf.to_string())
                    .one(&db)
                    .await
                    .expect("query")
                    .expect("row exists");
            let mut am: pending_oidc_flow::ActiveModel = model.into();
            am.pkce_verifier =
                Set(EncryptedString::plaintext_for_test("pkce_val".to_string()));
            am.update(&db).await.expect("set plaintext pkce_verifier");
        }

        // Run the top-level function — should process all tables.
        reencrypt_legacy_plaintext(&db).await;

        // Verify all columns are now encrypted.
        let ca_row =
            uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("fp_all".to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("ca row exists");
        assert!(ca_row.key_pem.is_db_value_encrypted(), "ca key_pem must be encrypted");

        let oidc_row = uptrakit_shared_db::entity::prelude::OidcProvider::find_by_id(oidc_id)
            .one(&db)
            .await
            .expect("query")
            .expect("oidc row exists");
        assert!(
            oidc_row.client_secret.is_db_value_encrypted(),
            "oidc client_secret must be encrypted"
        );

        let mqtt_row = uptrakit_shared_db::entity::prelude::MqttClient::find_by_id(mqtt_id)
            .one(&db)
            .await
            .expect("query")
            .expect("mqtt row exists");
        assert!(
            mqtt_row.password.as_ref().unwrap().is_db_value_encrypted(),
            "mqtt password must be encrypted"
        );

        let flow_row =
            uptrakit_shared_db::entity::prelude::PendingOidcFlow::find_by_id(csrf.to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("flow row exists");
        assert!(
            flow_row.pkce_verifier.is_db_value_encrypted(),
            "pkce_verifier must be encrypted"
        );
    }
}
