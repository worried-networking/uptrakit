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

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect,
};
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

    #[cfg_attr(
        not(feature = "embedded-ssh-agent"),
        expect(
            unused_mut,
            reason = "mut only needed when embedded-ssh-agent extends the entries list"
        )
    )]
    let mut entries: Vec<ColumnAadEntry> = vec![
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
        ColumnAadEntry {
            table: "user_totp",
            column: "secret",
            aad: "uptrakit:user_totp:secret",
        },
    ];

    #[cfg(feature = "embedded-ssh-agent")]
    entries.extend_from_slice(uptrakit_agent_ssh_runtime::AgentSshHandler::column_aad_entries());

    if let Err(e) = uptrakit_crypto::register_column_aad(&entries) {
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
    if uptrakit_crypto::master_key_available() {
        let mut total = 0u64;

        // Database columns
        total += upgrade_ca_certificate_keys(db).await;
        total += upgrade_oidc_client_secrets(db).await;
        total += upgrade_oidc_flow_pkce_verifiers(db).await;
        total += upgrade_notification_channel_configs(db).await;
        total += upgrade_plugin_configs(db).await;
        total += upgrade_plugin_type_settings(db).await;
        total += upgrade_instance_plugin_settings(db).await;

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
    } else {
        tracing::debug!(
            "no master key available: skipping ENC:v3 upgrade passes (plaintext-residue counters still run)"
        );
    }

    // Plaintext-residue observability: unconditional, independent of master
    // key availability. This is deliberately NOT gated behind the branch
    // above — an external-scheduler-only deployment (no controller, so this
    // fn never even runs there) or a db-migrate skip is exactly the
    // situation where rows are guaranteed to still be plaintext and this
    // warning is most needed, and it needs no key material to run (a raw
    // `NOT LIKE 'ENC:%'` count).
    warn_on_plugin_config_plaintext_residue(db).await;
    warn_on_plugin_type_setting_plaintext_residue(db).await;
    warn_on_instance_plugin_setting_plaintext_residue(db).await;
}

// ── Per-table upgrade helpers ──────────────────────────────────────────

