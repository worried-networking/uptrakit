//! Phase 4: Master key verification and data key ring initialization.

use rootcause::prelude::*;
use uptrakit_web_api::SettingKey;

use crate::AppError;

/// Verify the master key matches existing encrypted data, or store a new
/// verification token if this is the first run.
pub(crate) async fn verify_master_key(db: &sea_orm::DatabaseConnection) -> crate::Result<()> {
    if !uptrakit_crypto::master_key_available() {
        return Ok(());
    }

    let stored_token = uptrakit_web_api::settings_store::load_global_setting(
        db,
        SettingKey::MasterKeyVerification,
    )
    .await
    .context(AppError::Settings)?;

    match stored_token {
        Some(value) => {
            if let Some(token_str) = value.as_str()
                && uptrakit_crypto::is_encrypted(token_str)
            {
                #[expect(
                    clippy::map_err_ignore,
                    reason = "the underlying crypto error reveals only internal AAD/key details; the user-facing `master key mismatch` message provides actionable guidance"
                )]
                let verified =
                    uptrakit_crypto::verify_key_verification_token(token_str).map_err(|_| {
                        report!(AppError::Config(
                            "master key mismatch: the current UPTRAKIT_MASTER_KEY cannot \
                                 decrypt data encrypted by a previous instance. Ensure all \
                                 controller instances use the same master key."
                                .into()
                        ))
                    });
                verified?;
                tracing::info!("master key verification succeeded");
            }
        }
        None => {
            let token = uptrakit_crypto::create_key_verification_token().context_to()?;
            let inserted = uptrakit_web_api::settings_store::insert_global_setting_if_absent(
                db,
                SettingKey::MasterKeyVerification,
                serde_json::json!(token),
            )
            .await
            .context(AppError::Settings)?;

            if inserted {
                tracing::info!("master key verification token stored");
            } else {
                // Another instance raced and stored a token first — verify against it.
                let raced_value = uptrakit_web_api::settings_store::load_global_setting(
                    db,
                    SettingKey::MasterKeyVerification,
                )
                .await
                .context(AppError::Settings)?;
                if let Some(value) = raced_value
                    && let Some(token_str) = value.as_str()
                    && uptrakit_crypto::is_encrypted(token_str)
                {
                    #[expect(
                        clippy::map_err_ignore,
                        reason = "the underlying crypto error reveals only internal AAD/key details; the user-facing `master key mismatch` message provides actionable guidance"
                    )]
                    let verified = uptrakit_crypto::verify_key_verification_token(token_str)
                        .map_err(|_| {
                            report!(AppError::Config(
                                "master key mismatch: another controller instance stored a \
                                     verification token first, and the current UPTRAKIT_MASTER_KEY \
                                     cannot decrypt it. Ensure all controller instances use the \
                                     same master key."
                                    .into()
                            ))
                        });
                    verified?;
                    tracing::info!(
                        "master key verification succeeded (raced with another instance)"
                    );
                }
            }
        }
    }
    Ok(())
}

/// Initialize the global [`DataKeyRing`] for envelope encryption.
///
/// Loads existing DEKs from the `data_encryption_keys` table, or generates the
/// first DEK if the table is empty.  The ring enables `ENC:v3:` format which
/// encrypts data with a DEK rather than the KEK directly.
///
/// ## HA safety
///
/// On first start two controllers may race to insert the initial DEK.  The
/// `key_id` column has a UNIQUE constraint, so the loser's insert fails
/// with a DB error that is caught gracefully.  It then re-reads all rows
/// and uses whatever was committed.
pub(crate) async fn init_data_key_ring(db: &sea_orm::DatabaseConnection) -> crate::Result<()> {
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use uptrakit_shared_db::entity::data_encryption_key;

    if !uptrakit_crypto::master_key_available() {
        return Ok(());
    }

    let kek_fp = uptrakit_crypto::master_key_fingerprint().context_to()?;

    let rows = data_encryption_key::Entity::find()
        .all(db)
        .await
        .context(AppError::Database)?;

    if rows.is_empty() {
        // First start — generate the initial DEK.
        let dek = uptrakit_crypto::generate_data_key().context_to()?;
        let wrapped = uptrakit_crypto::wrap_data_key(&dek).context_to()?;

        let am = data_encryption_key::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            key_id: Set(dek.key_id.clone()),
            wrapped_key: Set(wrapped),
            kek_fingerprint: Set(kek_fp.clone()),
            status: Set("active".to_string()),
            created_at: Set(time::OffsetDateTime::now_utc()),
            retired_at: Set(None),
        };

        match am.insert(db).await {
            Ok(_) => {
                tracing::info!(key_id = %dek.key_id, "generated initial data encryption key");
            }
            Err(e) => {
                // HA race: another controller inserted first.  This is harmless —
                // fall through to load all rows below.
                tracing::debug!(
                    error = %e,
                    "initial DEK insert failed (likely HA race), will load existing keys"
                );
            }
        }

        // Re-read in case of HA race.
        let rows = data_encryption_key::Entity::find()
            .all(db)
            .await
            .context(AppError::Database)?;
        return build_and_init_ring(&rows, &kek_fp);
    }

    build_and_init_ring(&rows, &kek_fp)?;
    Ok(())
}

/// Unwrap all DEKs and initialize the global data key ring.
fn build_and_init_ring(
    rows: &[uptrakit_shared_db::entity::data_encryption_key::Model],
    kek_fp: &str,
) -> crate::Result<()> {
    use std::collections::HashMap;

    let mut keys = HashMap::new();
    let mut active_key_id: Option<String> = None;

    for row in rows {
        if row.kek_fingerprint != kek_fp {
            return Err(report!(AppError::Config(format!(
                "DEK '{}' was wrapped with KEK fingerprint '{}', but current KEK \
                 fingerprint is '{kek_fp}'. This indicates a master key mismatch. \
                 Use --rotate-master-key-file to rotate to a new master key.",
                row.key_id, row.kek_fingerprint,
            ))));
        }

        let dek = uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to unwrap DEK '{}': {e}",
                row.key_id
            )))
        })?;
        keys.insert(dek.key_id.clone(), dek.key);

        if row.status == "active" {
            active_key_id = Some(row.key_id.clone());
        }
    }

    let active = active_key_id.ok_or_else(|| {
        report!(AppError::Config(
            "no active DEK found in data_encryption_keys table".into()
        ))
    })?;

    let ring = uptrakit_crypto::DataKeyRing::new(keys, active.clone()).context_to()?;
    uptrakit_crypto::init_data_key_ring(ring).context_to()?;
    tracing::info!(
        active_key_id = %active,
        count = rows.len(),
        "data key ring initialized"
    );
    Ok(())
}
