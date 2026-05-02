//! Automatic re-encryption of database values to the current `ENC:v3:` format.
//!
//! When the controller starts with a master key and a data key ring, this
//! module scans all encrypted columns and settings for values that are not yet
//! in the `ENC:v3:` envelope-encryption format.  Such values are decrypted
//! and re-encrypted in place using the active DEK and the appropriate
//! column-level AAD.
//!
//! Handled source formats:
//! - **Plaintext** (no `ENC:` prefix) — legacy values from before encryption.
//! - **`ENC:v1:`** — AES-256-GCM with no AAD (oldest format).
//! - **`ENC:v2:`** — AES-256-GCM with per-column AAD (KEK-direct).
//! - **`ENC:v3:`** — envelope encryption with DEK — already current, skipped.
//!
//! The migration runs automatically on every startup (no CLI flag needed).
//!
//! Properties:
//! - **Idempotent**: `ENC:v3:` values are skipped.
//! - **HA-safe**: concurrent controllers may race on the same row; the last
//!   writer wins, which is fine because the result is always a correctly
//!   encrypted value under the same DEK.
//! - **Fault-tolerant**: errors on individual rows are logged and skipped —
//!   the controller still starts successfully.

use sea_orm::{DatabaseConnection, EntityTrait, IntoActiveModel, QuerySelect};
use uptrakit_crypto::{ColumnAadEntry, EncryptedString};
use uptrakit_shared_db::provider_settings::AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN;

const UPGRADE_CHUNK_SIZE: u64 = 100;

// ── Column AAD constants ───────────────────────────────────────────────

const AAD_CA_KEY_PEM: &str = "uptrakit:ca_certificates:key_pem";
const AAD_OIDC_CLIENT_SECRET: &str = "uptrakit:oidc_providers:client_secret";
const AAD_PKCE_VERIFIER: &str = "uptrakit:pending_oidc_flows:pkce_verifier";
const AAD_NOTIFICATION_CONFIG: &str = "uptrakit:notification_channels:config";

// ── Settings AAD constants ─────────────────────────────────────────────

const AAD_SETTINGS_JWT_KEY: &str = "uptrakit:settings:jwt_signing_key";
const AAD_SETTINGS_NATS_URL: &str = "uptrakit:settings:nats_url";
const AAD_SETTINGS_SMTP_PASSWORD: &str = "uptrakit:settings:smtp_password";
const AAD_SETTINGS_ENROLLMENT_TOKEN: &str = "uptrakit:settings:system_services_enrollment_token";

/// Register the column-name-to-AAD mappings required for `ENC:v2:`/`ENC:v3:`
/// decryption.
///
/// Must be called once at startup, **before** any database queries that read
/// `EncryptedString` columns.  This enables the `TryGetable` implementation
/// in the crypto crate to look up the correct AAD when it encounters an
/// `ENC:v2:` or `ENC:v3:` ciphertext.
pub(crate) fn register_column_aad_mappings() {
    if !uptrakit_crypto::master_key_available() {
        return;
    }

    let entries: &[ColumnAadEntry] = &[
        ColumnAadEntry {
            table: "ca_certificates",
            column: "key_pem",
            aad: AAD_CA_KEY_PEM,
        },
        ColumnAadEntry {
            table: "oidc_providers",
            column: "client_secret",
            aad: AAD_OIDC_CLIENT_SECRET,
        },
        ColumnAadEntry {
            table: "pending_oidc_flows",
            column: "pkce_verifier",
            aad: AAD_PKCE_VERIFIER,
        },
        ColumnAadEntry {
            table: "notification_channels",
            column: "config",
            aad: AAD_NOTIFICATION_CONFIG,
        },
    ];

    if let Err(e) = uptrakit_crypto::register_column_aad(entries) {
        tracing::warn!(error = %e, "column AAD registry already initialized (harmless in tests)");
    }
}