async fn upgrade_ca_certificate_keys(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::entity::{ca_certificate, prelude::CaCertificate};

    let mut count = 0u64;
    let mut last_fingerprint: Option<String> = None;

    loop {
        let mut query = CaCertificate::find()
            .order_by_asc(ca_certificate::Column::Fingerprint)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(fingerprint) = &last_fingerprint {
            query = query.filter(ca_certificate::Column::Fingerprint.gt(fingerprint.clone()));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query ca_certificates for v3 upgrade");
                break;
            }
        };

        let Some(last) = rows.last() else { break };
        last_fingerprint = Some(last.fingerprint.clone());
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
    use uptrakit_shared_db::entity::{oidc_provider, prelude::OidcProvider};

    let mut count = 0u64;
    let mut last_id: Option<uuid::Uuid> = None;

    loop {
        let mut query = OidcProvider::find()
            .order_by_asc(oidc_provider::Column::Id)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(id) = last_id {
            query = query.filter(oidc_provider::Column::Id.gt(id));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query oidc_providers for v3 upgrade");
                break;
            }
        };

        let Some(last) = rows.last() else { break };
        last_id = Some(last.id);
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
    use uptrakit_shared_db::entity::{pending_oidc_flow, prelude::PendingOidcFlow};

    let mut count = 0u64;
    let mut last_csrf_state: Option<String> = None;

    loop {
        let mut query = PendingOidcFlow::find()
            .order_by_asc(pending_oidc_flow::Column::CsrfState)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(csrf_state) = &last_csrf_state {
            query = query.filter(pending_oidc_flow::Column::CsrfState.gt(csrf_state.clone()));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query pending_oidc_flows for v3 upgrade");
                break;
            }
        };

        let Some(last) = rows.last() else { break };
        last_csrf_state = Some(last.csrf_state.clone());
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
    use uptrakit_shared_db::entity::{notification_channel, prelude::NotificationChannel};

    let mut count = 0u64;
    let mut last_id: Option<uuid::Uuid> = None;

    loop {
        let mut query = NotificationChannel::find()
            .order_by_asc(notification_channel::Column::Id)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(id) = last_id {
            query = query.filter(notification_channel::Column::Id.gt(id));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to query notification_channels for v3 upgrade");
                break;
            }
        };

        let Some(last) = rows.last() else { break };
        last_id = Some(last.id);
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

async fn upgrade_plugin_configs(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig;
    use uptrakit_shared_db::entity::{plugin_config, prelude::PluginConfig};

    let mut count = 0u64;
    let mut last_id: Option<uuid::Uuid> = None;

    loop {
        let mut query = PluginConfig::find()
            .order_by_asc(plugin_config::Column::Id)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(id) = last_id {
            query = query.filter(plugin_config::Column::Id.gt(id));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    cursor = ?last_id,
                    error = %e,
                    "failed to query plugin_configs page for v3 upgrade; attempting to skip poisoned page"
                );

                // The page load failed to decode (likely one undecryptable or
                // unparseable `config` value). Pks are unencrypted, so a
                // pk-only select of the same page can still succeed and let
                // us advance the cursor past the poisoned page instead of
                // stalling this table's pass forever.
                let mut pk_query = PluginConfig::find()
                    .select_only()
                    .column(plugin_config::Column::Id)
                    .order_by_asc(plugin_config::Column::Id)
                    .limit(UPGRADE_CHUNK_SIZE);
                if let Some(id) = last_id {
                    pk_query = pk_query.filter(plugin_config::Column::Id.gt(id));
                }
                match pk_query.into_tuple::<uuid::Uuid>().all(db).await {
                    Ok(ids) => {
                        let Some(&max_id) = ids.last() else { break };
                        let recovered_page_len = ids.len() as u64;
                        last_id = Some(max_id);
                        if recovered_page_len < UPGRADE_CHUNK_SIZE {
                            break;
                        }
                        continue;
                    }
                    Err(e2) => {
                        tracing::error!(
                            cursor = ?last_id,
                            error = %e2,
                            "pk-only recovery for plugin_configs also failed; aborting v3 upgrade pass for this table"
                        );
                        break;
                    }
                }
            }
        };

        let Some(last) = rows.last() else { break };
        last_id = Some(last.id);
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.config.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.config.expose_secret().to_string();
            let id = row.id;
            match EncryptedPluginConfig::new(plaintext) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.config = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(id = %id, error = %e, "v3 upgrade failed: plugin_configs.config");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(id = %id, error = %e, "v3 encrypt failed: plugin_configs.config");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
    }

    if count > 0 {
        tracing::info!(
            table = "plugin_configs",
            column = "config",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

async fn upgrade_plugin_type_settings(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::encrypted_columns::EncryptedPluginTypeConfig;
    use uptrakit_shared_db::entity::{plugin_type_setting, prelude::PluginTypeSetting};

    let mut count = 0u64;
    let mut last_id: Option<uuid::Uuid> = None;

    loop {
        let mut query = PluginTypeSetting::find()
            .order_by_asc(plugin_type_setting::Column::Id)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(id) = last_id {
            query = query.filter(plugin_type_setting::Column::Id.gt(id));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    cursor = ?last_id,
                    error = %e,
                    "failed to query plugin_type_settings page for v3 upgrade; attempting to skip poisoned page"
                );

                // The page load failed to decode (likely one undecryptable or
                // unparseable `config` value). Pks are unencrypted, so a
                // pk-only select of the same page can still succeed and let
                // us advance the cursor past the poisoned page instead of
                // stalling this table's pass forever.
                let mut pk_query = PluginTypeSetting::find()
                    .select_only()
                    .column(plugin_type_setting::Column::Id)
                    .order_by_asc(plugin_type_setting::Column::Id)
                    .limit(UPGRADE_CHUNK_SIZE);
                if let Some(id) = last_id {
                    pk_query = pk_query.filter(plugin_type_setting::Column::Id.gt(id));
                }
                match pk_query.into_tuple::<uuid::Uuid>().all(db).await {
                    Ok(ids) => {
                        let Some(&max_id) = ids.last() else { break };
                        let recovered_page_len = ids.len() as u64;
                        last_id = Some(max_id);
                        if recovered_page_len < UPGRADE_CHUNK_SIZE {
                            break;
                        }
                        continue;
                    }
                    Err(e2) => {
                        tracing::error!(
                            cursor = ?last_id,
                            error = %e2,
                            "pk-only recovery for plugin_type_settings also failed; aborting v3 upgrade pass for this table"
                        );
                        break;
                    }
                }
            }
        };

        let Some(last) = rows.last() else { break };
        last_id = Some(last.id);
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.config.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.config.expose_secret().to_string();
            let id = row.id;
            match EncryptedPluginTypeConfig::new(plaintext) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.config = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(id = %id, error = %e, "v3 upgrade failed: plugin_type_settings.config");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(id = %id, error = %e, "v3 encrypt failed: plugin_type_settings.config");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
    }

    if count > 0 {
        tracing::info!(
            table = "plugin_type_settings",
            column = "config",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

async fn upgrade_instance_plugin_settings(db: &DatabaseConnection) -> u64 {
    use sea_orm::ActiveModelTrait;
    use uptrakit_shared_db::encrypted_columns::EncryptedInstancePluginConfig;
    use uptrakit_shared_db::entity::{instance_plugin_setting, prelude::InstancePluginSetting};

    let mut count = 0u64;
    let mut last_plugin_type_id: Option<String> = None;

    loop {
        let mut query = InstancePluginSetting::find()
            .order_by_asc(instance_plugin_setting::Column::PluginTypeId)
            .limit(UPGRADE_CHUNK_SIZE);
        if let Some(plugin_type_id) = &last_plugin_type_id {
            query = query
                .filter(instance_plugin_setting::Column::PluginTypeId.gt(plugin_type_id.clone()));
        }
        let rows = match query.all(db).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    cursor = ?last_plugin_type_id,
                    error = %e,
                    "failed to query instance_plugin_setting page for v3 upgrade; attempting to skip poisoned page"
                );

                // The page load failed to decode (likely one undecryptable or
                // unparseable `config` value). Pks are unencrypted, so a
                // pk-only select of the same page can still succeed and let
                // us advance the cursor past the poisoned page instead of
                // stalling this table's pass forever.
                let mut pk_query = InstancePluginSetting::find()
                    .select_only()
                    .column(instance_plugin_setting::Column::PluginTypeId)
                    .order_by_asc(instance_plugin_setting::Column::PluginTypeId)
                    .limit(UPGRADE_CHUNK_SIZE);
                if let Some(plugin_type_id) = &last_plugin_type_id {
                    pk_query = pk_query.filter(
                        instance_plugin_setting::Column::PluginTypeId.gt(plugin_type_id.clone()),
                    );
                }
                match pk_query.into_tuple::<String>().all(db).await {
                    Ok(ids) => {
                        let Some(max_id) = ids.last().cloned() else {
                            break;
                        };
                        let recovered_page_len = ids.len() as u64;
                        last_plugin_type_id = Some(max_id);
                        if recovered_page_len < UPGRADE_CHUNK_SIZE {
                            break;
                        }
                        continue;
                    }
                    Err(e2) => {
                        tracing::error!(
                            cursor = ?last_plugin_type_id,
                            error = %e2,
                            "pk-only recovery for instance_plugin_setting also failed; aborting v3 upgrade pass for this table"
                        );
                        break;
                    }
                }
            }
        };

        let Some(last) = rows.last() else { break };
        last_plugin_type_id = Some(last.plugin_type_id.clone());
        let page_len = rows.len() as u64;

        for row in rows {
            if !row.config.needs_v3_upgrade() {
                continue;
            }
            let plaintext = row.config.expose_secret().to_string();
            let plugin_type_id = row.plugin_type_id.clone();
            match EncryptedInstancePluginConfig::new(plaintext) {
                Ok(encrypted) => {
                    let mut am = row.into_active_model();
                    am.config = sea_orm::Set(encrypted);
                    if let Err(e) = am.update(db).await {
                        tracing::warn!(plugin_type_id = %plugin_type_id, error = %e, "v3 upgrade failed: instance_plugin_setting.config");
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(plugin_type_id = %plugin_type_id, error = %e, "v3 encrypt failed: instance_plugin_setting.config");
                }
            }
        }

        if page_len < UPGRADE_CHUNK_SIZE {
            break;
        }
    }

    if count > 0 {
        tracing::info!(
            table = "instance_plugin_setting",
            column = "config",
            count,
            "upgraded to ENC:v3"
        );
    }
    count
}

// ── Plaintext-residue observability ─────────────────────────────────────

/// Count rows whose `config` column is not stored as `ENC:*` ciphertext.
///
/// Deliberately a typed `sea_query` `COUNT(*)` scalar select rather than an
/// entity load: with the eager-parse encrypted-column newtypes, a single
/// undecryptable row fails an entire `Vec<Model>` load — exactly when this
/// diagnostic is needed most (a stuck upgrade pass, or a deployment that
/// never ran it at all).
///
/// Split out from the warn-emitting wrapper below so the count itself is
/// directly unit-testable: the `remaining > 0` branch of the wrapper is only
/// reachable via a log assertion otherwise, and every failure path degrades
/// to `0`, which would make a malformed or backend-specific query silently
/// inert with no test ever catching it.
async fn count_plaintext_residue(
    db: &DatabaseConnection,
    stmt: &sea_orm::sea_query::SelectStatement,
    table: &'static str,
) -> i64 {
    use sea_orm::ConnectionTrait;

    match db.query_one(stmt).await {
        Ok(Some(row)) => match row.try_get::<i64>("", "cnt") {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(table, error = %e, "failed to parse plaintext-residue count");
                0
            }
        },
        Ok(None) => 0,
        Err(e) => {
            tracing::warn!(table, error = %e, "failed to count plaintext-residue rows");
            0
        }
    }
}

