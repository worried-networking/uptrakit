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
use uptrakit_shared_db::crypto::EncryptedString;

/// Re-encrypt all legacy plaintext values across all encrypted columns.
///
/// Should be called once at startup after the master key is initialised and
/// verified. Skips entirely when no master key is configured (dev mode).
pub(crate) async fn reencrypt_legacy_plaintext(db: &DatabaseConnection) {
    if !uptrakit_shared_db::crypto::master_key_available() {
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
        tracing::info!(table = "ca_certificates", column = "key_pem", count, "re-encrypted");
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
        tracing::info!(table = "oidc_providers", column = "client_secret", count, "re-encrypted");
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
        tracing::info!(table = "mqtt_clients", column = "password", count, "re-encrypted");
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
        tracing::info!(table = "mqtt_clients", column = "ca_cert_pem", count, "re-encrypted");
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