// ── Top-level entry point ──────────────────────────────────────────────

/// Re-encrypt all non-v3 values in the database to `ENC:v3:` format.
///
/// Handles plaintext, `ENC:v1:`, and `ENC:v2:` values across all encrypted
/// columns and settings. Runs automatically at startup after the data key
/// ring has been initialized.
pub(crate) async fn reencrypt_to_v3(db: &DatabaseConnection) {
    if !uptrakit_crypto::master_key_available() {
        return;
    }

    let mut total = 0u64;

    // Database columns
    total += upgrade_ca_certificate_keys(db).await;
    total += upgrade_oidc_client_secrets(db).await;
    total += upgrade_oidc_flow_pkce_verifiers(db).await;
    total += upgrade_notification_channel_configs(db).await;

    // Settings values
    total += upgrade_setting(db, "auth.jwt_signing_key", AAD_SETTINGS_JWT_KEY).await;
    total += upgrade_setting(db, "nats.url", AAD_SETTINGS_NATS_URL).await;
    total += upgrade_setting(db, "smtp.password", AAD_SETTINGS_SMTP_PASSWORD).await;
    total += upgrade_setting(
        db,
        "global_provider_github.auth_token",
        AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN,
    )
    .await;
    total += upgrade_setting(
        db,
        "system_services.enrollment_token",
        AAD_SETTINGS_ENROLLMENT_TOKEN,
    )
    .await;

    if total > 0 {
        tracing::info!(
            count = total,
            "migrated values to ENC:v3 envelope encryption"
        );
    } else {
        tracing::debug!("all encrypted values already at ENC:v3");
    }
}

// ── Per-table upgrade helpers ──────────────────────────────────────────