/// Count rows whose `config` column is not stored as `ENC:*` ciphertext and,
/// when any remain, warn with the table name and count.
async fn warn_on_plaintext_residue(
    db: &DatabaseConnection,
    stmt: &sea_orm::sea_query::SelectStatement,
    table: &'static str,
) {
    let remaining = count_plaintext_residue(db, stmt, table).await;

    if remaining > 0 {
        tracing::warn!(
            table,
            remaining,
            "plugin config rows still stored in plaintext — reencrypt pass has not converted \
             them (external-scheduler-only deployment or db-migrate skip?)"
        );
    }
}

async fn warn_on_plugin_config_plaintext_residue(db: &DatabaseConnection) {
    use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
    use uptrakit_shared_db::entity::plugin_config;

    let stmt = Query::select()
        .expr_as(
            Expr::col(plugin_config::Column::Config).count(),
            Alias::new("cnt"),
        )
        .from(plugin_config::Entity)
        .and_where(Expr::col(plugin_config::Column::Config).not_like("ENC:%"))
        .to_owned();
    warn_on_plaintext_residue(db, &stmt, "plugin_configs").await;
}

async fn warn_on_plugin_type_setting_plaintext_residue(db: &DatabaseConnection) {
    use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
    use uptrakit_shared_db::entity::plugin_type_setting;

    let stmt = Query::select()
        .expr_as(
            Expr::col(plugin_type_setting::Column::Config).count(),
            Alias::new("cnt"),
        )
        .from(plugin_type_setting::Entity)
        .and_where(Expr::col(plugin_type_setting::Column::Config).not_like("ENC:%"))
        .to_owned();
    warn_on_plaintext_residue(db, &stmt, "plugin_type_settings").await;
}

