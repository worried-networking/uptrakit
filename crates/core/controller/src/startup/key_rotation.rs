//! Master key rotation (KEK re-wrapping).

use rootcause::prelude::*;
use uptrakit_web_api::SettingKey;

use crate::AppError;

/// Rotate the master key (KEK) by re-wrapping all DEKs with a new KEK.
///
/// This is an O(1) operation regardless of data volume: only the DEK
/// wrappers in `data_encryption_keys` are updated, not the encrypted data
/// itself (since data is encrypted with DEKs, not the KEK directly).
///
/// After rotation, the operator must restart all controllers with
/// `--master-key-file` pointing to the new key file.
pub(crate) async fn rotate_master_key(
    db: &sea_orm::DatabaseConnection,
    new_key_path: &std::path::Path,
) -> crate::Result<()> {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, TransactionTrait};
    use uptrakit_shared_db::entity::data_encryption_key;

    // Read and parse the new key.
    let new_key_hex = std::fs::read_to_string(new_key_path).map_err(|e| {
        report!(AppError::Config(format!(
            "failed to read --rotate-master-key-file {}: {e}",
            new_key_path.display()
        )))
    })?;
    let new_key_bytes = super::master_key::parse_master_key_hex(new_key_hex.trim())?;
    let new_kek = zeroize::Zeroizing::new(new_key_bytes);

    // Compute the new KEK fingerprint.
    let new_kek_fp = {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(new_kek.as_slice());
        uptrakit_shared_types::hex::encode(&hash[..8])
    };

    let current_kek_fp = uptrakit_crypto::master_key_fingerprint().context_to()?;
    if new_kek_fp == current_kek_fp {
        tracing::warn!(
            "new master key has the same fingerprint as the current one — no rotation needed"
        );
        return Ok(());
    }

    tracing::info!(current_kek_fp, new_kek_fp, "starting master key rotation");

    // Re-wrap all DEKs in a transaction.
    let txn = db.begin().await.context(AppError::Database)?;

    let rows = data_encryption_key::Entity::find()
        .all(&txn)
        .await
        .context(AppError::Database)?;

    if rows.is_empty() {
        tracing::warn!("no DEKs found in database — nothing to rotate");
        txn.commit().await.context(AppError::Database)?;
        return Ok(());
    }

    for row in &rows {
        // Unwrap with current KEK.
        let dek = uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to unwrap DEK '{}' with current KEK: {e}",
                row.key_id
            )))
        })?;

        // Re-wrap with new KEK.
        let new_wrapped = uptrakit_crypto::wrap_data_key_with(&new_kek, &dek).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to wrap DEK '{}' with new KEK: {e}",
                row.key_id
            )))
        })?;

        // Update the row.
        let mut am: data_encryption_key::ActiveModel = row.clone().into_active_model();
        am.wrapped_key = sea_orm::Set(new_wrapped);
        am.kek_fingerprint = sea_orm::Set(new_kek_fp.clone());
        am.update(&txn).await.context(AppError::Database)?;
    }

    // Update the master key verification token with the new KEK.
    let new_token = uptrakit_crypto::create_verification_token_with_key(&new_kek).context_to()?;
    uptrakit_web_api::settings_store::upsert_global_setting(
        &txn,
        SettingKey::MasterKeyVerification,
        serde_json::json!(new_token),
    )
    .await
    .context(AppError::Settings)?;

    txn.commit().await.context(AppError::Database)?;

    tracing::info!(
        dek_count = rows.len(),
        new_kek_fp,
        "master key rotation complete — restart all controllers with the new key file"
    );

    Ok(())
}