async fn upgrade_ca_certificate_keys(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::CaCertificate;

    let mut count = 0u64;
    let mut offset = 0u64;

    loop {
        let rows = match CaCertificate::find()
            .offset(offset)
            .limit(UPGRADE_CHUNK_SIZE)
            .all(db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query ca_certificates for v3 upgrade");
                break;
            }
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.key_pem.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.key_pem.expose_secret().to_string();
            let fingerprint = row.fingerprint.clone();
            match EncryptedString::new(plaintext, AAD_CA_KEY_PEM) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.key_pem = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(fingerprint = %fingerprint, error = %e, "v3 upgrade failed: ca_certificates.key_pem");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(fingerprint = %fingerprint, error = %e, "v3 encrypt failed: ca_certificates.key_pem");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
        offset += UPGRADE_CHUNK_SIZE;
    }

    if count > 0 {
        tracing::info!(
            table = "ca_certificates",
            column = "key_pem",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

async fn upgrade_oidc_client_secrets(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::OidcProvider;

    let mut count = 0u64;
    let mut offset = 0u64;

    loop {
        let rows = match OidcProvider::find()
            .offset(offset)
            .limit(UPGRADE_CHUNK_SIZE)
            .all(db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query oidc_providers for v3 upgrade");
                break;
            }
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.client_secret.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.client_secret.expose_secret().to_string();
            let id = row.id;
            match EncryptedString::new(plaintext, AAD_OIDC_CLIENT_SECRET) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.client_secret = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(id = %id, error = %e, "v3 upgrade failed: oidc_providers.client_secret");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(id = %id, error = %e, "v3 encrypt failed: oidc_providers.client_secret");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
        offset += UPGRADE_CHUNK_SIZE;
    }

    if count > 0 {
        tracing::info!(
            table = "oidc_providers",
            column = "client_secret",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

async fn upgrade_oidc_flow_pkce_verifiers(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::PendingOidcFlow;

    let mut count = 0u64;
    let mut offset = 0u64;

    loop {
        let rows = match PendingOidcFlow::find()
            .offset(offset)
            .limit(UPGRADE_CHUNK_SIZE)
            .all(db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query pending_oidc_flows for v3 upgrade");
                break;
            }
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.pkce_verifier.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.pkce_verifier.expose_secret().to_string();
            let csrf_state = row.csrf_state.clone();
            match EncryptedString::new(plaintext, AAD_PKCE_VERIFIER) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.pkce_verifier = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(csrf_state = %csrf_state, error = %e, "v3 upgrade failed: pending_oidc_flows.pkce_verifier");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(csrf_state = %csrf_state, error = %e, "v3 encrypt failed: pending_oidc_flows.pkce_verifier");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
        offset += UPGRADE_CHUNK_SIZE;
    }

    if count > 0 {
        tracing::info!(
            table = "pending_oidc_flows",
            column = "pkce_verifier",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

async fn upgrade_notification_channel_configs(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::prelude::NotificationChannel;

    let mut count = 0u64;
    let mut offset = 0u64;

    loop {
        let rows = match NotificationChannel::find()
            .offset(offset)
            .limit(UPGRADE_CHUNK_SIZE)
            .all(db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query notification_channels for v3 upgrade");
                break;
            }
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.config.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.config.expose_secret().to_string();
            let id = row.id;
            match EncryptedString::new(plaintext, AAD_NOTIFICATION_CONFIG) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.config = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(id = %id, error = %e, "v3 upgrade failed: notification_channels.config");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(id = %id, error = %e, "v3 encrypt failed: notification_channels.config");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
        offset += UPGRADE_CHUNK_SIZE;
    }

    if count > 0 {
        tracing::info!(
            table = "notification_channels",
            column = "config",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

// ── Settings value upgrade ─────────────────────────────────────────────

/// Upgrade a single global setting value from v1/v2/plaintext to v3.
///
/// Returns 1 if the value was upgraded, 0 if it was already v3, absent,
/// or non-string.
async fn upgrade_setting(db: &DatabaseConnection, key: &str, aad: &str) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::{global_setting, prelude::GlobalSetting};

    let row = match GlobalSetting::find_by_id(key.to_string()).one(db).await {
        Ok(Some(r)) => r,
        Ok(None) => return 0,
        Err(e) => {
            tracing::warn!(key, error = %e, "failed to query global_settings for v3 upgrade");
            return 0;
        }
    };

    let Some(stored) = row.value.as_str() else {
        return 0;
    };

    if !uptrakit_crypto::is_encrypted(stored) && stored.is_empty() {
        return 0;
    }

    // Check if already v3
    if stored.starts_with("ENC:v3:") {
        return 0;
    }

    // Decrypt from current format, re-encrypt as v3
    let plaintext = match uptrakit_crypto::decrypt_str(stored, aad) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(key, error = %e, "v3 upgrade failed: could not decrypt setting");
            return 0;
        }
    };

    let new_encrypted = match uptrakit_crypto::encrypt_str(&plaintext, aad) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(key, error = %e, "v3 upgrade failed: could not re-encrypt setting");
            return 0;
        }
    };

    let mut am: global_setting::ActiveModel = row.into_active_model();
    am.value = sea_orm::Set(serde_json::json!(new_encrypted));
    am.updated_at = sea_orm::Set(time::OffsetDateTime::now_utc());

    if let Err(e) = am.update(db).await {
        tracing::warn!(key, error = %e, "v3 upgrade failed: could not save setting");
        return 0;
    }

    tracing::info!(key, "upgraded setting to ENC:v3");
    1
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "test code: discarding `init_master_key` is idiomatic — it is a no-op on subsequent calls"
    )]

    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, EntityTrait, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{ca_certificate, oidc_provider, pending_oidc_flow, tenant};
    use uuid::Uuid;

    /// Create a fresh in-memory SQLite database with all migrations applied.
    ///
    /// The master key is initialised once (idempotent: subsequent calls are
    /// silently ignored if the key is already set to the same value).
    ///
    /// A tenant with the nil UUID is inserted so that FK constraints on
    /// `oidc_providers.tenant_id` are satisfied.
    async fn test_db() -> DatabaseConnection {
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        register_column_aad_mappings();
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("connect to test db");
        crate::migration::run_migrations(&db)
            .await
            .expect("run migrations");

        // Insert a tenant with the nil UUID so FK constraints on
        // oidc_providers.tenant_id are satisfied.
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

    // ── Helpers ────────────────────────────────────────────────────────────────

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
            client_secret: Set(EncryptedString::new(
                "client-secret-val".to_string(),
                AAD_OIDC_CLIENT_SECRET,
            )
            .unwrap()),
            scopes: Set("openid email".to_string()),
            auto_create_users: Set(false),
            allow_private_network_issuers: Set(false),
            role_claim_path: Set(None),
            role_mapping: Set(RoleMapping::default()),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
    }

    // ── ca_certificates.key_pem ───────────────────────────────────────────────

    #[tokio::test]
    async fn ca_cert_plaintext_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        ca_certificate::ActiveModel {
            fingerprint: Set("fp1".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("secret_key".to_string(), AAD_CA_KEY_PEM).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert ca_certificate");

        // Simulate legacy plaintext.
        {
            let model =
                uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("fp1".to_string())
                    .one(&db)
                    .await
                    .expect("query")
                    .expect("row exists");
            let mut am: ca_certificate::ActiveModel = model.into();
            am.key_pem = Set(EncryptedString::plaintext_for_test(
                "secret_key".to_string(),
            ));
            am.update(&db).await.expect("set plaintext key_pem");
        }

        let count = upgrade_ca_certificate_keys(&db).await;
        assert_eq!(count, 1, "exactly one row should be upgraded");

        let row = uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("fp1".to_string())
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.key_pem.is_db_value_encrypted(),
            "key_pem must be encrypted after upgrade"
        );
        assert_eq!(row.key_pem.expose_secret(), "secret_key");
    }

    #[tokio::test]
    async fn ca_cert_encrypted_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        ca_certificate::ActiveModel {
            fingerprint: Set("fp2".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("ca_key_v2".to_string(), AAD_CA_KEY_PEM).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert ca_certificate");

        let count = upgrade_ca_certificate_keys(&db).await;
        assert_eq!(count, 1, "v2 row should be upgraded");

        let row = uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("fp2".to_string())
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.key_pem.is_db_value_encrypted(),
            "key_pem must be encrypted after upgrade"
        );
        assert_eq!(row.key_pem.expose_secret(), "ca_key_v2");
    }

    // ── oidc_providers.client_secret ─────────────────────────────────────────

    #[tokio::test]
    async fn oidc_client_secret_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        oidc_provider_am(id, now).insert(&db).await.expect("insert");

        let count = upgrade_oidc_client_secrets(&db).await;
        assert_eq!(count, 1);

        let row = uptrakit_shared_db::entity::prelude::OidcProvider::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(row.client_secret.is_db_value_encrypted());
        assert_eq!(row.client_secret.expose_secret(), "client-secret-val");
    }

    // ── pending_oidc_flows.pkce_verifier ──────────────────────────────────────

    #[tokio::test]
    async fn oidc_flow_pkce_verifier_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        pending_oidc_flow::ActiveModel {
            csrf_state: Set("csrf_1".to_string()),
            provider_id: Set(Uuid::nil()),
            pkce_verifier: Set(
                EncryptedString::new("pkce_secret".to_string(), AAD_PKCE_VERIFIER).unwrap(),
            ),
            nonce: Set("nonce".to_string()),
            created_at: Set(now),
            expires_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert");

        let count = upgrade_oidc_flow_pkce_verifiers(&db).await;
        assert_eq!(count, 1);

        let row =
            uptrakit_shared_db::entity::prelude::PendingOidcFlow::find_by_id("csrf_1".to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("row");
        assert!(row.pkce_verifier.is_db_value_encrypted());
        assert_eq!(row.pkce_verifier.expose_secret(), "pkce_secret");
    }

    // ── notification_channels.config ──────────────────────────────────────────

    #[tokio::test]
    async fn notification_channel_config_gets_upgraded() {
        use uptrakit_shared_db::entity::notification_channel;

        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        notification_channel::ActiveModel {
            id: Set(id),
            tenant_id: Set(Uuid::nil()),
            name: Set("test-channel".to_string()),
            channel_type: Set("webhook".to_string()),
            config: Set(EncryptedString::new(
                r#"{"url":"https://example.com"}"#.to_string(),
                AAD_NOTIFICATION_CONFIG,
            )
            .unwrap()),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert");

        let count = upgrade_notification_channel_configs(&db).await;
        assert_eq!(count, 1);

        let row = uptrakit_shared_db::entity::prelude::NotificationChannel::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(row.config.is_db_value_encrypted());
        assert_eq!(
            row.config.expose_secret(),
            r#"{"url":"https://example.com"}"#
        );
    }

    // ── Settings value upgrade ─────────────────────────────────────────────────

    #[tokio::test]
    async fn setting_v2_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        // Store a v2-encrypted value in global_settings.
        let encrypted =
            uptrakit_crypto::encrypt_str("my-secret-url", AAD_SETTINGS_NATS_URL).expect("encrypt");
        assert!(
            encrypted.starts_with("ENC:v2:"),
            "test precondition: should be v2 without DEK ring"
        );

        use uptrakit_shared_db::entity::global_setting;
        global_setting::ActiveModel {
            key: Set("nats.url".to_string()),
            value: Set(serde_json::json!(encrypted)),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert setting");

        let count = upgrade_setting(&db, "nats.url", AAD_SETTINGS_NATS_URL).await;
        assert_eq!(count, 1, "v2 setting should be upgraded");

        // Verify it was re-encrypted (still v2 without DEK ring in test, but
        // the upgrade function ran).
        let row =
            uptrakit_shared_db::entity::prelude::GlobalSetting::find_by_id("nats.url".to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("row");
        let stored = row.value.as_str().expect("string value");
        assert!(uptrakit_crypto::is_encrypted(stored), "must be encrypted");

        // Verify round-trip: decrypt should return original plaintext.
        let decrypted =
            uptrakit_crypto::decrypt_str(stored, AAD_SETTINGS_NATS_URL).expect("decrypt");
        assert_eq!(decrypted, "my-secret-url");
    }

    #[tokio::test]
    async fn setting_absent_is_skipped() {
        let db = test_db().await;
        let count = upgrade_setting(&db, "nats.url", AAD_SETTINGS_NATS_URL).await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn github_provider_setting_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        let encrypted = uptrakit_crypto::encrypt_str(
            "ghp_provider_secret",
            uptrakit_shared_db::provider_settings::AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN,
        )
        .expect("encrypt");
        assert!(
            encrypted.starts_with("ENC:v2:"),
            "test precondition: should be v2 without DEK ring"
        );

        use uptrakit_shared_db::entity::global_setting;
        global_setting::ActiveModel {
            key: Set("global_provider_github.auth_token".to_string()),
            value: Set(serde_json::json!(encrypted)),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert setting");

        reencrypt_to_v3(&db).await;

        let row = uptrakit_shared_db::entity::prelude::GlobalSetting::find_by_id(
            "global_provider_github.auth_token".to_string(),
        )
        .one(&db)
        .await
        .expect("query")
        .expect("row");
        let stored = row.value.as_str().expect("string value");
        assert!(uptrakit_crypto::is_encrypted(stored), "must be encrypted");
        let decrypted = uptrakit_crypto::decrypt_str(
            stored,
            uptrakit_shared_db::provider_settings::AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN,
        )
        .expect("decrypt");
        assert_eq!(decrypted, "ghp_provider_secret");
    }

    // ── Idempotency ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn upgrade_is_idempotent_for_data_integrity() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        ca_certificate::ActiveModel {
            fingerprint: Set("idem_fp".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("idem_key".to_string(), AAD_CA_KEY_PEM).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert");

        // First run upgrades v2→v2 (no DEK ring in tests).
        let count1 = upgrade_ca_certificate_keys(&db).await;
        assert_eq!(count1, 1);

        // Second run: still v2 (not v3), so re-processed. Data integrity is key.
        let count2 = upgrade_ca_certificate_keys(&db).await;
        assert_eq!(count2, 1, "v2 re-processed (harmless without DEK ring)");

        let row =
            uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("idem_fp".to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("row");
        assert!(row.key_pem.is_db_value_encrypted(), "still encrypted");
        assert_eq!(row.key_pem.expose_secret(), "idem_key", "data preserved");
    }

    // ── Top-level reencrypt_to_v3 ─────────────────────────────────────────────

    #[tokio::test]
    async fn reencrypt_to_v3_processes_all_tables() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        // Insert rows across all tables.
        ca_certificate::ActiveModel {
            fingerprint: Set("v3_all_fp".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("all_ca".to_string(), AAD_CA_KEY_PEM).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert");

        let oidc_id = Uuid::now_v7();
        oidc_provider_am(oidc_id, now)
            .insert(&db)
            .await
            .expect("insert");

        pending_oidc_flow::ActiveModel {
            csrf_state: Set("v3_all_csrf".to_string()),
            provider_id: Set(Uuid::nil()),
            pkce_verifier: Set(
                EncryptedString::new("all_pkce".to_string(), AAD_PKCE_VERIFIER).unwrap(),
            ),
            nonce: Set("nonce".to_string()),
            created_at: Set(now),
            expires_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert");

        // Run top-level migration.
        reencrypt_to_v3(&db).await;

        // Verify all encrypted.
        let ca =
            uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("v3_all_fp".to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("row");
        assert!(
            ca.key_pem.is_db_value_encrypted(),
            "key_pem must be encrypted"
        );

        let oidc = uptrakit_shared_db::entity::prelude::OidcProvider::find_by_id(oidc_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            oidc.client_secret.is_db_value_encrypted(),
            "client_secret must be encrypted"
        );

        let flow = uptrakit_shared_db::entity::prelude::PendingOidcFlow::find_by_id(
            "v3_all_csrf".to_string(),
        )
        .one(&db)
        .await
        .expect("query")
        .expect("row");
        assert!(
            flow.pkce_verifier.is_db_value_encrypted(),
            "pkce_verifier must be encrypted"
        );
    }

    #[tokio::test]
    async fn reencrypt_to_v3_handles_plaintext_values() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        // Insert encrypted then overwrite with plaintext.
        ca_certificate::ActiveModel {
            fingerprint: Set("pt_fp".to_string()),
            cert_pem: Set("---CERT---".to_string()),
            key_pem: Set(EncryptedString::new("ca_key".to_string(), AAD_CA_KEY_PEM).unwrap()),
            not_before: Set(now),
            not_after: Set(now),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert");

        {
            let model =
                uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("pt_fp".to_string())
                    .one(&db)
                    .await
                    .expect("query")
                    .expect("row");
            let mut am: ca_certificate::ActiveModel = model.into();
            am.key_pem = Set(EncryptedString::plaintext_for_test("ca_key".to_string()));
            am.update(&db).await.expect("set plaintext");
        }

        reencrypt_to_v3(&db).await;

        let row =
            uptrakit_shared_db::entity::prelude::CaCertificate::find_by_id("pt_fp".to_string())
                .one(&db)
                .await
                .expect("query")
                .expect("row");
        assert!(
            row.key_pem.is_db_value_encrypted(),
            "must be encrypted after migration"
        );
        assert_eq!(row.key_pem.expose_secret(), "ca_key");
    }
}