async fn warn_on_instance_plugin_setting_plaintext_residue(db: &DatabaseConnection) {
    use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
    use uptrakit_shared_db::entity::instance_plugin_setting;

    let stmt = Query::select()
        .expr_as(
            Expr::col(instance_plugin_setting::Column::Config).count(),
            Alias::new("cnt"),
        )
        .from(instance_plugin_setting::Entity)
        .and_where(Expr::col(instance_plugin_setting::Column::Config).not_like("ENC:%"))
        .to_owned();
    warn_on_plaintext_residue(db, &stmt, "instance_plugin_setting").await;
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

    /// Insert a legacy plaintext `plugin_configs` row via a raw `sea_query`
    /// insert, bypassing the `EncryptedPluginConfig` newtype (which always
    /// encrypts on construction — there is no `plaintext_for_test` escape
    /// hatch for it, unlike `EncryptedString`).
    async fn insert_plaintext_plugin_config(
        db: &DatabaseConnection,
        id: Uuid,
        plaintext_json: &str,
        now: OffsetDateTime,
    ) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Expr as SqExpr, Query};
        use uptrakit_shared_db::entity::plugin_config;

        let insert = Query::insert()
            .into_table(plugin_config::Entity)
            .columns([
                plugin_config::Column::Id,
                plugin_config::Column::TenantId,
                plugin_config::Column::Name,
                plugin_config::Column::PluginType,
                plugin_config::Column::Config,
                plugin_config::Column::Enabled,
                plugin_config::Column::CreatedAt,
                plugin_config::Column::UpdatedAt,
                plugin_config::Column::DeactivatedAt,
                plugin_config::Column::CredentialUpdatedAt,
            ])
            .values_panic([
                SqExpr::value(id),
                SqExpr::value(Uuid::nil()),
                SqExpr::value(format!("legacy-plugin-config-{id}")),
                SqExpr::value("legacy-plugin-type"),
                SqExpr::value(plaintext_json),
                SqExpr::value(true),
                SqExpr::value(now),
                SqExpr::value(now),
                SqExpr::value(sea_orm::Value::TimeDateTimeWithTimeZone(None)),
                SqExpr::value(sea_orm::Value::TimeDateTimeWithTimeZone(None)),
            ])
            .to_owned();

        db.execute(&insert)
            .await
            .expect("insert legacy plaintext plugin_config row");
    }

    /// Insert a legacy plaintext `plugin_type_settings` row via a raw
    /// `sea_query` insert (see `insert_plaintext_plugin_config` doc comment).
    async fn insert_plaintext_plugin_type_setting(
        db: &DatabaseConnection,
        id: Uuid,
        plaintext_json: &str,
        now: OffsetDateTime,
    ) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Expr as SqExpr, Query};
        use uptrakit_shared_db::entity::plugin_type_setting;

        let insert = Query::insert()
            .into_table(plugin_type_setting::Entity)
            .columns([
                plugin_type_setting::Column::Id,
                plugin_type_setting::Column::TenantId,
                plugin_type_setting::Column::PluginType,
                plugin_type_setting::Column::Config,
                plugin_type_setting::Column::CreatedAt,
                plugin_type_setting::Column::UpdatedAt,
            ])
            .values_panic([
                SqExpr::value(id),
                SqExpr::value(Uuid::nil()),
                SqExpr::value("legacy-plugin-type"),
                SqExpr::value(plaintext_json),
                SqExpr::value(now),
                SqExpr::value(now),
            ])
            .to_owned();

        db.execute(&insert)
            .await
            .expect("insert legacy plaintext plugin_type_setting row");
    }

    /// Insert a legacy plaintext `instance_plugin_setting` row via a raw
    /// `sea_query` insert (see `insert_plaintext_plugin_config` doc comment).
    async fn insert_plaintext_instance_plugin_setting(
        db: &DatabaseConnection,
        plugin_type_id: &str,
        plaintext_json: &str,
        now: OffsetDateTime,
    ) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Expr as SqExpr, Query};
        use uptrakit_shared_db::entity::instance_plugin_setting;

        let insert = Query::insert()
            .into_table(instance_plugin_setting::Entity)
            .columns([
                instance_plugin_setting::Column::PluginTypeId,
                instance_plugin_setting::Column::Enabled,
                instance_plugin_setting::Column::Config,
                instance_plugin_setting::Column::UpdatedAt,
            ])
            .values_panic([
                SqExpr::value(plugin_type_id),
                SqExpr::value(true),
                SqExpr::value(plaintext_json),
                SqExpr::value(now),
            ])
            .to_owned();

        db.execute(&insert)
            .await
            .expect("insert legacy plaintext instance_plugin_setting row");
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

    #[tokio::test]
    async fn notification_channel_config_pagination_upgrades_all_rows() {
        use uptrakit_shared_db::entity::notification_channel;

        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        for _ in 0..150 {
            let id = Uuid::now_v7();
            notification_channel::ActiveModel {
                id: Set(id),
                tenant_id: Set(Uuid::nil()),
                name: Set(format!("test-channel-{id}")),
                channel_type: Set("webhook".to_string()),
                config: Set(EncryptedString::plaintext_for_test(
                    r#"{"url":"https://example.com"}"#.to_string(),
                )),
                enabled: Set(true),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&db)
            .await
            .expect("insert");
        }

        let count = upgrade_notification_channel_configs(&db).await;
        assert_eq!(count, 150, "all 150 rows should be upgraded");

        let rows = uptrakit_shared_db::entity::prelude::NotificationChannel::find()
            .all(&db)
            .await
            .expect("query all");
        assert_eq!(rows.len(), 150);
        for row in rows {
            assert!(
                row.config.is_db_value_encrypted(),
                "every row's config must be encrypted after upgrade"
            );
        }
    }

    // ── plugin_configs.config ───────────────────────────────────────────────────

    #[tokio::test]
    async fn plugin_config_plaintext_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        insert_plaintext_plugin_config(&db, id, r#"{"token":"secret-token"}"#, now).await;

        let count = upgrade_plugin_configs(&db).await;
        assert_eq!(count, 1, "exactly one row should be upgraded");

        let row = uptrakit_shared_db::entity::prelude::PluginConfig::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.config.is_db_value_encrypted(),
            "config must be encrypted after upgrade"
        );
        assert_eq!(row.config.expose_secret(), r#"{"token":"secret-token"}"#);
    }

    #[tokio::test]
    async fn plugin_config_pagination_upgrades_all_rows() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        for _ in 0..150 {
            let id = Uuid::now_v7();
            insert_plaintext_plugin_config(&db, id, r#"{"token":"secret-token"}"#, now).await;
        }

        let count = upgrade_plugin_configs(&db).await;
        assert_eq!(count, 150, "all 150 rows should be upgraded");

        let rows = uptrakit_shared_db::entity::prelude::PluginConfig::find()
            .all(&db)
            .await
            .expect("query all");
        assert_eq!(rows.len(), 150);
        for row in rows {
            assert!(
                row.config.is_db_value_encrypted(),
                "every row's config must be encrypted after upgrade"
            );
        }
    }

    #[tokio::test]
    async fn plugin_config_upgrade_is_idempotent_for_data_integrity() {
        use uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig;
        use uptrakit_shared_db::entity::plugin_config;

        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        plugin_config::ActiveModel {
            id: Set(id),
            tenant_id: Set(Uuid::nil()),
            name: Set("idem-plugin-config".to_string()),
            plugin_type: Set("idem-plugin-type".to_string()),
            config: Set(
                EncryptedPluginConfig::new(r#"{"token":"idem-secret"}"#.to_string()).unwrap(),
            ),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            credential_updated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert");

        // First run upgrades v2→v2 (no DEK ring in tests).
        let count1 = upgrade_plugin_configs(&db).await;
        assert_eq!(count1, 1);

        // Second run: still v2 (not v3), so re-processed. Data integrity is key.
        let count2 = upgrade_plugin_configs(&db).await;
        assert_eq!(count2, 1, "v2 re-processed (harmless without DEK ring)");

        let row = uptrakit_shared_db::entity::prelude::PluginConfig::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(row.config.is_db_value_encrypted(), "still encrypted");
        assert_eq!(
            row.config.expose_secret(),
            r#"{"token":"idem-secret"}"#,
            "data preserved"
        );
    }

    // ── plugin_type_settings.config ─────────────────────────────────────────────

    #[tokio::test]
    async fn plugin_type_setting_plaintext_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        insert_plaintext_plugin_type_setting(&db, id, r#"{"default_timeout":30}"#, now).await;

        let count = upgrade_plugin_type_settings(&db).await;
        assert_eq!(count, 1, "exactly one row should be upgraded");

        let row = uptrakit_shared_db::entity::prelude::PluginTypeSetting::find_by_id(id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert!(
            row.config.is_db_value_encrypted(),
            "config must be encrypted after upgrade"
        );
        assert_eq!(row.config.expose_secret(), r#"{"default_timeout":30}"#);
    }

    // ── instance_plugin_setting.config ──────────────────────────────────────────

    #[tokio::test]
    async fn instance_plugin_setting_plaintext_gets_upgraded() {
        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        insert_plaintext_instance_plugin_setting(
            &db,
            "test-instance-plugin",
            r#"{"api_key":"secret"}"#,
            now,
        )
        .await;

        let count = upgrade_instance_plugin_settings(&db).await;
        assert_eq!(count, 1, "exactly one row should be upgraded");

        let row = uptrakit_shared_db::entity::prelude::InstancePluginSetting::find_by_id(
            "test-instance-plugin".to_string(),
        )
        .one(&db)
        .await
        .expect("query")
        .expect("row exists");
        assert!(
            row.config.is_db_value_encrypted(),
            "config must be encrypted after upgrade"
        );
        assert_eq!(row.config.expose_secret(), r#"{"api_key":"secret"}"#);
    }

    // ── Plaintext-residue observability ─────────────────────────────────────────

    #[tokio::test]
    async fn count_plaintext_residue_reports_remaining_rows() {
        use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
        use uptrakit_shared_db::entity::plugin_config;

        let db = test_db().await;
        let now = OffsetDateTime::now_utc();

        // Three plaintext rows left unconverted (no upgrade pass run) plus
        // one already-encrypted row that must NOT be counted.
        for _ in 0..3 {
            insert_plaintext_plugin_config(&db, Uuid::now_v7(), r#"{"token":"t"}"#, now).await;
        }
        insert_plaintext_plugin_config(&db, Uuid::now_v7(), r#"{"token":"t"}"#, now).await;
        let converted = upgrade_plugin_configs(&db).await;
        assert_eq!(converted, 4, "seed data sanity: all four start plaintext");

        // Re-seed three more plaintext rows that stay unconverted, mirroring
        // a stuck/skipped upgrade pass.
        for _ in 0..3 {
            insert_plaintext_plugin_config(&db, Uuid::now_v7(), r#"{"token":"t"}"#, now).await;
        }

        let stmt = Query::select()
            .expr_as(
                Expr::col(plugin_config::Column::Config).count(),
                Alias::new("cnt"),
            )
            .from(plugin_config::Entity)
            .and_where(Expr::col(plugin_config::Column::Config).not_like("ENC:%"))
            .to_owned();

        let remaining = count_plaintext_residue(&db, &stmt, "plugin_configs").await;
        assert_eq!(
            remaining, 3,
            "only the still-plaintext rows should be counted, not the four already upgraded"
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

        let plugin_config_id = Uuid::now_v7();
        insert_plaintext_plugin_config(
            &db,
            plugin_config_id,
            r#"{"token":"all_plugin_config"}"#,
            now,
        )
        .await;

        let plugin_type_setting_id = Uuid::now_v7();
        insert_plaintext_plugin_type_setting(
            &db,
            plugin_type_setting_id,
            r#"{"default_timeout":5}"#,
            now,
        )
        .await;

        insert_plaintext_instance_plugin_setting(
            &db,
            "v3_all_instance_plugin",
            r#"{"api_key":"all_instance"}"#,
            now,
        )
        .await;

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

        let plugin_config =
            uptrakit_shared_db::entity::prelude::PluginConfig::find_by_id(plugin_config_id)
                .one(&db)
                .await
                .expect("query")
                .expect("row");
        assert!(
            plugin_config.config.is_db_value_encrypted(),
            "plugin_configs.config must be encrypted"
        );

        let plugin_type_setting =
            uptrakit_shared_db::entity::prelude::PluginTypeSetting::find_by_id(
                plugin_type_setting_id,
            )
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            plugin_type_setting.config.is_db_value_encrypted(),
            "plugin_type_settings.config must be encrypted"
        );

        let instance_plugin_setting =
            uptrakit_shared_db::entity::prelude::InstancePluginSetting::find_by_id(
                "v3_all_instance_plugin".to_string(),
            )
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            instance_plugin_setting.config.is_db_value_encrypted(),
            "instance_plugin_setting.config must be encrypted"
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
